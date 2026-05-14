use crate::config::{self, AppConfig};
use crate::extensions::builtins::gateway::{config as gateway_config, service as gateway_service};
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
use tokio_tungstenite::tungstenite::protocol::Role;

type HttpBody = Full<Bytes>;

#[derive(Debug)]
pub(crate) struct ManagedInstance {
    child: Child,
    info: LaunchInfo,
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
    let listener = TcpListener::bind((config.http_host.as_str(), config.http_port))
        .await
        .map_err(|e| format!("failed to bind HTTP server: {}", e))?;
    let local_addr = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| format!("{}:{}", config.http_host, config.http_port));

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
    macos::request_automation_permission(&executable)?;

    let mut instances = state.instances.lock().await;
    if let Some(instance) = instances.get_mut(&profile_name) {
        if let Some(pid) = running_child_pid(&mut instance.child)? {
            if !requires_new_process(&instance.info, &requested_config) {
                return Ok(launch_info_from_instance(&instance.info, Some(pid)));
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
    let launch = launcher::launch_codex(
        &executable,
        actual_port,
        requested_config.active_codex_home(),
        Some(&requested_config.active_provider),
        active_cli_profile.as_deref(),
        active_cli_model_provider.as_deref(),
        active_provider_profile
            .as_ref()
            .map(|profile| profile.proxy_url.as_str()),
        active_bot_config,
        Some(requested_config.language.as_str()),
    )
    .map_err(|e| format!("Failed to launch Codex: {}", e))?;
    let pid = launch.child.id();
    let info = launch_info(&requested_config, Some(pid), launch.cli_stdio_path);

    instances.insert(
        profile_name.clone(),
        ManagedInstance {
            child: launch.child,
            info: info.clone(),
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
    if !config.codex_path.is_empty() {
        return Ok(config.codex_path.clone());
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
        config.codex_home = config::ensure_provider_codex_home(&profile)?;
        config.active_provider = profile.name;
    }
    if let Some(codex_home) = request.codex_home.as_ref() {
        config.codex_home = codex_home.clone();
    }
    config.normalize();
    Ok(())
}

fn requires_new_process(current: &LaunchInfo, requested: &AppConfig) -> bool {
    let requested_proxy_url = requested
        .provider_profile(&requested.active_provider)
        .map(|profile| profile.proxy_url)
        .unwrap_or_default();

    current.profile_name != requested.active_provider
        || current.codex_home != requested.codex_home
        || current.codex_path != requested.codex_path
        || current.proxy_url.trim() != requested_proxy_url.trim()
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
    Ok(launch_info(&config, None, None))
}

pub async fn instance_statuses(state: &AppState) -> Result<Vec<InstanceStatus>, String> {
    let infos = running_launch_infos(state).await?;
    let remote_controls = remote::remote_control_status_map(state).await;
    Ok(infos
        .into_iter()
        .map(|info| InstanceStatus {
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
            remote_control: remote_controls.get(&info.profile_name).cloned(),
            profile_name: info.profile_name,
        })
        .collect())
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
        _ if path == "/web" => {
            cdp_resources::web_root_redirect(request.uri().query()).map(add_cors)
        }
        (&Method::GET, "/web/_resource") if is_websocket_upgrade(request) => {
            proxy_codex_web_resource_websocket(request, state).await
        }
        (&Method::GET, "/web/_bridge") if is_websocket_upgrade(request) => {
            proxy_codex_web_bridge_websocket(request, state).await
        }
        (&Method::POST, "/web/_bridge") => proxy_codex_web_bridge(request, state).await,
        _ if path.starts_with("/web/") => proxy_codex_web_resource(request, state).await,
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

async fn proxy_codex_web_resource(
    request: &Request<Incoming>,
    state: AppState,
) -> Result<Response<HttpBody>, String> {
    let info = current_launch_info(&state).await?;
    cdp_resources::get_web_resource(
        &info.cdp_host,
        info.cdp_port,
        request.uri().path(),
        request.uri().query(),
    )
    .await?
    .into_response()
    .map(add_cors)
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
    let response =
        cdp_resources::dispatch_web_bridge_message(&info.cdp_host, info.cdp_port, message).await?;
    Ok(json_response(StatusCode::OK, response))
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
                    cdp_resources::handle_web_bridge_websocket(websocket, cdp_host, cdp_port, None)
                        .await
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

async fn proxy_codex_web_resource_websocket(
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
                if let Err(err) = cdp_resources::handle_web_resource_websocket(
                    websocket, cdp_host, cdp_port, None,
                )
                .await
                {
                    eprintln!("Codex web resource WebSocket failed: {}", err);
                }
            }
            Err(err) => eprintln!("Codex web resource WebSocket upgrade failed: {}", err),
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
    let infos = {
        let mut instances = state.instances.lock().await;
        let mut infos = Vec::new();

        for (profile_name, instance) in instances.iter_mut() {
            match running_child_pid(&mut instance.child)? {
                Some(pid) => infos.push(launch_info_from_instance(&instance.info, Some(pid))),
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
    }
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
