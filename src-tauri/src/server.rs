use crate::config::{self, AppConfig};
use crate::extensions::builtins::gateway::{config as gateway_config, service as gateway_service};
use crate::gateway_usage;
use crate::platforms::macos;
use crate::remote::cdp_resources;
use crate::{launcher, ports, remote, AppState};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    CONNECTION, CONTENT_TYPE, HOST, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::process::Child;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

type HttpBody = Full<Bytes>;

const RETIRED_CDP_WEB_RESOURCE_MESSAGE: &str =
    "Codex web assets are served from the configured registry.";

#[derive(Debug)]
pub(crate) struct ManagedInstance {
    child: Child,
    info: LaunchInfo,
    bot_signature: String,
    stopped: bool,
}

impl ManagedInstance {
    fn stop(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        launcher::stop_codex(&mut self.child).map_err(|e| e.to_string())?;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for ManagedInstance {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LaunchRequest {
    #[serde(default)]
    pub cdp_port: Option<u16>,
    #[serde(default)]
    pub codex_path: Option<String>,
    #[serde(default)]
    pub codex_home: Option<String>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchInfo {
    pub running: bool,
    pub pid: Option<u32>,
    pub cdp_host: String,
    pub cdp_port: u16,
    pub http_host: String,
    pub http_port: u16,
    pub codex_path: String,
    pub codex_home: String,
    pub proxy_url: String,
    pub profile_name: String,
    pub cli_stdio_path: Option<String>,
    pub core_mode: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub cdp_host: String,
    pub cdp_port: u16,
    pub http_host: String,
    pub http_port: u16,
    pub codex_path: String,
    pub codex_home: String,
    pub proxy_url: String,
    pub profile_name: String,
    pub cli_stdio_path: Option<String>,
    pub core_mode: String,
    pub remote_control: Option<remote::RemoteControlInfo>,
}

#[derive(Debug, Serialize)]
struct RemoteLinks {
    base_url: String,
    json_list_url: String,
    json_version_url: String,
    devtools_proxy_path: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    running: bool,
    pid: Option<u32>,
    cli_stdio_path: Option<String>,
    config: AppConfig,
    remote: RemoteLinks,
}

pub async fn serve(state: AppState) -> Result<(), String> {
    let config = state.config.lock().await.clone();
    let (listener, actual_port) = bind_http_listener(&config.http_host, config.http_port).await?;
    if actual_port != config.http_port {
        eprintln!(
            "CodexL HTTP port {} is unavailable; using {} instead",
            config.http_port, actual_port
        );
        let mut runtime_config = state.config.lock().await;
        if runtime_config.http_host == config.http_host
            && runtime_config.http_port == config.http_port
        {
            runtime_config.http_port = actual_port;
        }
    }
    let local_addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| format!("{}:{}", config.http_host, actual_port));

    eprintln!("CodexL HTTP server listening on http://{}", local_addr);

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("failed to accept HTTP connection: {}", e))?;
        let io = TokioIo::new(stream);
        let request_state = state.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| handle_request(req, request_state.clone()));
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                eprintln!("HTTP connection failed: {}", err);
            }
        });
    }
}

async fn bind_http_listener(host: &str, preferred_port: u16) -> Result<(TcpListener, u16), String> {
    if preferred_port == 0 {
        let listener = TcpListener::bind((host, 0))
            .await
            .map_err(|e| format!("failed to bind HTTP server: {}", e))?;
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        return Ok((listener, port));
    }

    let mut first_error = None;
    for offset in 0..100u16 {
        let Some(port) = preferred_port.checked_add(offset) else {
            break;
        };
        match TcpListener::bind((host, port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(err) => {
                if offset == 0 {
                    first_error = Some(err);
                }
            }
        }
    }

    Err(format!(
        "failed to bind HTTP server: {}",
        first_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "no available port found".to_string())
    ))
}

pub async fn launch_codex_instance(
    state: &AppState,
    request: LaunchRequest,
) -> Result<LaunchInfo, String> {
    let base_config = state.config.lock().await.clone();
    let mut requested_config = base_config.clone();
    apply_launch_request(&mut requested_config, &request)?;
    let profile_name = requested_config.active_provider.clone();

    let mut permission_config = requested_config.clone();
    let executable = resolve_codex_executable(&mut permission_config)?;
    requested_config.codex_path = executable.clone();
    let cli_executable = launcher::resolve_codex_cli_executable(None, &executable);
    let profile_config_format = config::codex_profile_config_format_for_cli(&cli_executable);
    if request.codex_home.is_none() {
        if let Some(profile) = requested_config.provider_profile(&requested_config.active_provider)
        {
            requested_config.codex_home =
                config::ensure_provider_codex_home_with_format(&profile, profile_config_format)?;
            requested_config.normalize();
        }
    }
    macos::request_automation_permission(&executable)?;

    let requested_remote_frontend_mode = requested_config
        .provider_profile(&requested_config.active_provider)
        .map(|profile| config::normalized_remote_frontend_mode(&profile.remote_frontend_mode))
        .unwrap_or_else(|| config::REMOTE_FRONTEND_MODE_APP.to_string());
    if should_reuse_external_cdp_endpoint(&base_config, &request, &profile_name)
        && !config::remote_frontend_mode_uses_cli(&requested_remote_frontend_mode)
    {
        let external_info = launch_info(&requested_config, None, None);
        if cdp_endpoint_has_codex_app_target(&external_info).await {
            let mut info = external_info;
            info.running = true;
            eprintln!(
                "Reusing existing Codex CDP endpoint for profile {} at http://{}:{}",
                info.profile_name, info.cdp_host, info.cdp_port
            );
            cdp_resources::spawn_codex_plugin_injector(
                info.cdp_host.clone(),
                info.cdp_port,
                info.http_host.clone(),
                info.http_port,
            );
            return Ok(info);
        }
    }

    let mut instances = state.instances.lock().await;
    if let Some(instance) = instances.get_mut(&profile_name) {
        if let Some(pid) = running_child_pid(&mut instance.child)? {
            if !requires_new_process(&instance.info, &requested_config, &instance.bot_signature) {
                let info = launch_info_from_instance(&instance.info, Some(pid));
                cdp_resources::spawn_codex_plugin_injector(
                    info.cdp_host.clone(),
                    info.cdp_port,
                    info.http_host.clone(),
                    info.http_port,
                );
                return Ok(info);
            }
        }

        let mut removed = instances.remove(&profile_name).expect("instance exists");
        removed.stop()?;
        drop(instances);
        remote::stop_remote_control(state, &profile_name).await?;
        instances = state.instances.lock().await;
    }

    if let Err(err) = launcher::stop_stale_profile_processes(&profile_name) {
        eprintln!(
            "failed to stop stale Codex processes for profile {}: {}",
            profile_name, err
        );
    }

    let actual_port =
        ports::prepare_cdp_port(&requested_config.cdp_host, requested_config.cdp_port).await;
    requested_config.cdp_port = actual_port;

    let active_cli_profile = requested_config.active_cli_profile();
    let active_cli_model_provider = requested_config.active_cli_model_provider();
    let active_provider_profile =
        requested_config.provider_profile(&requested_config.active_provider);
    let active_core_mode = active_provider_profile
        .as_ref()
        .map(|profile| profile.remote_frontend_mode.as_str());
    if requested_config.extensions.enabled
        && requested_config.extensions.next_ai_gateway_enabled
        && active_provider_profile
            .as_ref()
            .map(|profile| profile.provider_name == gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME)
            .unwrap_or(false)
    {
        gateway_service::ensure_running(state).await?;
    }
    let active_bot_config =
        if requested_config.extensions.enabled && requested_config.extensions.bot_gateway_enabled {
            active_provider_profile.as_ref().map(|profile| &profile.bot)
        } else {
            None
        };
    if let Some(profile) = active_provider_profile.as_ref() {
        config::sync_provider_bot_media_mcp_config_for_launch(
            profile,
            &requested_config.codex_home,
            active_bot_config
                .map(|bot| bot.bridge_enabled())
                .unwrap_or(false),
        )?;
    }
    config::remove_retired_builtin_mcp_configs_for_launch(&requested_config.codex_home)?;
    let launch = launcher::launch_codex(
        &executable,
        actual_port,
        requested_config.active_codex_home(),
        Some(&requested_config.active_provider),
        active_cli_profile.as_deref(),
        active_cli_model_provider.as_deref(),
        active_core_mode,
        active_provider_profile
            .as_ref()
            .map(|profile| profile.proxy_url.as_str()),
        active_bot_config,
        Some(requested_config.language.as_str()),
    )
    .map_err(|e| format!("Failed to launch Codex: {}", e))?;
    let pid = launch.child.id();
    let info = launch_info(&requested_config, Some(pid), launch.cli_stdio_path);
    cdp_resources::spawn_codex_plugin_injector(
        info.cdp_host.clone(),
        info.cdp_port,
        info.http_host.clone(),
        info.http_port,
    );

    instances.insert(
        profile_name.clone(),
        ManagedInstance {
            child: launch.child,
            info: info.clone(),
            bot_signature: bot_runtime_signature(&requested_config),
            stopped: false,
        },
    );
    drop(instances);

    let mut config = state.config.lock().await;
    config.cdp_host = requested_config.cdp_host;
    config.cdp_port = requested_config.cdp_port;
    config.codex_path = requested_config.codex_path;
    config.codex_home = requested_config.codex_home;
    config.active_provider = profile_name;

    Ok(info)
}

fn resolve_codex_executable(config: &mut AppConfig) -> Result<String, String> {
    let configured = config.codex_path.trim();
    if !configured.is_empty() && std::path::Path::new(configured).is_file() {
        return Ok(configured.to_string());
    }

    let detected = launcher::find_codex_app().ok_or_else(|| "Codex app not found".to_string())?;
    config.codex_path = detected.clone();
    Ok(detected)
}

fn apply_launch_request(config: &mut AppConfig, request: &LaunchRequest) -> Result<(), String> {
    if let Some(cdp_port) = request.cdp_port {
        config.cdp_port = cdp_port;
    }
    if let Some(codex_path) = request.codex_path.as_ref() {
        config.codex_path = codex_path.clone();
    }
    if let Some(profile_name) = request.profile_name.as_ref() {
        let profile = config
            .provider_profile(profile_name)
            .ok_or_else(|| format!("Provider profile not found: {}", profile_name))?;
        config.active_provider = config::provider_profile_key(&profile);
    }
    if let Some(codex_home) = request.codex_home.as_ref() {
        config.codex_home = codex_home.clone();
    }
    config.normalize();
    Ok(())
}

fn should_reuse_external_cdp_endpoint(
    base_config: &AppConfig,
    request: &LaunchRequest,
    requested_profile_name: &str,
) -> bool {
    if requested_profile_name != base_config.active_provider {
        return false;
    }
    if let Some(profile_name) = request
        .profile_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if profile_name != base_config.active_provider {
            return false;
        }
    }
    request
        .codex_home
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
}

fn requires_new_process(
    current: &LaunchInfo,
    requested: &AppConfig,
    current_bot_signature: &str,
) -> bool {
    let requested_proxy_url = requested
        .provider_profile(&requested.active_provider)
        .map(|profile| profile.proxy_url)
        .unwrap_or_default();
    let requested_core_mode = requested
        .provider_profile(&requested.active_provider)
        .map(|profile| config::normalized_remote_frontend_mode(&profile.remote_frontend_mode))
        .unwrap_or_else(|| config::REMOTE_FRONTEND_MODE_APP.to_string());

    current.profile_name != requested.active_provider
        || current.codex_home != requested.codex_home
        || current.codex_path != requested.codex_path
        || current.proxy_url.trim() != requested_proxy_url.trim()
        || current.core_mode != requested_core_mode
        || current_bot_signature != bot_runtime_signature(requested)
}

fn bot_runtime_signature(config: &AppConfig) -> String {
    if !config.extensions.enabled || !config.extensions.bot_gateway_enabled {
        return "disabled".to_string();
    }
    let Some(profile) = config.provider_profile(&config.active_provider) else {
        return "missing-profile".to_string();
    };
    serde_json::to_string(&profile.bot).unwrap_or_default()
}

pub async fn stop_codex_instance(
    state: &AppState,
    profile_name: Option<String>,
) -> Result<(), String> {
    let cleanup_profiles = {
        let mut instances = state.instances.lock().await;
        let names = match profile_name {
            Some(name) => vec![name],
            None => instances.keys().cloned().collect(),
        };

        for name in &names {
            if let Some(mut instance) = instances.remove(name) {
                instance.stop()?;
            }
        }

        names
    };

    for name in &cleanup_profiles {
        remote::stop_remote_control(state, &name).await?;
    }

    for name in &cleanup_profiles {
        cleanup_profile_extension_processes(name);
    }

    stop_gateway_if_no_running_next_ai_instances(state).await?;

    Ok(())
}

pub async fn current_launch_info(state: &AppState) -> Result<LaunchInfo, String> {
    let active_provider = {
        let config = state.config.lock().await;
        config.active_provider.clone()
    };
    let infos = running_launch_infos(state).await?;
    if let Some(info) = infos
        .iter()
        .find(|info| info.profile_name == active_provider)
        .cloned()
    {
        return Ok(info);
    }
    if let Some(info) = infos.into_iter().next() {
        return Ok(info);
    }
    let config = state.config.lock().await.clone();
    let mut info = launch_info(&config, None, None);
    if !config::remote_frontend_mode_uses_cli(&info.core_mode) && cdp_endpoint_is_alive(&info).await
    {
        info.running = true;
    }
    Ok(info)
}

pub async fn instance_statuses(state: &AppState) -> Result<Vec<InstanceStatus>, String> {
    let infos = running_launch_infos(state).await?;
    let mut remote_controls = remote::remote_control_status_map(state).await;
    let config = state.config.lock().await.clone();
    let mut statuses = infos
        .into_iter()
        .map(|info| {
            let remote_control = remote_controls.remove(&info.profile_name);
            InstanceStatus {
                running: info.running,
                pid: info.pid,
                cdp_host: info.cdp_host,
                cdp_port: info.cdp_port,
                http_host: info.http_host,
                http_port: info.http_port,
                codex_path: info.codex_path,
                codex_home: info.codex_home,
                proxy_url: info.proxy_url,
                cli_stdio_path: info.cli_stdio_path,
                core_mode: info.core_mode,
                remote_control,
                profile_name: info.profile_name,
            }
        })
        .collect::<Vec<_>>();

    for (profile_name, remote_control) in remote_controls {
        let profile = config.provider_profile(&profile_name);
        let codex_home = profile
            .as_ref()
            .map(|profile| {
                config::generated_codex_home(profile)
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| config.codex_home.clone());
        let proxy_url = profile
            .as_ref()
            .map(|profile| profile.proxy_url.clone())
            .unwrap_or_default();
        statuses.push(InstanceStatus {
            running: false,
            pid: None,
            cdp_host: remote_control.cdp_host.clone(),
            cdp_port: remote_control.cdp_port,
            http_host: config.http_host.clone(),
            http_port: config.http_port,
            codex_path: config.codex_path.clone(),
            codex_home,
            proxy_url,
            cli_stdio_path: None,
            core_mode: profile
                .as_ref()
                .map(|profile| {
                    config::normalized_remote_frontend_mode(&profile.remote_frontend_mode)
                })
                .unwrap_or_else(|| config::REMOTE_FRONTEND_MODE_APP.to_string()),
            remote_control: Some(remote_control),
            profile_name,
        });
    }

    Ok(statuses)
}

async fn handle_request(
    mut request: Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, Infallible> {
    let response = route_request(&mut request, state)
        .await
        .unwrap_or_else(|err| {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": err }))
        });
    Ok(response)
}

async fn route_request(
    request: &mut Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    if request.method() == Method::OPTIONS {
        return Ok(empty_response(StatusCode::NO_CONTENT));
    }

    let path = request.uri().path().to_string();
    match (request.method(), path.as_str()) {
        (&Method::GET, "/") | (&Method::GET, "/status") => {
            let status = status_response(&state, request).await?;
            Ok(json_response(StatusCode::OK, json!(status)))
        }
        (&Method::GET, "/health") => Ok(json_response(StatusCode::OK, json!({ "ok": true }))),
        (&Method::POST, "/gateway/auth/introspect") => {
            receive_gateway_auth_introspection(request).await
        }
        (&Method::POST, "/gateway/usage") | (&Method::POST, "/gateway/usage/report") => {
            receive_gateway_usage_report(request).await
        }
        (&Method::POST, "/launch") => {
            let launch_request = read_launch_request(request).await?;
            let info = launch_codex_instance(&state, launch_request).await?;
            Ok(json_response(StatusCode::OK, json!(info)))
        }
        (&Method::POST, "/stop") => {
            let stop_request = read_launch_request(request).await.unwrap_or_default();
            stop_codex_instance(&state, stop_request.profile_name).await?;
            Ok(json_response(StatusCode::OK, json!({ "stopped": true })))
        }
        _ if path == "/web" => Ok(retired_cdp_web_resource_response()),
        (&Method::GET, "/web/_resource") if is_websocket_upgrade(request) => {
            Ok(retired_cdp_web_resource_response())
        }
        (&Method::GET, "/web/_bridge") if is_websocket_upgrade(request) => {
            proxy_codex_web_bridge_websocket(request, state).await
        }
        (&Method::GET, "/plugin/_bridge") if is_websocket_upgrade(request) => {
            proxy_codex_plugin_bridge_websocket(request).await
        }
        (&Method::GET, "/plugin/_bridge") => probe_codex_plugin_bridge(request).await,
        (&Method::POST, "/web/_bridge") => proxy_codex_web_bridge(request, state).await,
        _ if path.starts_with("/web/") => Ok(retired_cdp_web_resource_response()),
        _ if path.starts_with("/json") => proxy_cdp_http(request, state).await,
        _ if path.starts_with("/devtools/") && is_websocket_upgrade(request) => {
            proxy_cdp_websocket(request, state).await
        }
        _ if path.starts_with("/devtools/") => proxy_cdp_http(request, state).await,
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "not found" }),
        )),
    }
}

async fn receive_gateway_auth_introspection(
    request: &mut Request<Incoming>,
) -> Result<Response<HttpBody>, String> {
    let expected = gateway_config::codex_provider_api_key()?;
    let credential = request_header(request, gateway_config::GATEWAY_AUTH_CREDENTIAL_HEADER)
        .unwrap_or_default()
        .trim();
    if !constant_time_eq(credential, &expected) {
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "invalid gateway auth credential" }),
        ));
    }

    let body = request
        .body_mut()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    if body.len() > 64 * 1024 {
        return Ok(json_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({ "error": "gateway auth payload is too large" }),
        ));
    }

    let payload = serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?;
    let token = payload
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let active = constant_time_eq(token, &expected);
    let response = if active {
        json!({
            "active": true,
            "userId": gateway_config::GATEWAY_AUTH_USER_ID,
            "tenantId": gateway_config::GATEWAY_AUTH_TENANT_ID,
            "sub": gateway_config::GATEWAY_AUTH_SUBJECT,
            "organizationId": "codexl",
            "plan": "local",
            "apiKeyId": gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME,
        })
    } else {
        json!({ "active": false })
    };

    Ok(json_response(StatusCode::OK, response))
}

async fn receive_gateway_usage_report(
    request: &mut Request<Incoming>,
) -> Result<Response<HttpBody>, String> {
    let body = request
        .body_mut()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    if body.len() > 5 * 1024 * 1024 {
        return Ok(json_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({ "error": "gateway usage payload is too large" }),
        ));
    }

    let payload = serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?;
    let result = gateway_usage::record_usage_report(payload).await?;
    Ok(json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "inserted": result.inserted,
            "eventId": result.event_id,
        }),
    ))
}

async fn probe_codex_plugin_bridge(
    request: &Request<Incoming>,
) -> Result<Response<HttpBody>, String> {
    let token_present = request
        .uri()
        .query()
        .map(|query| query.contains("token="))
        .unwrap_or(false);
    let token_valid = cdp_resources::plugin_bridge_token_valid(request.uri().query());
    let status = if token_valid {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    };
    Ok(json_response(
        status,
        json!({
            "ok": token_valid,
            "tokenPresent": token_present,
            "tokenValid": token_valid,
            "error": if token_valid { Value::Null } else { Value::String("invalid plugin bridge token".to_string()) },
        }),
    ))
}

async fn proxy_codex_plugin_bridge_websocket(
    request: &mut Request<Incoming>,
) -> Result<Response<HttpBody>, String> {
    if !cdp_resources::plugin_bridge_token_valid(request.uri().query()) {
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "invalid plugin bridge token" }),
        ));
    }
    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let token = request.uri().query().and_then(plugin_bridge_query_token);
    let on_upgrade = hyper::upgrade::on(request);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let websocket =
                    tokio_tungstenite::WebSocketStream::from_raw_socket(io, Role::Server, None)
                        .await;
                if let Err(err) =
                    cdp_resources::handle_plugin_bridge_websocket(websocket, token).await
                {
                    eprintln!("Codex plugin bridge WebSocket failed: {}", err);
                }
            }
            Err(err) => eprintln!("Codex plugin bridge WebSocket upgrade failed: {}", err),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .map(add_cors)
        .map_err(|e| e.to_string())
}

fn plugin_bridge_query_token(query: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key == "token" && !value.is_empty() {
            Some(value.to_string())
        } else {
            None
        }
    })
}

fn retired_cdp_web_resource_response() -> Response<HttpBody> {
    add_cors(json_response(
        StatusCode::GONE,
        json!({
            "error": RETIRED_CDP_WEB_RESOURCE_MESSAGE,
            "webAssetMode": "registry",
        }),
    ))
}

async fn proxy_codex_web_bridge(
    request: &mut Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    let info = current_launch_info(&state).await?;
    let body = request
        .body_mut()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    let message = serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?;
    let response = dispatch_codex_web_bridge_message(&state, &info, message).await?;
    Ok(json_response(StatusCode::OK, response))
}

async fn dispatch_codex_web_bridge_message(
    state: &AppState,
    info: &LaunchInfo,
    message: Value,
) -> Result<Value, String> {
    let config = state.config.lock().await.clone();
    if let Some(response) =
        remote::custom_transcribe_fetch_response_for_config(&message, &config, "local-app").await
    {
        return Ok(json!({ "messages": [response] }));
    }
    cdp_resources::dispatch_web_bridge_message(&info.cdp_host, info.cdp_port, message).await
}

async fn dispatch_codex_web_bridge_socket_payload_with_emitter<F>(
    state: &AppState,
    cdp_host: &str,
    cdp_port: u16,
    raw: &str,
    emit: F,
) -> Value
where
    F: Fn(Value) + Send + Sync,
{
    let (id, message) = cdp_resources::parse_web_bridge_socket_message(raw);
    if let Ok(message) = &message {
        let config = state.config.lock().await.clone();
        if let Some(response) =
            remote::custom_transcribe_fetch_response_for_config(message, &config, "local-app").await
        {
            return cdp_resources::web_bridge_socket_response(
                id,
                Ok(json!({ "messages": [response] })),
            );
        }
    }

    cdp_resources::dispatch_web_bridge_socket_payload_with_emitter(cdp_host, cdp_port, raw, emit)
        .await
}

fn handle_codex_web_bridge_socket_text(
    state: AppState,
    tx: &tokio::sync::mpsc::UnboundedSender<Message>,
    cdp_host: String,
    cdp_port: u16,
    raw: String,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let partial_tx = tx.clone();
        let response = dispatch_codex_web_bridge_socket_payload_with_emitter(
            &state,
            &cdp_host,
            cdp_port,
            &raw,
            move |partial| {
                let _ = partial_tx.send(Message::Text(partial.to_string()));
            },
        )
        .await;
        let _ = tx.send(Message::Text(response.to_string()));
    });
}

async fn handle_codex_web_bridge_websocket(
    websocket: tokio_tungstenite::WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    state: AppState,
    cdp_host: String,
    cdp_port: u16,
) -> Result<(), String> {
    eprintln!(
        "[codex-web] local bridge websocket opened: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    let (mut write, mut read) = websocket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let pump_tx = tx.clone();
    cdp_resources::spawn_web_bridge_notification_pump(cdp_host.clone(), cdp_port, move |partial| {
        let _ = pump_tx.send(Message::Text(partial.to_string()));
    });

    let writer = async {
        while let Some(message) = rx.recv().await {
            write.send(message).await.map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };

    let reader = async {
        while let Some(message) = read.next().await {
            match message.map_err(|e| e.to_string())? {
                Message::Text(raw) => {
                    handle_codex_web_bridge_socket_text(
                        state.clone(),
                        &tx,
                        cdp_host.clone(),
                        cdp_port,
                        raw,
                    );
                }
                Message::Binary(bytes) => match String::from_utf8(bytes) {
                    Ok(raw) => {
                        handle_codex_web_bridge_socket_text(
                            state.clone(),
                            &tx,
                            cdp_host.clone(),
                            cdp_port,
                            raw,
                        );
                    }
                    Err(err) => {
                        let response =
                            cdp_resources::web_bridge_socket_response(None, Err(err.to_string()));
                        let _ = tx.send(Message::Text(response.to_string()));
                    }
                },
                Message::Ping(payload) => {
                    let _ = tx.send(Message::Pong(payload));
                }
                Message::Close(frame) => {
                    let _ = tx.send(Message::Close(frame));
                    break;
                }
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    let result = tokio::select! {
        result = writer => result,
        result = reader => result,
    };
    eprintln!(
        "[codex-web] local bridge websocket closed: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    result
}

async fn proxy_codex_web_bridge_websocket(
    request: &mut Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let info = current_launch_info(&state).await?;
    let cdp_host = info.cdp_host.clone();
    let cdp_port = info.cdp_port;
    let on_upgrade = hyper::upgrade::on(request);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let websocket =
                    tokio_tungstenite::WebSocketStream::from_raw_socket(io, Role::Server, None)
                        .await;
                if let Err(err) =
                    handle_codex_web_bridge_websocket(websocket, state, cdp_host, cdp_port).await
                {
                    eprintln!("Codex web bridge WebSocket failed: {}", err);
                }
            }
            Err(err) => eprintln!("Codex web bridge WebSocket upgrade failed: {}", err),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .map(add_cors)
        .map_err(|e| e.to_string())
}

async fn read_launch_request(request: &mut Request<Incoming>) -> Result<LaunchRequest, String> {
    let body = request
        .body_mut()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();

    if body.is_empty() {
        return Ok(LaunchRequest::default());
    }

    serde_json::from_slice::<LaunchRequest>(&body).map_err(|e| e.to_string())
}

async fn status_response(
    state: &AppState,
    request: &Request<Incoming>,
) -> Result<StatusResponse, String> {
    let info = current_launch_info(state).await?;
    let config = state.config.lock().await.clone();
    let external_host = external_host(request, &config);
    let http_scheme = forwarded_proto(request).unwrap_or("http");
    let base_url = format!("{}://{}", http_scheme, external_host);

    Ok(StatusResponse {
        running: info.running,
        pid: info.pid,
        cli_stdio_path: info.cli_stdio_path,
        config,
        remote: RemoteLinks {
            json_list_url: format!("{}/json/list", base_url),
            json_version_url: format!("{}/json/version", base_url),
            devtools_proxy_path: "/devtools/*".to_string(),
            base_url,
        },
    })
}

async fn proxy_cdp_http(
    request: &mut Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    let info = current_launch_info(&state).await?;
    if !info.running {
        return Ok(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Codex workspace is not running" }),
        ));
    }
    let config = state.config.lock().await.clone();
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/")
        .to_string();
    let target_url = format!(
        "http://{}:{}{}",
        info.cdp_host, info.cdp_port, path_and_query
    );
    let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes())
        .map_err(|e| e.to_string())?;
    let body = request
        .body_mut()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();

    let client = reqwest::Client::new();
    let mut outbound = client.request(method, target_url);
    if !body.is_empty() {
        outbound = outbound.body(body);
    }

    let upstream = outbound.send().await.map_err(|e| e.to_string())?;
    let status = StatusCode::from_u16(upstream.status().as_u16()).map_err(|e| e.to_string())?;
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = upstream.bytes().await.map_err(|e| e.to_string())?;

    if is_json_content(content_type.as_deref(), &path_and_query) {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
            let external_host = external_host(request, &config);
            let ws_scheme = websocket_scheme(request);
            rewrite_debugger_urls(&mut value, &external_host, ws_scheme);
            return Ok(json_response(status, value));
        }
    }

    let mut response = Response::builder().status(status);
    if let Some(content_type) = content_type {
        response = response.header(CONTENT_TYPE, content_type);
    }
    response
        .body(Full::new(bytes))
        .map(add_cors)
        .map_err(|e| e.to_string())
}

async fn proxy_cdp_websocket(
    request: &mut Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let info = current_launch_info(&state).await?;
    if !info.running {
        return Ok(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Codex workspace is not running" }),
        ));
    }
    let target_url = format!("ws://{}:{}{}", info.cdp_host, info.cdp_port, path_and_query);
    let on_upgrade = hyper::upgrade::on(request);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                if let Err(err) = bridge_websocket(upgraded, target_url).await {
                    eprintln!("WebSocket proxy failed: {}", err);
                }
            }
            Err(err) => eprintln!("WebSocket upgrade failed: {}", err),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .map(add_cors)
        .map_err(|e| e.to_string())
}

async fn bridge_websocket(
    upgraded: hyper::upgrade::Upgraded,
    target_url: String,
) -> Result<(), String> {
    let client_io = TokioIo::new(upgraded);
    let client_ws =
        tokio_tungstenite::WebSocketStream::from_raw_socket(client_io, Role::Server, None).await;
    let (target_ws, _) = tokio_tungstenite::connect_async(&target_url)
        .await
        .map_err(|e| e.to_string())?;

    let (mut client_write, mut client_read) = client_ws.split();
    let (mut target_write, mut target_read) = target_ws.split();

    let client_to_target = async {
        while let Some(message) = client_read.next().await {
            let message = message.map_err(|e| e.to_string())?;
            target_write
                .send(message)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };

    let target_to_client = async {
        while let Some(message) = target_read.next().await {
            let message = message.map_err(|e| e.to_string())?;
            client_write
                .send(message)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };

    tokio::select! {
        result = client_to_target => result,
        result = target_to_client => result,
    }
}

async fn running_launch_infos(state: &AppState) -> Result<Vec<LaunchInfo>, String> {
    let mut stopped_profiles = Vec::new();
    let mut infos = {
        let mut instances = state.instances.lock().await;
        let mut infos = Vec::new();

        for (profile_name, instance) in instances.iter_mut() {
            match running_child_pid(&mut instance.child)? {
                Some(pid) => infos.push(launch_info_from_instance(&instance.info, Some(pid))),
                None if cdp_endpoint_is_alive(&instance.info).await => {
                    infos.push(launch_info_from_instance(&instance.info, instance.info.pid))
                }
                None => stopped_profiles.push(profile_name.clone()),
            }
        }

        for profile_name in &stopped_profiles {
            instances.remove(profile_name);
        }

        infos
    };

    for profile_name in &stopped_profiles {
        remote::stop_remote_control(state, &profile_name).await?;
        cleanup_profile_extension_processes(profile_name);
    }

    if !stopped_profiles.is_empty() {
        stop_gateway_if_no_running_next_ai_instances(state).await?;
    }

    if infos.is_empty() {
        let config = state.config.lock().await.clone();
        let mut info = launch_info(&config, None, None);
        if !config::remote_frontend_mode_uses_cli(&info.core_mode)
            && cdp_endpoint_is_alive(&info).await
        {
            info.running = true;
            infos.push(info);
        }
    }

    Ok(infos)
}

async fn stop_gateway_if_no_running_next_ai_instances(state: &AppState) -> Result<(), String> {
    let running_profiles: Vec<String> = {
        let instances = state.instances.lock().await;
        instances.keys().cloned().collect()
    };
    let has_running_next_ai_gateway_instance = {
        let config = state.config.lock().await;
        running_profiles.iter().any(|profile_name| {
            config
                .provider_profile(profile_name)
                .map(|profile| {
                    profile.provider_name == gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME
                })
                .unwrap_or(false)
        })
    };

    if !has_running_next_ai_gateway_instance {
        gateway_service::stop(state).await?;
    }

    Ok(())
}

fn cleanup_profile_extension_processes(profile_name: &str) {
    if let Err(err) = launcher::stop_profile_extension_processes(profile_name) {
        eprintln!(
            "failed to stop extension processes for profile {}: {}",
            profile_name, err
        );
    }
}

fn running_child_pid(child: &mut Child) -> Result<Option<u32>, String> {
    match child.try_wait() {
        Ok(None) => Ok(Some(child.id())),
        Ok(Some(_status)) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

async fn cdp_endpoint_is_alive(info: &LaunchInfo) -> bool {
    let url = format!("http://{}:{}/json/list", info.cdp_host, info.cdp_port);
    let Ok(result) =
        tokio::time::timeout(std::time::Duration::from_millis(500), reqwest::get(url)).await
    else {
        return false;
    };
    let Ok(response) = result else {
        return false;
    };
    response.status().is_success()
}

async fn cdp_endpoint_has_codex_app_target(info: &LaunchInfo) -> bool {
    let url = format!("http://{}:{}/json/list", info.cdp_host, info.cdp_port);
    let Ok(result) =
        tokio::time::timeout(std::time::Duration::from_millis(500), reqwest::get(url)).await
    else {
        return false;
    };
    let Ok(response) = result else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(targets) = response.json::<serde_json::Value>().await else {
        return false;
    };
    cdp_target_list_has_codex_app_target(&targets)
}

fn cdp_target_list_has_codex_app_target(targets: &serde_json::Value) -> bool {
    targets.as_array().into_iter().flatten().any(|target| {
        target.get("type").and_then(serde_json::Value::as_str) == Some("page")
            && target
                .get("url")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|url| url == "app://-/index.html")
    })
}

fn launch_info_from_instance(info: &LaunchInfo, pid: Option<u32>) -> LaunchInfo {
    LaunchInfo {
        running: pid.is_some(),
        pid,
        cdp_host: info.cdp_host.clone(),
        cdp_port: info.cdp_port,
        http_host: info.http_host.clone(),
        http_port: info.http_port,
        codex_path: info.codex_path.clone(),
        codex_home: info.codex_home.clone(),
        proxy_url: info.proxy_url.clone(),
        profile_name: info.profile_name.clone(),
        cli_stdio_path: info.cli_stdio_path.clone(),
        core_mode: info.core_mode.clone(),
    }
}

fn launch_info(config: &AppConfig, pid: Option<u32>, cli_stdio_path: Option<String>) -> LaunchInfo {
    LaunchInfo {
        running: pid.is_some(),
        pid,
        cdp_host: config.cdp_host.clone(),
        cdp_port: config.cdp_port,
        http_host: config.http_host.clone(),
        http_port: config.http_port,
        codex_path: config.codex_path.clone(),
        codex_home: config.codex_home.clone(),
        proxy_url: config
            .provider_profile(&config.active_provider)
            .map(|profile| profile.proxy_url)
            .unwrap_or_default(),
        profile_name: config.active_provider.clone(),
        cli_stdio_path,
        core_mode: config
            .provider_profile(&config.active_provider)
            .map(|profile| config::normalized_remote_frontend_mode(&profile.remote_frontend_mode))
            .unwrap_or_else(|| config::REMOTE_FRONTEND_MODE_APP.to_string()),
    }
}

fn request_header<'a>(request: &'a Request<Incoming>, name: &str) -> Option<&'a str> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in left.as_bytes().iter().zip(right.as_bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}

fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
    let has_upgrade = request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection_upgrade = request
        .headers()
        .get(CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);

    has_upgrade && connection_upgrade
}

fn is_json_content(content_type: Option<&str>, path: &str) -> bool {
    path.starts_with("/json")
        || content_type
            .map(|value| value.contains("application/json"))
            .unwrap_or(false)
}

fn rewrite_debugger_urls(value: &mut Value, external_host: &str, ws_scheme: &str) {
    match value {
        Value::Array(items) => {
            for item in items {
                rewrite_debugger_urls(item, external_host, ws_scheme);
            }
        }
        Value::Object(map) => {
            if let Some(Value::String(url)) = map.get_mut("webSocketDebuggerUrl") {
                *url = rewrite_ws_url(url, external_host, ws_scheme);
            }
            if let Some(Value::String(url)) = map.get_mut("devtoolsFrontendUrl") {
                *url = rewrite_devtools_frontend_url(url, external_host);
            }
            for value in map.values_mut() {
                rewrite_debugger_urls(value, external_host, ws_scheme);
            }
        }
        _ => {}
    }
}

fn rewrite_ws_url(url: &str, external_host: &str, ws_scheme: &str) -> String {
    if let Some(path_index) = url.find("/devtools/") {
        format!("{}://{}{}", ws_scheme, external_host, &url[path_index..])
    } else {
        url.to_string()
    }
}

fn rewrite_devtools_frontend_url(url: &str, external_host: &str) -> String {
    if let Some(ws_index) = url.find("ws=") {
        let (prefix, ws_target) = url.split_at(ws_index + 3);
        if let Some(path_index) = ws_target.find("/devtools/") {
            return format!("{}{}{}", prefix, external_host, &ws_target[path_index..]);
        }
    }
    url.to_string()
}

fn websocket_scheme(request: &Request<Incoming>) -> &'static str {
    match forwarded_proto(request) {
        Some("https") | Some("wss") => "wss",
        _ => "ws",
    }
}

fn forwarded_proto(request: &Request<Incoming>) -> Option<&str> {
    request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').next().unwrap_or(value).trim())
}

fn external_host(request: &Request<Incoming>, config: &AppConfig) -> String {
    if let Some(host) = request
        .headers()
        .get("x-forwarded-host")
        .or_else(|| request.headers().get(HOST))
        .and_then(|value| value.to_str().ok())
    {
        return host.to_string();
    }

    let host = if config.http_host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        config.http_host.as_str()
    };
    format!("{}:{}", host, config.http_port)
}

fn json_response(status: StatusCode, value: Value) -> Response<HttpBody> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
    add_cors(response)
}

fn empty_response(status: StatusCode) -> Response<HttpBody> {
    let response = Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
    add_cors(response)
}

fn add_cors(mut response: Response<HttpBody>) -> Response<HttpBody> {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        "GET,POST,OPTIONS".parse().unwrap(),
    );
    headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, "*".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener as StdTcpListener;

    #[tokio::test]
    async fn bind_http_listener_uses_next_port_when_preferred_is_busy() {
        let occupied = StdTcpListener::bind(("127.0.0.1", 0)).expect("bind occupied port");
        let preferred_port = occupied.local_addr().expect("occupied addr").port();

        let (listener, actual_port) = bind_http_listener("127.0.0.1", preferred_port)
            .await
            .expect("bind fallback listener");

        assert_ne!(actual_port, preferred_port);
        assert!(actual_port > preferred_port);
        drop(listener);
        drop(occupied);
    }

    #[test]
    fn external_cdp_reuse_is_limited_to_active_profile() {
        let mut config = AppConfig {
            active_provider: "default".to_string(),
            ..AppConfig::default()
        };
        config.provider_profiles = vec![
            config::ProviderProfile {
                id: "default".to_string(),
                name: "Default".to_string(),
                ..config::ProviderProfile::default()
            },
            config::ProviderProfile {
                id: "nextai".to_string(),
                name: "nextai".to_string(),
                ..config::ProviderProfile::default()
            },
        ];

        assert!(should_reuse_external_cdp_endpoint(
            &config,
            &LaunchRequest::default(),
            "default"
        ));
        assert!(!should_reuse_external_cdp_endpoint(
            &config,
            &LaunchRequest {
                profile_name: Some("nextai".to_string()),
                ..LaunchRequest::default()
            },
            "nextai"
        ));
        assert!(!should_reuse_external_cdp_endpoint(
            &config,
            &LaunchRequest {
                codex_home: Some("/tmp/other-codex-home".to_string()),
                ..LaunchRequest::default()
            },
            "default"
        ));
    }

    #[test]
    fn cdp_target_list_requires_codex_app_page() {
        assert!(cdp_target_list_has_codex_app_target(&json!([
            { "type": "page", "url": "app://-/index.html" }
        ])));
        assert!(!cdp_target_list_has_codex_app_target(&json!([
            { "type": "page", "url": "https://example.com" },
            { "type": "service_worker", "url": "app://-/index.html" }
        ])));
    }
}
