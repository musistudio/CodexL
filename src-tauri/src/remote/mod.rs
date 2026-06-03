use crate::{
    cli_middleware,
    config::{
        self, generated_codex_home, AppConfig, ProviderProfile, RemoteCloudAuthConfig,
        REMOTE_FRONTEND_MODE_CLAUDE_CODE,
    },
    launcher, ports, server, AppState,
};
use base64::{
    engine::general_purpose::{
        STANDARD as BASE64_STANDARD, URL_SAFE as BASE64_URL_SAFE,
        URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
    },
    Engine as _,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_TYPE, COOKIE, REFERER, SEC_WEBSOCKET_ACCEPT,
    SEC_WEBSOCKET_KEY, SET_COOKIE, UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::ffi::OsString;
use std::io::{BufRead, BufReader as StdBufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, ChildStdin, Command as TokioCommand};
use tokio::sync::{mpsc, oneshot, watch, Mutex, Semaphore};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

mod assets;
pub(crate) mod cdp_resources;
mod crypto;
mod input;
mod util;

use assets::{manifest_response, static_response};
use crypto::RemoteCrypto;
use input::*;
use util::*;

type HttpBody = Full<Bytes>;

const CONNECT_RETRY_MS: u64 = 1200;
const COMMAND_TIMEOUT_MS: u64 = 7000;
const HEARTBEAT_INTERVAL_MS: u64 = 5000;
const RELAY_RECONNECT_MAX_MS: u64 = 8000;
const RELAY_RECONNECT_MIN_MS: u64 = 1000;
const RELAY_WEB_BRIDGE_TASK_LIMIT: usize = 128;
const RELAY_WEB_BRIDGE_NOTIFICATION_PUMP_TTL_MS: u64 = 10 * 60_000;
const FRAME_META_INTERVAL_MS: u64 = 250;
const CLI_APP_SERVER_REQUEST_TIMEOUT_MS: u64 = 5 * 60_000;
const CLI_APP_SERVER_FETCH_TIMEOUT_MS: u64 = 30_000;
const CLI_READ_FILE_BINARY_MAX_BYTES: u64 = 20 * 1024 * 1024;
const CLI_APP_SERVER_RESUME_TURNS_LIMIT: u64 = 5;
const CLI_APP_SERVER_REQUEST_ID_PREFIX: &str = "codexl-remote-cli-";
const CLI_APP_SERVER_PROTOCOL_VERSION: &str = "2025-11-25";
const CODEX_DESKTOP_API_PROD_BASE_URL: &str = "https://chatgpt.com/backend-api";
const CODEX_DESKTOP_API_DEV_BASE_URL: &str = "http://localhost:8000/api";
const CODEX_DESKTOP_ORIGINATOR: &str = "Codex Desktop";
const DEFAULT_TRANSCRIBE_MODEL: &str = "gpt-4o-mini-transcribe";
const OPENAI_TRANSCRIBE_ENDPOINT_SUFFIX: &str = "/audio/transcriptions";
const CLI_ACTIVE_WORKSPACE_ROOTS_KEY: &str = "active-workspace-roots";
const CLI_PROJECTLESS_THREAD_IDS_KEY: &str = "projectless-thread-ids";
const CLI_THREAD_WORKSPACE_ROOT_HINTS_KEY: &str = "thread-workspace-root-hints";
const CLI_WORKSPACE_ROOT_OPTIONS_KEY: &str = "workspace-root-options";
const DEFAULT_SCREENSHOT_MAX_HEIGHT: u64 = 900;
const DEFAULT_SCREENSHOT_MAX_WIDTH: u64 = 1440;
const DEFAULT_PAGE_ZOOM_SCALE: f64 = 1.0;
const MIN_PAGE_ZOOM_SCALE: f64 = 1.0;
const MAX_PAGE_ZOOM_SCALE: f64 = 3.0;
const REMOTE_AUTH_COOKIE_NAME: &str = "codexl_remote_token";
const CLOUD_RELAY_DISCOVERY_URL: &str = "https://relay.codexl.io/";
const CLOUD_RELAY_DISCOVERY_TIMEOUT_MS: u64 = 8000;
const DEFAULT_REMOTE_WEB_ASSET_REGISTRY_URL: &str = "https://web.codexl.io";
const RETIRED_CDP_WEB_RESOURCE_MESSAGE: &str =
    "Codex web assets are served from the configured registry.";

const GOOD_PROFILE: ScreenProfile = ScreenProfile {
    every_nth_frame: 2,
    max_height: 900,
    max_width: 1440,
    name: "good",
    quality: 74,
};
const MEDIUM_PROFILE: ScreenProfile = ScreenProfile {
    every_nth_frame: 2,
    max_height: 720,
    max_width: 1080,
    name: "medium",
    quality: 60,
};
const BAD_PROFILE: ScreenProfile = ScreenProfile {
    every_nth_frame: 4,
    max_height: 480,
    max_width: 720,
    name: "bad",
    quality: 42,
};

#[derive(Debug, Clone, Serialize)]
pub struct RemoteControlInfo {
    pub running: bool,
    pub profile_name: String,
    pub connection_mode: String,
    pub auth_mode: String,
    pub cloud_user_id: Option<String>,
    pub cloud_user_label: Option<String>,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub lan_url: String,
    pub relay_url: Option<String>,
    pub relay_connected: bool,
    pub require_password: bool,
    pub web_asset_mode: String,
    pub web_asset_base_url: Option<String>,
    pub web_asset_version: String,
    pub cdp_host: String,
    pub cdp_port: u16,
    pub cdp_ready: bool,
    pub control_client_count: usize,
    pub frame_client_count: usize,
}

pub(crate) struct RemoteControlHandle {
    info: RemoteControlInfo,
    runtime: Arc<RemoteRuntimeState>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl RemoteControlHandle {
    async fn info(&self) -> RemoteControlInfo {
        let mut info = self.info.clone();
        info.running = !self.runtime.stopped.load(Ordering::Relaxed);
        info.relay_connected = self.runtime.relay_connected().await;
        info.cdp_ready = self.runtime.cdp_ready().await;
        info.control_client_count = self.runtime.control_client_count().await;
        info.frame_client_count = self.runtime.frame_client_count().await;
        info
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.runtime.stop().await;
    }
}

#[derive(Debug, Clone)]
struct RemoteServerConfig {
    host: String,
    port: u16,
    token: String,
    relay_url: Option<String>,
    relay_connection_id: Option<String>,
    crypto: Option<Arc<RemoteCrypto>>,
    remote_frontend_mode: String,
    device_uuid: String,
    workspace_id: String,
    workspace_name: String,
    workspace_path: String,
    cloud_auth: Option<RemoteCloudAuthConfig>,
    web_asset_base_url: Option<String>,
    web_asset_version: String,
    cdp_host: String,
    cdp_port: u16,
}

#[derive(Clone)]
enum RemoteBackend {
    App,
    Cli(Arc<CliAppBridge>),
}

#[derive(Debug, Clone)]
struct TranscribeApiConfig {
    url: String,
    api_key: String,
    model: String,
}

impl TranscribeApiConfig {
    fn from_app_config(config: &AppConfig) -> Self {
        Self {
            url: config.remote_transcribe_api_url.trim().to_string(),
            api_key: config.remote_transcribe_api_key.trim().to_string(),
            model: non_empty_string(&config.remote_transcribe_model)
                .unwrap_or_else(|| DEFAULT_TRANSCRIBE_MODEL.to_string()),
        }
    }

    fn is_custom(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

impl Default for TranscribeApiConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            api_key: String::new(),
            model: DEFAULT_TRANSCRIBE_MODEL.to_string(),
        }
    }
}

impl RemoteBackend {
    fn mode(&self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Cli(_) => "cli",
        }
    }

    fn cli_bridge(&self) -> Option<Arc<CliAppBridge>> {
        match self {
            Self::Cli(bridge) => Some(bridge.clone()),
            Self::App => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CloudRelayDiscoveryResponse {
    #[serde(default)]
    ok: bool,
    relay: Option<CloudRelayDiscoveryRelay>,
}

#[derive(Debug, Deserialize)]
struct CloudRelayDiscoveryRelay {
    url: String,
}

pub async fn start_remote_control(
    state: &AppState,
    profile_name: String,
    remote_password: Option<String>,
    use_cloud_relay: Option<bool>,
    _require_e2ee: Option<bool>,
) -> Result<RemoteControlInfo, String> {
    let _ = server::instance_statuses(state).await?;
    if let Some(info) = existing_remote_info(state, &profile_name).await {
        return Ok(info);
    }

    let (app_config, token) = {
        let mut app_config = state.config.lock().await;
        let (token, token_created) =
            app_config.remote_control_token_for_profile(&profile_name, make_token)?;
        if token_created {
            app_config.save()?;
        }
        (app_config.clone(), token)
    };
    let profile = app_config.provider_profile(&profile_name);
    let use_cloud_relay = use_cloud_relay
        .or_else(|| {
            profile
                .as_ref()
                .map(|profile| profile.start_remote_cloud_on_launch)
        })
        .unwrap_or(false);
    let workspace_id = profile
        .as_ref()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| profile_name.clone());
    let workspace_name = profile
        .as_ref()
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| profile_name.clone());
    let workspace_path = profile
        .as_ref()
        .map(|profile| generated_codex_home(profile).to_string_lossy().to_string())
        .unwrap_or_default();
    let relay_url = if use_cloud_relay {
        match cloud_relay_url_for_startup(&app_config).await {
            Ok(relay_url) => Some(relay_url),
            Err(err) => {
                eprintln!(
                    "Cloud relay unavailable for {}; continuing with LAN remote control: {}",
                    profile_name, err
                );
                None
            }
        }
    } else {
        None
    };
    let require_e2ee = relay_url.is_some();
    let cloud_auth = if relay_url.is_some() {
        let mut auth = app_config.remote_cloud_auth.clone();
        auth.normalize();
        if !auth.is_logged_in() {
            return Err(
                "Cloud remote control requires a signed-in cloud identity. LAN remote control can be used without signing in."
                    .to_string(),
            );
        }
        if !auth.is_pro {
            if let Some(existing) = existing_cloud_remote_profile(state, &profile_name).await {
                return Err(format!(
                    "Free cloud remote control allows one workspace connected to relay. Stop remote control for {} or upgrade to Pro for unlimited workspaces.",
                    existing
                ));
            }
        }
        Some(auth)
    } else {
        None
    };
    let port = ports::find_free_port(
        &app_config.remote_control_host,
        app_config.remote_control_port,
        200,
    )
    .await
    .ok_or_else(|| "No free remote control port found".to_string())?;

    let uses_cli_backend = profile_uses_cli_remote_frontend(profile.as_ref());
    let (backend, cdp_host, cdp_port) = if uses_cli_backend {
        let cli_bridge = start_cli_app_server_bridge(state, &app_config, &profile_name).await?;
        (RemoteBackend::Cli(cli_bridge), String::new(), 0)
    } else {
        let launch = server::launch_codex_instance(
            state,
            server::LaunchRequest {
                profile_name: Some(profile_name.clone()),
                ..server::LaunchRequest::default()
            },
        )
        .await?;
        (RemoteBackend::App, launch.cdp_host, launch.cdp_port)
    };
    let remote_frontend_mode = profile
        .as_ref()
        .map(|profile| config::normalized_remote_frontend_mode(&profile.remote_frontend_mode))
        .unwrap_or_else(|| backend.mode().to_string());

    let relay_connection_id = relay_url.as_ref().map(|_| make_relay_connection_id());
    let public_token = relay_connection_id.clone().unwrap_or_else(|| token.clone());
    let remote_password = if require_e2ee {
        remote_password
            .filter(|password| !password.is_empty())
            .or_else(|| {
                profile
                    .as_ref()
                    .map(|profile| profile.remote_e2ee_password.clone())
                    .filter(|password| !password.is_empty())
            })
    } else {
        None
    };
    if require_e2ee && remote_password.is_none() {
        return Err("End-to-end encrypted remote control requires a password.".to_string());
    }
    let crypto = RemoteCrypto::from_password(remote_password.as_deref(), &public_token)?;
    let web_asset_base_url = profile_web_asset_base_url(&app_config, profile.as_ref());
    let web_asset_version = profile_web_asset_version(&app_config, profile.as_ref());
    let transcribe_api = TranscribeApiConfig::from_app_config(&app_config);
    let server_config = RemoteServerConfig {
        host: app_config.remote_control_host,
        port,
        token: token.clone(),
        relay_url: relay_url.clone(),
        relay_connection_id: relay_connection_id.clone(),
        crypto: crypto.map(Arc::new),
        remote_frontend_mode,
        device_uuid: app_config.device_uuid.clone(),
        workspace_id,
        workspace_name,
        workspace_path,
        cloud_auth: cloud_auth.clone(),
        web_asset_base_url,
        web_asset_version,
        cdp_host,
        cdp_port,
    };
    let raw_lan_url = remote_url(&server_config.host, server_config.port, &token);
    let lan_url = append_remote_connection_params(raw_lan_url.clone(), &server_config, false)?;
    let raw_url = if let Some(relay_url) = relay_url.as_deref() {
        remote_relay_url(
            relay_url,
            relay_connection_id
                .as_deref()
                .ok_or_else(|| "missing relay connection id".to_string())?,
            cloud_auth.as_ref().map(|auth| auth.user_id.as_str()),
        )?
    } else {
        raw_lan_url
    };
    let url = append_remote_connection_params(
        raw_url,
        &server_config,
        remote_requires_crypto(&server_config),
    )?;
    let listener = TcpListener::bind((server_config.host.as_str(), server_config.port))
        .await
        .map_err(|e| format!("failed to bind remote control server: {}", e))?;

    let runtime =
        RemoteRuntimeState::new_with_backend(server_config.clone(), backend, transcribe_api);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_runtime = runtime.clone();
    tokio::spawn(async move {
        if let Err(err) = serve_remote(listener, server_runtime, shutdown_rx).await {
            eprintln!("Remote control server failed: {}", err);
        }
    });
    runtime.start();

    let handle = RemoteControlHandle {
        info: RemoteControlInfo {
            running: true,
            profile_name: profile_name.clone(),
            connection_mode: if relay_url.is_some() {
                "cloud".to_string()
            } else {
                "lan".to_string()
            },
            auth_mode: if relay_url.is_some() {
                "cloud_identity".to_string()
            } else {
                "token".to_string()
            },
            cloud_user_id: cloud_auth.as_ref().map(|auth| auth.user_id.clone()),
            cloud_user_label: cloud_auth
                .as_ref()
                .map(RemoteCloudAuthConfig::display_label),
            host: server_config.host.clone(),
            port: server_config.port,
            token,
            url,
            lan_url,
            relay_url,
            relay_connected: false,
            require_password: remote_requires_crypto(&server_config),
            web_asset_mode: if server_config.web_asset_base_url.is_some() {
                "registry".to_string()
            } else if runtime.backend.mode() == "cli" {
                "cli".to_string()
            } else {
                "cdp".to_string()
            },
            web_asset_base_url: server_config.web_asset_base_url.clone(),
            web_asset_version: server_config.web_asset_version.clone(),
            cdp_host: server_config.cdp_host.clone(),
            cdp_port: server_config.cdp_port,
            cdp_ready: false,
            control_client_count: 0,
            frame_client_count: 0,
        },
        runtime,
        shutdown: Some(shutdown_tx),
    };

    let info = handle.info().await;
    state
        .remote_controls
        .lock()
        .await
        .insert(profile_name, handle);
    Ok(info)
}

pub async fn stop_remote_control(state: &AppState, profile_name: &str) -> Result<(), String> {
    let handle = state.remote_controls.lock().await.remove(profile_name);
    if let Some(handle) = handle {
        handle.stop().await;
    }
    Ok(())
}

pub(crate) async fn update_remote_transcribe_api_config(state: &AppState, config: &AppConfig) {
    let transcribe_api = TranscribeApiConfig::from_app_config(config);
    let runtimes: Vec<Arc<RemoteRuntimeState>> = {
        let controls = state.remote_controls.lock().await;
        controls
            .values()
            .map(|handle| handle.runtime.clone())
            .collect()
    };
    for runtime in runtimes {
        runtime.set_transcribe_api(transcribe_api.clone()).await;
    }
}

pub(crate) async fn update_remote_cloud_auth_config(state: &AppState, config: &AppConfig) {
    let mut cloud_auth = config.remote_cloud_auth.clone();
    cloud_auth.normalize();
    let cloud_auth = if cloud_auth.is_logged_in() {
        Some(cloud_auth)
    } else {
        None
    };
    let runtimes: Vec<Arc<RemoteRuntimeState>> = {
        let controls = state.remote_controls.lock().await;
        controls
            .values()
            .filter(|handle| handle.info.relay_url.is_some())
            .map(|handle| handle.runtime.clone())
            .collect()
    };
    for runtime in runtimes {
        runtime.set_cloud_auth(cloud_auth.clone()).await;
    }
}

pub async fn remote_control_status_map(state: &AppState) -> HashMap<String, RemoteControlInfo> {
    let controls = state.remote_controls.lock().await;
    let mut statuses = HashMap::new();
    for (profile_name, handle) in controls.iter() {
        statuses.insert(profile_name.clone(), handle.info().await);
    }
    statuses
}

async fn existing_remote_info(state: &AppState, profile_name: &str) -> Option<RemoteControlInfo> {
    let controls = state.remote_controls.lock().await;
    let handle = controls.get(profile_name)?;
    Some(handle.info().await)
}

async fn existing_cloud_remote_profile(state: &AppState, profile_name: &str) -> Option<String> {
    let controls = state.remote_controls.lock().await;
    controls
        .iter()
        .find(|(name, handle)| {
            name.as_str() != profile_name
                && handle.info.relay_url.is_some()
                && !handle.runtime.stopped.load(Ordering::Relaxed)
        })
        .map(|(name, _)| name.clone())
}

async fn discover_cloud_relay_url() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(CLOUD_RELAY_DISCOVERY_TIMEOUT_MS))
        .build()
        .map_err(|e| format!("failed to initialize cloud relay discovery: {}", e))?;
    let response = client
        .get(CLOUD_RELAY_DISCOVERY_URL)
        .header("accept", "application/json")
        .header(
            "user-agent",
            concat!(
                "codexl/",
                env!("CARGO_PKG_VERSION"),
                " remote-relay-discovery"
            ),
        )
        .send()
        .await
        .map_err(|e| format!("failed to discover cloud relay: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "cloud relay discovery failed with HTTP {}",
            status.as_u16()
        ));
    }

    let discovery = response
        .json::<CloudRelayDiscoveryResponse>()
        .await
        .map_err(|e| format!("failed to parse cloud relay discovery response: {}", e))?;
    selected_cloud_relay_url(discovery)
}

async fn cloud_relay_url_for_startup(app_config: &AppConfig) -> Result<String, String> {
    if let Some(relay_url) = configured_cloud_relay_url(&app_config.remote_relay_url)? {
        return Ok(relay_url);
    }

    discover_cloud_relay_url().await
}

fn configured_cloud_relay_url(value: &str) -> Result<Option<String>, String> {
    let relay_url = value.trim().trim_end_matches('/');
    if relay_url.is_empty() {
        return Ok(None);
    }

    relay_host_ws_url(relay_url, "probe", true)
        .map_err(|err| format!("invalid configured cloud relay URL: {}", err))?;
    Ok(Some(relay_url.to_string()))
}

fn selected_cloud_relay_url(discovery: CloudRelayDiscoveryResponse) -> Result<String, String> {
    if !discovery.ok {
        return Err("cloud relay discovery returned ok=false".to_string());
    }
    let relay_url = discovery
        .relay
        .map(|relay| relay.url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "cloud relay discovery response did not include relay.url".to_string())?;

    relay_host_ws_url(&relay_url, "probe", true)?;
    Ok(relay_url)
}

async fn serve_remote(
    listener: TcpListener,
    runtime: Arc<RemoteRuntimeState>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), String> {
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|e| e.to_string())?;
                let io = TokioIo::new(stream);
                let request_runtime = runtime.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req| handle_remote_request(req, request_runtime.clone()));
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        eprintln!("Remote control HTTP connection failed: {}", err);
                    }
                });
            }
        }
    }
}

async fn handle_remote_request(
    mut request: Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
) -> Result<Response<HttpBody>, Infallible> {
    let response = route_remote_request(&mut request, runtime)
        .await
        .unwrap_or_else(|err| {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": err }))
        });
    Ok(response)
}

async fn route_remote_request(
    request: &mut Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
) -> Result<Response<HttpBody>, String> {
    let path = request.uri().path().to_string();
    if matches!(
        path.as_str(),
        "/api/remote-info" | "/web/_version" | "/web/index.html" | "/web/_bridge.js"
    ) {
        eprintln!(
            "[remote] http request: method={} backend={} {}",
            request.method(),
            runtime.backend.mode(),
            remote_request_query_summary(request)
        );
    }

    if request.method() == Method::GET && is_websocket_upgrade(request) && path == "/web/_resource"
    {
        return web_resource_websocket_response(request, runtime).await;
    }
    if request.method() == Method::GET && is_websocket_upgrade(request) && path == "/web/_bridge" {
        return web_bridge_websocket_response(request, runtime).await;
    }
    if request.method() == Method::GET && is_websocket_upgrade(request) && path == "/ws/control" {
        return websocket_response(request, runtime, WsChannel::Control).await;
    }
    if request.method() == Method::GET && is_websocket_upgrade(request) && path == "/ws/frame" {
        return websocket_response(request, runtime, WsChannel::Frame).await;
    }

    if request.method() == Method::POST
        && path == "/web/_bridge"
        && !runtime.authorized_web_bridge(request)
    {
        eprintln!(
            "[codex-web] bridge http auth failed: backend={} {}",
            runtime.backend.mode(),
            remote_request_query_summary(request)
        );
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "unauthorized" }),
        ));
    }

    if remote_http_path_requires_auth(&path) && !runtime.authorized(request) {
        eprintln!(
            "[remote] http auth failed: backend={} {}",
            runtime.backend.mode(),
            remote_request_query_summary(request)
        );
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "unauthorized" }),
        ));
    }

    let response = match (request.method(), path.as_str()) {
        (&Method::GET, "/api/status") => {
            Ok(json_response(StatusCode::OK, runtime.bridge_status().await))
        }
        (&Method::GET, "/api/remote-info") => Ok(json_response(
            StatusCode::OK,
            runtime.remote_connection_info(request_is_cloud_remote_context(request))?,
        )),
        (&Method::GET, "/manifest.webmanifest") => {
            manifest_response(runtime.home_screen_manifest_start_url(request)?)
        }
        (&Method::GET, "/api/targets") => {
            let targets = runtime.bridge_targets().await?;
            Ok(json_response(StatusCode::OK, json!({ "targets": targets })))
        }
        (&Method::POST, "/api/target") => {
            let body = request
                .body_mut()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            let value = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing target id".to_string())?;
            if matches!(&runtime.backend, RemoteBackend::App) {
                runtime.bridge.switch_target(id).await?;
            }
            Ok(json_response(StatusCode::OK, runtime.bridge_status().await))
        }
        (&Method::GET, "/web") => Ok(retired_cdp_web_resource_response()),
        (&Method::POST, "/web/_bridge") => {
            let body = request
                .body_mut()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            let message = serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?;
            let response = runtime.dispatch_web_bridge_message(message).await?;
            Ok(json_response(StatusCode::OK, response))
        }
        (&Method::GET, _) if path.starts_with("/web/") => Ok(retired_cdp_web_resource_response()),
        (&Method::GET, _) => static_response(&path),
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "not found" }),
        )),
    }?;

    Ok(runtime.with_auth_cookie(request, response))
}

async fn web_bridge_websocket_response(
    request: &mut Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
) -> Result<Response<HttpBody>, String> {
    let authorized = runtime.authorized_web_bridge(request);
    eprintln!(
        "[codex-web] bridge websocket upgrade: backend={} auth={} {}",
        runtime.backend.mode(),
        authorized,
        remote_request_query_summary(request)
    );
    if !authorized {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }

    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let backend = runtime.backend.clone();
    let on_upgrade = hyper::upgrade::on(request);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let websocket =
                    tokio_tungstenite::WebSocketStream::from_raw_socket(io, Role::Server, None)
                        .await;
                let result = match backend {
                    RemoteBackend::Cli(cli_bridge) => {
                        cli_bridge.handle_web_bridge_websocket(websocket).await
                    }
                    RemoteBackend::App => runtime.handle_app_web_bridge_websocket(websocket).await,
                };
                if let Err(err) = result {
                    eprintln!("Remote Codex web bridge WebSocket failed: {}", err);
                }
            }
            Err(err) => eprintln!("Remote Codex web bridge WebSocket upgrade failed: {}", err),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())
}

async fn web_resource_websocket_response(
    request: &mut Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
) -> Result<Response<HttpBody>, String> {
    let authorized = runtime.authorized_web_bridge(request);
    eprintln!(
        "[codex-web] resource websocket upgrade: backend={} auth={} {}",
        runtime.backend.mode(),
        authorized,
        remote_request_query_summary(request)
    );
    if !authorized {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }

    Ok(retired_cdp_web_resource_response())
}

fn retired_cdp_web_resource_response() -> Response<HttpBody> {
    json_response(
        StatusCode::GONE,
        json!({
            "error": RETIRED_CDP_WEB_RESOURCE_MESSAGE,
            "webAssetMode": "registry",
        }),
    )
}

fn remote_request_query_summary(request: &Request<Incoming>) -> String {
    let query = request.uri().query().unwrap_or("");
    let token = query_param(query, "token");
    let referer_token_present = request
        .headers()
        .get(REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(referer_query_token)
        .is_some();
    let mut parts = vec![
        format!("path={}", request.uri().path()),
        format!("tokenPresent={}", token.is_some()),
    ];
    if let Some(token) = token {
        parts.push(format!("tokenLen={}", token.len()));
    }
    for key in [
        "hostId",
        "remoteMode",
        "transport",
        "auth",
        "e2ee",
        "requirePassword",
        "webAssetVersion",
    ] {
        if let Some(value) = query_param(query, key).filter(|value| !value.is_empty()) {
            parts.push(format!("{}={}", key, log_query_value(&value)));
        }
    }
    if query_param(query, "cloudUser").is_some() {
        parts.push("cloudUserPresent=true".to_string());
    }
    if query_param(query, "jwt").is_some() {
        parts.push("jwtPresent=true".to_string());
    }
    if referer_token_present {
        parts.push("refererTokenPresent=true".to_string());
    }
    parts.join(" ")
}

fn log_query_value(value: &str) -> String {
    let mut preview = value
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .take(80)
        .collect::<String>();
    if value.chars().count() > preview.chars().count() {
        preview.push_str("...");
    }
    preview
}

async fn websocket_response(
    request: &mut Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
    channel: WsChannel,
) -> Result<Response<HttpBody>, String> {
    let authorized = runtime.authorized(request);
    eprintln!(
        "[remote] {} websocket upgrade: auth={} {}",
        channel.label(),
        authorized,
        remote_request_query_summary(request)
    );
    if !authorized {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }

    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let on_upgrade = hyper::upgrade::on(request);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let websocket =
                    tokio_tungstenite::WebSocketStream::from_raw_socket(io, Role::Server, None)
                        .await;
                runtime.handle_client(websocket, channel).await;
            }
            Err(err) => eprintln!("Remote control WebSocket upgrade failed: {}", err),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WsChannel {
    Control,
    Frame,
}

impl WsChannel {
    fn label(self) -> &'static str {
        match self {
            WsChannel::Control => "control",
            WsChannel::Frame => "frame",
        }
    }
}

#[derive(Clone)]
enum ControlTarget {
    Local(usize),
    Relay(String),
}

struct RemoteRuntimeState {
    backend: RemoteBackend,
    bridge: Arc<CdpBridge>,
    config: RemoteServerConfig,
    control_clients: Mutex<HashMap<usize, mpsc::UnboundedSender<Message>>>,
    frame_clients: Mutex<HashMap<usize, mpsc::UnboundedSender<Message>>>,
    last_frame_meta_at: AtomicU64,
    next_client_id: AtomicUsize,
    relay_control_clients: Mutex<HashSet<String>>,
    relay_control_tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    relay_bulk_tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    relay_frame_client_count: AtomicUsize,
    relay_frame_tx: Mutex<Option<watch::Sender<Option<Arc<Vec<u8>>>>>>,
    relay_web_bridge_notification_pumps: Mutex<HashSet<String>>,
    relay_web_bridge_tasks: Arc<Semaphore>,
    stopped: AtomicBool,
    cloud_auth: Mutex<Option<RemoteCloudAuthConfig>>,
    transcribe_api: Mutex<TranscribeApiConfig>,
}

impl RemoteRuntimeState {
    #[cfg(test)]
    fn new(config: RemoteServerConfig) -> Arc<Self> {
        Self::new_with_backend(config, RemoteBackend::App, TranscribeApiConfig::default())
    }

    fn new_with_backend(
        config: RemoteServerConfig,
        backend: RemoteBackend,
        transcribe_api: TranscribeApiConfig,
    ) -> Arc<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let bridge = Arc::new(CdpBridge::new(config.clone(), event_tx));
        let initial_cloud_auth = config.cloud_auth.clone();
        let runtime = Arc::new(Self {
            backend,
            bridge,
            config,
            control_clients: Mutex::new(HashMap::new()),
            frame_clients: Mutex::new(HashMap::new()),
            last_frame_meta_at: AtomicU64::new(0),
            next_client_id: AtomicUsize::new(1),
            relay_control_clients: Mutex::new(HashSet::new()),
            relay_control_tx: Mutex::new(None),
            relay_bulk_tx: Mutex::new(None),
            relay_frame_client_count: AtomicUsize::new(0),
            relay_frame_tx: Mutex::new(None),
            relay_web_bridge_notification_pumps: Mutex::new(HashSet::new()),
            relay_web_bridge_tasks: Arc::new(Semaphore::new(RELAY_WEB_BRIDGE_TASK_LIMIT)),
            stopped: AtomicBool::new(false),
            cloud_auth: Mutex::new(initial_cloud_auth),
            transcribe_api: Mutex::new(transcribe_api),
        });
        let event_runtime = runtime.clone();
        tokio::spawn(async move {
            event_runtime.handle_bridge_events(event_rx).await;
        });
        runtime
    }

    fn start(self: &Arc<Self>) {
        if matches!(&self.backend, RemoteBackend::App) {
            self.bridge.clone().start();
        }
        if self.config.relay_url.is_some() {
            self.clone().start_relay_loop();
        }
        let runtime = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(HEARTBEAT_INTERVAL_MS)).await;
                if runtime.stopped.load(Ordering::Relaxed) {
                    return;
                }
                runtime
                    .broadcast_control(json!({ "type": "heartbeat", "ts": now_millis() }))
                    .await;
            }
        });
    }

    async fn stop(&self) {
        self.send_relay_envelope(json!({ "type": "hostClosing" }))
            .await;
        self.stopped.store(true, Ordering::Relaxed);
        self.bridge.stop().await;
        if let Some(cli_bridge) = self.backend.cli_bridge() {
            cli_bridge.stop().await;
        }
        self.close_clients().await;
    }

    async fn set_transcribe_api(&self, transcribe_api: TranscribeApiConfig) {
        *self.transcribe_api.lock().await = transcribe_api.clone();
        if let Some(cli_bridge) = self.backend.cli_bridge() {
            cli_bridge.set_transcribe_api(transcribe_api).await;
        }
    }

    async fn set_cloud_auth(&self, cloud_auth: Option<RemoteCloudAuthConfig>) {
        *self.cloud_auth.lock().await = cloud_auth;
        if self.config.relay_url.is_some() {
            self.clear_relay_state().await;
        }
    }

    async fn current_cloud_auth(&self) -> Option<RemoteCloudAuthConfig> {
        self.cloud_auth.lock().await.clone()
    }

    fn authorized(&self, request: &Request<Incoming>) -> bool {
        self.query_token_authorized(request)
            || self.bearer_token_authorized(request)
            || self.cookie_token_authorized(request)
            || self.referer_token_authorized(request)
    }

    fn authorized_web_bridge(&self, request: &Request<Incoming>) -> bool {
        self.authorized(request)
    }

    fn query_token_authorized(&self, request: &Request<Incoming>) -> bool {
        query_param(request.uri().query().unwrap_or(""), "token")
            .map(|token| self.token_matches(&token))
            .unwrap_or(false)
    }

    fn bearer_token_authorized(&self, request: &Request<Incoming>) -> bool {
        request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token)
            .map(|token| self.token_matches(token))
            .unwrap_or(false)
    }

    fn cookie_token_authorized(&self, request: &Request<Incoming>) -> bool {
        request.headers().get_all(COOKIE).iter().any(|value| {
            value
                .to_str()
                .ok()
                .and_then(|header| cookie_value(header, REMOTE_AUTH_COOKIE_NAME))
                .map(|token| self.token_matches(&token))
                .unwrap_or(false)
        })
    }

    fn referer_token_authorized(&self, request: &Request<Incoming>) -> bool {
        request.headers().get_all(REFERER).iter().any(|value| {
            value
                .to_str()
                .ok()
                .and_then(referer_query_token)
                .map(|token| self.token_matches(&token))
                .unwrap_or(false)
        })
    }

    fn explicit_token_authorized(&self, request: &Request<Incoming>) -> bool {
        self.query_token_authorized(request) || self.bearer_token_authorized(request)
    }

    fn token_matches(&self, candidate: &str) -> bool {
        constant_time_eq(candidate.as_bytes(), self.config.token.as_bytes())
    }

    fn with_auth_cookie(
        &self,
        request: &Request<Incoming>,
        mut response: Response<HttpBody>,
    ) -> Response<HttpBody> {
        if self.explicit_token_authorized(request) {
            if let Ok(value) = HeaderValue::from_str(&format!(
                "{}={}; Path=/web; HttpOnly; SameSite=Lax",
                REMOTE_AUTH_COOKIE_NAME, self.config.token
            )) {
                response.headers_mut().append(SET_COOKIE, value);
            }
            if let Ok(value) = HeaderValue::from_str(&format!(
                "{}={}; Path=/api; HttpOnly; SameSite=Lax",
                REMOTE_AUTH_COOKIE_NAME, self.config.token
            )) {
                response.headers_mut().append(SET_COOKIE, value);
            }
            if let Ok(value) = HeaderValue::from_str(&format!(
                "{}={}; Path=/manifest.webmanifest; HttpOnly; SameSite=Lax",
                REMOTE_AUTH_COOKIE_NAME, self.config.token
            )) {
                response.headers_mut().append(SET_COOKIE, value);
            }
        }
        response
    }

    async fn bridge_status(&self) -> Value {
        match self.backend.cli_bridge() {
            Some(cli_bridge) => cli_bridge.status().await,
            None => self.bridge.status().await,
        }
    }

    async fn cdp_ready(&self) -> bool {
        match &self.backend {
            RemoteBackend::Cli(cli_bridge) => cli_bridge.connected.load(Ordering::Relaxed),
            RemoteBackend::App => self.bridge.ready().await,
        }
    }

    fn remote_connection_info(&self, cloud_context: bool) -> Result<Value, String> {
        let device_name = local_device_name();
        let instance_name = mobile_instance_item_name(&device_name, &self.config.workspace_name);
        let raw_lan_url = remote_url(&self.config.host, self.config.port, &self.config.token);
        let lan_url = append_remote_connection_params(raw_lan_url.clone(), &self.config, false)?;
        let raw_url = if let Some(relay_url) = self.config.relay_url.as_deref() {
            remote_relay_url(
                relay_url,
                self.config
                    .relay_connection_id
                    .as_deref()
                    .ok_or_else(|| "missing relay connection id".to_string())?,
                self.config
                    .cloud_auth
                    .as_ref()
                    .map(|auth| auth.user_id.as_str()),
            )?
        } else {
            raw_lan_url
        };
        let url = append_remote_connection_params(
            raw_url,
            &self.config,
            remote_requires_crypto(&self.config),
        )?;
        let web_asset_mode = if self.config.web_asset_base_url.is_some() {
            "registry"
        } else {
            self.backend.mode()
        };
        let connection_mode = if self.config.relay_url.is_some() {
            "cloud"
        } else {
            "lan"
        };
        let auth_mode = if self.config.relay_url.is_some() {
            "cloud_identity"
        } else {
            "token"
        };
        let cloud_user_id = self
            .config
            .cloud_auth
            .as_ref()
            .map(|auth| auth.user_id.clone());
        let cloud_user_label = self
            .config
            .cloud_auth
            .as_ref()
            .map(RemoteCloudAuthConfig::display_label);

        let require_password = cloud_context && remote_requires_crypto(&self.config);

        Ok(json!({
            "url": url,
            "lanUrl": lan_url.clone(),
            "lan_url": lan_url,
            "token": self.config.token.clone(),
            "name": instance_name.clone(),
            "itemName": instance_name.clone(),
            "item_name": instance_name,
            "deviceName": device_name.clone(),
            "device_name": device_name,
            "connectionMode": connection_mode,
            "connection_mode": connection_mode,
            "authMode": auth_mode,
            "auth_mode": auth_mode,
            "cloudUserId": cloud_user_id.clone(),
            "cloud_user_id": cloud_user_id,
            "cloudUserLabel": cloud_user_label.clone(),
            "cloud_user_label": cloud_user_label,
            "host": self.config.host.clone(),
            "port": self.config.port,
            "relayUrl": self.config.relay_url.clone(),
            "relay_url": self.config.relay_url.clone(),
            "requirePassword": require_password,
            "require_password": require_password,
            "remoteMode": "web",
            "remote_mode": "web",
            "remoteFrontendMode": self.config.remote_frontend_mode.clone(),
            "remote_frontend_mode": self.config.remote_frontend_mode.clone(),
            "webAssetMode": web_asset_mode,
            "web_asset_mode": web_asset_mode,
            "webAssetBaseUrl": self.config.web_asset_base_url.clone(),
            "web_asset_base_url": self.config.web_asset_base_url.clone(),
            "webAssetVersion": self.config.web_asset_version.clone(),
            "web_asset_version": self.config.web_asset_version.clone(),
            "workspaceId": self.config.workspace_id.clone(),
            "workspace_id": self.config.workspace_id.clone(),
            "workspaceName": self.config.workspace_name.clone(),
            "workspace_name": self.config.workspace_name.clone(),
            "workspacePath": self.config.workspace_path.clone(),
            "workspace_path": self.config.workspace_path.clone(),
        }))
    }

    fn home_screen_manifest_start_url(
        &self,
        request: &Request<Incoming>,
    ) -> Result<Option<String>, String> {
        if !self.authorized(request) {
            return Ok(None);
        }

        let raw_lan_url = remote_url(&self.config.host, self.config.port, &self.config.token);
        let lan_url = append_remote_connection_params(raw_lan_url, &self.config, false)?;
        let mut start_url =
            reqwest::Url::parse("http://codexl.invalid/").map_err(|e| e.to_string())?;
        start_url
            .query_pairs_mut()
            .append_pair("url", &lan_url)
            .append_pair("addOnly", "1");
        Ok(start_url.query().map(|query| format!("./?{}", query)))
    }

    async fn bridge_targets(&self) -> Result<Vec<CdpTarget>, String> {
        match &self.backend {
            RemoteBackend::Cli(_) => Ok(Vec::new()),
            RemoteBackend::App => self.bridge.list_targets().await,
        }
    }

    async fn dispatch_web_bridge_message(&self, message: Value) -> Result<Value, String> {
        if cdp_resources::is_plugin_bridge_message(&message) {
            return cdp_resources::dispatch_plugin_bridge_message(message).await;
        }

        match self.backend.cli_bridge() {
            Some(cli_bridge) => cli_bridge.dispatch_message(message).await,
            None => {
                if let Some(response) = self.dispatch_app_desktop_api_fetch_request(&message).await
                {
                    return Ok(json!({ "messages": [response] }));
                }
                cdp_resources::dispatch_web_bridge_message(
                    &self.config.cdp_host,
                    self.config.cdp_port,
                    message,
                )
                .await
            }
        }
    }

    async fn dispatch_app_desktop_api_fetch_request(&self, message: &Value) -> Option<Value> {
        if let Some(response) = self.dispatch_app_frontend_compat_fetch_request(message) {
            return Some(response);
        }
        let transcribe_api = self.transcribe_api.lock().await.clone();
        custom_transcribe_fetch_response(message, &transcribe_api, "app").await
    }

    fn dispatch_app_frontend_compat_fetch_request(&self, message: &Value) -> Option<Value> {
        let request_id = message
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let fetch_request = bridge_fetch_request(message)?;
        if fetch_request.endpoint != "ide-context" {
            return None;
        }
        let codex_home = crate::config::default_codex_home();
        let body = cli_frontend_compat_endpoint_response(
            &fetch_request.endpoint,
            &fetch_request.body,
            &codex_home,
            &self.config.workspace_name,
        )?;
        Some(fetch_response_body_success(request_id, body))
    }

    async fn dispatch_app_web_bridge_socket_payload_with_emitter<F>(
        &self,
        raw: &str,
        emit: F,
    ) -> Value
    where
        F: Fn(Value) + Send + Sync,
    {
        let (id, message) = cdp_resources::parse_web_bridge_socket_message(raw);
        let is_heartbeat = matches!(&message, Ok(message) if cdp_resources::is_web_bridge_socket_heartbeat(message));
        match &message {
            Ok(message) if !is_heartbeat => {
                eprintln!(
                    "[codex-web] app socket request id={} message={}",
                    id.as_deref().unwrap_or(""),
                    cli_bridge_message_label(message)
                );
            }
            Err(err) => {
                eprintln!(
                    "[codex-web] app socket request parse failed id={} error={}",
                    id.as_deref().unwrap_or(""),
                    err
                );
            }
            _ => {}
        }
        if let Ok(message) = &message {
            if let Some(response) = self.dispatch_app_desktop_api_fetch_request(message).await {
                let response = cdp_resources::web_bridge_socket_response(
                    id,
                    Ok(json!({ "messages": [response] })),
                );
                if !is_heartbeat {
                    eprintln!(
                        "[codex-web] app socket response id={} labels={}",
                        response.get("id").and_then(Value::as_str).unwrap_or(""),
                        cli_bridge_response_labels(&response)
                    );
                }
                return response;
            }
        }

        let response = cdp_resources::dispatch_web_bridge_socket_payload_with_emitter(
            &self.config.cdp_host,
            self.config.cdp_port,
            raw,
            emit,
        )
        .await;
        if !is_heartbeat {
            if let Some(err) = response.get("error").and_then(Value::as_str) {
                eprintln!(
                    "[codex-web] app socket response id={} error={}",
                    response.get("id").and_then(Value::as_str).unwrap_or(""),
                    err
                );
            } else {
                eprintln!(
                    "[codex-web] app socket response id={} labels={}",
                    response.get("id").and_then(Value::as_str).unwrap_or(""),
                    cli_bridge_response_labels(&response)
                );
            }
        }
        response
    }

    fn handle_app_web_bridge_socket_text(
        self: Arc<Self>,
        tx: &mpsc::UnboundedSender<Message>,
        raw: String,
    ) {
        let tx = tx.clone();
        tokio::spawn(async move {
            let partial_tx = tx.clone();
            let response = self
                .dispatch_app_web_bridge_socket_payload_with_emitter(&raw, move |partial| {
                    let _ = partial_tx.send(Message::Text(partial.to_string()));
                })
                .await;
            let _ = tx.send(Message::Text(response.to_string()));
        });
    }

    async fn handle_app_web_bridge_websocket(
        self: Arc<Self>,
        websocket: tokio_tungstenite::WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    ) -> Result<(), String> {
        let cdp_host = self.config.cdp_host.clone();
        let cdp_port = self.config.cdp_port;
        eprintln!(
            "[codex-web] bridge websocket opened: cdp=http://{}:{}",
            cdp_host, cdp_port
        );
        let (mut write, mut read) = websocket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let pump_tx = tx.clone();
        cdp_resources::spawn_web_bridge_notification_pump(
            cdp_host.clone(),
            cdp_port,
            move |partial| {
                let _ = pump_tx.send(Message::Text(partial.to_string()));
            },
        );

        let writer = async {
            while let Some(message) = rx.recv().await {
                write.send(message).await.map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        };

        let reader_runtime = self.clone();
        let reader = async {
            while let Some(message) = read.next().await {
                match message.map_err(|e| e.to_string())? {
                    Message::Text(raw) => {
                        reader_runtime
                            .clone()
                            .handle_app_web_bridge_socket_text(&tx, raw);
                    }
                    Message::Binary(bytes) => match String::from_utf8(bytes) {
                        Ok(raw) => {
                            reader_runtime
                                .clone()
                                .handle_app_web_bridge_socket_text(&tx, raw);
                        }
                        Err(err) => {
                            let response = cdp_resources::web_bridge_socket_response(
                                None,
                                Err(err.to_string()),
                            );
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
            "[codex-web] bridge websocket closed: cdp=http://{}:{}",
            cdp_host, cdp_port
        );
        result
    }

    async fn handle_client(
        self: Arc<Self>,
        websocket: tokio_tungstenite::WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
        channel: WsChannel,
    ) {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        eprintln!("[remote] {} websocket opened id={}", channel.label(), id);
        let (tx, mut rx) = mpsc::unbounded_channel();
        match channel {
            WsChannel::Control => {
                self.control_clients.lock().await.insert(id, tx);
                self.send_control(
                    ControlTarget::Local(id),
                    json!({ "type": "status", "status": self.bridge_status().await }),
                )
                .await;
            }
            WsChannel::Frame => {
                self.frame_clients.lock().await.insert(id, tx);
                self.update_screencast_streaming().await;
            }
        }

        let (mut write, mut read) = websocket.split();
        let writer = async {
            while let Some(message) = rx.recv().await {
                if write.send(message).await.is_err() {
                    break;
                }
            }
        };
        let reader = async {
            while let Some(message) = read.next().await {
                match message {
                    Ok(Message::Text(text)) if channel == WsChannel::Control => {
                        self.handle_control_message(ControlTarget::Local(id), &text)
                            .await;
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        };

        tokio::select! {
            _ = writer => {}
            _ = reader => {}
        }

        match channel {
            WsChannel::Control => {
                self.control_clients.lock().await.remove(&id);
            }
            WsChannel::Frame => {
                self.frame_clients.lock().await.remove(&id);
                self.update_screencast_streaming().await;
            }
        }
        eprintln!("[remote] {} websocket closed id={}", channel.label(), id);
    }

    async fn handle_control_message(&self, client: ControlTarget, raw: &str) {
        let message = match serde_json::from_str::<Value>(raw) {
            Ok(message) => message,
            Err(err) => {
                self.send_control(
                    client,
                    json!({ "type": "error", "message": err.to_string() }),
                )
                .await;
                return;
            }
        };

        let result = match message.get("type").and_then(Value::as_str).unwrap_or("") {
            "pong" => Ok(None),
            "viewport" => match self.bridge.set_client_viewport(&message).await {
                Ok(()) => Ok(Some(
                    json!({ "type": "status", "status": self.bridge_status().await }),
                )),
                Err(err) => Err(err),
            },
            "refresh" => self.bridge.restart_screencast().await.map(|_| None),
            "profileMode" => {
                match self
                    .bridge
                    .set_screencast_profile_mode(
                        message
                            .get("mode")
                            .and_then(Value::as_str)
                            .unwrap_or("auto"),
                    )
                    .await
                {
                    Ok(()) => Ok(Some(
                        json!({ "type": "status", "status": self.bridge_status().await }),
                    )),
                    Err(err) => Err(err),
                }
            }
            "pageZoom" => {
                match self
                    .bridge
                    .set_page_zoom_scale(number_field(&message, "scale", DEFAULT_PAGE_ZOOM_SCALE))
                    .await
                {
                    Ok(()) => Ok(Some(
                        json!({ "type": "status", "status": self.bridge_status().await }),
                    )),
                    Err(err) => Err(err),
                }
            }
            "click" => {
                let focus = self
                    .bridge
                    .click_and_check_editable(
                        number_field(&message, "x", 0.5),
                        number_field(&message, "y", 0.5),
                    )
                    .await;
                match focus {
                    Ok(focus) => Ok(Some(json!({ "type": "keyboard", "focus": focus }))),
                    Err(err) => Err(err),
                }
            }
            "pointerMove" => self
                .bridge
                .pointer_move(
                    number_field(&message, "x", 0.5),
                    number_field(&message, "y", 0.5),
                )
                .await
                .map(|_| None),
            "scroll" => self
                .bridge
                .scroll(
                    number_field(&message, "x", 0.5),
                    number_field(&message, "y", 0.5),
                    number_field(&message, "deltaY", 0.0),
                    number_field(&message, "deltaX", 0.0),
                )
                .await
                .map(|_| None),
            "text" => self
                .bridge
                .insert_text(message.get("text").and_then(Value::as_str).unwrap_or(""))
                .await
                .map(|_| None),
            "key" => self
                .bridge
                .key(message.get("key").and_then(Value::as_str).unwrap_or(""))
                .await
                .map(|_| None),
            "sidebarSwipe" => self
                .bridge
                .apply_sidebar_swipe(
                    message
                        .get("direction")
                        .and_then(Value::as_str)
                        .unwrap_or("right"),
                    number_value(&message, "x"),
                    number_value(&message, "y"),
                )
                .await
                .map(|_| None),
            "sidebar" => self
                .bridge
                .set_sidebar(
                    message
                        .get("side")
                        .and_then(Value::as_str)
                        .unwrap_or("left"),
                    message
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("open"),
                )
                .await
                .map(|_| None),
            other => Err(format!("unknown message type: {}", other)),
        };

        match result {
            Ok(Some(response)) => self.send_control(client, response).await,
            Ok(None) => {}
            Err(err) => {
                self.send_control(client, json!({ "type": "warning", "message": err }))
                    .await;
            }
        }
    }

    fn start_relay_loop(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut reconnect_delay = RELAY_RECONNECT_MIN_MS;
            while !self.stopped.load(Ordering::Relaxed) {
                match self.connect_relay_once().await {
                    Ok(()) => {
                        reconnect_delay = RELAY_RECONNECT_MIN_MS;
                    }
                    Err(err) => {
                        eprintln!("Remote relay connection failed: {}", err);
                    }
                }

                self.clear_relay_state().await;
                if self.stopped.load(Ordering::Relaxed) {
                    return;
                }

                tokio::time::sleep(Duration::from_millis(reconnect_delay)).await;
                reconnect_delay =
                    (reconnect_delay + reconnect_delay / 2).min(RELAY_RECONNECT_MAX_MS);
            }
        });
    }

    async fn connect_relay_once(self: &Arc<Self>) -> Result<(), String> {
        let relay_url = self
            .config
            .relay_url
            .as_deref()
            .ok_or_else(|| "missing remote relay URL".to_string())?;
        let cloud_auth = self.current_cloud_auth().await;
        if self.config.cloud_auth.is_some() && cloud_auth.is_none() {
            return Err("cloud remote identity is signed out".to_string());
        }
        let ws_url = relay_host_ws_url(
            relay_url,
            &self.config.token,
            cloud_auth.is_some(),
        )?;
        let ws_url = append_relay_metadata_to_ws_url(ws_url, &self.config)?;
        let mut request = ws_url.into_client_request().map_err(|e| e.to_string())?;
        if let Some(auth) = cloud_auth.as_ref() {
            let headers = request.headers_mut();
            let authorization = HeaderValue::from_str(&format!("Bearer {}", auth.access_token))
                .map_err(|e| e.to_string())?;
            headers.insert(AUTHORIZATION, authorization);
            headers.insert(
                "x-codexl-cloud-user",
                HeaderValue::from_str(&auth.user_id).map_err(|e| e.to_string())?,
            );
            if !auth.display_name.is_empty() {
                headers.insert(
                    "x-codexl-cloud-user-label",
                    HeaderValue::from_str(&auth.display_name).map_err(|e| e.to_string())?,
                );
            }
            headers.insert("x-codexl-cloud-auth", HeaderValue::from_static("user"));
        }
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| e.to_string())?;
        let (mut write, mut read) = socket.split();
        let (control_tx, mut control_rx) = mpsc::unbounded_channel::<String>();
        let (bulk_tx, mut bulk_rx) = mpsc::unbounded_channel::<String>();
        let (frame_tx, mut frame_rx) = watch::channel::<Option<Arc<Vec<u8>>>>(None);

        *self.relay_control_tx.lock().await = Some(control_tx);
        *self.relay_bulk_tx.lock().await = Some(bulk_tx);
        *self.relay_frame_tx.lock().await = Some(frame_tx);

        let writer = async {
            loop {
                while let Ok(outbound) = control_rx.try_recv() {
                    write
                        .send(Message::Text(outbound))
                        .await
                        .map_err(|e| e.to_string())?;
                }
                tokio::select! {
                    biased;
                    outbound = control_rx.recv() => {
                        match outbound {
                            Some(text) => write.send(Message::Text(text)).await.map_err(|e| e.to_string())?,
                            None => break,
                        }
                    }
                    outbound = bulk_rx.recv() => {
                        match outbound {
                            Some(text) => write.send(Message::Text(text)).await.map_err(|e| e.to_string())?,
                            None => break,
                        }
                    }
                    changed = frame_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let frame = { frame_rx.borrow_and_update().clone() };
                        if let Some(frame) = frame {
                            write.send(Message::Binary((*frame).clone()))
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            Ok::<(), String>(())
        };

        let reader_runtime = self.clone();
        let reader = async move {
            while let Some(message) = read.next().await {
                match message {
                    Ok(Message::Text(text)) => reader_runtime.handle_relay_message(&text).await,
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(err) => return Err(err.to_string()),
                }
            }
            Ok::<(), String>(())
        };

        let result = tokio::select! {
            result = writer => result,
            result = reader => result,
        };
        result?;

        Ok(())
    }

    async fn handle_relay_message(self: &Arc<Self>, raw: &str) {
        let message = match serde_json::from_str::<Value>(raw) {
            Ok(message) => message,
            Err(_) => return,
        };

        match message.get("type").and_then(Value::as_str).unwrap_or("") {
            "ready" | "clientStats" => {
                self.update_relay_client_stats(&message).await;
            }
            "controlConnected" => {
                if let Some(client_id) = message.get("clientId").and_then(Value::as_str) {
                    self.relay_control_clients
                        .lock()
                        .await
                        .insert(client_id.to_string());
                    self.send_control(
                        ControlTarget::Relay(client_id.to_string()),
                        json!({ "type": "status", "status": self.bridge_status().await }),
                    )
                    .await;
                    self.update_screencast_streaming().await;
                }
            }
            "controlDisconnected" => {
                if let Some(client_id) = message.get("clientId").and_then(Value::as_str) {
                    self.relay_control_clients.lock().await.remove(client_id);
                    self.update_screencast_streaming().await;
                }
            }
            "controlFromClient" => {
                let client_id = message
                    .get("clientId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let payload = message
                    .get("payload")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                if !client_id.is_empty() {
                    self.relay_control_clients
                        .lock()
                        .await
                        .insert(client_id.clone());
                    match self.decrypt_relay_socket_text(payload) {
                        Ok(payload) => {
                            self.handle_control_message(ControlTarget::Relay(client_id), &payload)
                                .await;
                        }
                        Err(err) => {
                            self.send_control(
                                ControlTarget::Relay(client_id),
                                json!({ "type": "warning", "message": err }),
                            )
                            .await;
                        }
                    }
                }
            }
            "webBridgeFromClient" | "webResourceFromClient" => {
                self.spawn_relay_web_message(message);
            }
            "warning" => {
                if let Some(warning) = message.get("message").and_then(Value::as_str) {
                    eprintln!("Remote relay warning: {}", warning);
                }
            }
            _ => {}
        }
    }

    fn spawn_relay_web_message(self: &Arc<Self>, message: Value) {
        let runtime = self.clone();
        let semaphore = self.relay_web_bridge_tasks.clone();
        tokio::spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return;
            };
            runtime.handle_relay_web_message(message).await;
        });
    }

    async fn spawn_relay_web_bridge_notification_pump(self: &Arc<Self>, client_id: &str) {
        {
            let mut pumps = self.relay_web_bridge_notification_pumps.lock().await;
            if !pumps.insert(client_id.to_string()) {
                return;
            }
        }

        if let Some(cli_bridge) = self.backend.cli_bridge() {
            let relay_sender = self.relay_control_tx.lock().await.clone();
            let stream_client_id = client_id.to_string();
            let cleanup_client_id = stream_client_id.clone();
            let runtime_for_notification = self.clone();
            let runtime = self.clone();
            cli_bridge.spawn_notification_pump(move |partial| {
                if let Some(sender) = relay_sender.as_ref() {
                    if let Some(payload) =
                        runtime_for_notification.encrypt_relay_socket_text(partial.to_string())
                    {
                        let _ = sender.send(
                            json!({
                                "clientId": stream_client_id.as_str(),
                                "payload": payload,
                                "type": "webBridgeToClient",
                            })
                            .to_string(),
                        );
                    }
                }
            });
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(
                    RELAY_WEB_BRIDGE_NOTIFICATION_PUMP_TTL_MS,
                ))
                .await;
                runtime
                    .relay_web_bridge_notification_pumps
                    .lock()
                    .await
                    .remove(&cleanup_client_id);
            });
            return;
        }

        let cdp_host = self.config.cdp_host.clone();
        let cdp_port = self.config.cdp_port;
        let relay_sender = self.relay_control_tx.lock().await.clone();
        let stream_client_id = client_id.to_string();
        let cleanup_client_id = stream_client_id.clone();
        let runtime_for_notification = self.clone();
        let runtime = self.clone();
        cdp_resources::spawn_web_bridge_notification_pump(cdp_host, cdp_port, move |partial| {
            if let Some(sender) = relay_sender.as_ref() {
                if let Some(payload) =
                    runtime_for_notification.encrypt_relay_socket_text(partial.to_string())
                {
                    let _ = sender.send(
                        json!({
                            "clientId": stream_client_id.as_str(),
                            "payload": payload,
                            "type": "webBridgeToClient",
                        })
                        .to_string(),
                    );
                }
            }
        });

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(
                RELAY_WEB_BRIDGE_NOTIFICATION_PUMP_TTL_MS,
            ))
            .await;
            runtime
                .relay_web_bridge_notification_pumps
                .lock()
                .await
                .remove(&cleanup_client_id);
        });
    }

    async fn handle_relay_web_message(self: Arc<Self>, message: Value) {
        let message_type = message.get("type").and_then(Value::as_str).unwrap_or("");
        let client_id = message
            .get("clientId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let payload = message
            .get("payload")
            .and_then(Value::as_str)
            .unwrap_or("{}");
        if client_id.is_empty() {
            return;
        }
        let payload = match self.decrypt_relay_socket_text(payload) {
            Ok(payload) => payload,
            Err(err) => {
                if let Some(payload) = self
                    .encrypt_relay_socket_text(json!({ "messages": [], "error": err }).to_string())
                {
                    self.send_relay_envelope(json!({
                        "clientId": client_id,
                        "payload": payload,
                        "type": if message_type == "webResourceFromClient" {
                            "webResourceToClient"
                        } else {
                            "webBridgeToClient"
                        },
                    }))
                    .await;
                }
                return;
            }
        };

        let response = match message_type {
            "webBridgeFromClient" => {
                self.spawn_relay_web_bridge_notification_pump(&client_id)
                    .await;
                let relay_sender = self.relay_control_tx.lock().await.clone();
                let stream_client_id = client_id.clone();
                let runtime_for_stream = self.clone();
                if let Some(cli_bridge) = self.backend.cli_bridge() {
                    cli_bridge
                        .dispatch_socket_payload_with_emitter(&payload, move |partial| {
                            if let Some(sender) = relay_sender.as_ref() {
                                if let Some(payload) = runtime_for_stream
                                    .encrypt_relay_socket_text(partial.to_string())
                                {
                                    let _ = sender.send(
                                        json!({
                                            "clientId": stream_client_id.as_str(),
                                            "payload": payload,
                                            "type": "webBridgeToClient",
                                        })
                                        .to_string(),
                                    );
                                }
                            }
                        })
                        .await
                } else {
                    self.dispatch_app_web_bridge_socket_payload_with_emitter(
                        &payload,
                        move |partial| {
                            if let Some(sender) = relay_sender.as_ref() {
                                if let Some(payload) = runtime_for_stream
                                    .encrypt_relay_socket_text(partial.to_string())
                                {
                                    let _ = sender.send(
                                        json!({
                                            "clientId": stream_client_id.as_str(),
                                            "payload": payload,
                                            "type": "webBridgeToClient",
                                        })
                                        .to_string(),
                                    );
                                }
                            }
                        },
                    )
                    .await
                }
            }
            "webResourceFromClient" => {
                json!({
                    "error": RETIRED_CDP_WEB_RESOURCE_MESSAGE,
                    "messages": [],
                    "webAssetMode": "registry",
                })
            }
            _ => return,
        };
        let outbound_type = if message_type == "webBridgeFromClient" {
            "webBridgeToClient"
        } else {
            "webResourceToClient"
        };
        if let Some(payload) = self.encrypt_relay_socket_text(response.to_string()) {
            self.send_relay_envelope(json!({
                "clientId": client_id,
                "payload": payload,
                "type": outbound_type,
            }))
            .await;
        }
    }

    async fn update_relay_client_stats(&self, message: &Value) {
        if let Some(frame_client_count) = message.get("frameClientCount").and_then(Value::as_u64) {
            let control_client_count = message
                .get("controlClientCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            eprintln!(
                "Remote relay client stats: control={}, frame={}",
                control_client_count, frame_client_count
            );
            self.relay_frame_client_count
                .store(frame_client_count as usize, Ordering::Relaxed);
            self.update_screencast_streaming().await;
        }
    }

    async fn clear_relay_state(&self) {
        *self.relay_control_tx.lock().await = None;
        *self.relay_bulk_tx.lock().await = None;
        *self.relay_frame_tx.lock().await = None;
        self.relay_control_clients.lock().await.clear();
        self.relay_web_bridge_notification_pumps
            .lock()
            .await
            .clear();
        self.relay_frame_client_count.store(0, Ordering::Relaxed);
        self.update_screencast_streaming().await;
    }

    async fn send_relay_envelope(&self, envelope: Value) {
        let envelope_type = envelope
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let sender = if envelope_type == "webResourceToClient" {
            self.relay_bulk_tx.lock().await.clone()
        } else {
            self.relay_control_tx.lock().await.clone()
        };
        if let Some(sender) = sender {
            let _ = sender.send(envelope.to_string());
        }
    }

    async fn send_relay_frame(&self, frame: Arc<Vec<u8>>) {
        let sender = self.relay_frame_tx.lock().await.clone();
        if let Some(sender) = sender {
            let _ = sender.send(Some(frame));
        }
    }

    async fn handle_bridge_events(&self, mut events: mpsc::UnboundedReceiver<BridgeEvent>) {
        while let Some(event) = events.recv().await {
            if self.stopped.load(Ordering::Relaxed) {
                return;
            }
            match event {
                BridgeEvent::Frame(frame) => self.broadcast_frame(frame).await,
                BridgeEvent::Status(status) => {
                    self.broadcast_control(json!({ "type": "status", "status": status }))
                        .await;
                }
                BridgeEvent::Warning(message) => {
                    self.broadcast_control(json!({ "type": "warning", "message": message }))
                        .await;
                }
            }
        }
    }

    async fn update_screencast_streaming(&self) {
        if matches!(&self.backend, RemoteBackend::Cli(_)) {
            return;
        }
        let enabled = self.frame_client_count().await > 0;
        eprintln!("Remote screencast streaming requested: {}", enabled);
        if let Err(err) = self.bridge.set_screencast_enabled(enabled).await {
            self.broadcast_control(json!({ "type": "warning", "message": err }))
                .await;
        }
    }

    async fn control_client_count(&self) -> usize {
        self.control_clients.lock().await.len() + self.relay_control_clients.lock().await.len()
    }

    async fn frame_client_count(&self) -> usize {
        self.frame_clients.lock().await.len()
            + self.relay_frame_client_count.load(Ordering::Relaxed)
    }

    async fn relay_connected(&self) -> bool {
        self.relay_control_tx.lock().await.is_some()
    }

    fn encrypt_relay_socket_text(&self, raw: String) -> Option<String> {
        match self.config.crypto.as_ref() {
            Some(crypto) => match crypto.encrypt_text(&raw) {
                Ok(encrypted) => Some(encrypted),
                Err(err) => {
                    eprintln!("Remote payload encryption failed: {}", err);
                    None
                }
            },
            None => Some(raw),
        }
    }

    fn decrypt_relay_socket_text(&self, raw: &str) -> Result<String, String> {
        match self.config.crypto.as_ref() {
            Some(crypto) => crypto.decrypt_text(raw),
            None => Ok(raw.to_string()),
        }
    }

    fn encrypt_relay_frame_bytes(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        match self.config.crypto.as_ref() {
            Some(crypto) => match crypto.encrypt_bytes(bytes) {
                Ok(encrypted) => Some(encrypted),
                Err(err) => {
                    eprintln!("Remote frame encryption failed: {}", err);
                    None
                }
            },
            None => Some(bytes.to_vec()),
        }
    }

    async fn send_control(&self, client: ControlTarget, payload: Value) {
        match client {
            ControlTarget::Local(client_id) => {
                let message = Message::Text(payload.to_string());
                let sender = self.control_clients.lock().await.get(&client_id).cloned();
                if let Some(sender) = sender {
                    let _ = sender.send(message);
                }
            }
            ControlTarget::Relay(client_id) => {
                let Some(payload) = self.encrypt_relay_socket_text(payload.to_string()) else {
                    return;
                };
                self.send_relay_envelope(json!({
                    "clientId": client_id,
                    "payload": payload,
                    "type": "controlToClient",
                }))
                .await;
            }
        }
    }

    async fn broadcast_control(&self, payload: Value) {
        let raw = payload.to_string();
        if let Some(relay_payload) = self.encrypt_relay_socket_text(raw.clone()) {
            self.send_relay_envelope(json!({
                "payload": relay_payload,
                "type": "controlBroadcast",
            }))
            .await;
        }

        let message = Message::Text(raw);
        let mut stale = Vec::new();
        let clients = self.control_clients.lock().await;
        for (id, sender) in clients.iter() {
            if sender.send(message.clone()).is_err() {
                stale.push(*id);
            }
        }
        drop(clients);
        if !stale.is_empty() {
            let mut clients = self.control_clients.lock().await;
            for id in stale {
                clients.remove(&id);
            }
        }
    }

    async fn broadcast_frame(&self, frame: FramePayload) {
        if self.relay_frame_client_count.load(Ordering::Relaxed) > 0 {
            if let Some(encrypted_frame) = self.encrypt_relay_frame_bytes(&frame.bytes) {
                self.send_relay_frame(Arc::new(encrypted_frame)).await;
            }
        }

        let mut stale = Vec::new();
        let clients = self.frame_clients.lock().await;
        for (id, sender) in clients.iter() {
            if sender.send(Message::Binary(frame.bytes.clone())).is_err() {
                stale.push(*id);
            }
        }
        drop(clients);
        if !stale.is_empty() {
            let mut clients = self.frame_clients.lock().await;
            for id in stale {
                clients.remove(&id);
            }
        }

        let now = now_millis();
        let previous = self.last_frame_meta_at.load(Ordering::Relaxed);
        if now.saturating_sub(previous) >= FRAME_META_INTERVAL_MS {
            self.last_frame_meta_at.store(now, Ordering::Relaxed);
            self.broadcast_control(json!({
                "type": "frameMeta",
                "editableRects": [],
                "format": "jpeg",
                "metadata": frame.metadata,
                "metrics": frame.metrics,
                "target": frame.target,
                "ts": frame.ts,
            }))
            .await;
        }
    }

    async fn close_clients(&self) {
        let control_clients = std::mem::take(&mut *self.control_clients.lock().await);
        for sender in control_clients.values() {
            let _ = sender.send(Message::Close(None));
        }
        let frame_clients = std::mem::take(&mut *self.frame_clients.lock().await);
        for sender in frame_clients.values() {
            let _ = sender.send(Message::Close(None));
        }
    }
}

fn append_relay_metadata_to_ws_url(
    ws_url: String,
    config: &RemoteServerConfig,
) -> Result<String, String> {
    let mut url = reqwest::Url::parse(&ws_url).map_err(|e| e.to_string())?;
    let device_name = local_device_name();
    let platform = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);

    {
        let mut query = url.query_pairs_mut();
        if let Some(connection_id) = config
            .relay_connection_id
            .as_deref()
            .filter(|connection_id| !connection_id.trim().is_empty())
        {
            query.append_pair("clientInstanceId", connection_id.trim());
        }
        if !config.workspace_id.trim().is_empty() {
            query.append_pair("workspaceId", config.workspace_id.trim());
        }
        if !config.workspace_name.trim().is_empty() {
            query.append_pair("workspaceName", config.workspace_name.trim());
        }
        if !config.workspace_path.trim().is_empty() {
            query.append_pair("workspacePath", config.workspace_path.trim());
        }
        if !device_name.is_empty() {
            query.append_pair("deviceName", &device_name);
        }
        if !config.device_uuid.trim().is_empty() {
            query.append_pair("deviceUuid", config.device_uuid.trim());
        }
        if config.crypto.is_some() {
            query
                .append_pair("requirePassword", "1")
                .append_pair("e2ee", "v1");
        }
        query
            .append_pair("deviceType", "desktop")
            .append_pair("platform", &platform)
            .append_pair("clientVersion", env!("CARGO_PKG_VERSION"))
            .append_pair("remoteFrontendMode", config.remote_frontend_mode.trim())
            .append_pair("remote_frontend_mode", config.remote_frontend_mode.trim());
    }

    Ok(url.to_string())
}

fn append_remote_crypto_params(
    remote_url: String,
    require_password: bool,
) -> Result<String, String> {
    if !require_password {
        return Ok(remote_url);
    }
    let mut url = reqwest::Url::parse(&remote_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("requirePassword", "1")
        .append_pair("e2ee", "v1");
    Ok(url.to_string())
}

fn append_remote_web_asset_params(
    remote_url: String,
    config: &RemoteServerConfig,
) -> Result<String, String> {
    let Some(base_url) = config.web_asset_base_url.as_deref() else {
        return Ok(remote_url);
    };
    let mut url = reqwest::Url::parse(&remote_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("webAssetMode", "registry")
        .append_pair("webAssetBaseUrl", base_url)
        .append_pair("webAssetVersion", &config.web_asset_version);
    Ok(url.to_string())
}

fn append_remote_instance_name_param(
    remote_url: String,
    config: &RemoteServerConfig,
) -> Result<String, String> {
    let item_name = mobile_instance_item_name(&local_device_name(), &config.workspace_name);
    if item_name.is_empty() {
        return Ok(remote_url);
    }

    let mut url = reqwest::Url::parse(&remote_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("name", &item_name);
    Ok(url.to_string())
}

fn append_remote_connection_params(
    remote_url: String,
    config: &RemoteServerConfig,
    include_crypto_params: bool,
) -> Result<String, String> {
    let remote_url = append_remote_web_asset_params(remote_url, config)?;
    let remote_url = append_remote_instance_name_param(remote_url, config)?;
    append_remote_crypto_params(
        remote_url,
        include_crypto_params && remote_requires_crypto(config),
    )
}

fn remote_requires_crypto(config: &RemoteServerConfig) -> bool {
    config.relay_url.is_some() && config.crypto.is_some()
}

fn request_is_cloud_remote_context(request: &Request<Incoming>) -> bool {
    query_is_cloud_remote_context(request.uri().query().unwrap_or(""))
}

fn query_is_cloud_remote_context(query: &str) -> bool {
    query_param(query, "auth")
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("cloud"))
        || query_param(query, "cloudUser")
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || query_param(query, "jwt")
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let value = value.trim().trim_end_matches('/').to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn normalized_web_asset_version(value: String) -> String {
    let value = value.trim();
    if value.is_empty() {
        "latest".to_string()
    } else {
        value.to_string()
    }
}

fn default_web_asset_registry_url() -> String {
    std::env::var("CODEXL_REMOTE_WEB_ASSET_REGISTRY_URL")
        .ok()
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| DEFAULT_REMOTE_WEB_ASSET_REGISTRY_URL.to_string())
}

fn profile_web_asset_base_url(
    app_config: &AppConfig,
    profile: Option<&ProviderProfile>,
) -> Option<String> {
    profile
        .and_then(|profile| non_empty_trimmed(profile.remote_web_asset_registry_url.clone()))
        .or_else(|| non_empty_trimmed(app_config.remote_web_asset_registry_url.clone()))
        .or_else(|| Some(default_web_asset_registry_url()))
}

fn profile_web_asset_version(app_config: &AppConfig, profile: Option<&ProviderProfile>) -> String {
    if profile_web_asset_base_url(app_config, profile).is_none() {
        return "latest".to_string();
    }

    let version = profile
        .map(|profile| profile.remote_web_asset_version.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| app_config.remote_web_asset_version.clone());
    normalized_web_asset_version(version)
}

fn profile_uses_cli_remote_frontend(profile: Option<&ProviderProfile>) -> bool {
    profile
        .map(|profile| config::remote_frontend_mode_uses_cli(&profile.remote_frontend_mode))
        .unwrap_or(false)
}

async fn start_cli_app_server_bridge(
    state: &AppState,
    app_config: &AppConfig,
    profile_name: &str,
) -> Result<Arc<CliAppBridge>, String> {
    let mut requested_config = app_config.clone();
    let profile = requested_config
        .provider_profile(profile_name)
        .ok_or_else(|| format!("Provider profile not found: {}", profile_name))?;
    requested_config.active_provider = config::provider_profile_key(&profile);
    let executable = launcher::resolve_codex_cli_executable(None, &requested_config.codex_path);
    let profile_config_format = config::codex_profile_config_format_for_cli(&executable);
    requested_config.codex_home =
        config::ensure_provider_codex_home_with_format(&profile, profile_config_format)?;
    requested_config.normalize();

    let active_provider_profile =
        requested_config.provider_profile(&requested_config.active_provider);
    if requested_config.extensions.enabled
        && requested_config.extensions.next_ai_gateway_enabled
        && active_provider_profile
            .as_ref()
            .map(|profile| {
                profile.provider_name
                    == crate::extensions::builtins::gateway::config::NEXT_AI_GATEWAY_PROVIDER_NAME
            })
            .unwrap_or(false)
    {
        crate::extensions::builtins::gateway::service::ensure_running(state).await?;
    }

    if let Some(profile) = active_provider_profile.as_ref() {
        config::sync_provider_bot_media_mcp_config_for_launch(
            profile,
            &requested_config.codex_home,
            false,
        )?;
    }
    let active_cli_profile = requested_config.active_cli_profile();
    let active_cli_model_provider = requested_config.active_cli_model_provider();
    let app_server_backend = active_provider_profile
        .as_ref()
        .filter(|profile| profile_uses_claude_code_app_server(profile))
        .map(|_| CliAppServerBackend::ClaudeCode)
        .unwrap_or(CliAppServerBackend::CodexCli);
    let proxy_url = active_provider_profile
        .as_ref()
        .map(|profile| profile.proxy_url.clone())
        .unwrap_or_default();
    let transcribe_api = TranscribeApiConfig::from_app_config(&requested_config);
    let bridge = CliAppBridge::spawn(
        executable,
        requested_config.codex_home.clone(),
        requested_config.active_provider.clone(),
        active_cli_profile,
        active_cli_model_provider,
        profile_config_format,
        app_server_backend,
        proxy_url,
        transcribe_api,
    )
    .await?;

    let mut config = state.config.lock().await;
    config.codex_home = requested_config.codex_home;
    config.active_provider = requested_config.active_provider;
    Ok(bridge)
}

fn profile_uses_claude_code_app_server(profile: &ProviderProfile) -> bool {
    if profile
        .remote_frontend_mode
        .trim()
        .eq_ignore_ascii_case(REMOTE_FRONTEND_MODE_CLAUDE_CODE)
    {
        return true;
    }
    [
        &profile.provider_name,
        &profile.codex_profile_name,
        &profile.name,
    ]
    .into_iter()
    .any(|value| is_claude_code_profile_label(value))
}

fn is_claude_code_profile_label(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized == crate::claude_code_app_server::PROVIDER_NAME
        || matches!(normalized.as_str(), "claude_code" | "claude code")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliAppServerBackend {
    CodexCli,
    ClaudeCode,
}

struct CliAppBridge {
    backend: CliAppServerBackend,
    child: Mutex<Option<Child>>,
    codex_home: String,
    connected: AtomicBool,
    global_state: Mutex<HashMap<String, Value>>,
    initialize_result: Mutex<Option<Value>>,
    next_notification_seq: AtomicU64,
    next_request_id: AtomicU64,
    next_subscription_id: AtomicUsize,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>,
    subscribers: Mutex<HashMap<usize, mpsc::UnboundedSender<Value>>>,
    transcribe_api: Mutex<TranscribeApiConfig>,
    workspace_name: String,
    writer: Mutex<ChildStdin>,
}

impl CliAppBridge {
    async fn spawn(
        executable: String,
        codex_home: String,
        workspace_name: String,
        cli_profile: Option<String>,
        cli_model_provider: Option<String>,
        profile_config_format: config::CodexProfileConfigFormat,
        backend: CliAppServerBackend,
        proxy_url: String,
        transcribe_api: TranscribeApiConfig,
    ) -> Result<Arc<Self>, String> {
        let mut command = match backend {
            CliAppServerBackend::CodexCli => {
                let mut command = TokioCommand::new(&executable);
                command.args(cli_middleware::real_cli_args(
                    cli_profile.as_deref(),
                    cli_model_provider.as_deref(),
                    profile_config_format,
                    vec![
                        OsString::from("app-server"),
                        OsString::from("--analytics-default-enabled"),
                    ],
                ));
                command
            }
            CliAppServerBackend::ClaudeCode => {
                let host_executable = std::env::current_exe()
                    .map_err(|err| format!("Failed to locate CodexL executable: {}", err))?;
                let mut command = TokioCommand::new(host_executable);
                command
                    .arg(cli_middleware::CLAUDE_CODE_APP_SERVER_RUN_MODE_ARG)
                    .arg("--workspace-name")
                    .arg(&workspace_name)
                    .env("CODEXL_BUNDLED_CODEX_CLI_PATH", &executable);
                command
            }
        };
        command
            .env("CODEX_HOME", &codex_home)
            .env(cli_middleware::CODEX_WORKSPACE_NAME_ENV, &workspace_name)
            .env("CODEXL_WORKSPACE_NAME", &workspace_name)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        configure_tokio_proxy_env(&mut command, &proxy_url);

        let mut child = command
            .spawn()
            .map_err(|e| format!("Failed to launch Codex CLI app-server: {}", e))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to open Codex CLI app-server stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to open Codex CLI app-server stdout".to_string())?;

        let bridge = Arc::new(Self {
            backend,
            child: Mutex::new(Some(child)),
            codex_home,
            connected: AtomicBool::new(true),
            global_state: Mutex::new(HashMap::new()),
            initialize_result: Mutex::new(None),
            next_notification_seq: AtomicU64::new(1),
            next_request_id: AtomicU64::new(1),
            next_subscription_id: AtomicUsize::new(1),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(HashMap::new()),
            transcribe_api: Mutex::new(transcribe_api),
            workspace_name,
            writer: Mutex::new(stdin),
        });
        bridge.clone().spawn_stdout_reader(stdout);
        if let Err(err) = bridge.initialize_app_server().await {
            bridge.stop().await;
            return Err(err);
        }
        Ok(bridge)
    }

    async fn set_transcribe_api(&self, transcribe_api: TranscribeApiConfig) {
        *self.transcribe_api.lock().await = transcribe_api;
    }

    fn spawn_stdout_reader(self: Arc<Self>, stdout: tokio::process::ChildStdout) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&line).into_owned();
                        self.handle_stdout_line(line).await;
                    }
                    Err(err) => {
                        eprintln!("Codex CLI app-server stdout failed: {}", err);
                        break;
                    }
                }
            }
            self.connected.store(false, Ordering::Relaxed);
            self.publish_messages(self.connection_event_messages(false))
                .await;
            self.reject_pending("Codex CLI app-server stopped").await;
        });
    }

    async fn handle_stdout_line(&self, line: String) {
        let value = match serde_json::from_str::<Value>(line.trim_end()) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("Codex CLI app-server emitted invalid JSON: {}", err);
                return;
            }
        };

        if let Some(id) = json_id_to_string(value.get("id")) {
            if let Some(tx) = self.pending.lock().await.remove(&id) {
                let _ = tx.send(Ok(value));
                return;
            }
        }

        let messages = self.app_server_value_to_bridge_messages(value);
        if !messages.is_empty() {
            self.publish_messages(messages).await;
        }
    }

    fn app_server_value_to_bridge_messages(&self, value: Value) -> Vec<Value> {
        let host_id = "local";
        let message = cli_app_server_value_to_bridge_message(host_id, value);
        vec![self.decorate_notification(message)]
    }

    fn decorate_notification(&self, mut message: Value) -> Value {
        if let Value::Object(map) = &mut message {
            let seq = self.next_notification_seq.fetch_add(1, Ordering::Relaxed);
            let event_method = map
                .get("method")
                .and_then(Value::as_str)
                .or_else(|| map.get("type").and_then(Value::as_str))
                .unwrap_or("message")
                .to_string();
            map.insert("__codexWebBridgeNotificationSeq".to_string(), json!(seq));
            map.insert(
                "__codexWebBridgeNotificationQueuedAt".to_string(),
                json!(now_millis()),
            );
            map.insert("__codexEventSource".to_string(), json!("cli-app-server"));
            map.insert("__codexEventChannel".to_string(), json!("cli-app-server"));
            map.insert("__codexEventMethod".to_string(), json!(event_method));
        }
        message
    }

    async fn publish_messages(&self, messages: Vec<Value>) {
        let payload = json!({ "messages": messages });
        let mut stale = Vec::new();
        let subscribers = self.subscribers.lock().await;
        eprintln!(
            "[codex-web][cli] publish messages={} subscribers={} labels={}",
            payload
                .get("messages")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            subscribers.len(),
            cli_bridge_response_labels(&payload)
        );
        for (id, sender) in subscribers.iter() {
            if sender.send(payload.clone()).is_err() {
                stale.push(*id);
            }
        }
        drop(subscribers);
        if !stale.is_empty() {
            let mut subscribers = self.subscribers.lock().await;
            for id in stale {
                subscribers.remove(&id);
            }
        }
    }

    async fn request_raw(&self, mut request: Value, timeout_ms: u64) -> Result<Value, String> {
        let id = json_id_to_string(request.get("id")).unwrap_or_else(|| {
            format!(
                "{}{}",
                CLI_APP_SERVER_REQUEST_ID_PREFIX,
                self.next_request_id.fetch_add(1, Ordering::Relaxed)
            )
        });
        if let Value::Object(map) = &mut request {
            map.insert("id".to_string(), Value::String(id.clone()));
        } else {
            return Err("Codex CLI app-server request must be a JSON object".to_string());
        }
        normalize_cli_app_server_request(&mut request);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        if let Err(err) = self.write_json_line(&request).await {
            self.pending.lock().await.remove(&id);
            return Err(err);
        }

        tokio::time::timeout(Duration::from_millis(timeout_ms), rx)
            .await
            .map_err(|_| "Timed out waiting for Codex CLI app-server response".to_string())?
            .map_err(|_| "Codex CLI app-server response channel closed".to_string())?
    }

    async fn initialize_app_server(&self) -> Result<(), String> {
        let response = self
            .request_raw(
                cli_app_server_initialize_request(),
                CLI_APP_SERVER_FETCH_TIMEOUT_MS,
            )
            .await
            .map_err(|err| format!("Failed to initialize Codex CLI app-server: {}", err))?;
        if let Some(error) = response.get("error") {
            return Err(format!(
                "Failed to initialize Codex CLI app-server: {}",
                json_error_message(error)
            ));
        }
        let result = response
            .get("result")
            .cloned()
            .unwrap_or_else(default_cli_initialize_result);
        *self.initialize_result.lock().await = Some(result);
        self.write_json_line(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        }))
        .await
        .map_err(|err| {
            format!(
                "Failed to finish Codex CLI app-server initialization: {}",
                err
            )
        })
    }

    async fn cached_initialize_result(&self) -> Value {
        self.initialize_result
            .lock()
            .await
            .clone()
            .unwrap_or_else(default_cli_initialize_result)
    }

    async fn write_json_line(&self, value: &Value) -> Result<(), String> {
        let line = serde_json::to_vec(value).map_err(|err| err.to_string())?;
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&line)
            .await
            .map_err(|err| format!("failed to write Codex CLI app-server request: {}", err))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|err| format!("failed to write Codex CLI app-server request: {}", err))?;
        writer
            .flush()
            .await
            .map_err(|err| format!("failed to write Codex CLI app-server request: {}", err))
    }

    async fn reject_pending(&self, message: &str) {
        let pending = std::mem::take(&mut *self.pending.lock().await);
        for tx in pending.into_values() {
            let _ = tx.send(Err(message.to_string()));
        }
    }

    async fn dispatch_message(&self, message: Value) -> Result<Value, String> {
        if cdp_resources::is_plugin_bridge_message(&message) {
            return cdp_resources::dispatch_plugin_bridge_message(message).await;
        }
        if cdp_resources::is_web_file_picker_message(&message) {
            return cdp_resources::dispatch_web_file_picker_message(message);
        }
        match message.get("type").and_then(Value::as_str).unwrap_or("") {
            "mcp-request" => self.dispatch_mcp_request(message).await,
            "mcp-response" => self.dispatch_mcp_response(message).await,
            "fetch" => self.dispatch_fetch_request(message).await,
            "fetch-stream" => Ok(json!({
                "messages": [{
                    "type": "fetch-stream-error",
                    "requestId": message.get("requestId").and_then(Value::as_str).unwrap_or(""),
                    "error": "CLI remote mode does not support this fetch stream endpoint yet.",
                }],
            })),
            "persisted-atom-sync-request" => Ok(json!({
                "messages": [{ "type": "persisted-atom-sync", "state": {} }],
            })),
            "shared-object-subscribe" => Ok(json!({
                "messages": [{
                    "type": "shared-object-updated",
                    "key": message.get("key").cloned().unwrap_or(Value::Null),
                    "value": Value::Null,
                }],
            })),
            "electron-add-new-workspace-root-option" => {
                self.dispatch_cli_add_workspace_root_message(message).await
            }
            "electron-set-active-workspace-root" => {
                self.dispatch_cli_set_active_workspace_root_message(message)
                    .await
            }
            "electron-update-workspace-root-options" => {
                self.dispatch_cli_update_workspace_root_options_message(message)
                    .await
            }
            "codex-web-bridge-request-snapshot" => Ok(json!({
                "messages": self.connection_event_messages(self.is_running().await),
            })),
            "codex-app-server-restart" => Ok(json!({
                "messages": self.connection_event_messages(self.is_running().await),
            })),
            "desktop-notification-hide" | "thread-stream-state-changed" => {
                Ok(json!({ "messages": [] }))
            }
            _ => Ok(json!({ "messages": [] })),
        }
    }

    async fn dispatch_cli_add_workspace_root_message(
        &self,
        message: Value,
    ) -> Result<Value, String> {
        let Some(root) = message.get("root").and_then(Value::as_str) else {
            return Ok(json!({ "messages": [] }));
        };
        let mut store = self.global_state.lock().await;
        let messages = cli_add_workspace_root_messages(&mut store, root, true, true)?;
        Ok(json!({ "messages": messages }))
    }

    async fn dispatch_cli_set_active_workspace_root_message(
        &self,
        message: Value,
    ) -> Result<Value, String> {
        let mut store = self.global_state.lock().await;
        let messages = cli_set_active_workspace_root_messages(&mut store, &message)?;
        Ok(json!({ "messages": messages }))
    }

    async fn dispatch_cli_update_workspace_root_options_message(
        &self,
        message: Value,
    ) -> Result<Value, String> {
        let mut store = self.global_state.lock().await;
        let messages = cli_update_workspace_root_options_messages(&mut store, &message)?;
        Ok(json!({ "messages": messages }))
    }

    async fn dispatch_mcp_request(&self, message: Value) -> Result<Value, String> {
        let host_id = message
            .get("hostId")
            .and_then(Value::as_str)
            .unwrap_or("local")
            .to_string();
        let request = message
            .get("request")
            .cloned()
            .ok_or_else(|| "missing MCP request".to_string())?;
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "notifications/initialized" || method == "initialized" {
            return Ok(json!({ "messages": [] }));
        }
        if method == "initialize" {
            let Some(id) = request.get("id").cloned() else {
                return Ok(json!({ "messages": [] }));
            };
            return Ok(json!({
                "messages": [{
                    "type": "mcp-response",
                    "hostId": host_id,
                    "message": {
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": self.cached_initialize_result().await,
                    },
                }],
            }));
        }
        if let Some(result) = cli_auth_method_result(
            method,
            request.get("params").unwrap_or(&Value::Null),
            &self.codex_home,
            &self.workspace_name,
        ) {
            let Some(id) = request.get("id").cloned() else {
                return Ok(json!({ "messages": [] }));
            };
            return Ok(json!({
                "messages": [{
                    "type": "mcp-response",
                    "hostId": host_id,
                    "message": {
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result,
                    },
                }],
            }));
        }
        let mut response = self
            .request_raw(request.clone(), CLI_APP_SERVER_REQUEST_TIMEOUT_MS)
            .await?;
        decorate_cli_app_server_response(method, &mut response);
        let mut messages = Vec::new();
        if method == "thread/resume"
            || (method == "thread/start" && self.backend != CliAppServerBackend::ClaudeCode)
        {
            if let Some(thread) = response
                .get("result")
                .and_then(|result| result.get("thread"))
                .cloned()
            {
                messages.push(self.decorate_notification(json!({
                    "type": "mcp-notification",
                    "hostId": host_id,
                    "method": "thread/started",
                    "params": { "thread": thread },
                })));
            }
        }
        if method == "thread/resume" {
            match self
                .thread_resume_snapshot_message_from_response(&request, &response)
                .await
            {
                Ok(Some(message)) => messages.push(message),
                Ok(None) => {}
                Err(err) => eprintln!(
                    "[codex-web][cli] mcp thread/resume snapshot failed error={}",
                    err
                ),
            }
        }
        messages.push(json!({
            "type": "mcp-response",
            "hostId": host_id,
            "message": response,
        }));
        Ok(json!({ "messages": messages }))
    }

    async fn dispatch_mcp_response(&self, message: Value) -> Result<Value, String> {
        let response = message
            .get("message")
            .or_else(|| message.get("response"))
            .cloned()
            .ok_or_else(|| "missing MCP response message".to_string())?;
        self.write_json_line(&response).await?;
        Ok(json!({ "messages": [] }))
    }

    async fn dispatch_fetch_request(&self, message: Value) -> Result<Value, String> {
        let request_id = message
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(fetch_request) = bridge_fetch_request(&message) else {
            let url = message
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            eprintln!(
                "[codex-web][cli] fetch unsupported requestId={} url={}",
                request_id, url
            );
            if let Some(response) =
                cli_external_fetch_response(url, &self.codex_home, &self.workspace_name)
            {
                return Ok(json!({
                    "messages": [fetch_response_body_success(&request_id, response)],
                }));
            }
            if let Some(response) = self
                .dispatch_desktop_api_fetch_request(&request_id, &message)
                .await
            {
                return Ok(json!({
                    "messages": [response],
                }));
            }
            return Ok(json!({
                "messages": [fetch_response_error(
                    &request_id,
                    &format!("Unsupported fetch endpoint in CLI remote mode: {}", url),
                )],
            }));
        };
        if fetch_request.endpoint != "ipc-request" {
            eprintln!(
                "[codex-web][cli] fetch endpoint={} requestId={}",
                fetch_request.endpoint, request_id
            );
            let response = self
                .dispatch_cli_fetch_endpoint(&fetch_request.endpoint, fetch_request.body)
                .await;
            let messages = match response {
                Ok((result, mut messages)) => {
                    messages.push(fetch_response_body_success(&request_id, result));
                    messages
                }
                Err(err) => {
                    eprintln!(
                        "[codex-web][cli] fetch endpoint={} requestId={} error={}",
                        fetch_request.endpoint, request_id, err
                    );
                    vec![fetch_response_error(&request_id, &err)]
                }
            };
            eprintln!(
                "[codex-web][cli] fetch endpoint={} requestId={} response labels={}",
                fetch_request.endpoint,
                request_id,
                cli_bridge_message_labels(&messages)
            );
            return Ok(json!({ "messages": messages }));
        }
        let ipc_request = fetch_request.body;
        let method = ipc_request
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "IPC request missing method".to_string())?;
        let params = ipc_request
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let app_request_id = json_id_to_string(ipc_request.get("requestId"))
            .or_else(|| json_id_to_string(ipc_request.get("id")))
            .unwrap_or_else(|| {
                format!(
                    "{}{}",
                    CLI_APP_SERVER_REQUEST_ID_PREFIX,
                    self.next_request_id.fetch_add(1, Ordering::Relaxed)
                )
            });
        eprintln!(
            "[codex-web][cli] ipc-request method={} requestId={} appRequestId={}",
            method, request_id, app_request_id
        );
        if method == "notifications/initialized" || method == "initialized" {
            return Ok(json!({
                "messages": [fetch_response_success(
                    &request_id,
                    &app_request_id,
                    json!({
                        "jsonrpc": "2.0",
                        "id": app_request_id,
                        "result": Value::Null,
                    }),
                )],
            }));
        }
        if method == "initialize" {
            return Ok(json!({
                "messages": [fetch_response_success(
                    &request_id,
                    &app_request_id,
                    json!({
                        "jsonrpc": "2.0",
                        "id": app_request_id,
                        "result": self.cached_initialize_result().await,
                    }),
                )],
            }));
        }
        if let Some(result) =
            cli_auth_method_result(method, &params, &self.codex_home, &self.workspace_name)
        {
            return Ok(json!({
                "messages": [fetch_response_success(
                    &request_id,
                    &app_request_id,
                    json!({
                        "jsonrpc": "2.0",
                        "id": app_request_id,
                        "result": result,
                    }),
                )],
            }));
        }
        let app_request_params = cli_app_server_method_params(method, params);
        let app_request = json!({
            "id": app_request_id,
            "method": method,
            "params": app_request_params,
        });
        let response = self
            .request_raw(app_request, CLI_APP_SERVER_FETCH_TIMEOUT_MS)
            .await;
        let messages = match response {
            Ok(mut response) => {
                decorate_cli_app_server_response(method, &mut response);
                let mut messages = Vec::new();
                if method == "thread/resume"
                    || (method == "thread/start" && self.backend != CliAppServerBackend::ClaudeCode)
                {
                    messages.extend(self.thread_started_messages_from_response(&response));
                }
                messages.push(fetch_response_success(
                    &request_id,
                    &app_request_id,
                    response,
                ));
                messages
            }
            Err(err) => vec![fetch_response_error(&request_id, &err)],
        };
        eprintln!(
            "[codex-web][cli] ipc-request method={} requestId={} response labels={}",
            method,
            request_id,
            cli_bridge_message_labels(&messages)
        );
        Ok(json!({ "messages": messages }))
    }

    async fn dispatch_desktop_api_fetch_request(
        &self,
        request_id: &str,
        message: &Value,
    ) -> Option<Value> {
        if !cli_is_supported_desktop_api_fetch_request(message) {
            return None;
        }
        let transcribe_api = self.transcribe_api.lock().await.clone();
        match cli_desktop_api_fetch_response(
            message,
            &self.codex_home,
            &self.workspace_name,
            &transcribe_api,
        )
        .await
        {
            Ok(response) => Some(response),
            Err((status, error)) => Some(fetch_response_error_status(request_id, status, &error)),
        }
    }

    async fn dispatch_cli_fetch_endpoint(
        &self,
        endpoint: &str,
        params: Value,
    ) -> Result<(Value, Vec<Value>), String> {
        match endpoint {
            "app-server-connection-state" => {
                return Ok((
                    cli_app_server_connection_state_response(self.is_running().await),
                    Vec::new(),
                ));
            }
            "active-workspace-roots" => {
                let store = self.global_state.lock().await;
                return Ok((cli_active_workspace_roots_response(&store), Vec::new()));
            }
            "add-workspace-root-option" => {
                let mut store = self.global_state.lock().await;
                let (response, messages) =
                    cli_add_workspace_root_endpoint_response(&mut store, &params)?;
                return Ok((response, messages));
            }
            "get-global-state" => {
                let key = cli_global_state_key(&params)?;
                let store = self.global_state.lock().await;
                return Ok((
                    cli_global_state_get_response_for_key(&store, &key, &params, &self.codex_home)?,
                    Vec::new(),
                ));
            }
            "remote-workspace-directory-entries" => {
                return Ok((
                    cli_remote_workspace_directory_entries_response(&params)?,
                    Vec::new(),
                ));
            }
            "set-global-state" => {
                let mut store = self.global_state.lock().await;
                return Ok((
                    cli_global_state_set_response(&mut store, &params)?,
                    Vec::new(),
                ));
            }
            "workspace-root-options" => {
                let store = self.global_state.lock().await;
                return Ok((cli_workspace_root_options_response(&store), Vec::new()));
            }
            _ => {}
        }
        if let Some(response) = cli_frontend_compat_endpoint_response(
            endpoint,
            &params,
            &self.codex_home,
            &self.workspace_name,
        ) {
            return Ok((response, Vec::new()));
        }
        match endpoint {
            "read-config" | "read-config-for-host" => {
                let response = self
                    .request_cli_method_response(
                        "config/read",
                        strip_host_id(params),
                        CLI_APP_SERVER_FETCH_TIMEOUT_MS,
                    )
                    .await?;
                if response_error_is_method_not_found(&response) {
                    return Ok((json!({ "config": {} }), Vec::new()));
                }
                Ok((json_response_result(response)?, Vec::new()))
            }
            "send-cli-request-for-host" => {
                let method = params
                    .get("method")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "send-cli-request-for-host missing method".to_string())?;
                let request_params = cli_app_server_method_params(
                    method,
                    params.get("params").cloned().unwrap_or(Value::Null),
                );
                let timeout_ms = params
                    .get("timeoutMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(CLI_APP_SERVER_REQUEST_TIMEOUT_MS);
                let response = self
                    .request_cli_method_response(method, request_params, timeout_ms)
                    .await?;
                Ok((json_response_result(response)?, Vec::new()))
            }
            "start-thread-for-host" => {
                let response = self
                    .request_cli_method_response(
                        "thread/start",
                        cli_thread_start_params(strip_host_id(params)),
                        CLI_APP_SERVER_FETCH_TIMEOUT_MS,
                    )
                    .await?;
                let messages = self.thread_started_messages_from_response(&response);
                Ok((json_response_result(response)?, messages))
            }
            "prewarm-thread-start-for-host"
            | "prewarm-conversation-for-host"
            | "clear-prewarmed-threads-for-host" => Ok((Value::Null, Vec::new())),
            "projectless-thread-cwd" => {
                let endpoint_params = codex_fetch_endpoint_params(&params);
                let prompt = endpoint_params
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let directory_name = endpoint_params
                    .get("directoryName")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty());
                Ok((
                    create_cli_projectless_thread_cwd(directory_name, prompt)?,
                    Vec::new(),
                ))
            }
            "read-file-binary" => Ok((cli_read_file_binary_response(&params)?, Vec::new())),
            "start-turn-for-host" => {
                let conversation_id = params
                    .get("conversationId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "start-turn-for-host missing conversationId".to_string())?;
                let turn_params = cli_turn_start_params(
                    conversation_id,
                    params.get("params").cloned().unwrap_or_else(|| json!({})),
                    None,
                );
                let response = self
                    .request_cli_method_response(
                        "turn/start",
                        turn_params,
                        CLI_APP_SERVER_REQUEST_TIMEOUT_MS,
                    )
                    .await?;
                Ok((json_response_result(response)?, Vec::new()))
            }
            "start-conversation" => self.start_cli_conversation(params).await,
            "maybe-resume-conversation" => self.maybe_resume_cli_conversation(params).await,
            _ => Err(format!(
                "Unsupported fetch endpoint in CLI remote mode: vscode://codex/{}",
                endpoint
            )),
        }
    }

    async fn start_cli_conversation(&self, params: Value) -> Result<(Value, Vec<Value>), String> {
        let response = self
            .request_cli_method_response(
                "thread/start",
                cli_thread_start_params(params.clone()),
                CLI_APP_SERVER_FETCH_TIMEOUT_MS,
            )
            .await?;
        let _ = json_response_result(response.clone())?;
        let thread_id = thread_id_from_thread_start_response(&response)
            .ok_or_else(|| "thread/start response missing thread.id".to_string())?;
        let messages = self.thread_started_messages_from_response(&response);
        if !messages.is_empty() {
            self.publish_messages(messages.clone()).await;
        }
        if cli_start_conversation_has_first_turn(&params) {
            let turn_params = cli_turn_start_params(
                &thread_id,
                params.clone(),
                thread_start_response_model(&response),
            );
            let turn_response = self
                .request_cli_method_response(
                    "turn/start",
                    turn_params,
                    CLI_APP_SERVER_REQUEST_TIMEOUT_MS,
                )
                .await?;
            let _ = json_response_result(turn_response)?;
        }
        Ok((Value::String(thread_id), messages))
    }

    async fn maybe_resume_cli_conversation(
        &self,
        params: Value,
    ) -> Result<(Value, Vec<Value>), String> {
        let endpoint_params = codex_fetch_endpoint_params(&params).clone();
        let conversation_id = cli_conversation_id_from_params(&endpoint_params)?;
        eprintln!(
            "[codex-web][cli] maybe-resume start conversationId={}",
            conversation_id
        );
        let thread_metadata = self
            .request_cli_method_response(
                "thread/read",
                json!({
                    "threadId": conversation_id,
                    "includeTurns": false,
                }),
                CLI_APP_SERVER_FETCH_TIMEOUT_MS,
            )
            .await
            .and_then(json_response_result)
            .ok()
            .and_then(|result| result.get("thread").cloned());
        eprintln!(
            "[codex-web][cli] maybe-resume metadata conversationId={} found={} path={} cwd={}",
            conversation_id,
            thread_metadata.is_some(),
            thread_metadata
                .as_ref()
                .and_then(|thread| thread.get("path"))
                .and_then(Value::as_str)
                .map(|_| "yes")
                .unwrap_or("no"),
            thread_metadata
                .as_ref()
                .and_then(|thread| thread.get("cwd"))
                .and_then(Value::as_str)
                .unwrap_or("")
        );

        let turns_page = match self.read_cli_thread_turns_tail(&conversation_id).await {
            Ok(turns_page) => turns_page,
            Err(err) if thread_metadata.is_none() && cli_thread_missing_error(&err) => {
                eprintln!(
                    "[codex-web][cli] maybe-resume skipped conversationId={} reason={}",
                    conversation_id, err
                );
                return Ok((Value::Null, Vec::new()));
            }
            Err(err) => return Err(err),
        };
        eprintln!(
            "[codex-web][cli] maybe-resume turns conversationId={} count={} nextCursor={}",
            conversation_id,
            turns_page.turns.len(),
            if turns_page.next_cursor.is_null() {
                "none"
            } else {
                "present"
            }
        );
        let resume_params =
            cli_maybe_resume_params(&endpoint_params, thread_metadata.as_ref(), &conversation_id);
        let response = self
            .request_cli_method_response(
                "thread/resume",
                cli_thread_resume_params(Value::Object(resume_params)),
                CLI_APP_SERVER_FETCH_TIMEOUT_MS,
            )
            .await?;
        let resume_result = json_response_result(response.clone())?;
        let mut messages = self.thread_started_messages_from_response(&response);
        messages.push(
            self.decorate_notification(cli_conversation_snapshot_message(
                &conversation_id,
                &resume_result,
                &turns_page,
                &endpoint_params,
            )?),
        );
        eprintln!(
            "[codex-web][cli] maybe-resume done conversationId={} responseThread={} messages={}",
            conversation_id,
            resume_result
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            cli_bridge_message_labels(&messages)
        );
        Ok((Value::Null, messages))
    }

    async fn thread_resume_snapshot_message_from_response(
        &self,
        request: &Value,
        response: &Value,
    ) -> Result<Option<Value>, String> {
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let endpoint_params = codex_fetch_endpoint_params(&params).clone();
        let conversation_id = cli_conversation_id_from_params(&endpoint_params).or_else(|_| {
            thread_id_from_thread_start_response(response)
                .ok_or_else(|| "thread/resume response missing thread.id".to_string())
        })?;
        let resume_result = json_response_result(response.clone())?;
        let turns_page = self.read_cli_thread_turns_tail(&conversation_id).await?;
        let message = self.decorate_notification(cli_conversation_snapshot_message(
            &conversation_id,
            &resume_result,
            &turns_page,
            &endpoint_params,
        )?);
        eprintln!(
            "[codex-web][cli] mcp thread/resume snapshot conversationId={} messages={}",
            conversation_id,
            cli_bridge_message_label(&message)
        );
        Ok(Some(message))
    }

    async fn read_cli_thread_turns_tail(
        &self,
        thread_id: &str,
    ) -> Result<CliThreadTurnsPage, String> {
        let paged_response = self
            .request_cli_method_response(
                "thread/turns/list",
                json!({
                    "threadId": thread_id,
                    "cursor": Value::Null,
                    "limit": CLI_APP_SERVER_RESUME_TURNS_LIMIT,
                }),
                CLI_APP_SERVER_FETCH_TIMEOUT_MS,
            )
            .await
            .and_then(json_response_result);

        match paged_response {
            Ok(result) => {
                let page = CliThreadTurnsPage {
                    next_cursor: result.get("nextCursor").cloned().unwrap_or(Value::Null),
                    turns: result
                        .get("data")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                };
                eprintln!(
                    "[codex-web][cli] thread/turns/list threadId={} count={} nextCursor={}",
                    thread_id,
                    page.turns.len(),
                    if page.next_cursor.is_null() {
                        "none"
                    } else {
                        "present"
                    }
                );
                Ok(page)
            }
            Err(paged_error) => {
                eprintln!(
                    "[codex-web][cli] thread/turns/list threadId={} failed={}, falling back to thread/read includeTurns=true",
                    thread_id, paged_error
                );
                let legacy_response = self
                    .request_cli_method_response(
                        "thread/read",
                        json!({
                            "threadId": thread_id,
                            "includeTurns": true,
                        }),
                        CLI_APP_SERVER_REQUEST_TIMEOUT_MS,
                    )
                    .await
                    .and_then(json_response_result)
                    .map_err(|legacy_error| {
                        format!(
                            "Failed to load thread turns: {}; fallback thread/read failed: {}",
                            paged_error, legacy_error
                        )
                    })?;
                let turns = legacy_response
                    .get("thread")
                    .and_then(|thread| thread.get("turns"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                eprintln!(
                    "[codex-web][cli] thread/read includeTurns fallback threadId={} count={}",
                    thread_id,
                    turns.len()
                );
                Ok(CliThreadTurnsPage {
                    next_cursor: Value::Null,
                    turns,
                })
            }
        }
    }

    async fn request_cli_method_response(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        if let Some(result) =
            cli_auth_method_result(method, &params, &self.codex_home, &self.workspace_name)
        {
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "result": result,
            }));
        }
        let mut response = self
            .request_raw(
                json!({
                    "method": method,
                    "params": params,
                }),
                timeout_ms,
            )
            .await?;
        decorate_cli_app_server_response(method, &mut response);
        Ok(response)
    }

    fn thread_started_messages_from_response(&self, response: &Value) -> Vec<Value> {
        response
            .get("result")
            .and_then(|result| result.get("thread"))
            .cloned()
            .map(|thread| {
                self.decorate_notification(json!({
                    "type": "mcp-notification",
                    "hostId": "local",
                    "method": "thread/started",
                    "params": { "thread": thread },
                }))
            })
            .into_iter()
            .collect()
    }

    async fn dispatch_socket_payload_with_emitter<F>(&self, raw: &str, emit: F) -> Value
    where
        F: Fn(Value) + Send + Sync,
    {
        let _ = &emit;
        let (id, message) = cdp_resources::parse_web_bridge_socket_message(raw);
        let is_heartbeat = matches!(&message, Ok(message) if cdp_resources::is_web_bridge_socket_heartbeat(message));
        match &message {
            Ok(message) if !is_heartbeat => {
                eprintln!(
                    "[codex-web][cli] socket request id={} message={}",
                    id.as_deref().unwrap_or(""),
                    cli_bridge_message_label(message)
                );
                if message.get("type").and_then(Value::as_str) == Some("log-message") {
                    eprintln!(
                        "[codex-web][cli] frontend log id={} {}",
                        id.as_deref().unwrap_or(""),
                        cli_log_message_preview(message)
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "[codex-web][cli] socket request parse failed id={} error={}",
                    id.as_deref().unwrap_or(""),
                    err
                );
            }
            _ => {}
        }
        let result = match message {
            Ok(message) if cdp_resources::is_web_bridge_socket_heartbeat(&message) => {
                Ok(json!({ "type": "bridge-heartbeat-ack" }))
            }
            Ok(message) => self.dispatch_message(message).await,
            Err(err) => Err(err),
        };
        if !is_heartbeat {
            match &result {
                Ok(value) => eprintln!(
                    "[codex-web][cli] socket response id={} labels={}",
                    id.as_deref().unwrap_or(""),
                    cli_bridge_response_labels(value)
                ),
                Err(err) => eprintln!(
                    "[codex-web][cli] socket response id={} error={}",
                    id.as_deref().unwrap_or(""),
                    err
                ),
            }
        }
        cdp_resources::web_bridge_socket_response(id, result)
    }

    async fn handle_web_bridge_websocket<S>(
        self: Arc<Self>,
        websocket: tokio_tungstenite::WebSocketStream<S>,
    ) -> Result<(), String>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        eprintln!("[codex-web] CLI bridge websocket opened");
        let (mut write, mut read) = websocket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let (subscription_id, mut notification_rx) = self.subscribe().await;
        let notification_tx = tx.clone();
        tokio::spawn(async move {
            while let Some(partial) = notification_rx.recv().await {
                let _ = notification_tx.send(Message::Text(partial.to_string()));
            }
        });

        let writer = async {
            while let Some(message) = rx.recv().await {
                write.send(message).await.map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        };

        let bridge = self.clone();
        let reader = async {
            while let Some(message) = read.next().await {
                match message.map_err(|e| e.to_string())? {
                    Message::Text(raw) => {
                        let tx = tx.clone();
                        let bridge = bridge.clone();
                        tokio::spawn(async move {
                            let response = bridge
                                .dispatch_socket_payload_with_emitter(&raw, {
                                    let tx = tx.clone();
                                    move |partial| {
                                        let _ = tx.send(Message::Text(partial.to_string()));
                                    }
                                })
                                .await;
                            let _ = tx.send(Message::Text(response.to_string()));
                        });
                    }
                    Message::Binary(bytes) => match String::from_utf8(bytes) {
                        Ok(raw) => {
                            let tx = tx.clone();
                            let bridge = bridge.clone();
                            tokio::spawn(async move {
                                let response = bridge
                                    .dispatch_socket_payload_with_emitter(&raw, {
                                        let tx = tx.clone();
                                        move |partial| {
                                            let _ = tx.send(Message::Text(partial.to_string()));
                                        }
                                    })
                                    .await;
                                let _ = tx.send(Message::Text(response.to_string()));
                            });
                        }
                        Err(err) => {
                            let response = cdp_resources::web_bridge_socket_response(
                                None,
                                Err(err.to_string()),
                            );
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
        self.unsubscribe(subscription_id).await;
        eprintln!("[codex-web] CLI bridge websocket closed");
        result
    }

    async fn subscribe(&self) -> (usize, mpsc::UnboundedReceiver<Value>) {
        let id = self.next_subscription_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let initial_messages = self.connection_event_messages(self.is_running().await);
        let _ = tx.send(json!({ "messages": initial_messages }));
        self.subscribers.lock().await.insert(id, tx);
        (id, rx)
    }

    async fn unsubscribe(&self, id: usize) {
        self.subscribers.lock().await.remove(&id);
    }

    fn spawn_notification_pump<F>(self: Arc<Self>, emit: F)
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        tokio::spawn(async move {
            let (subscription_id, mut rx) = self.subscribe().await;
            loop {
                match tokio::time::timeout(
                    Duration::from_millis(RELAY_WEB_BRIDGE_NOTIFICATION_PUMP_TTL_MS),
                    rx.recv(),
                )
                .await
                {
                    Ok(Some(partial)) => emit(partial),
                    Ok(None) | Err(_) => break,
                }
            }
            self.unsubscribe(subscription_id).await;
        });
    }

    async fn status(&self) -> Value {
        let running = self.is_running().await;
        json!({
            "backend": "cli",
            "cdpUrl": Value::Null,
            "captureViewport": Value::Null,
            "clientViewport": Value::Null,
            "connected": running,
            "network": {
                "bufferedAmount": 0,
                "droppedFramesInLast5s": 0,
                "frameClientCount": 0,
                "rtt": null,
            },
            "pageZoomScale": DEFAULT_PAGE_ZOOM_SCALE,
            "screencastActive": false,
            "screencastProfile": "none",
            "screencastProfileMode": "off",
            "screencastProfileSettings": Value::Null,
            "streamingEnabled": false,
            "target": Value::Null,
            "viewportOverrideSuspended": true,
        })
    }

    async fn stop(&self) {
        self.connected.store(false, Ordering::Relaxed);
        self.publish_messages(self.connection_event_messages(false))
            .await;
        self.reject_pending("Codex CLI app-server stopped").await;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    async fn is_running(&self) -> bool {
        let mut child = self.child.lock().await;
        match child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_status)) => {
                    self.connected.store(false, Ordering::Relaxed);
                    false
                }
                Ok(None) => self.connected.load(Ordering::Relaxed),
                Err(_) => self.connected.load(Ordering::Relaxed),
            },
            None => {
                self.connected.store(false, Ordering::Relaxed);
                false
            }
        }
    }

    fn connection_event_messages(&self, connected: bool) -> Vec<Value> {
        cli_app_server_connection_event_messages(connected)
            .into_iter()
            .map(|message| self.decorate_notification(message))
            .collect()
    }
}

fn cli_app_server_value_to_bridge_message(host_id: &str, value: Value) -> Value {
    if value.get("type").and_then(Value::as_str) == Some("ipc-broadcast") {
        return value;
    }
    let method = value.get("method").and_then(Value::as_str);
    let id = json_id_to_string(value.get("id"));
    if method.is_some() && id.is_some() {
        json!({
            "type": "mcp-request",
            "hostId": host_id,
            "request": value,
        })
    } else if let Some(method) = method {
        json!({
            "type": "mcp-notification",
            "hostId": host_id,
            "method": method,
            "params": value.get("params").cloned().unwrap_or(Value::Null),
        })
    } else {
        value
    }
}

fn configure_tokio_proxy_env(command: &mut TokioCommand, proxy_url: &str) {
    let proxy_url = proxy_url.trim();
    if proxy_url.is_empty() {
        return;
    }
    for key in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        command.env(key, proxy_url);
    }
}

fn json_id_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn default_cli_initialize_result() -> Value {
    json!({
        "protocolVersion": CLI_APP_SERVER_PROTOCOL_VERSION,
        "capabilities": {},
        "serverInfo": {
            "name": "codex-cli-app-server",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "userAgent": format!("codexl-cli-app-server/{}", env!("CARGO_PKG_VERSION")),
        "codexHome": crate::config::default_codex_home(),
        "platformFamily": std::env::consts::FAMILY,
        "platformOs": std::env::consts::OS,
    })
}

fn cli_app_server_connection_state(connected: bool) -> &'static str {
    if connected {
        "connected"
    } else {
        "error"
    }
}

fn cli_app_server_connection_error(connected: bool) -> Value {
    if connected {
        Value::Null
    } else {
        json!({
            "code": "connection-failed",
            "message": "Codex app-server is not available",
        })
    }
}

fn cli_app_server_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn cli_app_server_connection_state_response(connected: bool) -> Value {
    json!({
        "state": cli_app_server_connection_state(connected),
        "error": cli_app_server_connection_error(connected),
        "appServerVersion": cli_app_server_version(),
        "installedCodexVersion": cli_app_server_version(),
    })
}

fn cli_app_server_connection_event_messages(connected: bool) -> Vec<Value> {
    vec![
        json!({
            "type": "codex-app-server-initialized",
            "hostId": "local",
            "appServerVersion": cli_app_server_version(),
            "installedCodexVersion": cli_app_server_version(),
        }),
        json!({
            "type": "codex-app-server-connection-changed",
            "hostId": "local",
            "state": cli_app_server_connection_state(connected),
            "error": cli_app_server_connection_error(connected),
            "transport": "websocket",
        }),
    ]
}

fn cli_app_server_initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": format!("{}initialize", CLI_APP_SERVER_REQUEST_ID_PREFIX),
        "method": "initialize",
        "params": {
            "protocolVersion": CLI_APP_SERVER_PROTOCOL_VERSION,
            "capabilities": {
                "experimentalApi": true,
            },
            "clientInfo": {
                "name": "codexl-remote-cli",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
    })
}

fn json_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

struct BridgeFetchRequest {
    endpoint: String,
    body: Value,
}

struct CliThreadTurnsPage {
    turns: Vec<Value>,
    next_cursor: Value,
}

fn bridge_fetch_request(message: &Value) -> Option<BridgeFetchRequest> {
    if message.get("type").and_then(Value::as_str) != Some("fetch") {
        return None;
    }
    let url = message.get("url").and_then(Value::as_str)?;
    let url = reqwest::Url::parse(url).ok()?;
    if url.scheme() != "vscode" || url.host_str() != Some("codex") {
        return None;
    }
    let endpoint = url.path().trim_start_matches('/').trim();
    if endpoint.is_empty() {
        return None;
    }
    let body = match message.get("body") {
        Some(Value::String(raw)) if raw.trim().is_empty() => Value::Null,
        Some(Value::String(raw)) => serde_json::from_str(raw).ok()?,
        Some(Value::Object(_)) => message.get("body")?.clone(),
        Some(Value::Null) | None => Value::Null,
        Some(other) => other.clone(),
    };
    Some(BridgeFetchRequest {
        endpoint: endpoint.to_string(),
        body,
    })
}

fn cli_is_supported_desktop_api_fetch_request(message: &Value) -> bool {
    if message.get("type").and_then(Value::as_str) != Some("fetch") {
        return false;
    }
    if !message
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("POST"))
    {
        return false;
    }
    message
        .get("url")
        .and_then(Value::as_str)
        .and_then(cli_desktop_api_fetch_path)
        .is_some()
}

pub(crate) async fn custom_transcribe_fetch_response_for_config(
    message: &Value,
    config: &AppConfig,
    source: &str,
) -> Option<Value> {
    let transcribe_api = TranscribeApiConfig::from_app_config(config);
    custom_transcribe_fetch_response(message, &transcribe_api, source).await
}

async fn custom_transcribe_fetch_response(
    message: &Value,
    transcribe_api: &TranscribeApiConfig,
    source: &str,
) -> Option<Value> {
    if !cli_is_supported_desktop_api_fetch_request(message) {
        return None;
    }
    if !transcribe_api.is_custom() {
        return None;
    }

    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("");
    eprintln!(
        "[codex-web][{}] custom transcribe requestId={} url={}",
        source,
        request_id,
        message
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    );
    match cli_custom_transcribe_fetch_response(message, transcribe_api).await {
        Ok(response) => Some(response),
        Err((status, error)) => Some(fetch_response_error_status(request_id, status, &error)),
    }
}

fn cli_desktop_api_fetch_path(url: &str) -> Option<&'static str> {
    let url = url.trim();
    if url == "/transcribe" {
        return Some("/transcribe");
    }
    if let Ok(parsed) = reqwest::Url::parse(url) {
        if parsed.path() == "/transcribe" && matches!(parsed.scheme(), "app" | "http" | "https") {
            return Some("/transcribe");
        }
    }
    None
}

async fn cli_desktop_api_fetch_response(
    message: &Value,
    codex_home: &str,
    workspace_name: &str,
    transcribe_api: &TranscribeApiConfig,
) -> Result<Value, (u16, String)> {
    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let url = message
        .get("url")
        .and_then(Value::as_str)
        .and_then(cli_desktop_api_fetch_path)
        .ok_or_else(|| (500, "Unsupported desktop API fetch endpoint".to_string()))?;
    if transcribe_api.is_custom() {
        return cli_custom_transcribe_fetch_response(message, transcribe_api).await;
    }

    let url = cli_desktop_api_url(url);
    let (request_headers, request_body) = cli_desktop_api_fetch_init(message)?;
    let auth_token = cli_chatgpt_auth_token(codex_home, workspace_name).ok_or_else(|| {
        (
            432,
            "Sign in to ChatGPT in Codex Desktop to transcribe audio.".to_string(),
        )
    })?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(CLI_APP_SERVER_FETCH_TIMEOUT_MS))
        .build()
        .map_err(|e| (500, e.to_string()))?;
    let mut request = client
        .request(reqwest::Method::POST, url)
        .header(AUTHORIZATION.as_str(), format!("Bearer {}", auth_token))
        .header("originator", CODEX_DESKTOP_ORIGINATOR)
        .header("User-Agent", cli_desktop_user_agent());
    if let Some(account_id) = cli_chatgpt_account_id_from_access_token(&auth_token) {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    for (name, value) in request_headers {
        request = request.header(name, value);
    }

    let response = request
        .body(request_body)
        .send()
        .await
        .map_err(|e| (500, e.to_string()))?;
    let status = response.status();
    let response_headers = cli_reqwest_headers_json(response.headers());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response.bytes().await.map_err(|e| (500, e.to_string()))?;

    if status.is_success() {
        let body = if content_type.contains("application/json") {
            serde_json::from_slice(&body)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&body).to_string()))
        } else {
            json!({
                "base64": BASE64_STANDARD.encode(&body),
                "contentType": content_type,
            })
        };
        Ok(fetch_response_body_success_status(
            request_id,
            status.as_u16(),
            response_headers,
            body,
        ))
    } else {
        let error = String::from_utf8_lossy(&body).to_string();
        Err((
            status.as_u16(),
            if error.trim().is_empty() {
                format!("Request failed with status {}", status.as_u16())
            } else {
                error
            },
        ))
    }
}

async fn cli_custom_transcribe_fetch_response(
    message: &Value,
    transcribe_api: &TranscribeApiConfig,
) -> Result<Value, (u16, String)> {
    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let url = custom_transcribe_endpoint_url(&transcribe_api.url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err((500, "Transcribe API URL must use http or https".to_string()));
    }

    let (request_headers, request_body) =
        cli_openai_transcribe_fetch_init(message, &transcribe_api.model)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(CLI_APP_SERVER_FETCH_TIMEOUT_MS))
        .build()
        .map_err(|e| (500, e.to_string()))?;
    let mut request = client.request(reqwest::Method::POST, url);
    if !transcribe_api.api_key.trim().is_empty() {
        request = request.header(
            AUTHORIZATION.as_str(),
            format!("Bearer {}", transcribe_api.api_key.trim()),
        );
    }
    for (name, value) in request_headers {
        request = request.header(name, value);
    }

    let response = request
        .body(request_body)
        .send()
        .await
        .map_err(|e| (500, e.to_string()))?;
    let status = response.status();
    let response_headers = cli_reqwest_headers_json(response.headers());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response.bytes().await.map_err(|e| (500, e.to_string()))?;

    if status.is_success() {
        let body = if content_type.contains("application/json") {
            serde_json::from_slice(&body)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&body).to_string()))
        } else {
            json!({
                "text": String::from_utf8_lossy(&body).to_string(),
            })
        };
        Ok(fetch_response_body_success_status(
            request_id,
            status.as_u16(),
            response_headers,
            body,
        ))
    } else {
        let error = String::from_utf8_lossy(&body).to_string();
        Err((
            status.as_u16(),
            if error.trim().is_empty() {
                format!("Request failed with status {}", status.as_u16())
            } else {
                error
            },
        ))
    }
}

fn custom_transcribe_endpoint_url(base_url: &str) -> Result<reqwest::Url, (u16, String)> {
    let mut url = reqwest::Url::parse(base_url)
        .map_err(|e| (500, format!("Invalid transcribe API URL: {}", e)))?;
    let path = url.path().trim_end_matches('/').to_string();
    if path
        .to_ascii_lowercase()
        .ends_with(OPENAI_TRANSCRIBE_ENDPOINT_SUFFIX)
    {
        url.set_path(&path);
        return Ok(url);
    }

    let next_path = if path.is_empty() {
        OPENAI_TRANSCRIBE_ENDPOINT_SUFFIX.to_string()
    } else {
        format!("{}{}", path, OPENAI_TRANSCRIBE_ENDPOINT_SUFFIX)
    };
    url.set_path(&next_path);
    Ok(url)
}

fn cli_desktop_api_fetch_init(
    message: &Value,
) -> Result<
    (
        Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
        Vec<u8>,
    ),
    (u16, String),
> {
    let mut headers = Vec::new();
    let mut base64_body = false;
    if let Some(header_map) = message.get("headers").and_then(Value::as_object) {
        for (key, value) in header_map {
            if key.eq_ignore_ascii_case("x-codex-base64") {
                base64_body = value.as_str() == Some("1");
                continue;
            }
            if cli_should_skip_desktop_api_request_header(key) {
                continue;
            }
            let Some(value) = value.as_str() else {
                continue;
            };
            let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) else {
                continue;
            };
            let Ok(value) = reqwest::header::HeaderValue::from_str(value) else {
                continue;
            };
            headers.push((name, value));
        }
    }

    let body = match message.get("body") {
        Some(Value::String(raw)) if base64_body => BASE64_STANDARD
            .decode(raw)
            .map_err(|e| (500, format!("Failed to decode base64 request body: {}", e)))?,
        Some(Value::String(raw)) => raw.as_bytes().to_vec(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => serde_json::to_vec(other).map_err(|e| (500, e.to_string()))?,
    };
    Ok((headers, body))
}

fn cli_openai_transcribe_fetch_init(
    message: &Value,
    model: &str,
) -> Result<
    (
        Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
        Vec<u8>,
    ),
    (u16, String),
> {
    let (mut headers, body) = cli_desktop_api_fetch_init(message)?;
    let content_type = headers
        .iter()
        .find(|(name, _)| name == reqwest::header::CONTENT_TYPE)
        .and_then(|(_, value)| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let model = non_empty_string(model).unwrap_or_else(|| DEFAULT_TRANSCRIBE_MODEL.to_string());
    let body = if let Some(boundary) = cli_multipart_boundary(&content_type) {
        cli_multipart_body_with_text_field(body, &boundary, "model", &model)?
    } else {
        let boundary = cli_transcribe_multipart_boundary();
        let body = cli_openai_transcribe_multipart_body(body, &boundary, &content_type, &model);
        headers.retain(|(name, _)| name != reqwest::header::CONTENT_TYPE);
        headers.push((
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_str(&format!(
                "multipart/form-data; boundary={}",
                boundary
            ))
            .map_err(|e| (500, e.to_string()))?,
        ));
        body
    };
    Ok((headers, body))
}

fn cli_transcribe_multipart_boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("codexl-transcribe-{}", nanos)
}

fn cli_openai_transcribe_multipart_body(
    audio: Vec<u8>,
    boundary: &str,
    content_type: &str,
    model: &str,
) -> Vec<u8> {
    let mime_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream");
    let filename = format!(
        "codex{}",
        audio_extension_for_content_type(mime_type).unwrap_or(".audio")
    );
    let mut output = Vec::with_capacity(audio.len() + boundary.len() * 3 + model.len() + 192);
    output.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    output.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            filename
        )
        .as_bytes(),
    );
    output.extend_from_slice(format!("Content-Type: {}\r\n\r\n", mime_type).as_bytes());
    output.extend_from_slice(&audio);
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    output.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
    output.extend_from_slice(model.as_bytes());
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    output
}

fn audio_extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/aac" => Some(".aac"),
        "audio/flac" => Some(".flac"),
        "audio/mp4" | "audio/m4a" => Some(".m4a"),
        "audio/mpeg" | "audio/mp3" => Some(".mp3"),
        "audio/ogg" | "audio/opus" => Some(".ogg"),
        "audio/wav" | "audio/wave" | "audio/x-wav" => Some(".wav"),
        "audio/webm" => Some(".webm"),
        _ => None,
    }
}

fn cli_multipart_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("boundary"))
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

fn cli_multipart_body_with_text_field(
    body: Vec<u8>,
    boundary: &str,
    field_name: &str,
    value: &str,
) -> Result<Vec<u8>, (u16, String)> {
    if cli_multipart_body_has_field(&body, field_name) {
        return Ok(body);
    }
    let closing = format!("--{}--", boundary).into_bytes();
    let Some(index) = find_last_subslice(&body, &closing) else {
        return Err((
            500,
            "Transcribe request multipart body is incomplete".to_string(),
        ));
    };

    let mut output = Vec::with_capacity(body.len() + boundary.len() + value.len() + 96);
    output.extend_from_slice(&body[..index]);
    if !output.ends_with(b"\r\n") {
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    output.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
            field_name
        )
        .as_bytes(),
    );
    output.extend_from_slice(value.as_bytes());
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(&body[index..]);
    Ok(output)
}

fn cli_multipart_body_has_field(body: &[u8], field_name: &str) -> bool {
    let needle = format!("name=\"{}\"", field_name);
    body.windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn find_last_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn cli_should_skip_desktop_api_request_header(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "chatgpt-account-id"
            | "connection"
            | "content-length"
            | "host"
            | "originator"
            | "user-agent"
            | "x-codex-base64"
    )
}

fn cli_desktop_api_url(path: &str) -> String {
    let base = cli_desktop_api_base_url();
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn cli_desktop_api_base_url() -> String {
    std::env::var("CODEX_API_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if std::env::var("CODEX_API_ENDPOINT")
                .ok()
                .map(|value| value.trim().eq_ignore_ascii_case("localhost"))
                .unwrap_or(false)
            {
                CODEX_DESKTOP_API_DEV_BASE_URL.to_string()
            } else {
                CODEX_DESKTOP_API_PROD_BASE_URL.to_string()
            }
        })
}

fn cli_chatgpt_auth_token(codex_home: &str, workspace_name: &str) -> Option<String> {
    let auth = cli_middleware::ChatGptAuth::load_for_codex_home(codex_home, workspace_name);
    auth.auth_status_result(true)
        .get("authToken")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .map(ToString::to_string)
}

fn cli_chatgpt_account_id_from_access_token(token: &str) -> Option<String> {
    let claims = cli_jwt_payload_claims(token)?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.as_object())
        .and_then(|auth| {
            auth.get("chatgpt_account_id")
                .or_else(|| auth.get("account_id"))
        })
        .and_then(Value::as_str)
        .filter(|account_id| !account_id.trim().is_empty())
        .map(ToString::to_string)
}

fn cli_jwt_payload_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| BASE64_URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn cli_desktop_user_agent() -> String {
    let platform = if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X"
    } else if cfg!(target_os = "windows") {
        "Windows NT 10.0"
    } else if cfg!(target_os = "linux") {
        "X11; Linux"
    } else {
        std::env::consts::OS
    };
    format!(
        "Codex Desktop/{} ({}; {})",
        env!("CARGO_PKG_VERSION"),
        platform,
        std::env::consts::ARCH
    )
}

fn cli_reqwest_headers_json(headers: &reqwest::header::HeaderMap) -> Value {
    let mut map = Map::new();
    for (name, value) in headers {
        if let Ok(value) = value.to_str() {
            map.insert(name.as_str().to_string(), json!(value));
        }
    }
    Value::Object(map)
}

fn strip_host_id(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            map.remove("hostId");
            Value::Object(map)
        }
        other => other,
    }
}

fn codex_fetch_endpoint_params(value: &Value) -> &Value {
    value
        .get("params")
        .filter(|params| params.is_object())
        .unwrap_or(value)
}

fn cli_read_file_binary_response(params: &Value) -> Result<Value, String> {
    let endpoint_params = codex_fetch_endpoint_params(params);
    let path = endpoint_params
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "read-file-binary missing path".to_string())?;
    let path = Path::new(path);
    let metadata = std::fs::metadata(path)
        .map_err(|err| format!("cannot open file {}: {}", path.display(), err))?;
    if !metadata.is_file() {
        return Err(format!("not a file: {}", path.display()));
    }
    if metadata.len() > CLI_READ_FILE_BINARY_MAX_BYTES {
        return Err(format!(
            "file too large for binary preview: {} bytes (limit {} bytes)",
            metadata.len(),
            CLI_READ_FILE_BINARY_MAX_BYTES
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|err| format!("cannot read file {}: {}", path.display(), err))?;
    Ok(json!({ "contentsBase64": BASE64_STANDARD.encode(&bytes) }))
}

fn cli_workspace_root_options_response(store: &HashMap<String, Value>) -> Value {
    json!({
        "roots": cli_workspace_roots(store),
        "labels": {},
    })
}

fn cli_active_workspace_roots_response(store: &HashMap<String, Value>) -> Value {
    json!({ "roots": cli_active_workspace_roots(store) })
}

fn cli_add_workspace_root_endpoint_response(
    store: &mut HashMap<String, Value>,
    params: &Value,
) -> Result<(Value, Vec<Value>), String> {
    let endpoint_params = codex_fetch_endpoint_params(params);
    let root = endpoint_params
        .get("root")
        .and_then(Value::as_str)
        .ok_or_else(|| "add-workspace-root-option missing root".to_string())?;
    let set_active = endpoint_params
        .get("setActive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let messages = cli_add_workspace_root_messages(store, root, set_active, false)?;
    Ok((json!({ "success": true }), messages))
}

fn cli_remote_workspace_directory_entries_response(params: &Value) -> Result<Value, String> {
    let endpoint_params = codex_fetch_endpoint_params(params);
    let directory_path = endpoint_params
        .get("directoryPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let directories_only = endpoint_params
        .get("directoriesOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let directory = normalize_cli_workspace_picker_path(Some(directory_path))?;
    let metadata = std::fs::metadata(&directory)
        .map_err(|err| format!("cannot open directory {}: {}", directory.display(), err))?;
    if !metadata.is_dir() {
        return Err(format!("not a directory: {}", directory.display()));
    }

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&directory)
        .map_err(|err| format!("cannot read directory {}: {}", directory.display(), err))?;
    for entry in read_dir.flatten() {
        let Some((entry_type, _size_bytes)) = cli_workspace_picker_entry_type(&entry) else {
            continue;
        };
        if directories_only && entry_type != "directory" {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() {
            continue;
        }
        entries.push(json!({
            "name": name,
            "path": entry.path().to_string_lossy().into_owned(),
            "type": entry_type,
        }));
    }
    entries.sort_by(|left, right| {
        let left_type = left.get("type").and_then(Value::as_str).unwrap_or("");
        let right_type = right.get("type").and_then(Value::as_str).unwrap_or("");
        let left_name = left.get("name").and_then(Value::as_str).unwrap_or("");
        let right_name = right.get("name").and_then(Value::as_str).unwrap_or("");
        let left_hidden = left_name.starts_with('.');
        let right_hidden = right_name.starts_with('.');
        type_rank(left_type)
            .cmp(&type_rank(right_type))
            .then_with(|| left_hidden.cmp(&right_hidden))
            .then_with(|| {
                left_name
                    .to_lowercase()
                    .cmp(&right_name.to_lowercase())
                    .then_with(|| left_name.cmp(right_name))
            })
    });

    Ok(json!({
        "directoryPath": directory.to_string_lossy().into_owned(),
        "entries": entries,
        "parentPath": directory.parent().map(|path| path.to_string_lossy().into_owned()),
    }))
}

fn type_rank(entry_type: &str) -> u8 {
    if entry_type == "directory" {
        0
    } else {
        1
    }
}

fn cli_workspace_picker_entry_type(
    entry: &std::fs::DirEntry,
) -> Option<(&'static str, Option<u64>)> {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => Some(("directory", None)),
        Ok(file_type) if file_type.is_file() => {
            let size_bytes = entry.metadata().ok().map(|metadata| metadata.len());
            Some(("file", size_bytes))
        }
        Ok(file_type) if file_type.is_symlink() => {
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                Some(("directory", None))
            } else if metadata.is_file() {
                Some(("file", Some(metadata.len())))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn cli_workspace_roots(store: &HashMap<String, Value>) -> Vec<String> {
    cli_string_vec_from_store(store, CLI_WORKSPACE_ROOT_OPTIONS_KEY)
}

fn cli_active_workspace_roots(store: &HashMap<String, Value>) -> Vec<String> {
    cli_string_vec_from_store(store, CLI_ACTIVE_WORKSPACE_ROOTS_KEY)
}

fn cli_string_vec_from_store(store: &HashMap<String, Value>, key: &str) -> Vec<String> {
    store
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn cli_store_string_vec(store: &mut HashMap<String, Value>, key: &str, values: Vec<String>) {
    store.insert(
        key.to_string(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
}

fn cli_add_workspace_root_messages(
    store: &mut HashMap<String, Value>,
    root: &str,
    set_active: bool,
    navigate: bool,
) -> Result<Vec<Value>, String> {
    let root = normalize_cli_workspace_root(root)?;
    let mut roots = cli_workspace_roots(store);
    let changed = !roots.iter().any(|existing| existing == &root);
    if changed {
        roots.retain(|existing| existing != &root);
        roots.insert(0, root.clone());
        cli_store_string_vec(store, CLI_WORKSPACE_ROOT_OPTIONS_KEY, roots);
    }

    let mut messages = Vec::new();
    if changed {
        messages.push(json!({ "type": "workspace-root-options-updated" }));
    }
    if set_active {
        cli_store_string_vec(store, CLI_ACTIVE_WORKSPACE_ROOTS_KEY, vec![root.clone()]);
        messages.push(json!({ "type": "active-workspace-roots-updated" }));
    }
    messages.push(json!({ "type": "workspace-root-option-added", "root": root }));
    if navigate {
        messages.push(json!({
            "type": "navigate-to-route",
            "path": "/",
            "state": { "focusComposerNonce": now_millis() },
        }));
    }
    Ok(messages)
}

fn cli_set_active_workspace_root_messages(
    store: &mut HashMap<String, Value>,
    params: &Value,
) -> Result<Vec<Value>, String> {
    let roots = if let Some(values) = params.get("roots").and_then(Value::as_array) {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(normalize_cli_workspace_root)
            .collect::<Result<Vec<_>, _>>()?
    } else if let Some(root) = params.get("root").and_then(Value::as_str) {
        vec![normalize_cli_workspace_root(root)?]
    } else {
        Vec::new()
    };
    cli_store_string_vec(store, CLI_ACTIVE_WORKSPACE_ROOTS_KEY, roots);
    Ok(vec![json!({ "type": "active-workspace-roots-updated" })])
}

fn cli_update_workspace_root_options_messages(
    store: &mut HashMap<String, Value>,
    params: &Value,
) -> Result<Vec<Value>, String> {
    let roots = params
        .get("roots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(normalize_cli_workspace_root)
        .collect::<Result<Vec<_>, _>>()?;
    let root_set = roots.iter().cloned().collect::<HashSet<_>>();
    let active = cli_active_workspace_roots(store)
        .into_iter()
        .filter(|root| root_set.contains(root))
        .collect::<Vec<_>>();
    cli_store_string_vec(store, CLI_WORKSPACE_ROOT_OPTIONS_KEY, roots);
    cli_store_string_vec(store, CLI_ACTIVE_WORKSPACE_ROOTS_KEY, active);
    Ok(vec![
        json!({ "type": "workspace-root-options-updated" }),
        json!({ "type": "active-workspace-roots-updated" }),
    ])
}

fn normalize_cli_workspace_root(path: &str) -> Result<String, String> {
    let path = normalize_cli_workspace_picker_path(Some(path))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|err| format!("cannot open workspace root {}: {}", path.display(), err))?;
    if !metadata.is_dir() {
        return Err(format!(
            "workspace root is not a directory: {}",
            path.display()
        ));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn normalize_cli_workspace_picker_path(path: Option<&str>) -> Result<PathBuf, String> {
    let trimmed = path.unwrap_or("").trim();
    let mut directory = if trimmed.is_empty() {
        home_directory()
    } else if trimmed == "~" {
        home_directory()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        match home_directory_opt() {
            Some(home) => home.join(rest),
            None => PathBuf::from(trimmed),
        }
    } else {
        PathBuf::from(trimmed)
    };
    if let Ok(canonical) = std::fs::canonicalize(&directory) {
        directory = canonical;
    }
    Ok(directory)
}

fn home_directory() -> PathBuf {
    home_directory_opt().unwrap_or_else(|| {
        if cfg!(windows) {
            PathBuf::from(r"C:\")
        } else {
            PathBuf::from("/")
        }
    })
}

fn home_directory_opt() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path_without_home_expansion("USERPROFILE")
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                let combined = format!("{}{}", drive.trim(), path.trim());
                if combined.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(combined))
                }
            })
            .or_else(|| env_path_without_home_expansion("HOME"))
    } else {
        env_path_without_home_expansion("HOME")
    }
}

fn env_path_without_home_expansion(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn cli_frontend_compat_endpoint_response(
    endpoint: &str,
    params: &Value,
    codex_home: &str,
    workspace_name: &str,
) -> Option<Value> {
    let params = codex_fetch_endpoint_params(params);
    match endpoint {
        "account-info" => Some(cli_account_info_response(codex_home, workspace_name)),
        "active-workspace-roots" => Some(json!({ "roots": [] })),
        "ambient-suggestions" => Some(json!({
            "file": {
                "currentSuggestionIds": [],
                "suggestions": [],
            },
        })),
        "codex-command-keymap-state" => Some(Value::Null),
        "codex-home" => Some(json!({ "codexHome": codex_home })),
        "ide-context" => Some(json!({ "ideContext": Value::Null })),
        "extension-info" => Some(json!({
            "name": "CodexL",
            "version": env!("CARGO_PKG_VERSION"),
            "buildFlavor": "cli-remote",
        })),
        "get-setting" => Some(json!({ "value": Value::Null })),
        "get-configuration" => Some(json!({ "value": Value::Null })),
        "get-copilot-api-proxy-info" => Some(Value::Null),
        "home-directory" => Some(json!({
            "homeDirectory": home_directory_opt()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
        })),
        "inbox-items" => Some(json!({ "items": [] })),
        "is-copilot-api-available" => Some(json!(false)),
        "list-automations" => Some(json!({ "items": [] })),
        "list-pending-automation-run-threads" => Some(json!({ "threadIds": [] })),
        "list-pinned-threads" => Some(json!({ "threadIds": [] })),
        "locale-info" => Some(cli_locale_info_response()),
        "mcp-codex-config" => Some(json!({ "config": {} })),
        "os-info" => Some(json!({
            "platform": codex_platform(),
            "arch": std::env::consts::ARCH,
        })),
        "paths-exist" => Some(cli_paths_exist_response(params)),
        "projectless-workspace-root" => Some(json!({
            "workspaceRoot": Value::Null,
        })),
        "set-configuration" => Some(json!({
            "value": params.get("value").cloned().unwrap_or(Value::Null),
        })),
        "set-setting" => Some(json!({
            "value": params.get("value").cloned().unwrap_or(Value::Null),
        })),
        "set-remote-control-connections-enabled" => Some(Value::Null),
        "workspace-root-options" => Some(json!({
            "roots": [],
            "labels": {},
        })),
        "worktree-shell-environment-config" => Some(json!({ "shellEnvironment": Value::Null })),
        "git-origins" => Some(json!({ "origins": [] })),
        "developer-instructions" => {
            let instructions = params
                .get("baseInstructions")
                .and_then(Value::as_str)
                .unwrap_or("");
            Some(json!({ "instructions": instructions }))
        }
        _ => None,
    }
}

fn cli_external_fetch_response(url: &str, codex_home: &str, workspace_name: &str) -> Option<Value> {
    if url.starts_with("/wham/accounts/check") {
        return Some(cli_wham_accounts_check_response(codex_home, workspace_name));
    }
    if url.starts_with("/wham/tasks/list") {
        return Some(json!({ "items": [] }));
    }
    if url.starts_with("/wham/usage") {
        return Some(Value::Null);
    }
    if cli_is_statsig_initialize_url(url) {
        return Some(cli_statsig_initialize_response());
    }
    if cli_is_statsig_event_url(url) {
        return Some(json!({ "success": true }));
    }
    None
}

fn cli_account_info_response(codex_home: &str, workspace_name: &str) -> Value {
    let auth = cli_middleware::ChatGptAuth::load_for_codex_home(codex_home, workspace_name);
    let account = auth.account_read_result();
    let account = account.get("account").unwrap_or(&Value::Null);
    let email = account
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("codex");
    let plan = account
        .get("planType")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    json!({
        "account_id": email,
        "account_user_id": email,
        "email": email,
        "id": email,
        "plan": plan,
        "plan_type": plan,
        "profile_picture_url": Value::Null,
        "structure": "personal",
    })
}

fn cli_wham_accounts_check_response(codex_home: &str, workspace_name: &str) -> Value {
    let account = cli_account_info_response(codex_home, workspace_name);
    json!({
        "accounts": [account.clone()],
        "current_account": account,
    })
}

fn cli_is_statsig_initialize_url(url: &str) -> bool {
    matches!(
        statsig_url_host_and_path(url).as_deref(),
        Some("ab.chatgpt.com/v1/initialize")
            | Some("featureassets.org/v1/initialize")
            | Some("api.statsig.com/v1/initialize")
    )
}

fn cli_is_statsig_event_url(url: &str) -> bool {
    matches!(
        statsig_url_host_and_path(url).as_deref(),
        Some("chatgpt.com/ces/v1/rgstr")
            | Some("ab.chatgpt.com/v1/rgstr")
            | Some("prodregistryv2.org/v1/rgstr")
            | Some("ab.chatgpt.com/v1/sdk_exception")
    )
}

fn statsig_url_host_and_path(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let path = path.split('?').next().unwrap_or(path).trim_matches('/');
    Some(format!("{}/{}", host, path))
}

fn cli_statsig_initialize_response() -> Value {
    json!({
        "has_updates": true,
        "time": now_millis(),
        "feature_gates": {
            "4100906017": cli_enabled_feature_gate("4100906017"),
            "2380644311": cli_enabled_feature_gate("2380644311"),
            "1506311413": cli_enabled_feature_gate("1506311413"),
        },
        "dynamic_configs": {},
        "layer_configs": {},
        "param_stores": {},
        "exposures": {},
        "sdk_flags": {},
        "hash_used": "djb2",
        "full_checksum": "",
        "derived_fields": {},
    })
}

fn cli_enabled_feature_gate(name: &str) -> Value {
    json!({
        "name": name,
        "value": true,
        "rule_id": "codexl-cli-enabled",
        "secondary_exposures": [],
        "id_type": "userID",
    })
}

fn cli_paths_exist_response(params: &Value) -> Value {
    let paths = params
        .get("paths")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            params
                .get("path")
                .and_then(Value::as_str)
                .map(|path| vec![json!(path)])
        })
        .unwrap_or_default();
    let mut by_path = Map::new();
    let mut exists = Vec::new();
    let mut existing_paths = Vec::new();
    for path in paths {
        let Some(path) = path.as_str() else {
            continue;
        };
        let exists_value = Path::new(path).exists();
        by_path.insert(path.to_string(), json!(exists_value));
        exists.push(json!(exists_value));
        if exists_value {
            existing_paths.push(json!(path));
        }
    }
    json!({
        "existingPaths": existing_paths,
        "paths": by_path,
        "exists": exists,
        "allExist": exists.iter().all(|value| value.as_bool().unwrap_or(false)),
    })
}

fn cli_locale_info_response() -> Value {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .and_then(|value| cli_normalize_locale_tag(&value))
        })
        .unwrap_or_else(|| "en-US".to_string());
    json!({
        "ideLocale": locale,
        "systemLocale": locale,
    })
}

fn cli_normalize_locale_tag(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let base = raw
        .split(':')
        .next()
        .unwrap_or(raw)
        .split('.')
        .next()
        .unwrap_or(raw)
        .split('@')
        .next()
        .unwrap_or(raw)
        .trim();
    if base.is_empty() {
        return None;
    }
    let lower = base.to_ascii_lowercase();
    if lower == "c" || lower == "posix" {
        return None;
    }
    let tag = base.replace('_', "-");
    if !tag
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return None;
    }
    let parts = tag
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let language = parts.first()?;
    if !(2..=8).contains(&language.len()) || !language.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }

    let normalized = parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_ascii_lowercase()
            } else if part.len() == 2 && part.chars().all(|ch| ch.is_ascii_alphabetic()) {
                part.to_ascii_uppercase()
            } else if part.len() == 3 && part.chars().all(|ch| ch.is_ascii_digit()) {
                part.to_string()
            } else if part.len() == 4 && part.chars().all(|ch| ch.is_ascii_alphabetic()) {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => {
                        format!(
                            "{}{}",
                            first.to_ascii_uppercase(),
                            chars.as_str().to_ascii_lowercase()
                        )
                    }
                    None => String::new(),
                }
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("-");
    Some(normalized)
}

fn codex_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        "linux" => "linux",
        other => other,
    }
}

fn cli_global_state_key(params: &Value) -> Result<String, String> {
    codex_fetch_endpoint_params(params)
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "global state request missing key".to_string())
}

fn cli_global_state_get_response(
    store: &HashMap<String, Value>,
    params: &Value,
) -> Result<Value, String> {
    let key = cli_global_state_key(params)?;
    Ok(json!({ "value": store.get(&key).cloned().unwrap_or(Value::Null) }))
}

fn cli_global_state_get_response_for_key(
    store: &HashMap<String, Value>,
    key: &str,
    params: &Value,
    codex_home: &str,
) -> Result<Value, String> {
    let parsed_key = cli_global_state_key(params)?;
    if parsed_key != key {
        return Err("global state key mismatch".to_string());
    }
    let stored = store.get(key).cloned().unwrap_or(Value::Null);
    let value = match key {
        CLI_PROJECTLESS_THREAD_IDS_KEY => cli_projectless_thread_ids_global_value(
            stored,
            cli_projectless_threads_from_sessions(codex_home)
                .into_iter()
                .map(|thread| thread.conversation_id),
        ),
        CLI_THREAD_WORKSPACE_ROOT_HINTS_KEY => cli_thread_workspace_root_hints_global_value(
            stored,
            cli_projectless_threads_from_sessions(codex_home),
        ),
        _ => return cli_global_state_get_response(store, params),
    };
    Ok(json!({ "value": value }))
}

fn cli_global_state_set_response(
    store: &mut HashMap<String, Value>,
    params: &Value,
) -> Result<Value, String> {
    let endpoint_params = codex_fetch_endpoint_params(params);
    let key = cli_global_state_key(endpoint_params)?;
    match endpoint_params.get("value").cloned() {
        Some(value) if !value.is_null() => {
            store.insert(key, value.clone());
            Ok(json!({ "value": value }))
        }
        _ => {
            store.remove(&key);
            Ok(json!({ "value": Value::Null }))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliProjectlessThreadMetadata {
    conversation_id: String,
    cwd: String,
    workspace_root: String,
}

fn decorate_cli_app_server_response(method: &str, response: &mut Value) {
    match method {
        "thread/list" => {
            if let Some(threads) = response
                .get_mut("result")
                .and_then(|result| result.get_mut("data"))
                .and_then(Value::as_array_mut)
            {
                for thread in threads {
                    decorate_cli_thread_metadata(thread);
                }
            }
        }
        "thread/read" | "thread/resume" | "thread/start" => {
            if let Some(thread) = response
                .get_mut("result")
                .and_then(|result| result.get_mut("thread"))
            {
                decorate_cli_thread_metadata(thread);
            }
        }
        _ => {}
    }
}

fn decorate_cli_thread_metadata(thread: &mut Value) {
    let Some(cwd) = thread
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let Some(workspace_root) = cli_projectless_workspace_root_for_cwd(&cwd) else {
        return;
    };
    let Some(object) = thread.as_object_mut() else {
        return;
    };
    let workspace_root = workspace_root.to_string_lossy().to_string();
    object.insert("workspaceKind".to_string(), json!("projectless"));
    object
        .entry("workspaceBrowserRoot".to_string())
        .or_insert_with(|| json!(workspace_root.clone()));
    object
        .entry("workspaceRoot".to_string())
        .or_insert_with(|| json!(workspace_root));
    object
        .entry("projectlessOutputDirectory".to_string())
        .or_insert_with(|| json!(cwd));
}

fn cli_projectless_thread_ids_global_value<I>(stored: Value, discovered_ids: I) -> Value
where
    I: IntoIterator<Item = String>,
{
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    if let Value::Array(values) = stored {
        for value in values {
            let Some(id) = value.as_str().and_then(normalize_cli_conversation_id) else {
                continue;
            };
            if seen.insert(id.clone()) {
                ids.push(Value::String(id));
            }
        }
    }
    for id in discovered_ids {
        let Some(id) = normalize_cli_conversation_id(&id) else {
            continue;
        };
        if seen.insert(id.clone()) {
            ids.push(Value::String(id));
        }
    }
    Value::Array(ids)
}

fn cli_thread_workspace_root_hints_global_value(
    stored: Value,
    discovered_threads: Vec<CliProjectlessThreadMetadata>,
) -> Value {
    let mut hints = stored.as_object().cloned().unwrap_or_default();
    for thread in discovered_threads {
        hints
            .entry(thread.conversation_id)
            .or_insert_with(|| Value::String(thread.workspace_root));
    }
    Value::Object(hints)
}

fn normalize_cli_conversation_id(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    if id.starts_with("local:") {
        Some(id.to_string())
    } else {
        Some(format!("local:{}", id))
    }
}

fn cli_projectless_threads_from_sessions(codex_home: &str) -> Vec<CliProjectlessThreadMetadata> {
    let sessions_dir = Path::new(codex_home).join("sessions");
    let mut files = Vec::new();
    collect_cli_jsonl_files(&sessions_dir, &mut files);
    files.sort();

    let mut threads = Vec::new();
    let mut seen = HashSet::new();
    for file in files {
        let Some(thread) = cli_projectless_thread_from_session_file(&file) else {
            continue;
        };
        if seen.insert(thread.conversation_id.clone()) {
            threads.push(thread);
        }
    }
    threads
}

fn collect_cli_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cli_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn cli_projectless_thread_from_session_file(path: &Path) -> Option<CliProjectlessThreadMetadata> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = StdBufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    cli_projectless_thread_from_session_meta_line(&line)
}

fn cli_projectless_thread_from_session_meta_line(
    line: &str,
) -> Option<CliProjectlessThreadMetadata> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    let id = payload.get("id").and_then(Value::as_str)?;
    let cwd = payload.get("cwd").and_then(Value::as_str)?.trim();
    let workspace_root = cli_projectless_workspace_root_for_cwd(cwd)?;
    Some(CliProjectlessThreadMetadata {
        conversation_id: normalize_cli_conversation_id(id)?,
        cwd: cwd.to_string(),
        workspace_root: workspace_root.to_string_lossy().to_string(),
    })
}

fn cli_projectless_workspace_root_for_cwd(cwd: &str) -> Option<PathBuf> {
    let home = home_directory_opt()?;
    let workspace_root = home.join("Documents").join("Codex");
    if cli_is_projectless_cwd_under_root(Path::new(cwd.trim()), &workspace_root) {
        Some(workspace_root)
    } else {
        None
    }
}

fn cli_is_projectless_cwd_under_root(cwd: &Path, workspace_root: &Path) -> bool {
    let Ok(relative) = cwd.strip_prefix(workspace_root) else {
        return false;
    };
    let segments = relative
        .iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [date_slug] => cli_is_projectless_date_slug_segment(date_slug),
        [date, slug] => cli_is_projectless_date_segment(date) && !slug.trim().is_empty(),
        _ => false,
    }
}

fn cli_is_projectless_date_slug_segment(segment: &str) -> bool {
    let Some(date) = segment.get(..10) else {
        return false;
    };
    segment.as_bytes().get(10).is_some_and(|byte| *byte == b'-')
        && cli_is_projectless_date_segment(date)
        && segment.len() > 11
}

fn cli_is_projectless_date_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn json_response_result(response: Value) -> Result<Value, String> {
    if let Some(error) = response.get("error") {
        return Err(json_error_message(error));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

fn cli_auth_method_result(
    method: &str,
    params: &Value,
    codex_home: &str,
    workspace_name: &str,
) -> Option<Value> {
    let auth = cli_middleware::ChatGptAuth::load_for_codex_home(codex_home, workspace_name);
    match method {
        "account/read" => Some(auth.account_read_result()),
        "getAuthStatus" => {
            let include_token = params
                .get("includeToken")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(auth.auth_status_result(include_token))
        }
        _ => None,
    }
}

fn response_error_is_method_not_found(response: &Value) -> bool {
    let Some(error) = response.get("error") else {
        return false;
    };
    if error.get("code").and_then(Value::as_i64) == Some(-32601) {
        return true;
    }
    json_error_message(error)
        .to_ascii_lowercase()
        .contains("method not found")
}

fn cli_thread_missing_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("thread not found")
        || error.contains("thread not loaded")
        || error.contains("thread/turns/list is unavailable before first user message")
        || error.contains("includeturns is unavailable before first user message")
}

fn create_cli_projectless_thread_cwd(
    directory_name: Option<&str>,
    prompt: &str,
) -> Result<Value, String> {
    let home = home_directory_opt().ok_or_else(|| "home directory is not set".to_string())?;
    let seconds = now_millis() / 1000;
    let slug_source = directory_name.unwrap_or(prompt);
    let (cwd, workspace_root) = projectless_thread_paths_for_home(&home, slug_source, seconds);
    std::fs::create_dir_all(&cwd).map_err(|err| {
        format!(
            "failed to create projectless Codex session directory {}: {}",
            cwd.to_string_lossy(),
            err
        )
    })?;
    Ok(projectless_thread_cwd_response(&cwd, &workspace_root))
}

fn projectless_thread_cwd_response(cwd: &Path, workspace_root: &Path) -> Value {
    let cwd = cwd.to_string_lossy().to_string();
    let workspace_root = workspace_root.to_string_lossy().to_string();
    json!({
        "cwd": cwd.clone(),
        "workspaceRoot": workspace_root,
        "projectlessOutputDirectory": cwd.clone(),
        "outputDirectory": cwd,
    })
}

fn projectless_thread_paths_for_home(
    home: &Path,
    prompt: &str,
    seconds: u64,
) -> (PathBuf, PathBuf) {
    let (year, month, day) = utc_date_from_unix_seconds(seconds);
    let date = format!("{:04}-{:02}-{:02}", year, month, day);
    let mut slug = sanitize_projectless_path_segment(prompt);
    if slug.is_empty() {
        slug = "chat".to_string();
    }
    let workspace_root = home.join("Documents").join("Codex");
    let cwd = workspace_root
        .join(date)
        .join(format!("{}-{}", slug, seconds));
    (cwd, workspace_root)
}

fn sanitize_projectless_path_segment(text: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in text.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '.' | ':' | '/') {
            Some('-')
        } else {
            None
        };

        let Some(next) = next else {
            continue;
        };
        if next == '-' {
            if slug.is_empty() || last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        slug.push(next);
        if slug.len() >= 48 {
            break;
        }
    }
    slug.trim_matches('-').to_string()
}

fn utc_date_from_unix_seconds(seconds: u64) -> (i64, i64, i64) {
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn cli_thread_start_params(params: Value) -> Value {
    let source = strip_host_id(params);
    let source = source.as_object().cloned().unwrap_or_default();
    let mut output = Map::new();

    copy_json_field(&source, &mut output, "cwd");
    copy_json_field(&source, &mut output, "serviceTier");
    copy_json_field(&source, &mut output, "config");
    copy_json_field(&source, &mut output, "threadSource");
    copy_json_field(&source, &mut output, "model");
    copy_json_field(&source, &mut output, "modelProvider");
    copy_json_field(&source, &mut output, "reasoningEffort");
    copy_json_field(&source, &mut output, "workspaceKind");
    copy_json_field(&source, &mut output, "workspaceRoots");
    copy_json_field(&source, &mut output, "projectlessOutputDirectory");
    copy_json_field(&source, &mut output, "sandbox");
    copy_json_field(&source, &mut output, "baseInstructions");
    copy_json_field(&source, &mut output, "developerInstructions");
    copy_json_field(&source, &mut output, "personality");
    copy_json_field(&source, &mut output, "ephemeral");
    copy_json_field(&source, &mut output, "persistExtendedHistory");

    if let Some(additional) = source.get("additionalDeveloperInstructions") {
        output.insert("developerInstructions".to_string(), additional.clone());
    }

    ensure_cli_projectless_output_directory(&source, &mut output);

    copy_permission_fields(&source, &mut output);
    copy_collaboration_model_fields(&source, &mut output);

    output
        .entry("threadSource".to_string())
        .or_insert_with(|| json!("user"));
    output
        .entry("serviceName".to_string())
        .or_insert_with(|| json!("codexl_remote_cli"));
    output
        .entry("ephemeral".to_string())
        .or_insert_with(|| json!(false));
    output
        .entry("personality".to_string())
        .or_insert_with(|| json!("pragmatic"));

    Value::Object(output)
}

fn cli_app_server_method_params(method: &str, params: Value) -> Value {
    match method {
        "thread/start" => cli_thread_start_params(params),
        "thread/resume" => cli_thread_resume_params(params),
        "turn/start" => cli_turn_start_params_for_app_server(params),
        _ => params,
    }
}

fn normalize_cli_app_server_request(request: &mut Value) {
    let Value::Object(map) = request else {
        return;
    };
    let Some(method) = map
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    if !matches!(
        method.as_str(),
        "thread/start" | "thread/resume" | "turn/start"
    ) {
        return;
    }
    let params = map.remove("params").unwrap_or(Value::Null);
    map.insert(
        "params".to_string(),
        cli_app_server_method_params(&method, params),
    );
}

fn ensure_cli_projectless_output_directory(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
) {
    if source.get("workspaceKind").and_then(Value::as_str) != Some("projectless") {
        return;
    }

    let output_directory = target
        .get("projectlessOutputDirectory")
        .and_then(Value::as_str)
        .or_else(|| {
            source
                .get("projectlessOutputDirectory")
                .and_then(Value::as_str)
        })
        .or_else(|| source.get("outputDirectory").and_then(Value::as_str))
        .or_else(|| source.get("cwd").and_then(Value::as_str))
        .or_else(|| {
            source
                .get("workspaceRoots")
                .and_then(Value::as_array)
                .and_then(|roots| roots.first())
                .and_then(Value::as_str)
        })
        .map(str::to_string);

    let Some(output_directory) = output_directory else {
        return;
    };

    target.insert(
        "projectlessOutputDirectory".to_string(),
        json!(output_directory),
    );
    target
        .entry("cwd".to_string())
        .or_insert_with(|| json!(output_directory));
    append_developer_instruction(
        target,
        format!(
            "When using local files for this projectless thread, write scratch files, drafts, generated assets, and other outputs under {}. Do not write directly in the home directory unless the user explicitly asks.",
            output_directory
        ),
    );
}

fn append_developer_instruction(target: &mut Map<String, Value>, instruction: String) {
    let next = match target.get("developerInstructions").and_then(Value::as_str) {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{}\n\n{}", existing, instruction)
        }
        _ => instruction,
    };
    target.insert("developerInstructions".to_string(), json!(next));
}

fn cli_thread_resume_params(params: Value) -> Value {
    let source = strip_host_id(params);
    let source = source.as_object().cloned().unwrap_or_default();
    let mut output = Map::new();

    copy_json_field(&source, &mut output, "threadId");
    if !output.contains_key("threadId") {
        if let Some(conversation_id) = source
            .get("conversationId")
            .filter(|value| !value.is_null())
        {
            output.insert("threadId".to_string(), conversation_id.clone());
        }
    }
    copy_json_field(&source, &mut output, "cwd");
    copy_json_field(&source, &mut output, "path");
    copy_json_field(&source, &mut output, "history");
    copy_json_field(&source, &mut output, "serviceTier");
    copy_json_field(&source, &mut output, "config");
    copy_json_field(&source, &mut output, "model");
    copy_json_field(&source, &mut output, "modelProvider");
    copy_json_field(&source, &mut output, "reasoningEffort");
    copy_json_field(&source, &mut output, "workspaceKind");
    copy_json_field(&source, &mut output, "workspaceRoots");
    copy_json_field(&source, &mut output, "projectlessOutputDirectory");
    copy_json_field(&source, &mut output, "sandbox");
    copy_json_field(&source, &mut output, "baseInstructions");
    copy_json_field(&source, &mut output, "developerInstructions");
    copy_json_field(&source, &mut output, "personality");
    copy_json_field(&source, &mut output, "excludeTurns");
    copy_json_field(&source, &mut output, "persistExtendedHistory");
    copy_permission_fields(&source, &mut output);
    copy_collaboration_model_fields(&source, &mut output);

    Value::Object(output)
}

fn cli_turn_start_params(thread_id: &str, params: Value, fallback_model: Option<Value>) -> Value {
    let source = strip_host_id(params);
    let mut source = source.as_object().cloned().unwrap_or_default();
    source.insert("threadId".to_string(), json!(thread_id));
    cli_turn_start_params_from_source(source, fallback_model)
}

fn cli_turn_start_params_for_app_server(params: Value) -> Value {
    let source = strip_host_id(params);
    let source = source.as_object().cloned().unwrap_or_default();
    cli_turn_start_params_from_source(source, None)
}

fn cli_turn_start_params_from_source(
    source: Map<String, Value>,
    fallback_model: Option<Value>,
) -> Value {
    let mut output = Map::new();

    copy_json_field(&source, &mut output, "threadId");
    copy_json_field(&source, &mut output, "cwd");
    copy_json_field(&source, &mut output, "input");
    copy_json_field(&source, &mut output, "attachments");
    copy_json_field(&source, &mut output, "commentAttachments");
    copy_json_field(&source, &mut output, "serviceTier");
    copy_json_field(&source, &mut output, "model");
    copy_json_field(&source, &mut output, "effort");
    copy_json_field(&source, &mut output, "reasoningEffort");
    copy_json_field(&source, &mut output, "workspaceKind");
    copy_json_field(&source, &mut output, "projectlessOutputDirectory");
    copy_permission_fields(&source, &mut output);
    copy_collaboration_model_fields(&source, &mut output);

    if !output.contains_key("model") {
        if let Some(model) = fallback_model {
            output.insert("model".to_string(), model);
        }
    }

    Value::Object(output)
}

fn cli_start_conversation_has_first_turn(params: &Value) -> bool {
    let input_len = params
        .get("input")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let attachments_len = params
        .get("attachments")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let comment_attachments_len = params
        .get("commentAttachments")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    input_len > 0 || attachments_len > 0 || comment_attachments_len > 0
}

fn cli_conversation_id_from_params(params: &Value) -> Result<String, String> {
    let params = codex_fetch_endpoint_params(params);
    ["conversationId", "threadId"]
        .iter()
        .find_map(|key| non_empty_json_string(params.get(*key)))
        .ok_or_else(|| "maybe-resume-conversation missing conversationId".to_string())
}

fn cli_maybe_resume_params(
    params: &Value,
    thread_metadata: Option<&Value>,
    conversation_id: &str,
) -> Map<String, Value> {
    let mut output = codex_fetch_endpoint_params(params)
        .as_object()
        .cloned()
        .unwrap_or_default();
    output.remove("hostId");
    output.remove("conversationId");
    output.insert("threadId".to_string(), json!(conversation_id));
    output.insert("excludeTurns".to_string(), json!(true));

    if !output.contains_key("path") {
        if let Some(path) = thread_metadata
            .and_then(|thread| non_empty_json_string(thread.get("path")))
            .or_else(|| non_empty_json_string(params.get("rolloutPath")))
        {
            output.insert("path".to_string(), json!(path));
        }
    }

    if !output.contains_key("cwd") {
        if let Some(cwd) = thread_metadata
            .and_then(|thread| non_empty_json_string(thread.get("cwd")))
            .or_else(|| first_workspace_root(params))
        {
            output.insert("cwd".to_string(), json!(cwd));
        }
    }

    if !output.contains_key("workspaceRoots") {
        if let Some(cwd) = non_empty_json_string(output.get("cwd")) {
            output.insert("workspaceRoots".to_string(), json!([cwd]));
        }
    }

    output
}

fn cli_conversation_snapshot_message(
    conversation_id: &str,
    resume_result: &Value,
    turns_page: &CliThreadTurnsPage,
    endpoint_params: &Value,
) -> Result<Value, String> {
    let thread = resume_result
        .get("thread")
        .ok_or_else(|| "thread/resume response missing thread".to_string())?;
    let model = non_empty_json_string(resume_result.get("model"))
        .or_else(|| non_empty_json_string(endpoint_params.get("model")))
        .or_else(|| {
            endpoint_params
                .get("collaborationMode")
                .and_then(|value| value.get("settings"))
                .and_then(|settings| non_empty_json_string(settings.get("model")))
        })
        .unwrap_or_default();
    let reasoning_effort = resume_result
        .get("reasoningEffort")
        .cloned()
        .or_else(|| endpoint_params.get("reasoningEffort").cloned())
        .or_else(|| {
            endpoint_params
                .get("collaborationMode")
                .and_then(|value| value.get("settings"))
                .and_then(|settings| settings.get("reasoning_effort"))
                .cloned()
        })
        .unwrap_or(Value::Null);
    let cwd = non_empty_json_string(resume_result.get("cwd"))
        .or_else(|| non_empty_json_string(thread.get("cwd")))
        .or_else(|| non_empty_json_string(endpoint_params.get("cwd")))
        .or_else(|| first_workspace_root(endpoint_params))
        .unwrap_or_else(|| "/".to_string());
    let approval_policy = cli_permission_value(resume_result, endpoint_params, "approvalPolicy")
        .unwrap_or_else(|| json!("on-request"));
    let approvals_reviewer =
        cli_permission_value(resume_result, endpoint_params, "approvalsReviewer")
            .unwrap_or_else(|| json!("user"));
    let sandbox_policy = resume_result
        .get("sandbox")
        .cloned()
        .or_else(|| cli_permission_value(resume_result, endpoint_params, "sandboxPolicy"))
        .unwrap_or_else(|| cli_default_sandbox_policy(&cwd));
    let turns = cli_conversation_turns_from_thread_turns(
        conversation_id,
        &turns_page.turns,
        &model,
        &reasoning_effort,
        &cwd,
        &approval_policy,
        &approvals_reviewer,
        &sandbox_policy,
    );
    let created_at = json_seconds_to_millis(thread.get("createdAt")).unwrap_or_else(now_millis);
    let updated_at = json_seconds_to_millis(thread.get("updatedAt")).unwrap_or(created_at);
    let title = non_empty_json_string(thread.get("name"))
        .or_else(|| non_empty_json_string(thread.get("preview")))
        .map(Value::String)
        .unwrap_or(Value::Null);
    let rollout_path = non_empty_json_string(thread.get("path"))
        .or_else(|| non_empty_json_string(endpoint_params.get("path")))
        .or_else(|| non_empty_json_string(endpoint_params.get("rolloutPath")))
        .unwrap_or_default();
    let inferred_projectless_workspace_root =
        cli_projectless_workspace_root_for_cwd(&cwd).map(|path| path.to_string_lossy().to_string());
    let workspace_kind = non_empty_json_string(endpoint_params.get("workspaceKind"))
        .or_else(|| non_empty_json_string(thread.get("workspaceKind")))
        .or_else(|| {
            inferred_projectless_workspace_root
                .as_ref()
                .map(|_| "projectless".to_string())
        })
        .unwrap_or_else(|| "project".to_string());
    let workspace_browser_root = endpoint_params
        .get("workspaceBrowserRoot")
        .cloned()
        .or_else(|| thread.get("workspaceBrowserRoot").cloned())
        .or_else(|| inferred_projectless_workspace_root.map(Value::String))
        .unwrap_or(Value::Null);
    let collaboration_mode = endpoint_params
        .get("collaborationMode")
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "mode": "default",
                "settings": {
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                    "developer_instructions": Value::Null,
                },
            })
        });
    let latest_collaboration_mode =
        cli_collaboration_mode_with_model(collaboration_mode, &model, &reasoning_effort);
    let forked_from_id = thread
        .get("forkedFromId")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or(Value::Null);

    let conversation_state = json!({
        "id": conversation_id,
        "forkedFromId": forked_from_id,
        "hostId": "local",
        "turns": turns,
        "requests": [],
        "createdAt": created_at,
        "updatedAt": updated_at,
        "title": title,
        "source": thread.get("source").cloned().unwrap_or(Value::Null),
        "modelProvider": thread
            .get("modelProvider")
            .cloned()
            .or_else(|| resume_result.get("modelProvider").cloned())
            .unwrap_or(Value::Null),
        "latestModel": model,
        "latestReasoningEffort": reasoning_effort,
        "previousTurnModel": Value::Null,
        "latestCollaborationMode": latest_collaboration_mode,
        "hasUnreadTurn": false,
        "threadGoal": Value::Null,
        "threadGoalResumeConfirmation": Value::Null,
        "completedThreadGoal": Value::Null,
        "threadRuntimeStatus": thread.get("status").cloned().unwrap_or(Value::Null),
        "rolloutPath": rollout_path,
        "cwd": cwd,
        "gitInfo": thread.get("gitInfo").cloned().unwrap_or(Value::Null),
        "resumeState": "resumed",
        "latestTokenUsageInfo": Value::Null,
        "workspaceKind": workspace_kind,
        "workspaceBrowserRoot": workspace_browser_root,
        "turnsPagination": {
            "olderCursor": turns_page.next_cursor.clone(),
            "isLoadingOlder": false,
            "hasLoadedOldest": turns_page.next_cursor.is_null(),
        },
    });

    Ok(json!({
        "type": "ipc-broadcast",
        "method": "thread-stream-state-changed",
        "sourceClientId": "codexl-remote-cli",
        "version": 6,
        "params": {
            "conversationId": conversation_id,
            "hostId": "local",
            "version": 6,
            "change": {
                "type": "snapshot",
                "conversationState": conversation_state,
            },
        },
    }))
}

fn cli_conversation_turns_from_thread_turns(
    thread_id: &str,
    turns: &[Value],
    model: &str,
    reasoning_effort: &Value,
    cwd: &str,
    approval_policy: &Value,
    approvals_reviewer: &Value,
    sandbox_policy: &Value,
) -> Value {
    Value::Array(
        turns
            .iter()
            .map(|turn| {
                cli_conversation_turn_from_thread_turn(
                    thread_id,
                    turn,
                    model,
                    reasoning_effort,
                    cwd,
                    approval_policy,
                    approvals_reviewer,
                    sandbox_policy,
                )
            })
            .collect(),
    )
}

fn cli_conversation_turn_from_thread_turn(
    thread_id: &str,
    turn: &Value,
    model: &str,
    reasoning_effort: &Value,
    cwd: &str,
    approval_policy: &Value,
    approvals_reviewer: &Value,
    sandbox_policy: &Value,
) -> Value {
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let input = items
        .first()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("userMessage"))
        .and_then(|item| item.get("content"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let normalized_items: Vec<Value> = items.iter().map(cli_normalize_thread_item).collect();

    json!({
        "params": {
            "threadId": thread_id,
            "input": input,
            "approvalPolicy": approval_policy.clone(),
            "approvalsReviewer": approvals_reviewer.clone(),
            "sandboxPolicy": sandbox_policy.clone(),
            "model": model,
            "cwd": cwd,
            "attachments": [],
            "effort": reasoning_effort.clone(),
            "summary": "none",
            "personality": Value::Null,
            "outputSchema": Value::Null,
            "collaborationMode": Value::Null,
        },
        "turnId": turn.get("id").cloned().unwrap_or(Value::Null),
        "turnStartedAtMs": json_seconds_to_millis_value(turn.get("startedAt")),
        "durationMs": turn.get("durationMs").cloned().unwrap_or(Value::Null),
        "finalAssistantStartedAtMs": json_seconds_to_millis_value(turn.get("completedAt")),
        "status": turn.get("status").cloned().unwrap_or_else(|| json!("completed")),
        "error": turn.get("error").cloned().unwrap_or(Value::Null),
        "diff": Value::Null,
        "items": normalized_items,
    })
}

fn cli_normalize_thread_item(item: &Value) -> Value {
    let mut output = item.clone();
    if output.get("type").and_then(Value::as_str) != Some("collabAgentToolCall") {
        return output;
    }
    let receiver_threads = output
        .get("receiverThreadIds")
        .and_then(Value::as_array)
        .map(|ids| {
            Value::Array(
                ids.iter()
                    .filter_map(|id| {
                        non_empty_json_string(Some(id)).map(|thread_id| {
                            json!({
                                "threadId": thread_id,
                                "thread": Value::Null,
                            })
                        })
                    })
                    .collect(),
            )
        })
        .unwrap_or_else(|| json!([]));
    if let Value::Object(map) = &mut output {
        map.entry("receiverThreads".to_string())
            .or_insert(receiver_threads);
    }
    output
}

fn cli_permission_value(result: &Value, params: &Value, key: &str) -> Option<Value> {
    result
        .get(key)
        .cloned()
        .or_else(|| {
            params
                .get("permissions")
                .and_then(|value| value.get(key))
                .cloned()
        })
        .or_else(|| params.get(key).cloned())
        .filter(|value| !value.is_null())
}

fn cli_default_sandbox_policy(cwd: &str) -> Value {
    json!({
        "type": "workspaceWrite",
        "writableRoots": [cwd],
    })
}

fn cli_collaboration_mode_with_model(
    mut collaboration_mode: Value,
    model: &str,
    reasoning_effort: &Value,
) -> Value {
    let Value::Object(map) = &mut collaboration_mode else {
        return json!({
            "mode": "default",
            "settings": {
                "model": model,
                "reasoning_effort": reasoning_effort.clone(),
                "developer_instructions": Value::Null,
            },
        });
    };
    map.entry("mode".to_string())
        .or_insert_with(|| json!("default"));
    let settings = map
        .entry("settings".to_string())
        .or_insert_with(|| json!({}));
    if let Value::Object(settings) = settings {
        settings.insert("model".to_string(), json!(model));
        settings.insert("reasoning_effort".to_string(), reasoning_effort.clone());
        settings
            .entry("developer_instructions".to_string())
            .or_insert(Value::Null);
    }
    collaboration_mode
}

fn json_seconds_to_millis(value: Option<&Value>) -> Option<u64> {
    let seconds = value.and_then(Value::as_f64)?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn json_seconds_to_millis_value(value: Option<&Value>) -> Value {
    json_seconds_to_millis(value)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn non_empty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn first_workspace_root(params: &Value) -> Option<String> {
    params
        .get("workspaceRoots")
        .and_then(Value::as_array)
        .and_then(|roots| {
            roots
                .iter()
                .find_map(|root| non_empty_json_string(Some(root)))
        })
}

fn cli_bridge_response_labels(value: &Value) -> String {
    value
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| cli_bridge_message_labels(messages))
        .unwrap_or_else(|| "-".to_string())
}

fn cli_bridge_message_labels(messages: &[Value]) -> String {
    if messages.is_empty() {
        return "-".to_string();
    }
    let mut labels = messages
        .iter()
        .take(8)
        .map(cli_bridge_message_label)
        .collect::<Vec<_>>();
    if messages.len() > labels.len() {
        labels.push(format!("+{}", messages.len() - labels.len()));
    }
    labels.join(",")
}

fn cli_bridge_message_label(message: &Value) -> String {
    let message_type = message.get("type").and_then(Value::as_str).unwrap_or("?");
    match message_type {
        "fetch" => bridge_fetch_request(message)
            .map(|request| format!("fetch:{}", request.endpoint))
            .unwrap_or_else(|| "fetch:?".to_string()),
        "fetch-response" => {
            let status = message
                .get("status")
                .and_then(Value::as_u64)
                .map(|status| format!(":{status}"))
                .unwrap_or_default();
            let error = message
                .get("error")
                .and_then(Value::as_str)
                .map(|error| {
                    let mut preview = error
                        .chars()
                        .map(|ch| if ch.is_control() { ' ' } else { ch })
                        .take(120)
                        .collect::<String>();
                    if error.chars().count() > preview.chars().count() {
                        preview.push_str("...");
                    }
                    format!(":{preview}")
                })
                .unwrap_or_default();
            format!(
                "fetch-response:{}:{}{}{}",
                message
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                message
                    .get("responseType")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                status,
                error
            )
        }
        "ipc-broadcast" => {
            let params = message.get("params");
            if message.get("method").and_then(Value::as_str) == Some("client-status-changed") {
                return format!(
                    "ipc-broadcast:client-status-changed:{}:{}:{}",
                    params
                        .and_then(|params| params.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    params
                        .and_then(|params| params.get("clientType"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    params
                        .and_then(|params| params.get("clientId"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                );
            }
            let change = params.and_then(|params| params.get("change"));
            let turn_count = change
                .and_then(|change| change.get("conversationState"))
                .and_then(|state| state.get("turns"))
                .and_then(Value::as_array)
                .map(|turns| format!(":turns={}", turns.len()))
                .unwrap_or_default();
            format!(
                "ipc-broadcast:{}:{}:{}{}",
                message.get("method").and_then(Value::as_str).unwrap_or(""),
                change
                    .and_then(|change| change.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                params
                    .and_then(|params| params.get("conversationId"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                turn_count
            )
        }
        "mcp-notification" => format!(
            "mcp-notification:{}",
            message.get("method").and_then(Value::as_str).unwrap_or("")
        ),
        "mcp-request" => format!(
            "mcp-request:{}",
            message
                .get("request")
                .and_then(|request| request.get("method"))
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "mcp-response" => format!(
            "mcp-response:{}",
            message
                .get("message")
                .and_then(|response| response.get("method"))
                .and_then(Value::as_str)
                .or_else(|| {
                    message
                        .get("message")
                        .and_then(|response| response.get("id"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("")
        ),
        other => other.to_string(),
    }
}

fn cli_log_message_preview(message: &Value) -> String {
    let raw = message.to_string();
    let mut preview = raw
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(1200)
        .collect::<String>();
    if raw.chars().count() > preview.chars().count() {
        preview.push_str("...");
    }
    preview
}

fn thread_id_from_thread_start_response(response: &Value) -> Option<String> {
    response
        .get("result")
        .and_then(|result| result.get("thread"))
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn thread_start_response_model(response: &Value) -> Option<Value> {
    response
        .get("result")
        .and_then(|result| result.get("model"))
        .cloned()
}

fn copy_json_field(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        if !value.is_null() {
            target.insert(key.to_string(), value.clone());
        }
    }
}

fn copy_permission_fields(source: &Map<String, Value>, target: &mut Map<String, Value>) {
    if let Some(permissions) = source.get("permissions").and_then(Value::as_object) {
        copy_json_field(permissions, target, "approvalPolicy");
        copy_json_field(permissions, target, "sandboxPolicy");
        copy_json_field(permissions, target, "approvalsReviewer");
    }
    copy_json_field(source, target, "approvalPolicy");
    copy_json_field(source, target, "sandboxPolicy");
    copy_json_field(source, target, "approvalsReviewer");
}

fn copy_collaboration_model_fields(source: &Map<String, Value>, target: &mut Map<String, Value>) {
    let Some(settings) = source
        .get("collaborationMode")
        .and_then(|value| value.get("settings"))
        .and_then(Value::as_object)
    else {
        return;
    };
    if !target.contains_key("model") {
        if let Some(model) = settings.get("model").filter(|value| !value.is_null()) {
            target.insert("model".to_string(), model.clone());
        }
    }
    if !target.contains_key("reasoningEffort") {
        if let Some(effort) = settings
            .get("reasoning_effort")
            .filter(|value| !value.is_null())
        {
            target.insert("reasoningEffort".to_string(), effort.clone());
        }
    }
}

fn fetch_response_body_success(request_id: &str, body: Value) -> Value {
    json!({
        "type": "fetch-response",
        "requestId": request_id,
        "responseType": "success",
        "status": 200,
        "headers": { "content-type": "application/json" },
        "bodyJsonString": body.to_string(),
    })
}

fn fetch_response_body_success_status(
    request_id: &str,
    status: u16,
    headers: Value,
    body: Value,
) -> Value {
    json!({
        "type": "fetch-response",
        "requestId": request_id,
        "responseType": "success",
        "status": status,
        "headers": headers,
        "bodyJsonString": body.to_string(),
    })
}

fn fetch_response_success(request_id: &str, app_request_id: &str, response: Value) -> Value {
    let body = match response.get("error") {
        Some(error) => json!({
            "requestId": app_request_id,
            "resultType": "error",
            "error": error,
            "type": "response",
        }),
        None => json!({
            "requestId": app_request_id,
            "result": response.get("result").cloned().unwrap_or(Value::Null),
            "resultType": "success",
            "type": "response",
        }),
    };
    json!({
        "type": "fetch-response",
        "requestId": request_id,
        "responseType": "success",
        "status": 200,
        "headers": { "content-type": "application/json" },
        "bodyJsonString": body.to_string(),
    })
}

fn fetch_response_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "fetch-response",
        "requestId": request_id,
        "responseType": "error",
        "status": 500,
        "error": error,
    })
}

fn fetch_response_error_status(request_id: &str, status: u16, error: &str) -> Value {
    json!({
        "type": "fetch-response",
        "requestId": request_id,
        "responseType": "error",
        "status": status,
        "error": error,
    })
}

fn local_device_name() -> String {
    static LOCAL_DEVICE_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LOCAL_DEVICE_NAME
        .get_or_init(|| {
            local_platform_device_name()
                .or_else(|| {
                    std::env::var("COMPUTERNAME")
                        .or_else(|_| std::env::var("HOSTNAME"))
                        .ok()
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Desktop".to_string())
        })
        .clone()
}

#[cfg(target_os = "macos")]
fn local_platform_device_name() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn local_platform_device_name() -> Option<String> {
    None
}

fn mobile_instance_item_name(device_name: &str, workspace_name: &str) -> String {
    let device = mobile_instance_name_segment(device_name);
    let workspace = mobile_instance_name_segment(workspace_name);
    match (device.is_empty(), workspace.is_empty()) {
        (false, false) => format!("{device}-{workspace}"),
        (false, true) => device,
        (true, false) => workspace,
        (true, true) => String::new(),
    }
}

fn mobile_instance_name_segment(value: &str) -> String {
    let mut trimmed = value.trim();
    if trimmed.len() >= ".local".len()
        && trimmed[trimmed.len() - ".local".len()..].eq_ignore_ascii_case(".local")
    {
        trimmed = &trimmed[..trimmed.len() - ".local".len()];
    }

    let mut segment = String::new();

    for ch in trimmed.chars() {
        if matches!(ch, '-' | '/' | '\\') {
            push_mobile_name_hyphen(&mut segment);
        } else if ch.is_control() || ch.is_whitespace() {
            push_mobile_name_space(&mut segment);
        } else {
            for lower in ch.to_lowercase() {
                segment.push(lower);
            }
        }
    }

    while matches!(segment.chars().last(), Some(' ' | '-')) {
        segment.pop();
    }
    segment
}

fn push_mobile_name_space(segment: &mut String) {
    if !segment.is_empty() && !matches!(segment.chars().last(), Some(' ' | '-')) {
        segment.push(' ');
    }
}

fn push_mobile_name_hyphen(segment: &mut String) {
    while matches!(segment.chars().last(), Some(' ' | '-')) {
        segment.pop();
    }
    if !segment.is_empty() {
        segment.push('-');
    }
}

enum BridgeEvent {
    Frame(FramePayload),
    Status(Value),
    Warning(String),
}

struct FramePayload {
    bytes: Vec<u8>,
    metadata: Value,
    metrics: ViewportMetrics,
    target: Option<CdpTarget>,
    ts: u64,
}

struct CdpBridge {
    config: RemoteServerConfig,
    active_screencast_profile: Mutex<Option<String>>,
    client_viewport: Mutex<Option<ClientViewport>>,
    connected: AtomicBool,
    desired_screencast_profile: Mutex<String>,
    event_tx: mpsc::UnboundedSender<BridgeEvent>,
    last_metrics: Mutex<Option<ViewportMetrics>>,
    next_id: AtomicU64,
    page_zoom_scale: Mutex<f64>,
    pending: Mutex<HashMap<u64, PendingCommand>>,
    screencast_active: AtomicBool,
    screencast_profile_mode: Mutex<String>,
    selected_target: Mutex<Option<CdpTarget>>,
    sender: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    stopped: AtomicBool,
    streaming_enabled: AtomicBool,
    target: Mutex<Option<CdpTarget>>,
}

struct PendingCommand {
    method: String,
    tx: oneshot::Sender<Result<Value, String>>,
}

impl CdpBridge {
    fn new(config: RemoteServerConfig, event_tx: mpsc::UnboundedSender<BridgeEvent>) -> Self {
        Self {
            config,
            active_screencast_profile: Mutex::new(None),
            client_viewport: Mutex::new(None),
            connected: AtomicBool::new(false),
            desired_screencast_profile: Mutex::new("good".to_string()),
            event_tx,
            last_metrics: Mutex::new(None),
            next_id: AtomicU64::new(1),
            page_zoom_scale: Mutex::new(DEFAULT_PAGE_ZOOM_SCALE),
            pending: Mutex::new(HashMap::new()),
            screencast_active: AtomicBool::new(false),
            screencast_profile_mode: Mutex::new("auto".to_string()),
            selected_target: Mutex::new(None),
            sender: Mutex::new(None),
            stopped: AtomicBool::new(false),
            streaming_enabled: AtomicBool::new(false),
            target: Mutex::new(None),
        }
    }

    fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            while !self.stopped.load(Ordering::Relaxed) {
                match self.connect_once().await {
                    Ok(()) => {}
                    Err(err) => {
                        let _ = self
                            .event_tx
                            .send(BridgeEvent::Warning(format!("CDP connect failed: {}", err)));
                    }
                }
                if self.stopped.load(Ordering::Relaxed) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(CONNECT_RETRY_MS)).await;
            }
        });
    }

    async fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(sender) = self.sender.lock().await.take() {
            let _ = sender.send(Message::Close(None));
        }
        self.reject_pending("CDP socket closed").await;
    }

    async fn connect_once(self: &Arc<Self>) -> Result<(), String> {
        let target = match self.selected_target.lock().await.clone() {
            Some(target) => target,
            None => select_target(&self.list_targets().await?)
                .ok_or_else(|| "no page target with webSocketDebuggerUrl".to_string())?,
        };

        *self.target.lock().await = Some(target.clone());
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));

        let (socket, _) = tokio_tungstenite::connect_async(&target.web_socket_debugger_url)
            .await
            .map_err(|e| e.to_string())?;
        let (mut write, mut read) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        *self.sender.lock().await = Some(tx);
        self.connected.store(true, Ordering::Relaxed);
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));

        let init_bridge = self.clone();
        tokio::spawn(async move {
            if let Err(err) = init_bridge.initialize_target().await {
                let _ = init_bridge.event_tx.send(BridgeEvent::Warning(err));
            }
        });

        loop {
            tokio::select! {
                outbound = rx.recv() => {
                    match outbound {
                        Some(Message::Close(frame)) => {
                            let _ = write.send(Message::Close(frame)).await;
                            break;
                        }
                        Some(message) => {
                            write.send(message).await.map_err(|e| e.to_string())?;
                        }
                        None => break,
                    }
                }
                inbound = read.next() => {
                    match inbound {
                        Some(Ok(message)) => self.handle_cdp_message(message).await,
                        Some(Err(err)) => return Err(err.to_string()),
                        None => break,
                    }
                }
            }
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
        }

        self.connected.store(false, Ordering::Relaxed);
        self.screencast_active.store(false, Ordering::Relaxed);
        *self.sender.lock().await = None;
        self.reject_pending("CDP socket closed").await;
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn initialize_target(&self) -> Result<(), String> {
        self.send("Page.enable", json!({})).await?;
        self.send("Runtime.enable", json!({})).await?;
        self.apply_client_viewport_override().await?;
        if self.streaming_enabled.load(Ordering::Relaxed) {
            self.start_screencast().await?;
        }
        Ok(())
    }

    async fn list_targets(&self) -> Result<Vec<CdpTarget>, String> {
        let url = format!(
            "http://{}:{}/json/list",
            self.config.cdp_host, self.config.cdp_port
        );
        reqwest::get(url)
            .await
            .map_err(|e| e.to_string())?
            .json::<Vec<CdpTarget>>()
            .await
            .map_err(|e| e.to_string())
    }

    async fn switch_target(&self, target_id: &str) -> Result<(), String> {
        let targets = self.list_targets().await?;
        let target = targets
            .into_iter()
            .find(|target| target.id == target_id)
            .ok_or_else(|| format!("CDP target not found: {}", target_id))?;
        *self.selected_target.lock().await = Some(target);
        if let Some(sender) = self.sender.lock().await.clone() {
            let _ = sender.send(Message::Close(None));
        }
        Ok(())
    }

    async fn status(&self) -> Value {
        let active_profile = self.active_screencast_profile.lock().await.clone();
        let desired_profile = self.desired_screencast_profile.lock().await.clone();
        let profile_mode = self.screencast_profile_mode.lock().await.clone();
        let page_zoom_scale = *self.page_zoom_scale.lock().await;
        let client_viewport = self.client_viewport.lock().await.clone();
        let profile = self.screencast_profile_for(&desired_profile, client_viewport.as_ref());
        json!({
            "cdpUrl": format!("http://{}:{}", self.config.cdp_host, self.config.cdp_port),
            "captureViewport": client_viewport.as_ref().map(|viewport| self.screencast_size_for_profile(&GOOD_PROFILE, Some(viewport))),
            "clientViewport": client_viewport,
            "connected": self.connected.load(Ordering::Relaxed),
            "network": {
                "bufferedAmount": 0,
                "droppedFramesInLast5s": 0,
                "frameClientCount": 0,
                "rtt": null,
            },
            "pageZoomScale": page_zoom_scale,
            "screencastActive": self.screencast_active.load(Ordering::Relaxed),
            "screencastProfile": active_profile.unwrap_or(desired_profile),
            "screencastProfileMode": profile_mode,
            "screencastProfileSettings": profile.to_json(),
            "streamingEnabled": self.streaming_enabled.load(Ordering::Relaxed),
            "target": self.target.lock().await.clone(),
            "viewportOverrideSuspended": false,
        })
    }

    async fn ready(&self) -> bool {
        self.connected.load(Ordering::Relaxed) && self.target.lock().await.is_some()
    }

    async fn set_screencast_enabled(&self, enabled: bool) -> Result<(), String> {
        self.streaming_enabled.store(enabled, Ordering::Relaxed);
        if !self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }
        if enabled {
            self.start_screencast().await
        } else {
            self.stop_screencast().await
        }
    }

    async fn restart_screencast(&self) -> Result<(), String> {
        self.streaming_enabled.store(true, Ordering::Relaxed);
        if self.connected.load(Ordering::Relaxed) {
            if self.screencast_active.load(Ordering::Relaxed) {
                let _ = self.stop_screencast().await;
            }
            self.start_screencast().await?;
        }
        Ok(())
    }

    async fn start_screencast(&self) -> Result<(), String> {
        if self.screencast_active.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.apply_client_viewport_override().await?;
        let desired_profile = self.desired_screencast_profile.lock().await.clone();
        let client_viewport = self.client_viewport.lock().await.clone();
        let profile = self.screencast_profile_for(&desired_profile, client_viewport.as_ref());
        self.send(
            "Page.startScreencast",
            json!({
                "everyNthFrame": profile.every_nth_frame,
                "format": "jpeg",
                "maxHeight": profile.max_height,
                "maxWidth": profile.max_width,
                "quality": profile.quality,
            }),
        )
        .await?;
        self.screencast_active.store(true, Ordering::Relaxed);
        *self.active_screencast_profile.lock().await = Some(profile.name.to_string());
        eprintln!(
            "Remote CDP screencast started ({}: {}x{}, q{}, every {})",
            profile.name,
            profile.max_width,
            profile.max_height,
            profile.quality,
            profile.every_nth_frame
        );
        if let Err(err) = self.capture_screenshot_frame().await {
            let _ = self.event_tx.send(BridgeEvent::Warning(format!(
                "screenshot fallback failed: {}",
                err
            )));
        }
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn stop_screencast(&self) -> Result<(), String> {
        if !self.screencast_active.load(Ordering::Relaxed) {
            return Ok(());
        }
        let _ = self.send("Page.stopScreencast", json!({})).await;
        self.screencast_active.store(false, Ordering::Relaxed);
        *self.active_screencast_profile.lock().await = None;
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn set_screencast_profile_mode(&self, mode: &str) -> Result<(), String> {
        let normalized = match mode {
            "good" | "medium" | "bad" => mode,
            _ => "auto",
        };
        *self.screencast_profile_mode.lock().await = normalized.to_string();
        let desired = if normalized == "auto" {
            "good"
        } else {
            normalized
        };
        *self.desired_screencast_profile.lock().await = desired.to_string();
        if self.screencast_active.load(Ordering::Relaxed) {
            self.restart_screencast().await?;
        }
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn set_page_zoom_scale(&self, scale: f64) -> Result<(), String> {
        let normalized = clamp(scale, MIN_PAGE_ZOOM_SCALE, MAX_PAGE_ZOOM_SCALE);
        *self.page_zoom_scale.lock().await = (normalized * 100.0).round() / 100.0;
        self.apply_client_viewport_override().await?;
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn set_client_viewport(&self, viewport: &Value) -> Result<(), String> {
        let Some(next_viewport) = ClientViewport::from_value(viewport) else {
            return Ok(());
        };
        let changed = {
            let mut current = self.client_viewport.lock().await;
            let changed = current
                .as_ref()
                .map(|current| !current.same_size(&next_viewport))
                .unwrap_or(true);
            if changed {
                *current = Some(next_viewport);
            }
            changed
        };
        if changed && self.screencast_active.load(Ordering::Relaxed) {
            self.restart_screencast().await?;
        } else if changed {
            self.apply_client_viewport_override().await?;
        }
        let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
        Ok(())
    }

    async fn apply_client_viewport_override(&self) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(viewport) = self.client_viewport.lock().await.clone() else {
            return Ok(());
        };
        let zoom_scale = *self.page_zoom_scale.lock().await;
        let size = self.screencast_size_for_profile(&GOOD_PROFILE, Some(&viewport));
        let emulated_width = ((size.width as f64) / zoom_scale).round().max(1.0) as u64;
        let emulated_height = ((size.height as f64) / zoom_scale).round().max(1.0) as u64;
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "deviceScaleFactor": zoom_scale,
                "height": emulated_height,
                "mobile": false,
                "screenHeight": emulated_height,
                "screenOrientation": {
                    "angle": if emulated_height >= emulated_width { 0 } else { 90 },
                    "type": if emulated_height >= emulated_width { "portraitPrimary" } else { "landscapePrimary" },
                },
                "screenWidth": emulated_width,
                "width": emulated_width,
            }),
        )
        .await?;
        *self.last_metrics.lock().await = None;
        Ok(())
    }

    async fn click_and_check_editable(
        &self,
        normalized_x: f64,
        normalized_y: f64,
    ) -> Result<bool, String> {
        let editable_at_point = self
            .is_editable_at(normalized_x, normalized_y)
            .await
            .unwrap_or(false);
        self.click(normalized_x, normalized_y).await?;
        Ok(editable_at_point || self.has_editable_focus().await.unwrap_or(false))
    }

    async fn click(&self, normalized_x: f64, normalized_y: f64) -> Result<(), String> {
        let point = self
            .point_from_normalized(normalized_x, normalized_y)
            .await?;
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "button": "left",
                "clickCount": 1,
                "type": "mousePressed",
                "x": point.0,
                "y": point.1,
            }),
        )
        .await?;
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "button": "left",
                "clickCount": 1,
                "type": "mouseReleased",
                "x": point.0,
                "y": point.1,
            }),
        )
        .await?;
        Ok(())
    }

    async fn pointer_move(&self, normalized_x: f64, normalized_y: f64) -> Result<(), String> {
        let point = self
            .point_from_normalized(normalized_x, normalized_y)
            .await?;
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "button": "none",
                "type": "mouseMoved",
                "x": point.0,
                "y": point.1,
            }),
        )
        .await?;
        Ok(())
    }

    async fn scroll(
        &self,
        normalized_x: f64,
        normalized_y: f64,
        delta_y: f64,
        delta_x: f64,
    ) -> Result<(), String> {
        let point = self
            .point_from_normalized(normalized_x, normalized_y)
            .await?;
        self.send(
            "Input.dispatchMouseEvent",
            json!({
                "deltaX": delta_x,
                "deltaY": delta_y,
                "type": "mouseWheel",
                "x": point.0,
                "y": point.1,
            }),
        )
        .await?;
        Ok(())
    }

    async fn is_editable_at(&self, normalized_x: f64, normalized_y: f64) -> Result<bool, String> {
        let point = self
            .point_from_normalized(normalized_x, normalized_y)
            .await?;
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": editable_probe_expression(point.0, point.1),
                    "returnByValue": true,
                }),
            )
            .await?;
        Ok(result
            .get("result")
            .and_then(|result| result.get("value"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn is_scrollable_at(
        &self,
        normalized_x: Option<f64>,
        normalized_y: Option<f64>,
    ) -> Result<bool, String> {
        let (Some(x), Some(y)) = (normalized_x, normalized_y) else {
            return Ok(false);
        };
        if !x.is_finite() || !y.is_finite() {
            return Ok(false);
        }
        let point = self.point_from_normalized(x, y).await?;
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": scrollable_probe_expression(point.0, point.1),
                    "returnByValue": true,
                }),
            )
            .await?;
        Ok(result
            .get("result")
            .and_then(|result| result.get("value"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn apply_sidebar_swipe(
        &self,
        direction: &str,
        start_x: Option<f64>,
        start_y: Option<f64>,
    ) -> Result<(), String> {
        let normalized_direction = if direction == "left" { "left" } else { "right" };
        let close_side = if normalized_direction == "right" {
            "right"
        } else {
            "left"
        };
        let open_side = if normalized_direction == "right" {
            "left"
        } else {
            "right"
        };

        let close_result = self
            .set_sidebar_with_result(close_side, "close", false)
            .await
            .unwrap_or_else(|_| json!({ "ok": false }));
        if bool_field(&close_result, "clicked") || bool_field(&close_result, "sideOpen") {
            return Ok(());
        }

        if self
            .is_scrollable_at(start_x, start_y)
            .await
            .unwrap_or(false)
        {
            return Ok(());
        }

        self.set_sidebar(open_side, "open").await
    }

    async fn set_sidebar(&self, side: &str, action: &str) -> Result<(), String> {
        self.set_sidebar_with_result(side, action, true)
            .await
            .map(|_| ())
    }

    async fn set_sidebar_with_result(
        &self,
        side: &str,
        action: &str,
        strict: bool,
    ) -> Result<Value, String> {
        let normalized_side = if side == "right" { "right" } else { "left" };
        let normalized_action = if action == "close" { "close" } else { "open" };
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "awaitPromise": true,
                    "expression": set_sidebar_expression(normalized_side, normalized_action),
                    "returnByValue": true,
                }),
            )
            .await?;
        let response = result
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or_else(|| json!({ "ok": false }));
        if strict && !bool_field(&response, "ok") {
            return Err(response
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("no matching sidebar control")
                .to_string());
        }
        Ok(response)
    }

    async fn insert_text(&self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        self.send("Input.insertText", json!({ "text": text }))
            .await?;
        Ok(())
    }

    async fn key(&self, key: &str) -> Result<(), String> {
        let event = key_event_for(key);
        self.send(
            "Input.dispatchKeyEvent",
            json!({
                "code": event.code,
                "key": event.key,
                "type": "keyDown",
                "windowsVirtualKeyCode": event.windows_virtual_key_code,
            }),
        )
        .await?;
        self.send(
            "Input.dispatchKeyEvent",
            json!({
                "code": event.code,
                "key": event.key,
                "type": "keyUp",
                "windowsVirtualKeyCode": event.windows_virtual_key_code,
            }),
        )
        .await?;
        Ok(())
    }

    async fn has_editable_focus(&self) -> Result<bool, String> {
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "awaitPromise": true,
                    "expression": editable_focus_expression(),
                    "returnByValue": true,
                }),
            )
            .await?;
        Ok(result
            .get("result")
            .and_then(|result| result.get("value"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    async fn point_from_normalized(
        &self,
        normalized_x: f64,
        normalized_y: f64,
    ) -> Result<(f64, f64), String> {
        let metrics = self.viewport_metrics().await?;
        Ok((
            clamp(normalized_x, 0.0, 1.0) * metrics.width,
            clamp(normalized_y, 0.0, 1.0) * metrics.height,
        ))
    }

    async fn viewport_metrics(&self) -> Result<ViewportMetrics, String> {
        match self.send("Page.getLayoutMetrics", json!({})).await {
            Ok(value) => {
                let viewport = value
                    .get("cssVisualViewport")
                    .or_else(|| value.get("visualViewport"))
                    .or_else(|| value.get("cssLayoutViewport"))
                    .or_else(|| value.get("layoutViewport"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let metrics = ViewportMetrics {
                    height: number_value(&viewport, "clientHeight")
                        .or_else(|| number_value(&viewport, "height"))
                        .unwrap_or(1.0)
                        .max(1.0),
                    scale: number_value(&viewport, "scale").unwrap_or(1.0),
                    width: number_value(&viewport, "clientWidth")
                        .or_else(|| number_value(&viewport, "width"))
                        .unwrap_or(1.0)
                        .max(1.0),
                    x: number_value(&viewport, "pageX")
                        .or_else(|| number_value(&viewport, "x"))
                        .unwrap_or(0.0),
                    y: number_value(&viewport, "pageY")
                        .or_else(|| number_value(&viewport, "y"))
                        .unwrap_or(0.0),
                };
                *self.last_metrics.lock().await = Some(metrics.clone());
                Ok(metrics)
            }
            Err(err) => self.last_metrics.lock().await.clone().ok_or(err),
        }
    }

    async fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        let sender = self
            .sender
            .lock()
            .await
            .clone()
            .ok_or_else(|| "CDP socket is not connected".to_string())?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            id,
            PendingCommand {
                method: method.to_string(),
                tx,
            },
        );
        if sender
            .send(Message::Text(
                json!({ "id": id, "method": method, "params": params }).to_string(),
            ))
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err("CDP socket is not connected".to_string());
        }

        match tokio::time::timeout(Duration::from_millis(COMMAND_TIMEOUT_MS), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("CDP socket closed".to_string()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("CDP command timed out: {}", method))
            }
        }
    }

    async fn handle_cdp_message(&self, message: Message) {
        let text = match message {
            Message::Text(text) => text,
            _ => return,
        };
        let value = match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => return,
        };

        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let pending = self.pending.lock().await.remove(&id);
            if let Some(pending) = pending {
                let result = if let Some(error) = value.get("error") {
                    Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or(&pending.method)
                        .to_string())
                } else {
                    Ok(value.get("result").cloned().unwrap_or_else(|| json!({})))
                };
                let _ = pending.tx.send(result);
            }
            return;
        }

        match value.get("method").and_then(Value::as_str) {
            Some("Page.screencastFrame") => self.handle_screencast_frame(&value).await,
            Some("Page.screencastVisibilityChanged") => {
                let _ = self.event_tx.send(BridgeEvent::Status(self.status().await));
            }
            _ => {}
        }
    }

    async fn handle_screencast_frame(&self, value: &Value) {
        let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
        if let Some(session_id) = params.get("sessionId").and_then(Value::as_u64) {
            self.send_no_wait(
                "Page.screencastFrameAck",
                json!({ "sessionId": session_id }),
            )
            .await;
        }
        if !self.streaming_enabled.load(Ordering::Relaxed) {
            return;
        }
        let data = match params.get("data").and_then(Value::as_str) {
            Some(data) => data,
            None => return,
        };
        let Some(bytes) = decode_base64(data) else {
            let _ = self.event_tx.send(BridgeEvent::Warning(
                "failed to decode screencast frame".to_string(),
            ));
            return;
        };
        let metadata = params.get("metadata").cloned().unwrap_or_else(|| json!({}));
        let metrics = metrics_from_metadata(
            &metadata,
            self.last_metrics
                .lock()
                .await
                .clone()
                .unwrap_or_else(ViewportMetrics::default),
        );
        *self.last_metrics.lock().await = Some(metrics.clone());
        let frame = FramePayload {
            bytes,
            metadata,
            metrics,
            target: self.target.lock().await.clone(),
            ts: now_millis(),
        };
        eprintln!(
            "Remote CDP screencast frame received: {} bytes",
            frame.bytes.len()
        );
        let _ = self.event_tx.send(BridgeEvent::Frame(frame));
    }

    async fn capture_screenshot_frame(&self) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed)
            || !self.streaming_enabled.load(Ordering::Relaxed)
        {
            return Ok(());
        }

        let metrics = self.viewport_metrics().await?;
        let desired_profile = self.desired_screencast_profile.lock().await.clone();
        let client_viewport = self.client_viewport.lock().await.clone();
        let profile = self.screencast_profile_for(&desired_profile, client_viewport.as_ref());
        let image_scale = (profile.max_width as f64 / metrics.width)
            .min(profile.max_height as f64 / metrics.height)
            .min(1.0);
        let mut params = json!({
            "captureBeyondViewport": false,
            "format": "jpeg",
            "quality": profile.quality,
        });
        if image_scale < 0.999 {
            params["clip"] = json!({
                "height": metrics.height,
                "scale": image_scale,
                "width": metrics.width,
                "x": metrics.x,
                "y": metrics.y,
            });
        }

        let screenshot = self.send("Page.captureScreenshot", params).await?;
        let data = screenshot
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing screenshot data".to_string())?;
        let bytes = decode_base64(data).ok_or_else(|| "failed to decode screenshot".to_string())?;
        let frame = FramePayload {
            bytes,
            metadata: json!({}),
            metrics,
            target: self.target.lock().await.clone(),
            ts: now_millis(),
        };
        eprintln!(
            "Remote CDP screenshot fallback frame received: {} bytes",
            frame.bytes.len()
        );
        let _ = self.event_tx.send(BridgeEvent::Frame(frame));
        Ok(())
    }

    async fn reject_pending(&self, reason: &str) {
        let pending = std::mem::take(&mut *self.pending.lock().await);
        for command in pending.into_values() {
            let _ = command.tx.send(Err(reason.to_string()));
        }
    }

    async fn send_no_wait(&self, method: &str, params: Value) {
        let sender = self.sender.lock().await.clone();
        if let Some(sender) = sender {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let _ = sender.send(Message::Text(
                json!({ "id": id, "method": method, "params": params }).to_string(),
            ));
        }
    }

    fn screencast_profile_for(
        &self,
        profile_name: &str,
        viewport: Option<&ClientViewport>,
    ) -> ScreenProfile {
        let base = match profile_name {
            "medium" => MEDIUM_PROFILE,
            "bad" => BAD_PROFILE,
            _ => GOOD_PROFILE,
        };
        let size = self.screencast_size_for_profile(&base, viewport);
        ScreenProfile {
            max_height: size.height,
            max_width: size.width,
            ..base
        }
    }

    fn screencast_size_for_profile(
        &self,
        profile: &ScreenProfile,
        viewport: Option<&ClientViewport>,
    ) -> ScreencastSize {
        let Some(viewport) = viewport else {
            return ScreencastSize {
                height: profile.max_height.min(DEFAULT_SCREENSHOT_MAX_HEIGHT),
                width: profile.max_width.min(DEFAULT_SCREENSHOT_MAX_WIDTH),
            };
        };

        let long_edge = profile.max_width.max(profile.max_height);
        let aspect = clamp(viewport.aspect, 0.25, 4.0);
        let (width, height) = if aspect >= 1.0 {
            (long_edge, ((long_edge as f64) / aspect).round() as u64)
        } else {
            (((long_edge as f64) * aspect).round() as u64, long_edge)
        };

        ScreencastSize {
            height: height.max(320),
            width: width.max(320),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CdpTarget {
    #[serde(default)]
    description: String,
    #[serde(default, rename = "devtoolsFrontendUrl")]
    devtools_frontend_url: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    #[serde(rename = "type")]
    target_type: String,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct ViewportMetrics {
    height: f64,
    scale: f64,
    width: f64,
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ClientViewport {
    aspect: f64,
    dpr: f64,
    height: u64,
    width: u64,
}

impl ClientViewport {
    fn from_value(value: &Value) -> Option<Self> {
        let width = number_value(value, "width")?;
        let height = number_value(value, "height")?;
        if !width.is_finite() || !height.is_finite() || width < 100.0 || height < 100.0 {
            return None;
        }
        let dpr = clamp(number_value(value, "dpr").unwrap_or(1.0), 1.0, 4.0);
        Some(Self {
            aspect: width / height,
            dpr,
            height: height.round() as u64,
            width: width.round() as u64,
        })
    }

    fn same_size(&self, other: &Self) -> bool {
        let width_delta = self.width.abs_diff(other.width);
        let height_delta = self.height.abs_diff(other.height);
        let aspect_delta = (self.aspect - other.aspect).abs();
        width_delta < 8 && height_delta < 8 && aspect_delta < 0.015
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ScreencastSize {
    height: u64,
    width: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ScreenProfile {
    #[serde(rename = "everyNthFrame")]
    every_nth_frame: u64,
    #[serde(rename = "maxHeight")]
    max_height: u64,
    #[serde(rename = "maxWidth")]
    max_width: u64,
    name: &'static str,
    quality: u64,
}

impl ScreenProfile {
    fn to_json(self) -> Value {
        json!({
            "everyNthFrame": self.every_nth_frame,
            "format": "jpeg",
            "maxHeight": self.max_height,
            "maxWidth": self.max_width,
            "name": self.name,
            "quality": self.quality,
        })
    }
}

impl Default for ViewportMetrics {
    fn default() -> Self {
        Self {
            height: 1.0,
            scale: 1.0,
            width: 1.0,
            x: 0.0,
            y: 0.0,
        }
    }
}

fn metrics_from_metadata(metadata: &Value, fallback: ViewportMetrics) -> ViewportMetrics {
    ViewportMetrics {
        height: number_value(metadata, "deviceHeight")
            .or_else(|| number_value(metadata, "height"))
            .unwrap_or(fallback.height)
            .max(1.0),
        scale: number_value(metadata, "pageScaleFactor").unwrap_or(fallback.scale),
        width: number_value(metadata, "deviceWidth")
            .or_else(|| number_value(metadata, "width"))
            .unwrap_or(fallback.width)
            .max(1.0),
        x: number_value(metadata, "scrollOffsetX").unwrap_or(fallback.x),
        y: number_value(metadata, "scrollOffsetY").unwrap_or(fallback.y),
    }
}

fn remote_http_path_requires_auth(path: &str) -> bool {
    path.starts_with("/api/") || path == "/web" || path.starts_with("/web/")
}

fn bearer_token(value: &str) -> Option<&str> {
    let value = value.trim();
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token.trim())
    } else {
        None
    }
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    for part in header.split(';') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        if key.trim() == name {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn referer_query_token(header: &str) -> Option<String> {
    let query = header.split('#').next()?.split_once('?')?.1;
    query_param(query, "token")
}

fn json_response(status: StatusCode, value: Value) -> Response<HttpBody> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn empty_response(status: StatusCode) -> Response<HttpBody> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_remote_server_config() -> RemoteServerConfig {
        RemoteServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3147,
            token: "remote-token".to_string(),
            relay_url: None,
            relay_connection_id: None,
            crypto: None,
            remote_frontend_mode: "app".to_string(),
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
            web_asset_base_url: None,
            web_asset_version: "latest".to_string(),
            cdp_host: "127.0.0.1".to_string(),
            cdp_port: 9222,
        }
    }

    #[test]
    fn bearer_token_parses_case_insensitive_scheme() {
        assert_eq!(bearer_token("Bearer secret"), Some("secret"));
        assert_eq!(bearer_token("bearer   secret"), Some("secret"));
        assert_eq!(bearer_token("Basic secret"), None);
        assert_eq!(bearer_token("Bearer"), None);
    }

    #[test]
    fn cli_app_server_preserves_ipc_broadcast_thread_snapshots() {
        let message = cli_app_server_value_to_bridge_message(
            "local",
            json!({
                "type": "ipc-broadcast",
                "method": "thread-stream-state-changed",
                "params": {
                    "conversationId": "thread-1",
                    "hostId": "local",
                    "change": {
                        "type": "snapshot",
                        "conversationState": {
                            "id": "thread-1",
                            "title": "Greeting received"
                        }
                    }
                }
            }),
        );

        assert_eq!(
            message.get("type").and_then(Value::as_str),
            Some("ipc-broadcast")
        );
        assert_eq!(
            message.get("method").and_then(Value::as_str),
            Some("thread-stream-state-changed")
        );
        assert_eq!(
            message
                .pointer("/params/change/conversationState/title")
                .and_then(Value::as_str),
            Some("Greeting received")
        );
    }

    #[tokio::test]
    async fn app_bridge_custom_transcribe_fetch_is_handled_locally() {
        let runtime = RemoteRuntimeState::new(test_remote_server_config());
        runtime
            .set_transcribe_api(TranscribeApiConfig {
                url: "not a valid url".to_string(),
                api_key: String::new(),
                model: DEFAULT_TRANSCRIBE_MODEL.to_string(),
            })
            .await;

        let response = runtime
            .dispatch_web_bridge_message(json!({
                "type": "fetch",
                "requestId": "transcribe-1",
                "method": "POST",
                "url": "/transcribe",
            }))
            .await
            .expect("app bridge response");
        let message = response
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| messages.first())
            .expect("fetch response");

        assert_eq!(
            message.get("type").and_then(Value::as_str),
            Some("fetch-response")
        );
        assert_eq!(
            message.get("requestId").and_then(Value::as_str),
            Some("transcribe-1")
        );
        assert_eq!(message.get("status").and_then(Value::as_u64), Some(500));
        assert!(message
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|error| error.contains("Invalid transcribe API URL")));
    }

    #[tokio::test]
    async fn app_bridge_custom_transcribe_socket_fetch_is_handled_locally() {
        let runtime = RemoteRuntimeState::new(test_remote_server_config());
        runtime
            .set_transcribe_api(TranscribeApiConfig {
                url: "not a valid url".to_string(),
                api_key: String::new(),
                model: DEFAULT_TRANSCRIBE_MODEL.to_string(),
            })
            .await;
        let raw = json!({
            "id": "socket-1",
            "message": {
                "type": "fetch",
                "requestId": "transcribe-2",
                "method": "POST",
                "url": "/transcribe",
            }
        })
        .to_string();

        let response = runtime
            .dispatch_app_web_bridge_socket_payload_with_emitter(&raw, |_| {})
            .await;
        let message = response
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| messages.first())
            .expect("fetch response");

        assert_eq!(response.get("id").and_then(Value::as_str), Some("socket-1"));
        assert_eq!(
            message.get("type").and_then(Value::as_str),
            Some("fetch-response")
        );
        assert_eq!(
            message.get("requestId").and_then(Value::as_str),
            Some("transcribe-2")
        );
        assert_eq!(message.get("status").and_then(Value::as_u64), Some(500));
    }

    #[test]
    fn cookie_value_finds_named_cookie() {
        assert_eq!(
            cookie_value(
                "theme=dark; codexl_remote_token=secret; path=/",
                REMOTE_AUTH_COOKIE_NAME
            )
            .as_deref(),
            Some("secret")
        );
        assert_eq!(
            cookie_value("codexl_remote_token_extra=secret", REMOTE_AUTH_COOKIE_NAME),
            None
        );
    }

    #[test]
    fn referer_query_token_finds_top_level_remote_token() {
        assert_eq!(
            referer_query_token(
                "http://127.0.0.1:3147/web/index.html?hostId=local&token=remote-token#hash"
            )
            .as_deref(),
            Some("remote-token")
        );
        assert_eq!(
            referer_query_token(
                "http://127.0.0.1:3147/web/index.html?codexBridgeUrl=ws%3A%2F%2Flocal%2Fweb%2F_bridge%3Ftoken%3Dnested"
            ),
            None
        );
    }

    #[test]
    fn remote_http_auth_scope_covers_control_surfaces() {
        assert!(remote_http_path_requires_auth("/api/status"));
        assert!(remote_http_path_requires_auth("/web"));
        assert!(remote_http_path_requires_auth("/web/_bridge"));
        assert!(remote_http_path_requires_auth("/web/assets/app.js"));
        assert!(!remote_http_path_requires_auth("/"));
        assert!(!remote_http_path_requires_auth("/app.js"));
    }

    #[test]
    fn remote_url_includes_web_asset_registry_metadata() {
        let config = RemoteServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3147,
            token: "remote-token".to_string(),
            relay_url: None,
            relay_connection_id: None,
            crypto: None,
            remote_frontend_mode: "cli".to_string(),
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
            web_asset_base_url: Some("https://assets.example.com/codex-web".to_string()),
            web_asset_version: "latest".to_string(),
            cdp_host: "127.0.0.1".to_string(),
            cdp_port: 9222,
        };

        let url = append_remote_web_asset_params(
            "http://127.0.0.1:3147/?token=remote-token".to_string(),
            &config,
        )
        .expect("web asset url");
        let parsed = reqwest::Url::parse(&url).expect("parse url");
        let pairs = parsed
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            pairs.get("webAssetBaseUrl").map(String::as_str),
            Some("https://assets.example.com/codex-web")
        );
        assert_eq!(
            pairs.get("webAssetVersion").map(String::as_str),
            Some("latest")
        );
        assert_eq!(
            pairs.get("webAssetMode").map(String::as_str),
            Some("registry")
        );
    }

    #[test]
    fn lan_remote_url_omits_cloud_relay_crypto_params() {
        let crypto = RemoteCrypto::from_password(Some("secret"), "connection-1")
            .expect("remote crypto")
            .map(Arc::new);
        let config = RemoteServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3147,
            token: "remote-token".to_string(),
            relay_url: Some("https://relay.example.com".to_string()),
            relay_connection_id: Some("connection-1".to_string()),
            crypto,
            remote_frontend_mode: "app".to_string(),
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
            web_asset_base_url: None,
            web_asset_version: "latest".to_string(),
            cdp_host: "127.0.0.1".to_string(),
            cdp_port: 9222,
        };

        let lan_url = append_remote_connection_params(
            "http://127.0.0.1:3147/?token=remote-token".to_string(),
            &config,
            false,
        )
        .expect("lan url");
        let lan_pairs = reqwest::Url::parse(&lan_url)
            .expect("parse lan url")
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<HashMap<_, _>>();

        assert_eq!(lan_pairs.get("requirePassword"), None);
        assert_eq!(lan_pairs.get("e2ee"), None);

        let cloud_url = append_remote_connection_params(
            "https://relay.example.com/?auth=cloud&token=connection-1".to_string(),
            &config,
            true,
        )
        .expect("cloud url");
        let cloud_pairs = reqwest::Url::parse(&cloud_url)
            .expect("parse cloud url")
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            cloud_pairs.get("requirePassword").map(String::as_str),
            Some("1")
        );
        assert_eq!(cloud_pairs.get("e2ee").map(String::as_str), Some("v1"));
    }

    #[test]
    fn cloud_remote_context_is_detected_from_query_metadata() {
        assert!(query_is_cloud_remote_context("auth=cloud&token=relay"));
        assert!(query_is_cloud_remote_context(
            "token=relay&cloudUser=user-1"
        ));
        assert!(query_is_cloud_remote_context("token=relay&jwt=jwt-1"));
        assert!(!query_is_cloud_remote_context("token=remote-token"));
    }

    #[test]
    fn cli_profile_uses_profile_web_asset_registry() {
        let app_config = AppConfig {
            remote_web_asset_registry_url: "https://global.example.com".to_string(),
            remote_web_asset_version: "global".to_string(),
            ..AppConfig::default()
        };
        let profile = ProviderProfile {
            remote_frontend_mode: config::REMOTE_FRONTEND_MODE_CLI.to_string(),
            remote_web_asset_registry_url: "https://profile.example.com/".to_string(),
            remote_web_asset_version: "26.513.31313".to_string(),
            ..ProviderProfile::default()
        };

        assert_eq!(
            profile_web_asset_base_url(&app_config, Some(&profile)).as_deref(),
            Some("https://profile.example.com")
        );
        assert_eq!(
            profile_web_asset_version(&app_config, Some(&profile)),
            "26.513.31313"
        );
    }

    #[test]
    fn claude_code_profile_uses_profile_web_asset_registry_and_claude_backend() {
        let app_config = AppConfig {
            remote_web_asset_registry_url: "https://global.example.com".to_string(),
            remote_web_asset_version: "global".to_string(),
            ..AppConfig::default()
        };
        let profile = ProviderProfile {
            remote_frontend_mode: config::REMOTE_FRONTEND_MODE_CLAUDE_CODE.to_string(),
            remote_web_asset_registry_url: "https://profile.example.com/".to_string(),
            remote_web_asset_version: "26.513.31313".to_string(),
            ..ProviderProfile::default()
        };

        assert_eq!(
            profile_web_asset_base_url(&app_config, Some(&profile)).as_deref(),
            Some("https://profile.example.com")
        );
        assert_eq!(
            profile_web_asset_version(&app_config, Some(&profile)),
            "26.513.31313"
        );
        assert!(profile_uses_claude_code_app_server(&profile));
    }

    #[test]
    fn app_profile_uses_profile_web_asset_registry_when_configured() {
        let app_config = AppConfig {
            remote_web_asset_registry_url: "https://global.example.com".to_string(),
            remote_web_asset_version: "global".to_string(),
            ..AppConfig::default()
        };
        let profile = ProviderProfile {
            remote_frontend_mode: "app".to_string(),
            remote_web_asset_registry_url: "https://profile.example.com".to_string(),
            remote_web_asset_version: "26.513.31313".to_string(),
            ..ProviderProfile::default()
        };

        assert_eq!(
            profile_web_asset_base_url(&app_config, Some(&profile)).as_deref(),
            Some("https://profile.example.com")
        );
        assert_eq!(
            profile_web_asset_version(&app_config, Some(&profile)),
            "26.513.31313"
        );
    }

    #[test]
    fn app_profile_without_web_asset_registry_uses_default_registry() {
        let app_config = AppConfig::default();
        let profile = ProviderProfile {
            remote_frontend_mode: "app".to_string(),
            ..ProviderProfile::default()
        };

        assert_eq!(
            profile_web_asset_base_url(&app_config, Some(&profile)).as_deref(),
            Some(DEFAULT_REMOTE_WEB_ASSET_REGISTRY_URL)
        );
        assert_eq!(
            profile_web_asset_version(&app_config, Some(&profile)),
            "latest"
        );
    }

    #[test]
    fn cli_bridge_fetch_parses_dynamic_codex_endpoint() {
        let request = bridge_fetch_request(&json!({
            "type": "fetch",
            "requestId": "fetch-1",
            "url": "vscode://codex/start-conversation",
            "body": "{\"hostId\":\"local\",\"cwd\":\"/tmp/project\"}",
        }))
        .expect("fetch request");

        assert_eq!(request.endpoint, "start-conversation");
        assert_eq!(
            request.body.get("hostId").and_then(Value::as_str),
            Some("local")
        );
        assert_eq!(
            request.body.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
    }

    #[test]
    fn cli_desktop_api_fetch_supports_only_transcribe_post() {
        assert!(cli_is_supported_desktop_api_fetch_request(&json!({
            "type": "fetch",
            "method": "POST",
            "url": "/transcribe",
        })));
        assert!(cli_is_supported_desktop_api_fetch_request(&json!({
            "type": "fetch",
            "method": "POST",
            "url": "http://192.168.1.10:3147/transcribe",
        })));
        assert!(cli_is_supported_desktop_api_fetch_request(&json!({
            "type": "fetch",
            "method": "POST",
            "url": "app://-/transcribe",
        })));
        assert!(!cli_is_supported_desktop_api_fetch_request(&json!({
            "type": "fetch",
            "method": "GET",
            "url": "/transcribe",
        })));
        assert!(!cli_is_supported_desktop_api_fetch_request(&json!({
            "type": "fetch",
            "method": "POST",
            "url": "https://example.com/other",
        })));
    }

    #[test]
    fn cli_desktop_api_fetch_init_decodes_base64_body_and_strips_internal_header() {
        let (headers, body) = cli_desktop_api_fetch_init(&json!({
            "headers": {
                "Content-Type": "audio/webm",
                "X-Codex-Base64": "1",
                "Authorization": "Bearer caller",
            },
            "body": BASE64_STANDARD.encode("audio-bytes"),
        }))
        .expect("fetch init");

        assert_eq!(body, b"audio-bytes");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0.as_str(), "content-type");
        assert_eq!(headers[0].1.to_str().ok(), Some("audio/webm"));
    }

    #[test]
    fn custom_transcribe_endpoint_url_appends_openai_path_to_base_url() {
        let url = custom_transcribe_endpoint_url("https://api.openai.com/v1")
            .expect("transcribe endpoint url");

        assert_eq!(
            url.as_str(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn custom_transcribe_endpoint_url_accepts_full_endpoint_url() {
        let url = custom_transcribe_endpoint_url("https://api.openai.com/v1/audio/transcriptions/")
            .expect("transcribe endpoint url");

        assert_eq!(
            url.as_str(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn cli_openai_transcribe_fetch_init_wraps_raw_audio_as_multipart() {
        let (headers, body) = cli_openai_transcribe_fetch_init(
            &json!({
                "headers": {
                    "Content-Type": "audio/webm;codecs=opus",
                    "X-Codex-Base64": "1",
                },
                "body": BASE64_STANDARD.encode("audio-bytes"),
            }),
            "whisper-1",
        )
        .expect("openai transcribe fetch init");
        let content_type = headers
            .iter()
            .find(|(name, _)| name == reqwest::header::CONTENT_TYPE)
            .and_then(|(_, value)| value.to_str().ok())
            .expect("multipart content-type");
        let boundary = cli_multipart_boundary(content_type).expect("boundary");
        let body = String::from_utf8(body).expect("utf8 multipart");

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(body.contains("name=\"file\"; filename=\"codex.webm\""));
        assert!(body.contains("Content-Type: audio/webm"));
        assert!(body.contains("audio-bytes"));
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1"));
        assert!(body.ends_with(&format!("--{}--\r\n", boundary)));
    }

    #[test]
    fn cli_openai_transcribe_fetch_init_appends_official_model_field() {
        let boundary = "codex-test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.webm\"\r\nContent-Type: audio/webm\r\n\r\naudio\r\n--{boundary}--\r\n"
        );
        let (_headers, body) = cli_openai_transcribe_fetch_init(
            &json!({
                "headers": {
                    "Content-Type": format!("multipart/form-data; boundary={}", boundary),
                    "X-Codex-Base64": "1",
                },
                "body": BASE64_STANDARD.encode(body),
            }),
            "whisper-1",
        )
        .expect("openai transcribe fetch init");
        let body = String::from_utf8(body).expect("utf8 multipart");

        assert!(body.contains("name=\"file\""));
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1"));
        assert!(body.ends_with(&format!("--{}--\r\n", boundary)));
    }

    #[test]
    fn cli_openai_transcribe_fetch_init_preserves_existing_model_field() {
        let boundary = "codex-test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.webm\"\r\n\r\naudio\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ncustom-model\r\n--{boundary}--\r\n"
        );
        let (_headers, body) = cli_openai_transcribe_fetch_init(
            &json!({
                "headers": {
                    "Content-Type": format!("multipart/form-data; boundary=\"{}\"", boundary),
                    "X-Codex-Base64": "1",
                },
                "body": BASE64_STANDARD.encode(body),
            }),
            "whisper-1",
        )
        .expect("openai transcribe fetch init");
        let body = String::from_utf8(body).expect("utf8 multipart");

        assert_eq!(body.matches("name=\"model\"").count(), 1);
        assert!(body.contains("custom-model"));
    }

    #[test]
    fn cli_chatgpt_account_id_from_access_token_reads_jwt_claim() {
        let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = BASE64_URL_SAFE_NO_PAD
            .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account-1"}}"#);
        let token = format!("{}.{}.", header, payload);

        assert_eq!(
            cli_chatgpt_account_id_from_access_token(&token),
            Some("account-1".to_string())
        );
    }

    #[test]
    fn cli_codex_fetch_endpoint_params_unwraps_app_action_body() {
        let body = json!({
            "params": {
                "prompt": "Hello Codex",
                "directoryName": "Manual Session Name"
            }
        });
        let params = codex_fetch_endpoint_params(&body);

        assert_eq!(
            params.get("prompt").and_then(Value::as_str),
            Some("Hello Codex")
        );
        assert_eq!(
            params.get("directoryName").and_then(Value::as_str),
            Some("Manual Session Name")
        );
    }

    #[test]
    fn cli_read_file_binary_response_returns_base64_contents() {
        let root = std::env::temp_dir().join(format!(
            "codexl-cli-read-file-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let file = root.join("image.png");
        std::fs::write(&file, b"hello").expect("write temp file");

        let response = cli_read_file_binary_response(&json!({
            "params": { "path": file.to_string_lossy() }
        }))
        .expect("read binary response");

        assert_eq!(
            response.get("contentsBase64").and_then(Value::as_str),
            Some("aGVsbG8=")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_workspace_root_endpoint_tracks_added_root() {
        let root = std::env::temp_dir().join(format!(
            "codexl-cli-workspace-root-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let canonical_root = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut store = HashMap::new();

        let (response, messages) = cli_add_workspace_root_endpoint_response(
            &mut store,
            &json!({ "params": { "root": root.to_string_lossy(), "setActive": true } }),
        )
        .expect("add workspace root");

        assert_eq!(response.get("success").and_then(Value::as_bool), Some(true));
        assert_eq!(
            cli_workspace_root_options_response(&store)
                .get("roots")
                .and_then(Value::as_array)
                .and_then(|roots| roots.first())
                .and_then(Value::as_str),
            Some(canonical_root.as_str())
        );
        assert_eq!(
            cli_active_workspace_roots_response(&store)
                .get("roots")
                .and_then(Value::as_array)
                .and_then(|roots| roots.first())
                .and_then(Value::as_str),
            Some(canonical_root.as_str())
        );
        assert!(messages.iter().any(|message| {
            message.get("type").and_then(Value::as_str) == Some("workspace-root-option-added")
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_remote_workspace_directory_entries_lists_directories() {
        let root = std::env::temp_dir().join(format!(
            "codexl-cli-remote-dir-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(root.join("Beta")).expect("create beta");
        std::fs::create_dir_all(root.join("alpha")).expect("create alpha");
        std::fs::write(root.join("note.txt"), b"note").expect("write note");
        let canonical_root = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let response = cli_remote_workspace_directory_entries_response(&json!({
            "params": {
                "directoryPath": root.to_string_lossy(),
                "directoriesOnly": true
            }
        }))
        .expect("directory entries");
        let names = response
            .get("entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["alpha", "Beta"]);
        assert_eq!(
            response.get("directoryPath").and_then(Value::as_str),
            Some(canonical_root.as_str())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_direct_fetch_response_body_is_not_ipc_wrapped() {
        let response = fetch_response_body_success(
            "fetch-1",
            json!({
                "cwd": "/Users/alice/Documents/Codex/1970-01-01/hello-0",
                "outputDirectory": "/Users/alice/Documents/Codex/1970-01-01/hello-0",
            }),
        );
        let body = response
            .get("bodyJsonString")
            .and_then(Value::as_str)
            .and_then(|body| serde_json::from_str::<Value>(body).ok())
            .expect("fetch response body");

        assert_eq!(
            body.get("outputDirectory").and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
        assert_eq!(body.get("result"), None);
        assert_eq!(body.get("resultType"), None);
    }

    #[test]
    fn cli_initialize_request_enables_experimental_api() {
        let request = cli_app_server_initialize_request();

        assert_eq!(
            request
                .get("params")
                .and_then(|params| params.get("capabilities"))
                .and_then(|capabilities| capabilities.get("experimentalApi"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn cli_app_server_connection_state_reports_connected() {
        let response = cli_app_server_connection_state_response(true);

        assert_eq!(
            response.get("state").and_then(Value::as_str),
            Some("connected")
        );
        assert!(response.get("error").is_some_and(Value::is_null));
        assert!(response
            .get("appServerVersion")
            .and_then(Value::as_str)
            .is_some_and(|version| !version.is_empty()));
        assert!(response
            .get("installedCodexVersion")
            .and_then(Value::as_str)
            .is_some_and(|version| !version.is_empty()));
    }

    #[test]
    fn cli_app_server_connection_events_cover_snapshot_state() {
        let messages = cli_app_server_connection_event_messages(false);

        assert_eq!(
            messages
                .first()
                .and_then(|message| message.get("type"))
                .and_then(Value::as_str),
            Some("codex-app-server-initialized")
        );
        assert_eq!(
            messages
                .get(1)
                .and_then(|message| message.get("type"))
                .and_then(Value::as_str),
            Some("codex-app-server-connection-changed")
        );
        assert_eq!(
            messages
                .get(1)
                .and_then(|message| message.get("transport"))
                .and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            messages
                .get(1)
                .and_then(|message| message.get("state"))
                .and_then(Value::as_str),
            Some("error")
        );
        assert_eq!(
            messages
                .get(1)
                .and_then(|message| message.get("error"))
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("connection-failed")
        );
    }

    #[test]
    fn cli_frontend_compat_endpoints_return_expected_shapes() {
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "get-copilot-api-proxy-info",
                &Value::Null,
                "/tmp/codex-home",
                "Default",
            ),
            Some(Value::Null)
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "mcp-codex-config",
                &Value::Null,
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "config": {} }))
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "ide-context",
                &json!({ "params": { "workspaceRoot": "/tmp/project" } }),
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "ideContext": Value::Null }))
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "get-setting",
                &json!({ "params": { "key": "theme" } }),
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "value": Value::Null }))
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "developer-instructions",
                &json!({ "params": { "baseInstructions": "base" } }),
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "instructions": "base" }))
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "worktree-shell-environment-config",
                &Value::Null,
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "shellEnvironment": Value::Null }))
        );
        assert_eq!(
            cli_frontend_compat_endpoint_response(
                "git-origins",
                &Value::Null,
                "/tmp/codex-home",
                "Default",
            ),
            Some(json!({ "origins": [] }))
        );
    }

    #[test]
    fn cli_statsig_initialize_response_has_required_shape() {
        let response = cli_external_fetch_response(
            "https://ab.chatgpt.com/v1/initialize?k=client-key",
            "/tmp/codex-home",
            "Default",
        )
        .expect("statsig response");

        assert_eq!(
            response.get("has_updates").and_then(Value::as_bool),
            Some(true)
        );
        let feature_gates = response
            .get("feature_gates")
            .and_then(Value::as_object)
            .expect("feature gates");
        assert_eq!(
            feature_gates
                .get("4100906017")
                .and_then(|gate| gate.get("value"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            feature_gates
                .get("2380644311")
                .and_then(|gate| gate.get("value"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            feature_gates
                .get("1506311413")
                .and_then(|gate| gate.get("value"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(response
            .get("dynamic_configs")
            .is_some_and(Value::is_object));
        assert!(response.get("layer_configs").is_some_and(Value::is_object));
    }

    #[test]
    fn cli_account_info_uses_workspace_fallback() {
        let response = cli_frontend_compat_endpoint_response(
            "account-info",
            &Value::Null,
            "/tmp/codex-home",
            "workspace-a",
        )
        .expect("account info");

        assert_eq!(
            response.get("email").and_then(Value::as_str),
            Some("workspace-a")
        );
        assert_eq!(
            response.get("plan").and_then(Value::as_str),
            Some("unknown")
        );

        let wham =
            cli_external_fetch_response("/wham/accounts/check", "/tmp/codex-home", "workspace-a")
                .expect("accounts check");
        assert_eq!(
            wham.get("accounts")
                .and_then(Value::as_array)
                .and_then(|accounts| accounts.first())
                .and_then(|account| account.get("email"))
                .and_then(Value::as_str),
            Some("workspace-a")
        );
    }

    #[test]
    fn cli_auth_method_result_reads_codex_home_auth_json() {
        let root = std::env::temp_dir().join(format!(
            "codexl-cli-auth-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let token = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL3Byb2ZpbGUiOnsiZW1haWwiOiJ1c2VyQGV4YW1wbGUuY29tIn0sImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3BsYW5fdHlwZSI6InBsdXMifX0.signature";
        std::fs::write(
            root.join("auth.json"),
            format!(
                r#"{{
  "auth_mode": "chatgpt",
  "tokens": {{
    "access_token": "{}",
    "id_token": "{}",
    "refresh_token": "refresh",
    "account_id": "account"
  }}
}}"#,
                token, token
            ),
        )
        .expect("write auth json");
        let codex_home = root.to_string_lossy();

        let account = cli_auth_method_result("account/read", &Value::Null, &codex_home, "cli")
            .expect("account result");
        assert_eq!(account["account"]["type"], "chatgpt");
        assert_eq!(account["account"]["email"], "user@example.com");
        assert_eq!(account["account"]["planType"], "plus");
        assert_eq!(account["requiresOpenaiAuth"], true);

        let status = cli_auth_method_result(
            "getAuthStatus",
            &json!({ "includeToken": true }),
            &codex_home,
            "cli",
        )
        .expect("auth status");
        assert_eq!(status["authMethod"], "chatgpt");
        assert_eq!(status["authToken"], token);
        assert_eq!(status["requiresOpenaiAuth"], true);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_auth_method_result_falls_back_to_workspace_name() {
        let root = std::env::temp_dir().join(format!(
            "codexl-cli-auth-empty-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let codex_home = root.to_string_lossy();

        let account =
            cli_auth_method_result("account/read", &Value::Null, &codex_home, "workspace-a")
                .expect("account result");
        assert_eq!(account["account"]["type"], "chatgpt");
        assert_eq!(account["account"]["email"], "workspace-a");
        assert_eq!(account["account"]["planType"], "unknown");
        assert_eq!(account["requiresOpenaiAuth"], true);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_locale_normalization_ignores_invalid_posix_locale() {
        assert_eq!(cli_normalize_locale_tag("C"), None);
        assert_eq!(cli_normalize_locale_tag("C.UTF-8"), None);
        assert_eq!(cli_normalize_locale_tag("POSIX"), None);
        assert_eq!(
            cli_normalize_locale_tag("en_US.UTF-8"),
            Some("en-US".to_string())
        );
        assert_eq!(
            cli_normalize_locale_tag("zh_CN.UTF-8"),
            Some("zh-CN".to_string())
        );
    }

    #[test]
    fn cli_paths_exist_response_includes_frontend_existing_paths() {
        let existing = std::env::current_dir()
            .expect("current dir")
            .to_string_lossy()
            .to_string();
        let missing = format!("{}/__codexl_missing_path_for_test__", existing);
        let response = cli_paths_exist_response(&json!({
            "paths": [existing, missing]
        }));

        assert_eq!(
            response
                .get("existingPaths")
                .and_then(Value::as_array)
                .and_then(|paths| paths.first())
                .and_then(Value::as_str),
            Some(existing.as_str())
        );
        assert_eq!(
            response.get("allExist").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn cli_global_state_get_and_set_round_trip() {
        let mut store = HashMap::new();

        assert_eq!(
            cli_global_state_get_response(
                &store,
                &json!({ "params": { "key": "projectless-thread-ids" } }),
            )
            .expect("get missing state"),
            json!({ "value": Value::Null })
        );
        assert_eq!(
            cli_global_state_set_response(
                &mut store,
                &json!({
                    "params": {
                        "key": "projectless-thread-ids",
                        "value": ["thread-1"]
                    }
                }),
            )
            .expect("set state"),
            json!({ "value": ["thread-1"] })
        );
        assert_eq!(
            cli_global_state_get_response(
                &store,
                &json!({ "params": { "key": "projectless-thread-ids" } }),
            )
            .expect("get stored state"),
            json!({ "value": ["thread-1"] })
        );
    }

    #[test]
    fn cli_global_state_set_without_value_removes_key() {
        let mut store = HashMap::from([(
            "thread-workspace-root-hints".to_string(),
            json!({ "thread-1": "/tmp/project" }),
        )]);

        assert_eq!(
            cli_global_state_set_response(
                &mut store,
                &json!({ "params": { "key": "thread-workspace-root-hints" } }),
            )
            .expect("remove missing value"),
            json!({ "value": Value::Null })
        );
        assert_eq!(
            cli_global_state_get_response(
                &store,
                &json!({ "params": { "key": "thread-workspace-root-hints" } }),
            )
            .expect("get removed state"),
            json!({ "value": Value::Null })
        );
    }

    #[test]
    fn cli_projectless_global_state_merges_discovered_ids() {
        let value = cli_projectless_thread_ids_global_value(
            json!(["local:thread-1", "thread-2"]),
            ["thread-2".to_string(), "thread-3".to_string()],
        );

        assert_eq!(
            value,
            json!(["local:thread-1", "local:thread-2", "local:thread-3"])
        );
    }

    #[test]
    fn cli_thread_workspace_root_hints_merge_discovered_projectless_threads() {
        let value = cli_thread_workspace_root_hints_global_value(
            json!({ "local:thread-1": "/tmp/project" }),
            vec![CliProjectlessThreadMetadata {
                conversation_id: "local:thread-2".to_string(),
                cwd: "/Users/alice/Documents/Codex/1970-01-01/hello-0".to_string(),
                workspace_root: "/Users/alice/Documents/Codex".to_string(),
            }],
        );

        assert_eq!(
            value,
            json!({
                "local:thread-1": "/tmp/project",
                "local:thread-2": "/Users/alice/Documents/Codex",
            })
        );
    }

    #[test]
    fn cli_projectless_cwd_matcher_accepts_codex_documents_patterns() {
        let root = Path::new("/Users/alice/Documents/Codex");

        assert!(cli_is_projectless_cwd_under_root(
            Path::new("/Users/alice/Documents/Codex/1970-01-01/hello-0"),
            root
        ));
        assert!(cli_is_projectless_cwd_under_root(
            Path::new("/Users/alice/Documents/Codex/1970-01-01-hello-0"),
            root
        ));
        assert!(!cli_is_projectless_cwd_under_root(
            Path::new("/Users/alice/projects/hello"),
            root
        ));
    }

    #[test]
    fn cli_projectless_session_meta_line_discovers_local_conversation_id() {
        let home = std::env::var("HOME").expect("HOME");
        let cwd = Path::new(&home)
            .join("Documents")
            .join("Codex")
            .join("1970-01-01")
            .join("hello-0");
        let line = json!({
            "type": "session_meta",
            "payload": {
                "id": "thread-1",
                "cwd": cwd,
                "originator": "codexl-remote-cli"
            }
        })
        .to_string();

        let metadata =
            cli_projectless_thread_from_session_meta_line(&line).expect("projectless metadata");
        assert_eq!(metadata.conversation_id, "local:thread-1");
        assert_eq!(
            metadata.workspace_root,
            Path::new(&home)
                .join("Documents")
                .join("Codex")
                .to_string_lossy()
        );
    }

    #[test]
    fn cli_decorates_projectless_thread_response_metadata() {
        let home = std::env::var("HOME").expect("HOME");
        let workspace_root = Path::new(&home).join("Documents").join("Codex");
        let cwd = workspace_root.join("1970-01-01").join("hello-0");
        let mut response = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "result": {
                "data": [{
                    "id": "thread-1",
                    "cwd": cwd,
                    "createdAt": 0,
                    "updatedAt": 0
                }],
                "nextCursor": Value::Null
            }
        });

        decorate_cli_app_server_response("thread/list", &mut response);
        let thread = response
            .get("result")
            .and_then(|result| result.get("data"))
            .and_then(Value::as_array)
            .and_then(|threads| threads.first())
            .expect("thread");

        assert_eq!(
            thread.get("workspaceKind").and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            thread.get("workspaceBrowserRoot").and_then(Value::as_str),
            Some(workspace_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            thread
                .get("projectlessOutputDirectory")
                .and_then(Value::as_str),
            Some(cwd.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn cli_thread_start_params_strip_host_and_copy_permissions() {
        let params = cli_thread_start_params(json!({
            "hostId": "local",
            "cwd": "/tmp/project",
            "permissions": {
                "approvalPolicy": "on-request",
                "sandboxPolicy": { "mode": "workspace-write" },
                "approvalsReviewer": "user"
            },
            "collaborationMode": {
                "settings": {
                    "model": "gpt-5.3-codex",
                    "reasoning_effort": "medium"
                }
            },
            "additionalDeveloperInstructions": "extra instructions"
        }));

        assert_eq!(params.get("hostId"), None);
        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            params.get("approvalPolicy").and_then(Value::as_str),
            Some("on-request")
        );
        assert_eq!(
            params.get("approvalsReviewer").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            params.get("model").and_then(Value::as_str),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            params.get("reasoningEffort").and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            params.get("developerInstructions").and_then(Value::as_str),
            Some("extra instructions")
        );
        assert_eq!(
            params.get("threadSource").and_then(Value::as_str),
            Some("user")
        );
    }

    #[test]
    fn cli_thread_start_params_drop_dynamic_tools_without_experimental_capability() {
        let params = cli_thread_start_params(json!({
            "hostId": "local",
            "cwd": "/tmp/project",
            "model": "gpt-5.3-codex",
            "modelProvider": "openai",
            "developerInstructions": "base instructions",
            "dynamicTools": null,
            "experimentalRawEvents": false,
            "mockExperimentalField": null,
            "sandbox": { "type": "workspaceWrite", "writableRoots": ["/tmp/project"] },
            "personality": "pragmatic",
            "persistExtendedHistory": true
        }));

        assert_eq!(params.get("dynamicTools"), None);
        assert_eq!(params.get("experimentalRawEvents"), None);
        assert_eq!(params.get("mockExperimentalField"), None);
        assert_eq!(
            params.get("modelProvider").and_then(Value::as_str),
            Some("openai")
        );
        assert_eq!(
            params.get("developerInstructions").and_then(Value::as_str),
            Some("base instructions")
        );
        assert_eq!(
            params
                .get("sandbox")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str),
            Some("workspaceWrite")
        );
        assert_eq!(
            params.get("personality").and_then(Value::as_str),
            Some("pragmatic")
        );
        assert_eq!(
            params
                .get("persistExtendedHistory")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn cli_app_server_method_params_normalizes_thread_start() {
        let params = cli_app_server_method_params(
            "thread/start",
            json!({
                "hostId": "local",
                "cwd": "/tmp/project",
                "dynamicTools": []
            }),
        );

        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(params.get("dynamicTools"), None);
    }

    #[test]
    fn cli_app_server_method_params_normalizes_turn_start() {
        let params = cli_app_server_method_params(
            "turn/start",
            json!({
                "hostId": "local",
                "threadId": "thread-1",
                "cwd": "/tmp/project",
                "input": [{ "type": "text", "text": "hello", "text_elements": [] }],
                "collaborationMode": {
                    "mode": "default",
                    "settings": {
                        "model": "gpt-5.3-codex",
                        "reasoning_effort": "medium"
                    }
                }
            }),
        );

        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("thread-1")
        );
        assert_eq!(
            params.get("model").and_then(Value::as_str),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            params.get("reasoningEffort").and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(params.get("collaborationMode"), None);
    }

    #[test]
    fn cli_app_server_method_params_normalizes_thread_resume() {
        let params = cli_app_server_method_params(
            "thread/resume",
            json!({
                "hostId": "local",
                "threadId": "0123456789abcdef0123456789abcdef",
                "path": "/Users/alice/.codex/sessions/thread.jsonl",
                "cwd": "/tmp/project",
                "excludeTurns": true,
                "config": { "model_reasoning_effort": "medium" },
                "sandbox": { "type": "workspaceWrite", "writableRoots": ["/tmp/project"] },
                "collaborationMode": {
                    "mode": "default",
                    "settings": {
                        "model": "gpt-5.3-codex",
                        "reasoning_effort": "medium"
                    }
                }
            }),
        );

        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            params.get("model").and_then(Value::as_str),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            params.get("reasoningEffort").and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(
            params.get("path").and_then(Value::as_str),
            Some("/Users/alice/.codex/sessions/thread.jsonl")
        );
        assert_eq!(
            params.get("excludeTurns").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            params
                .get("sandbox")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str),
            Some("workspaceWrite")
        );
        assert_eq!(params.get("collaborationMode"), None);
    }

    #[test]
    fn cli_thread_resume_params_maps_conversation_id_to_thread_id() {
        let params = cli_thread_resume_params(json!({
            "conversationId": "0123456789abcdef0123456789abcdef",
            "path": "/Users/alice/.codex/sessions/thread.jsonl"
        }));

        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(params.get("conversationId"), None);
        assert_eq!(
            params.get("path").and_then(Value::as_str),
            Some("/Users/alice/.codex/sessions/thread.jsonl")
        );
    }

    #[test]
    fn cli_maybe_resume_params_uses_thread_metadata_and_excludes_turns() {
        let params = cli_maybe_resume_params(
            &json!({
                "hostId": "local",
                "conversationId": "thread-1",
                "workspaceKind": "project"
            }),
            Some(&json!({
                "path": "/Users/alice/.codex/sessions/thread.jsonl",
                "cwd": "/tmp/project"
            })),
            "thread-1",
        );

        assert_eq!(params.get("hostId"), None);
        assert_eq!(params.get("conversationId"), None);
        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("thread-1")
        );
        assert_eq!(
            params.get("path").and_then(Value::as_str),
            Some("/Users/alice/.codex/sessions/thread.jsonl")
        );
        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            params
                .get("workspaceRoots")
                .and_then(Value::as_array)
                .and_then(|roots| roots.first())
                .and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            params.get("excludeTurns").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn cli_conversation_snapshot_message_contains_tail_turns_and_pagination() {
        let turns_page = CliThreadTurnsPage {
            next_cursor: json!("cursor-1"),
            turns: vec![json!({
                "id": "turn-1",
                "items": [
                    {
                        "type": "userMessage",
                        "id": "item-1",
                        "content": [{ "type": "text", "text": "hello", "text_elements": [] }]
                    },
                    {
                        "type": "agentMessage",
                        "id": "item-2",
                        "text": "hi",
                        "phase": "final_answer"
                    }
                ],
                "status": "completed",
                "startedAt": 10,
                "completedAt": 12,
                "durationMs": 2000
            })],
        };

        let message = cli_conversation_snapshot_message(
            "thread-1",
            &json!({
                "thread": {
                    "id": "thread-1",
                    "name": "Test thread",
                    "createdAt": 1,
                    "updatedAt": 2,
                    "path": "/Users/alice/.codex/sessions/thread.jsonl",
                    "cwd": "/tmp/project",
                    "status": { "type": "idle" },
                    "modelProvider": "openai"
                },
                "model": "gpt-5.3-codex",
                "reasoningEffort": "medium",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandbox": { "type": "workspaceWrite", "writableRoots": ["/tmp/project"] },
                "cwd": "/tmp/project"
            }),
            &turns_page,
            &json!({
                "workspaceKind": "project",
                "collaborationMode": {
                    "mode": "default",
                    "settings": {
                        "model": "gpt-5.3-codex",
                        "reasoning_effort": "medium"
                    }
                }
            }),
        )
        .expect("snapshot");

        assert_eq!(
            message.get("method").and_then(Value::as_str),
            Some("thread-stream-state-changed")
        );
        let state = message
            .pointer("/params/change/conversationState")
            .expect("conversation state");
        assert_eq!(state.get("id").and_then(Value::as_str), Some("thread-1"));
        assert_eq!(
            state.get("resumeState").and_then(Value::as_str),
            Some("resumed")
        );
        assert_eq!(
            state
                .pointer("/turns/0/params/input/0/text")
                .and_then(Value::as_str),
            Some("hello")
        );
        assert_eq!(
            state
                .pointer("/turns/0/turnStartedAtMs")
                .and_then(Value::as_u64),
            Some(10_000)
        );
        assert_eq!(
            state
                .pointer("/turnsPagination/olderCursor")
                .and_then(Value::as_str),
            Some("cursor-1")
        );
        assert_eq!(
            state
                .pointer("/turnsPagination/hasLoadedOldest")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn cli_conversation_snapshot_message_infers_projectless_workspace_from_cwd() {
        let home = std::env::var("HOME").expect("HOME");
        let workspace_root = Path::new(&home).join("Documents").join("Codex");
        let cwd = workspace_root.join("1970-01-01").join("hello-0");
        let turns_page = CliThreadTurnsPage {
            next_cursor: Value::Null,
            turns: Vec::new(),
        };

        let message = cli_conversation_snapshot_message(
            "thread-1",
            &json!({
                "thread": {
                    "id": "thread-1",
                    "createdAt": 1,
                    "updatedAt": 2,
                    "cwd": cwd,
                    "status": { "type": "idle" }
                },
                "model": "gpt-5.3-codex"
            }),
            &turns_page,
            &json!({}),
        )
        .expect("snapshot");

        let state = message
            .pointer("/params/change/conversationState")
            .expect("conversation state");
        assert_eq!(
            state.get("workspaceKind").and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            state.get("workspaceBrowserRoot").and_then(Value::as_str),
            Some(workspace_root.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn cli_app_server_request_normalization_covers_mcp_thread_start() {
        let mut request = json!({
            "id": "request-1",
            "method": "thread/start",
            "params": {
                "hostId": "local",
                "cwd": "/tmp/project",
                "modelProvider": "openai",
                "dynamicTools": [],
                "experimentalRawEvents": false
            }
        });

        normalize_cli_app_server_request(&mut request);
        let params = request.get("params").expect("params");

        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/tmp/project")
        );
        assert_eq!(
            params.get("modelProvider").and_then(Value::as_str),
            Some("openai")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(params.get("dynamicTools"), None);
        assert_eq!(params.get("experimentalRawEvents"), None);
    }

    #[test]
    fn cli_app_server_request_normalization_covers_mcp_thread_resume() {
        let mut request = json!({
            "id": "request-1",
            "method": "thread/resume",
            "params": {
                "hostId": "local",
                "threadId": "0123456789abcdef0123456789abcdef",
                "path": "/Users/alice/.codex/sessions/thread.jsonl",
                "excludeTurns": true
            }
        });

        normalize_cli_app_server_request(&mut request);
        let params = request.get("params").expect("params");

        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(
            params.get("path").and_then(Value::as_str),
            Some("/Users/alice/.codex/sessions/thread.jsonl")
        );
        assert_eq!(
            params.get("excludeTurns").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn cli_app_server_request_normalization_covers_mcp_turn_start() {
        let mut request = json!({
            "id": "request-1",
            "method": "turn/start",
            "params": {
                "hostId": "local",
                "threadId": "thread-1",
                "cwd": "/tmp/project",
                "collaborationMode": {
                    "mode": "default",
                    "settings": {
                        "model": "gpt-5.3-codex",
                        "reasoning_effort": "medium"
                    }
                }
            }
        });

        normalize_cli_app_server_request(&mut request);
        let params = request.get("params").expect("params");

        assert_eq!(
            params.get("threadId").and_then(Value::as_str),
            Some("thread-1")
        );
        assert_eq!(
            params.get("model").and_then(Value::as_str),
            Some("gpt-5.3-codex")
        );
        assert_eq!(params.get("hostId"), None);
        assert_eq!(params.get("collaborationMode"), None);
    }

    #[test]
    fn cli_thread_start_params_preserve_projectless_output_directory() {
        let params = cli_thread_start_params(json!({
            "hostId": "local",
            "workspaceKind": "projectless",
            "cwd": "/Users/alice/Documents/Codex/1970-01-01/hello-0",
            "workspaceRoots": ["/Users/alice/Documents/Codex"],
            "projectlessOutputDirectory": "/Users/alice/Documents/Codex/1970-01-01/hello-0",
            "additionalDeveloperInstructions": "extra instructions"
        }));

        assert_eq!(
            params.get("workspaceKind").and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            params
                .get("projectlessOutputDirectory")
                .and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
        assert_eq!(
            params.get("cwd").and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
        let instructions = params
            .get("developerInstructions")
            .and_then(Value::as_str)
            .expect("developer instructions");
        assert!(instructions.contains("extra instructions"));
        assert!(
            instructions.contains(
                "write scratch files, drafts, generated assets, and other outputs under /Users/alice/Documents/Codex/1970-01-01/hello-0"
            )
        );
    }

    #[test]
    fn projectless_thread_cwd_uses_codex_documents_root() {
        let (cwd, workspace_root) =
            projectless_thread_paths_for_home(Path::new("/Users/alice"), "Hello Codex", 0);

        assert_eq!(
            workspace_root,
            PathBuf::from("/Users/alice/Documents/Codex")
        );
        assert_eq!(
            cwd,
            PathBuf::from("/Users/alice/Documents/Codex/1970-01-01/hello-codex-0")
        );
    }

    #[test]
    fn projectless_thread_cwd_response_includes_frontend_output_directory() {
        let response = projectless_thread_cwd_response(
            Path::new("/Users/alice/Documents/Codex/1970-01-01/hello-0"),
            Path::new("/Users/alice/Documents/Codex"),
        );

        assert_eq!(
            response.get("cwd").and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
        assert_eq!(
            response.get("workspaceRoot").and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex")
        );
        assert_eq!(
            response
                .get("projectlessOutputDirectory")
                .and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
        assert_eq!(
            response.get("outputDirectory").and_then(Value::as_str),
            Some("/Users/alice/Documents/Codex/1970-01-01/hello-0")
        );
    }

    #[test]
    fn projectless_slug_is_ascii_and_has_fallback() {
        assert_eq!(
            sanitize_projectless_path_segment("Hello Codex"),
            "hello-codex"
        );
        assert_eq!(sanitize_projectless_path_segment("测试一下"), "");

        let (cwd, _) = projectless_thread_paths_for_home(Path::new("/tmp/home"), "测试一下", 0);
        assert_eq!(
            cwd,
            PathBuf::from("/tmp/home/Documents/Codex/1970-01-01/chat-0")
        );
    }

    #[test]
    fn mobile_instance_item_name_uses_device_and_workspace() {
        assert_eq!(
            mobile_instance_item_name("xxxxdeMacBook", "abc"),
            "xxxxdemacbook-abc"
        );
        assert_eq!(
            mobile_instance_item_name("Jin's MacBook Pro.local", "Client Workspace"),
            "jin's macbook pro-client workspace"
        );
        assert_eq!(
            mobile_instance_item_name("Name's MacBookxxx", "Workspace名"),
            "name's macbookxxx-workspace名"
        );
    }

    #[test]
    fn mobile_instance_item_name_ignores_repeated_separators() {
        assert_eq!(
            mobile_instance_item_name("  Office  MacBook  ", " API / Worker "),
            "office macbook-api-worker"
        );
    }

    #[test]
    fn relay_metadata_includes_device_uuid() {
        let config = RemoteServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3147,
            token: "remote-token".to_string(),
            relay_url: Some("https://relay.example.com".to_string()),
            relay_connection_id: Some("connection-1".to_string()),
            crypto: None,
            remote_frontend_mode: "cli".to_string(),
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
            web_asset_base_url: None,
            web_asset_version: "latest".to_string(),
            cdp_host: "127.0.0.1".to_string(),
            cdp_port: 9222,
        };

        let url = append_relay_metadata_to_ws_url(
            "wss://relay.example.com/ws/host?token=remote-token".to_string(),
            &config,
        )
        .expect("relay metadata url");
        let parsed = reqwest::Url::parse(&url).expect("parse relay url");
        let device_uuid = parsed
            .query_pairs()
            .find(|(key, _)| key == "deviceUuid")
            .map(|(_, value)| value.into_owned());
        let remote_frontend_mode = parsed
            .query_pairs()
            .find(|(key, _)| key == "remoteFrontendMode")
            .map(|(_, value)| value.into_owned());

        assert_eq!(
            device_uuid.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(remote_frontend_mode.as_deref(), Some("cli"));
    }

    #[tokio::test]
    async fn clear_relay_state_allows_web_bridge_notification_pumps_to_restart() {
        let runtime = RemoteRuntimeState::new(RemoteServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3147,
            token: "remote-token".to_string(),
            relay_url: Some("https://relay.example.com".to_string()),
            relay_connection_id: Some("connection-1".to_string()),
            crypto: None,
            remote_frontend_mode: "app".to_string(),
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
            web_asset_base_url: None,
            web_asset_version: "latest".to_string(),
            cdp_host: "127.0.0.1".to_string(),
            cdp_port: 9222,
        });

        runtime
            .relay_web_bridge_notification_pumps
            .lock()
            .await
            .insert("client-1".to_string());
        runtime
            .relay_control_clients
            .lock()
            .await
            .insert("client-1".to_string());
        runtime.relay_frame_client_count.store(1, Ordering::Relaxed);

        runtime.clear_relay_state().await;

        assert!(runtime
            .relay_web_bridge_notification_pumps
            .lock()
            .await
            .is_empty());
        assert!(runtime.relay_control_clients.lock().await.is_empty());
        assert_eq!(runtime.relay_frame_client_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cloud_relay_discovery_selects_relay_url() {
        let url = selected_cloud_relay_url(CloudRelayDiscoveryResponse {
            ok: true,
            relay: Some(CloudRelayDiscoveryRelay {
                url: "https://us1.codexl.io/".to_string(),
            }),
        })
        .expect("relay url");

        assert_eq!(url, "https://us1.codexl.io");
    }

    #[test]
    fn configured_cloud_relay_url_trims_and_validates_url() {
        let url = configured_cloud_relay_url("https://relay.example.com/")
            .expect("configured relay URL should parse");

        assert_eq!(url.as_deref(), Some("https://relay.example.com"));
    }

    #[test]
    fn configured_cloud_relay_url_allows_empty_value() {
        let url = configured_cloud_relay_url("   ").expect("empty relay URL is not an error");

        assert_eq!(url, None);
    }

    #[test]
    fn cloud_relay_discovery_rejects_missing_relay_url() {
        let error = selected_cloud_relay_url(CloudRelayDiscoveryResponse {
            ok: true,
            relay: None,
        })
        .expect_err("missing relay URL should fail");

        assert!(error.contains("relay.url"));
    }
}
