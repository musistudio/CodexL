use crate::{
    config::{generated_codex_home, RemoteCloudAuthConfig},
    ports, server, AppState,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_TYPE, COOKIE, SEC_WEBSOCKET_ACCEPT,
    SEC_WEBSOCKET_KEY, SET_COOKIE, UPGRADE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch, Mutex, Semaphore};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

mod assets;
pub(crate) mod cdp_resources;
mod crypto;
mod input;
mod util;

use assets::static_response;
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
const RELAY_WEB_RESOURCE_TASK_LIMIT: usize = 64;
const RELAY_WEB_BRIDGE_NOTIFICATION_PUMP_TTL_MS: u64 = 10 * 60_000;
const FRAME_META_INTERVAL_MS: u64 = 250;
const DEFAULT_SCREENSHOT_MAX_HEIGHT: u64 = 900;
const DEFAULT_SCREENSHOT_MAX_WIDTH: u64 = 1440;
const DEFAULT_PAGE_ZOOM_SCALE: f64 = 1.0;
const MIN_PAGE_ZOOM_SCALE: f64 = 1.0;
const MAX_PAGE_ZOOM_SCALE: f64 = 3.0;
const REMOTE_AUTH_COOKIE_NAME: &str = "codexl_remote_token";
const CLOUD_RELAY_DISCOVERY_URL: &str = "https://relay.codexl.io/";
const CLOUD_RELAY_DISCOVERY_TIMEOUT_MS: u64 = 8000;

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
    pub cdp_host: String,
    pub cdp_port: u16,
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
    device_uuid: String,
    workspace_id: String,
    workspace_name: String,
    workspace_path: String,
    cloud_auth: Option<RemoteCloudAuthConfig>,
    cdp_host: String,
    cdp_port: u16,
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

    let app_config = state.config.lock().await.clone();
    let profile = app_config.provider_profile(&profile_name);
    let use_cloud_relay = use_cloud_relay
        .or_else(|| {
            profile
                .as_ref()
                .map(|profile| profile.start_remote_cloud_on_launch)
        })
        .unwrap_or(false);
    let require_e2ee = use_cloud_relay;
    let workspace_id = profile
        .as_ref()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| profile_name.clone());
    let workspace_path = profile
        .as_ref()
        .map(|profile| generated_codex_home(profile).to_string_lossy().to_string())
        .unwrap_or_default();
    let cloud_auth = if use_cloud_relay {
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
    let relay_url = if use_cloud_relay {
        Some(discover_cloud_relay_url().await?)
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

    let launch = server::launch_codex_instance(
        state,
        server::LaunchRequest {
            profile_name: Some(profile_name.clone()),
            ..server::LaunchRequest::default()
        },
    )
    .await?;

    let token = make_token();
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
    let server_config = RemoteServerConfig {
        host: app_config.remote_control_host,
        port,
        token: token.clone(),
        relay_url: relay_url.clone(),
        relay_connection_id: relay_connection_id.clone(),
        crypto: crypto.map(Arc::new),
        device_uuid: app_config.device_uuid.clone(),
        workspace_id,
        workspace_name: profile_name.clone(),
        workspace_path,
        cloud_auth: cloud_auth.clone(),
        cdp_host: launch.cdp_host,
        cdp_port: launch.cdp_port,
    };
    let lan_url = remote_url(&server_config.host, server_config.port, &token);
    let url = if let Some(relay_url) = relay_url.as_deref() {
        remote_relay_url(
            relay_url,
            relay_connection_id
                .as_deref()
                .ok_or_else(|| "missing relay connection id".to_string())?,
            cloud_auth.as_ref().map(|auth| auth.user_id.as_str()),
        )?
    } else {
        lan_url.clone()
    };
    let url = append_remote_crypto_params(url, server_config.crypto.is_some())?;
    let listener = TcpListener::bind((server_config.host.as_str(), server_config.port))
        .await
        .map_err(|e| format!("failed to bind remote control server: {}", e))?;

    let runtime = RemoteRuntimeState::new(server_config.clone());
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
            require_password: server_config.crypto.is_some(),
            cdp_host: server_config.cdp_host.clone(),
            cdp_port: server_config.cdp_port,
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
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "unauthorized" }),
        ));
    }

    if remote_http_path_requires_auth(&path) && !runtime.authorized(request) {
        return Ok(json_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "unauthorized" }),
        ));
    }

    let response = match (request.method(), path.as_str()) {
        (&Method::GET, "/api/status") => {
            Ok(json_response(StatusCode::OK, runtime.bridge.status().await))
        }
        (&Method::GET, "/api/targets") => {
            let targets = runtime.bridge.list_targets().await?;
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
            runtime.bridge.switch_target(id).await?;
            Ok(json_response(StatusCode::OK, runtime.bridge.status().await))
        }
        (&Method::GET, "/web") => cdp_resources::web_root_redirect(request.uri().query()),
        (&Method::POST, "/web/_bridge") => {
            let body = request
                .body_mut()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            let message = serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?;
            let response = cdp_resources::dispatch_web_bridge_message(
                &runtime.config.cdp_host,
                runtime.config.cdp_port,
                message,
            )
            .await?;
            Ok(json_response(StatusCode::OK, response))
        }
        (&Method::GET, _) if path.starts_with("/web/") => cdp_resources::get_web_resource(
            &runtime.config.cdp_host,
            runtime.config.cdp_port,
            request.uri().path(),
            request.uri().query(),
        )
        .await?
        .into_response(),
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
    if !runtime.authorized_web_bridge(request) {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }

    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let cdp_host = runtime.config.cdp_host.clone();
    let cdp_port = runtime.config.cdp_port;
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
    if !runtime.authorized_web_bridge(request) {
        return Ok(empty_response(StatusCode::UNAUTHORIZED));
    }

    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?
        .to_string();
    let cdp_host = runtime.config.cdp_host.clone();
    let cdp_port = runtime.config.cdp_port;
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
                    eprintln!("Remote Codex web resource WebSocket failed: {}", err);
                }
            }
            Err(err) => eprintln!(
                "Remote Codex web resource WebSocket upgrade failed: {}",
                err
            ),
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

async fn websocket_response(
    request: &mut Request<Incoming>,
    runtime: Arc<RemoteRuntimeState>,
    channel: WsChannel,
) -> Result<Response<HttpBody>, String> {
    if !runtime.authorized(request) {
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

#[derive(Clone)]
enum ControlTarget {
    Local(usize),
    Relay(String),
}

struct RemoteRuntimeState {
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
    relay_web_resource_tasks: Arc<Semaphore>,
    stopped: AtomicBool,
}

impl RemoteRuntimeState {
    fn new(config: RemoteServerConfig) -> Arc<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let bridge = Arc::new(CdpBridge::new(config.clone(), event_tx));
        let runtime = Arc::new(Self {
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
            relay_web_resource_tasks: Arc::new(Semaphore::new(RELAY_WEB_RESOURCE_TASK_LIMIT)),
            stopped: AtomicBool::new(false),
        });
        let event_runtime = runtime.clone();
        tokio::spawn(async move {
            event_runtime.handle_bridge_events(event_rx).await;
        });
        runtime
    }

    fn start(self: &Arc<Self>) {
        self.bridge.clone().start();
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
        self.close_clients().await;
    }

    fn authorized(&self, request: &Request<Incoming>) -> bool {
        self.query_token_authorized(request)
            || self.bearer_token_authorized(request)
            || self.cookie_token_authorized(request)
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
        }
        response
    }

    async fn handle_client(
        self: Arc<Self>,
        websocket: tokio_tungstenite::WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
        channel: WsChannel,
    ) {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::unbounded_channel();
        match channel {
            WsChannel::Control => {
                self.control_clients.lock().await.insert(id, tx);
                self.send_control(
                    ControlTarget::Local(id),
                    json!({ "type": "status", "status": self.bridge.status().await }),
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
                    json!({ "type": "status", "status": self.bridge.status().await }),
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
                        json!({ "type": "status", "status": self.bridge.status().await }),
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
                        json!({ "type": "status", "status": self.bridge.status().await }),
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
        let ws_url = relay_host_ws_url(
            relay_url,
            &self.config.token,
            self.config.cloud_auth.is_some(),
        )?;
        let ws_url = append_relay_metadata_to_ws_url(ws_url, &self.config)?;
        let mut request = ws_url.into_client_request().map_err(|e| e.to_string())?;
        if let Some(auth) = self.config.cloud_auth.as_ref() {
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
                        json!({ "type": "status", "status": self.bridge.status().await }),
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
        let semaphore = match message.get("type").and_then(Value::as_str) {
            Some("webResourceFromClient") => self.relay_web_resource_tasks.clone(),
            _ => self.relay_web_bridge_tasks.clone(),
        };
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
                cdp_resources::dispatch_web_bridge_socket_payload_with_emitter(
                    &self.config.cdp_host,
                    self.config.cdp_port,
                    &payload,
                    move |partial| {
                        if let Some(sender) = relay_sender.as_ref() {
                            if let Some(payload) =
                                runtime_for_stream.encrypt_relay_socket_text(partial.to_string())
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
            "webResourceFromClient" => {
                cdp_resources::dispatch_web_resource_socket_payload(
                    &self.config.cdp_host,
                    self.config.cdp_port,
                    &payload,
                )
                .await
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
            .append_pair("clientVersion", env!("CARGO_PKG_VERSION"));
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

fn local_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Desktop".to_string())
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

    #[test]
    fn bearer_token_parses_case_insensitive_scheme() {
        assert_eq!(bearer_token("Bearer secret"), Some("secret"));
        assert_eq!(bearer_token("bearer   secret"), Some("secret"));
        assert_eq!(bearer_token("Basic secret"), None);
        assert_eq!(bearer_token("Bearer"), None);
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
    fn remote_http_auth_scope_covers_control_surfaces() {
        assert!(remote_http_path_requires_auth("/api/status"));
        assert!(remote_http_path_requires_auth("/web"));
        assert!(remote_http_path_requires_auth("/web/_bridge"));
        assert!(remote_http_path_requires_auth("/web/assets/app.js"));
        assert!(!remote_http_path_requires_auth("/"));
        assert!(!remote_http_path_requires_auth("/app.js"));
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
            device_uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            workspace_id: "workspace-1".to_string(),
            workspace_name: "Workspace 1".to_string(),
            workspace_path: "/tmp/workspace-1".to_string(),
            cloud_auth: None,
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

        assert_eq!(
            device_uuid.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
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
    fn cloud_relay_discovery_rejects_missing_relay_url() {
        let error = selected_cloud_relay_url(CloudRelayDiscoveryResponse {
            ok: true,
            relay: None,
        })
        .expect_err("missing relay URL should fail");

        assert!(error.contains("relay.url"));
    }
}
