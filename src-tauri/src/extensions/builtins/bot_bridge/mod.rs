use crate::config::{self, BotHandoffConfig, BotProfileConfig};
use crate::extensions::{self, BuiltinNodeExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ENABLED_ENV: &str = "CODEXL_BOT_GATEWAY_ENABLED";
const INTEGRATION_ID_ENV: &str = "CODEXL_BOT_GATEWAY_INTEGRATION_ID";
const PLATFORM_ENV: &str = "CODEXL_BOT_GATEWAY_PLATFORM";
const TENANT_ID_ENV: &str = "CODEXL_BOT_GATEWAY_TENANT_ID";
const STATE_DIR_ENV: &str = "CODEXL_BOT_GATEWAY_STATE_DIR";
const BOT_GATEWAY_STATE_DIR_ENV: &str = "BOT_GATEWAY_STATE_DIR";
const BOT_GATEWAY_PROXY_URL_ENV: &str = "BOT_GATEWAY_PROXY_URL";
const POLL_INTERVAL_ENV: &str = "CODEXL_BOT_GATEWAY_POLL_INTERVAL_MS";
const TURN_TIMEOUT_ENV: &str = "CODEXL_BOT_GATEWAY_TURN_TIMEOUT_MS";
const FORWARD_ALL_CODEX_MESSAGES_ENV: &str = "CODEXL_BOT_GATEWAY_FORWARD_ALL_CODEX_MESSAGES";
const HANDOFF_ENABLED_ENV: &str = "CODEXL_BOT_HANDOFF_ENABLED";
const HANDOFF_IDLE_SECONDS_ENV: &str = "CODEXL_BOT_HANDOFF_IDLE_SECONDS";
const HANDOFF_SCREEN_LOCK_ENV: &str = "CODEXL_BOT_HANDOFF_SCREEN_LOCK";
const HANDOFF_USER_IDLE_ENV: &str = "CODEXL_BOT_HANDOFF_USER_IDLE";
const HANDOFF_PHONE_WIFI_TARGETS_ENV: &str = "CODEXL_BOT_HANDOFF_PHONE_WIFI_TARGETS";
const HANDOFF_PHONE_BLUETOOTH_TARGETS_ENV: &str = "CODEXL_BOT_HANDOFF_PHONE_BLUETOOTH_TARGETS";
const LOG_ENV: &str = "CODEXL_BOT_GATEWAY_LOG";
const PROFILE_ENV: &str = "CODEXL_CODEX_PROFILE";
const LANGUAGE_ENV: &str = "CODEXL_LANGUAGE";
const APP_REQUEST_ID_PREFIX: &str = "codexl-bot-";
const BOT_REQUEST_ID_PREFIX: &str = "codexl-bot-gateway-";
const BOT_GATEWAY_HEALTH_TIMEOUT_SECS: u64 = 10;
const BOT_GATEWAY_DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const BOT_GATEWAY_MCP_OUTBOUND_TIMEOUT_SECS: u64 = 170;
const BOT_APPROVAL_POLL_INTERVAL_MS: u64 = 500;
const DINGTALK_STREAM_OPEN_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";
const DINGTALK_ACCESS_TOKEN_URL: &str = "https://api.dingtalk.com/v1.0/oauth2/accessToken";
const DINGTALK_MEDIA_UPLOAD_URL: &str = "https://oapi.dingtalk.com/media/upload";
const DINGTALK_ROBOT_TOPIC: &str = "/v1.0/im/bot/messages/get";
const CORE_BLUETOOTH_SCAN_SWIFT: &str = r#"
import Foundation
import CoreBluetooth

final class Scanner: NSObject, CBCentralManagerDelegate {
    var central: CBCentralManager?
    var devices: [String: [String: Any]] = [:]
    var didFinish = false

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: DispatchQueue.main)
        DispatchQueue.main.asyncAfter(deadline: .now() + 8.0) {
            self.finish()
        }
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn {
            central.scanForPeripherals(
                withServices: nil,
                options: [CBCentralManagerScanOptionAllowDuplicatesKey: true]
            )
            DispatchQueue.main.asyncAfter(deadline: .now() + 5.0) {
                self.finish()
            }
        } else if central.state == .unsupported || central.state == .unauthorized || central.state == .poweredOff {
            finish()
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String : Any],
        rssi RSSI: NSNumber
    ) {
        let identifier = peripheral.identifier.uuidString
        let advertisedName = advertisementData[CBAdvertisementDataLocalNameKey] as? String
        let name = peripheral.name ?? advertisedName ?? ""
        devices[identifier] = [
            "identifier": identifier,
            "name": name,
            "rssi": RSSI.intValue
        ]
    }

    func finish() {
        if didFinish {
            return
        }
        didFinish = true
        central?.stopScan()
        let list = Array(devices.values)
        if let data = try? JSONSerialization.data(withJSONObject: list, options: []),
           let text = String(data: data, encoding: .utf8) {
            print(text)
        } else {
            print("[]")
        }
        Foundation.exit(0)
    }
}

let scanner = Scanner()
RunLoop.main.run()
"#;
const PROJECTLESS_PROJECT_LABEL: &str = "(projectless)";
const BOT_MEDIA_CONTEXT_FILE: &str = "bot-media-context.json";
const CODEX_EVENT_HUB_CAPACITY: usize = 8192;
#[cfg(unix)]
const FLOCK_EXCLUSIVE: std::os::raw::c_int = 2;
#[cfg(unix)]
const FLOCK_NONBLOCKING: std::os::raw::c_int = 4;

type SharedAppStdin = Arc<Mutex<ChildStdin>>;

static APP_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static BOT_MEDIA_SESSION_FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(1);
static DINGTALK_ACCESS_TOKEN_CACHE: OnceLock<Mutex<BTreeMap<String, DingtalkAccessToken>>> =
    OnceLock::new();

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int;
}

#[derive(Debug, Clone)]
struct BotBridgeConfig {
    extension: BuiltinNodeExtension,
    state_dir: Option<PathBuf>,
    platform: String,
    tenant_id: String,
    integration_id: String,
    poll_interval: Duration,
    turn_timeout: Duration,
    forward_all_codex_messages: bool,
    handoff: BotHandoffConfig,
    language: AppLanguage,
    log_path: PathBuf,
}

#[derive(Debug, Clone)]
struct BotGatewayRuntimeConfig {
    profile_name: String,
    extension: BuiltinNodeExtension,
    state_dir: Option<PathBuf>,
    platform: String,
    tenant_id: String,
    integration_id: String,
}

struct AppServerBridge {
    writer: SharedAppStdin,
    event_hub: CodexEventHub,
    idle_cursor: CodexEventCursor,
    dingtalk_rx: Option<mpsc::Receiver<Value>>,
    pending_dingtalk_events: VecDeque<Value>,
    current_session_key: Option<String>,
    current_media_session_id: Option<String>,
    thread_id: Option<String>,
    selected_cwd: Option<String>,
    config: BotBridgeConfig,
    completed_events: BTreeMap<String, CompletedEventResponse>,
    handoff_active_threads: BTreeMap<String, u64>,
    idle_handoff_turn_captures: BTreeMap<String, TurnCapture>,
    idle_handoff_message_counter: u64,
}

struct BotGatewayClient {
    child: Child,
    stdin: ChildStdin,
    response_rx: mpsc::Receiver<Value>,
    next_id: u64,
}

struct BotBridgeLease {
    _file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum CodexEventSource {
    CliAppServer,
    CdpWebview,
    RemoteBridge,
    BotGateway,
}

impl CodexEventSource {
    #[allow(dead_code)]
    fn as_str(self) -> &'static str {
        match self {
            Self::CliAppServer => "cli-app-server",
            Self::CdpWebview => "cdp-webview",
            Self::RemoteBridge => "remote-bridge",
            Self::BotGateway => "bot-gateway",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexEventChannel {
    Notification,
    Request,
    Response,
    Stream,
    Unknown,
}

impl CodexEventChannel {
    #[allow(dead_code)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Notification => "notification",
            Self::Request => "request",
            Self::Response => "response",
            Self::Stream => "stream",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CodexEvent {
    seq: u64,
    source: CodexEventSource,
    channel: CodexEventChannel,
    method: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    value: Option<Value>,
    raw: Vec<u8>,
    received_at: Instant,
}

#[derive(Debug, Clone)]
struct CodexEventCursor {
    next_seq: u64,
}

struct CodexEventHub {
    stdout_rx: mpsc::Receiver<Vec<u8>>,
    events: VecDeque<CodexEvent>,
    first_seq: u64,
    next_seq: u64,
    disconnected: bool,
}

#[derive(Debug)]
enum CodexEventHubError {
    Disconnected,
    Gap { requested: u64, first: u64 },
}

impl CodexEvent {
    fn from_app_server_stdout(seq: u64, raw: Vec<u8>) -> Self {
        let value = serde_json::from_slice::<Value>(trim_json_line(&raw)).ok();
        let method = value
            .as_ref()
            .and_then(|value| value.get("method"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let null_params = Value::Null;
        let params = value
            .as_ref()
            .and_then(|value| value.get("params"))
            .unwrap_or(&null_params);
        let channel = codex_event_channel_for_app_server_value(value.as_ref(), method.as_deref());
        let thread_id = nested_param_id(params, "threadId", "thread").map(str::to_string);
        let turn_id = nested_param_id(params, "turnId", "turn").map(str::to_string);

        Self {
            seq,
            source: CodexEventSource::CliAppServer,
            channel,
            method,
            thread_id,
            turn_id,
            value,
            raw,
            received_at: Instant::now(),
        }
    }
}

impl CodexEventHubError {
    fn message(&self) -> String {
        match self {
            Self::Disconnected => "Codex app-server output channel closed".to_string(),
            Self::Gap { requested, first } => format!(
                "Codex app-server event cursor fell behind requested_seq={} first_seq={}",
                requested, first
            ),
        }
    }
}

impl CodexEventHub {
    fn new(stdout_rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            stdout_rx,
            events: VecDeque::new(),
            first_seq: 1,
            next_seq: 1,
            disconnected: false,
        }
    }

    fn cursor_now(&self) -> CodexEventCursor {
        CodexEventCursor {
            next_seq: self.next_seq,
        }
    }

    fn publish_app_server_stdout(&mut self, raw: Vec<u8>) -> CodexEvent {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let event = CodexEvent::from_app_server_stdout(seq, raw);
        self.events.push_back(event.clone());
        while self.events.len() > CODEX_EVENT_HUB_CAPACITY {
            self.events.pop_front();
        }
        self.first_seq = self
            .events
            .front()
            .map(|event| event.seq)
            .unwrap_or(self.next_seq);
        event
    }

    fn drain_available(&mut self, limit: usize) {
        for _ in 0..limit {
            match self.stdout_rx.try_recv() {
                Ok(raw) => {
                    self.publish_app_server_stdout(raw);
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.disconnected = true;
                    return;
                }
            }
        }
    }

    fn buffered_event_for_cursor(
        &self,
        cursor: &CodexEventCursor,
    ) -> Result<Option<CodexEvent>, CodexEventHubError> {
        if cursor.next_seq < self.first_seq {
            return Err(CodexEventHubError::Gap {
                requested: cursor.next_seq,
                first: self.first_seq,
            });
        }
        Ok(self
            .events
            .iter()
            .find(|event| event.seq >= cursor.next_seq)
            .cloned())
    }

    fn next_event(
        &mut self,
        cursor: &mut CodexEventCursor,
        timeout: Duration,
    ) -> Result<Option<CodexEvent>, CodexEventHubError> {
        if let Some(event) = self.buffered_event_for_cursor(cursor)? {
            cursor.next_seq = event.seq.saturating_add(1);
            return Ok(Some(event));
        }
        if timeout.is_zero() {
            return Ok(None);
        }

        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let wait = deadline - now;
            match self.stdout_rx.recv_timeout(wait) {
                Ok(raw) => {
                    self.publish_app_server_stdout(raw);
                    if let Some(event) = self.buffered_event_for_cursor(cursor)? {
                        cursor.next_seq = event.seq.saturating_add(1);
                        return Ok(Some(event));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.disconnected = true;
                    return Err(CodexEventHubError::Disconnected);
                }
            }
        }
    }

    fn try_next_event(
        &mut self,
        cursor: &mut CodexEventCursor,
    ) -> Result<Option<CodexEvent>, CodexEventHubError> {
        self.drain_available(100);
        self.next_event(cursor, Duration::ZERO)
    }
}

fn codex_event_channel_for_app_server_value(
    value: Option<&Value>,
    method: Option<&str>,
) -> CodexEventChannel {
    let has_id = value
        .and_then(|value| value.get("id"))
        .is_some_and(|id| !id.is_null());
    match (method, has_id) {
        (Some("item/agentMessage/delta"), _) => CodexEventChannel::Stream,
        (Some(_), true) => CodexEventChannel::Request,
        (Some(_), false) => CodexEventChannel::Notification,
        (None, true) => CodexEventChannel::Response,
        (None, false) => CodexEventChannel::Unknown,
    }
}

#[derive(Debug, Clone)]
struct DingtalkIntegrationAuth {
    app_key: String,
    app_secret: String,
}

#[derive(Debug, Clone)]
struct DingtalkAccessToken {
    token: String,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct ProjectSummary {
    cwd: String,
    name: String,
    threads: Vec<ThreadSummary>,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct ThreadSummary {
    id: String,
    preview: String,
    cwd: Option<String>,
    path: Option<String>,
    updated_at: i64,
    status: Option<String>,
}

enum BotMessageAction {
    Reply(String),
    Run(String),
    SwitchProjectAndRun(BotProjectSwitch),
}

struct BotProjectSwitch {
    project: ProjectSummary,
    message_text: String,
}

#[derive(Debug, Clone)]
struct BotApprovalPrompt {
    request_key: String,
    title: String,
    body: String,
    fields: Vec<BotApprovalField>,
    actions: Vec<BotApprovalAction>,
}

#[derive(Debug, Clone)]
struct BotApprovalField {
    label: String,
    value: String,
}

#[derive(Debug, Clone)]
struct BotApprovalAction {
    key: String,
    label: String,
    result: Value,
}

pub struct BotQrLoginSession {
    runtime: BotGatewayRuntimeConfig,
    client: Mutex<BotGatewayClient>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotQrLoginStartInfo {
    pub profile_name: String,
    pub tenant_id: String,
    pub integration_id: String,
    pub session_id: String,
    pub qr_code_url: String,
    pub expires_at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotQrLoginWaitInfo {
    pub profile_name: String,
    pub tenant_id: String,
    pub integration_id: String,
    pub session_id: String,
    pub status: String,
    pub message: String,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotIntegrationConfigureInfo {
    pub profile_name: String,
    pub tenant_id: String,
    pub integration_id: String,
    pub platform: String,
    pub auth_type: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotHandoffScanTarget {
    pub id: String,
    pub label: String,
    pub target: String,
    pub detail: String,
    pub source: String,
}

#[derive(Debug, Default)]
struct TurnCapture {
    fallback_text: String,
    final_text: Option<String>,
}

#[derive(Debug, Default)]
struct CodexTurnResult {
    response_text: String,
    sent_messages: usize,
}

#[derive(Debug, Clone)]
struct CompletedEventResponse {
    response_text: String,
    already_sent: bool,
}

#[derive(Debug, Clone)]
struct HandoffPresence {
    away: bool,
    reasons: Vec<String>,
    evidence: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct HandoffSignals {
    screen_locked: Option<bool>,
    idle_seconds: Option<u64>,
    phone_wifi_seen: Option<bool>,
    phone_bluetooth_seen: Option<bool>,
}

#[derive(Debug, Clone)]
struct HandoffSignalSnapshot {
    signals: HandoffSignals,
    diagnostics: Vec<String>,
}

struct CodexForwardDecision {
    should_forward: bool,
    handoff_presence: Option<HandoffPresence>,
    handoff_evaluation: Option<HandoffPresence>,
}

struct IdleHandoffContext {
    event: Value,
    thread_id: String,
    project: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppLanguage {
    En,
    Zh,
}

impl AppLanguage {
    fn from_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "chinese" => Self::Zh,
            _ => Self::En,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PersistedBotSessionStore {
    #[serde(default)]
    sessions: BTreeMap<String, PersistedBotSessionState>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedBotSessionState {
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_session_id: Option<String>,
    #[serde(default)]
    updated_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BotMediaMcpContext {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    session_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    tenant_id: String,
    integration_id: String,
    platform: String,
    conversation_ref: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default)]
    updated_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BotMediaMcpContextStore {
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_session_id: Option<String>,
    #[serde(default)]
    sessions: BTreeMap<String, BotMediaMcpContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BotMediaToolKind {
    Media,
    Image,
    File,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpMessageFraming {
    JsonLine,
    ContentLength,
}

struct McpMessage {
    value: Value,
    framing: McpMessageFraming,
}

pub fn spawn_app_stdio_bot_bridge(app_stdin: SharedAppStdin) -> Option<mpsc::Sender<Vec<u8>>> {
    let config = match BotBridgeConfig::from_env() {
        Some(config) => config,
        None => return None,
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn({
        let config = config.clone();
        move || {
            if let Err(err) = run_bridge(config.clone(), app_stdin, rx) {
                log_bridge(&config, &format!("bridge stopped: {}", err));
            }
        }
    });
    Some(tx)
}

pub fn should_intercept_app_server_line(line: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(trim_json_line(line)) else {
        return false;
    };
    value
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with(APP_REQUEST_ID_PREFIX))
}

pub fn run_bot_media_mcp_stdio() -> Result<i32, String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_bot_media_mcp_stdio_with_io(stdin.lock(), stdout.lock())
}

fn run_bot_media_mcp_stdio_with_io<R, W>(input: R, output: W) -> Result<i32, String>
where
    R: Read,
    W: Write,
{
    let mut reader = BufReader::new(input);
    let mut writer = output;
    let log_path = log_path();
    log_bridge_path(&log_path, "bot media MCP stdio started");
    loop {
        let request = match read_mcp_message(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) => {
                log_bridge_path(&log_path, "bot media MCP stdio stopped: input closed");
                break;
            }
            Err(err) => {
                log_bridge_path(&log_path, &format!("bot media MCP stdio stopped: {}", err));
                return Err(err);
            }
        };
        let started_at = Instant::now();
        let framing = request.framing;
        let request = request.value;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let request_id = request
            .get("id")
            .map(Value::to_string)
            .unwrap_or_else(|| "notification".to_string());
        log_bridge_path(
            &log_path,
            &format!("mcp request method={} id={}", method, request_id),
        );
        if let Some(response) = handle_bot_media_mcp_request(request) {
            let status = if response.get("error").is_some() {
                "error"
            } else {
                "ok"
            };
            write_mcp_message(&mut writer, &response, framing)?;
            log_bridge_path(
                &log_path,
                &format!(
                    "mcp response method={} id={} status={} elapsed_ms={}",
                    method,
                    request_id,
                    status,
                    started_at.elapsed().as_millis()
                ),
            );
        } else {
            log_bridge_path(
                &log_path,
                &format!(
                    "mcp notification handled method={} elapsed_ms={}",
                    method,
                    started_at.elapsed().as_millis()
                ),
            );
        }
    }
    Ok(0)
}

fn handle_bot_media_mcp_request(request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let is_notification = id.is_none();

    let result = match method {
        "initialize" => {
            let protocol_version = request
                .get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("2024-11-05");
            Ok(json!({
                "protocolVersion": protocol_version,
                "capabilities": {
                    "tools": { "listChanged": false },
                },
                "serverInfo": {
                    "name": "codexl-bot-media",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }))
        }
        "notifications/initialized" => {
            return None;
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": bot_media_mcp_tools() })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if let Some(kind) = BotMediaToolKind::from_tool_name(name) {
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                send_bot_media_tool_call(kind, args).map(|text| {
                    json!({
                        "content": [{
                            "type": "text",
                            "text": text,
                        }],
                    })
                })
            } else {
                Err(format!("unknown tool: {}", name))
            }
        }
        _ => Err(format!("unknown MCP method: {}", method)),
    };

    if is_notification {
        return None;
    }

    Some(match result {
        Ok(result) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": result,
        }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": -32000,
                "message": message,
            },
        }),
    })
}

impl BotMediaToolKind {
    fn from_tool_name(name: &str) -> Option<Self> {
        match name {
            "send_media" => Some(Self::Media),
            "send_image" => Some(Self::Image),
            "send_file" => Some(Self::File),
            "send_video" => Some(Self::Video),
            "send_audio" => Some(Self::Audio),
            _ => None,
        }
    }

    fn tool_name(self) -> &'static str {
        match self {
            Self::Media => "send_media",
            Self::Image => "send_image",
            Self::File => "send_file",
            Self::Video => "send_video",
            Self::Audio => "send_audio",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Media => "Send an image, file, video, or audio/voice message to the current external bot conversation. Prefer absolute local paths for generated files.",
            Self::Image => "Send an image to the current external bot conversation.",
            Self::File => "Send a file attachment to the current external bot conversation.",
            Self::Video => "Send a video to the current external bot conversation.",
            Self::Audio => "Send an audio or voice file to the current external bot conversation.",
        }
    }

    fn default_mime_type(self) -> Option<&'static str> {
        match self {
            Self::Media => None,
            Self::Image => Some("image/png"),
            Self::File => Some("application/octet-stream"),
            Self::Video => Some("video/mp4"),
            Self::Audio => Some("audio/mpeg"),
        }
    }
}

fn bot_media_mcp_tools() -> Value {
    Value::Array(
        [
            BotMediaToolKind::Media,
            BotMediaToolKind::Image,
            BotMediaToolKind::File,
            BotMediaToolKind::Video,
            BotMediaToolKind::Audio,
        ]
        .into_iter()
        .map(bot_media_mcp_tool)
        .collect(),
    )
}

fn bot_media_mcp_tool(kind: BotMediaToolKind) -> Value {
    json!({
        "name": kind.tool_name(),
        "description": kind.description(),
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative local file path. Use this for files generated in the current Codex workspace."
                },
                "url": {
                    "type": "string",
                    "description": "file://, http(s), or local path URL. Used when path is not provided."
                },
                "caption": {
                    "type": "string",
                    "description": "Optional visible caption text. Omit this unless the user explicitly asks for a caption; never put the user's send instruction or file path here."
                },
                "botSessionId": {
                    "type": "string",
                    "description": "Required CodexL Bot session id from the current Bot bridge prompt. This keeps the MCP call bound to the same external bot conversation."
                },
                "filename": {
                    "type": "string",
                    "description": "Optional display filename. Inferred from path/url when omitted."
                },
                "mimeType": {
                    "type": "string",
                    "description": "Optional MIME type, such as image/png, video/mp4, audio/mpeg, or application/pdf. Inferred from filename/path when omitted."
                },
                "sizeBytes": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional file size in bytes. Inferred for existing local files when omitted."
                },
                "durationMs": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional duration for audio/video media in milliseconds."
                },
                "id": {
                    "type": "string",
                    "description": "Optional platform media key for platforms that can reuse uploaded media. Weixin iLink still requires path or url."
                }
            },
            "required": ["botSessionId"],
            "additionalProperties": false
        }
    })
}

fn send_bot_media_tool_call(kind: BotMediaToolKind, args: Value) -> Result<String, String> {
    let config =
        BotBridgeConfig::from_env().ok_or_else(|| "Bot media MCP is not enabled".to_string())?;
    let args = args.as_object().cloned().unwrap_or_default();
    let context = load_bot_media_context_for_tool(&config, &args)?;
    let platform = if context.platform.is_empty() {
        config.platform.as_str()
    } else {
        context.platform.as_str()
    };
    let intent = build_bot_media_intent(kind, &args, context.cwd.as_deref(), platform)?;

    let tenant_id = if context.tenant_id.is_empty() {
        config.tenant_id.clone()
    } else {
        context.tenant_id.clone()
    };
    let integration_id = if context.integration_id.is_empty() {
        config.integration_id.clone()
    } else {
        context.integration_id.clone()
    };
    let conversation_ref = context.conversation_ref.clone();
    let event_id = context.event_id.as_deref().unwrap_or("event").to_string();

    log_bridge(
        &config,
        &format!(
            "mcp tool call tool={} session={} path_present={} url_present={} id_present={}",
            kind.tool_name(),
            context.session_id,
            args.get("path").is_some(),
            args.get("url").is_some(),
            args.get("id").is_some()
        ),
    );
    if platform == config::BOT_PLATFORM_DINGTALK {
        send_dingtalk_media_tool_call(&config, &context, &intent)?;
        log_bridge(
            &config,
            &format!(
                "mcp tool call completed tool={} session={} via=dingtalk_session_webhook",
                kind.tool_name(),
                context.session_id
            ),
        );
        return Ok(format!(
            "{} sent to the current bot conversation.",
            match kind {
                BotMediaToolKind::Image => "Image",
                BotMediaToolKind::File => "File",
                BotMediaToolKind::Video => "Video",
                BotMediaToolKind::Audio => "Audio",
                BotMediaToolKind::Media => "Media",
            }
        ));
    }

    let mut bot = BotGatewayClient::start(&config.extension, config.state_dir.as_deref())?;
    let result = bot.request_with_timeout(
        "outbound.send",
        json!({
            "tenantId": tenant_id,
            "integrationId": integration_id,
            "conversationRef": conversation_ref,
            "intent": intent,
            "idempotencyKey": format!("codexl:mcp:{}:{}:{}", event_id, kind.tool_name(), unix_seconds()),
        }),
        Duration::from_secs(BOT_GATEWAY_MCP_OUTBOUND_TIMEOUT_SECS),
    )?;
    ensure_outbound_sent(&result)?;
    log_bridge(
        &config,
        &format!(
            "mcp tool call completed tool={} session={}",
            kind.tool_name(),
            context.session_id
        ),
    );
    Ok(format!(
        "{} sent to the current bot conversation.",
        match kind {
            BotMediaToolKind::Image => "Image",
            BotMediaToolKind::File => "File",
            BotMediaToolKind::Video => "Video",
            BotMediaToolKind::Audio => "Audio",
            BotMediaToolKind::Media => "Media",
        }
    ))
}

fn send_dingtalk_media_tool_call(
    config: &BotBridgeConfig,
    context: &BotMediaMcpContext,
    intent: &Value,
) -> Result<(), String> {
    let event = bot_event_from_media_context(context);
    send_dingtalk_media_message(config, &event, intent)
}

fn send_dingtalk_media_message(
    config: &BotBridgeConfig,
    event: &Value,
    intent: &Value,
) -> Result<(), String> {
    let media = intent
        .get("media")
        .and_then(Value::as_object)
        .ok_or_else(|| "DingTalk media send requires media payload".to_string())?;
    let caption = intent
        .get("caption")
        .or_else(|| intent.get("fallbackText"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let filename = media
        .get("filename")
        .and_then(Value::as_str)
        .unwrap_or("image");
    let media_ref = dingtalk_media_reference(config, media)?;

    if dingtalk_media_is_image(media) {
        let body = dingtalk_image_markdown_message(caption, &media_ref, filename);
        return send_dingtalk_session_webhook_message(config, event, &body);
    }

    if let Some(caption) = caption {
        send_dingtalk_text_response(config, event, caption)?;
    }
    let body = json!({
        "msgtype": "file",
        "file": {
            "media_id": media_ref
        }
    });
    send_dingtalk_session_webhook_message(config, event, &body)
}

fn dingtalk_media_reference(
    config: &BotBridgeConfig,
    media: &Map<String, Value>,
) -> Result<String, String> {
    let url = media
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(url) = url {
        if is_http_url(url) {
            return Ok(url.to_string());
        }
        if let Some(path) = local_path_from_media_url(url).filter(|path| path.is_file()) {
            return upload_dingtalk_media(config, media, &path);
        }
    }

    let media_id = media
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(media_id) = media_id {
        let id_path = Path::new(media_id);
        if id_path.is_file() {
            return upload_dingtalk_media(config, media, id_path);
        }
        return Ok(media_id.to_string());
    }

    Err("DingTalk media send requires an HTTP URL, local file path, or media id".to_string())
}

fn upload_dingtalk_media(
    config: &BotBridgeConfig,
    media: &Map<String, Value>,
    path: &Path,
) -> Result<String, String> {
    let auth = load_dingtalk_integration_auth(config)?;
    let access_token = dingtalk_access_token(&auth)?;
    let media_type = dingtalk_upload_media_type(media);
    let filename = media
        .get("filename")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "attachment".to_string());
    let mime_type = media.get("mimeType").and_then(Value::as_str);
    let bytes = fs::read(path).map_err(|err| {
        format!(
            "failed to read DingTalk media file {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to create DingTalk media upload runtime: {}", err))?;
    runtime.block_on(async {
        let mut part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
        if let Some(mime_type) = mime_type {
            part = part.mime_str(mime_type).map_err(|err| {
                format!("invalid DingTalk media MIME type {}: {}", mime_type, err)
            })?;
        }
        let form = reqwest::multipart::Form::new().part("media", part);
        let response = reqwest::Client::new()
            .post(DINGTALK_MEDIA_UPLOAD_URL)
            .query(&[
                ("access_token", access_token.as_str()),
                ("type", media_type),
            ])
            .multipart(form)
            .send()
            .await
            .map_err(|err| format!("DingTalk media upload failed: {}", err))?;
        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|err| format!("failed to read DingTalk media upload response: {}", err))?;
        if !status.is_success() {
            return Err(format!(
                "DingTalk media upload returned HTTP {}: {}",
                status, response_text
            ));
        }
        ensure_dingtalk_send_response_ok(&response_text)?;
        let value = serde_json::from_str::<Value>(&response_text)
            .map_err(|err| format!("failed to parse DingTalk media upload response: {}", err))?;
        value
            .get("media_id")
            .or_else(|| value.get("mediaId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| {
                format!(
                    "DingTalk media upload response missing media_id: {}",
                    response_text
                )
            })
    })
}

fn dingtalk_image_markdown_message(
    caption: Option<&str>,
    media_ref: &str,
    filename: &str,
) -> Value {
    let title = caption
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(filename);
    let text = if let Some(caption) = caption {
        format!("{}\n\n![{}]({})", caption, filename, media_ref)
    } else {
        format!("![{}]({})", filename, media_ref)
    };
    json!({
        "msgtype": "markdown",
        "markdown": {
            "title": title,
            "text": text
        }
    })
}

fn dingtalk_upload_media_type(media: &Map<String, Value>) -> &'static str {
    if dingtalk_media_is_image(media) {
        "image"
    } else if dingtalk_media_is_audio(media) {
        "voice"
    } else {
        "file"
    }
}

fn dingtalk_media_is_image(media: &Map<String, Value>) -> bool {
    let value = dingtalk_media_descriptor(media);
    value.contains("image/")
        || matches!(
            Path::new(&value).extension().and_then(|ext| ext.to_str()),
            Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff")
        )
}

fn dingtalk_media_is_audio(media: &Map<String, Value>) -> bool {
    let value = dingtalk_media_descriptor(media);
    value.contains("audio/")
        || matches!(
            Path::new(&value).extension().and_then(|ext| ext.to_str()),
            Some("amr" | "mp3" | "m4a" | "wav" | "ogg" | "opus")
        )
}

fn dingtalk_media_descriptor(media: &Map<String, Value>) -> String {
    ["mimeType", "filename", "url", "id"]
        .iter()
        .filter_map(|key| media.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn build_bot_media_intent(
    kind: BotMediaToolKind,
    args: &Map<String, Value>,
    cwd: Option<&str>,
    platform: &str,
) -> Result<Value, String> {
    let raw_url = args
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| args.get("url").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let media_id = args
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if raw_url.is_none() && media_id.is_none() {
        return Err(format!("{} requires path, url, or id", kind.tool_name()));
    }
    if raw_url.is_none() && platform == config::BOT_PLATFORM_WEIXIN_ILINK {
        return Err(
            "Weixin iLink media sends require path or url; media id alone cannot be uploaded"
                .to_string(),
        );
    }

    let normalized_url = raw_url.map(|url| normalize_tool_media_url(url, cwd));
    let local_path = normalized_url
        .as_deref()
        .and_then(local_path_from_media_url)
        .filter(|path| path.is_file());
    let filename = string_arg(args, &["filename"])
        .or_else(|| normalized_url.as_deref().and_then(filename_from_media_url));
    let mime_type = string_arg(args, &["mimeType", "mime_type"])
        .or_else(|| filename.as_deref().and_then(guess_mime_type))
        .or_else(|| {
            normalized_url
                .as_deref()
                .and_then(filename_from_media_url)
                .as_deref()
                .and_then(guess_mime_type)
        })
        .or_else(|| kind.default_mime_type().map(ToString::to_string));
    let mut media = Map::new();
    if let Some(id) = media_id {
        media.insert("id".to_string(), Value::String(id.to_string()));
    }
    if let Some(url) = normalized_url.clone() {
        media.insert("url".to_string(), Value::String(url));
    }
    if let Some(filename) = filename {
        media.insert("filename".to_string(), Value::String(filename));
    }
    if let Some(mime_type) = mime_type {
        media.insert("mimeType".to_string(), Value::String(mime_type));
    }
    if let Some(size_bytes) =
        nonnegative_u64_arg(args, &["sizeBytes", "size_bytes"]).or_else(|| {
            local_path
                .as_ref()
                .and_then(|path| fs::metadata(path).ok())
                .map(|metadata| metadata.len())
        })
    {
        media.insert(
            "sizeBytes".to_string(),
            Value::Number(serde_json::Number::from(size_bytes)),
        );
    }
    if let Some(duration_ms) = nonnegative_u64_arg(args, &["durationMs", "duration_ms"]) {
        media.insert(
            "durationMs".to_string(),
            Value::Number(serde_json::Number::from(duration_ms)),
        );
    }

    let caption = string_arg(args, &["caption"]);
    let fallback_text = caption
        .clone()
        .or_else(|| {
            media
                .get("filename")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            media
                .get("url")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            media
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "[media]".to_string());
    let mut intent = Map::new();
    intent.insert("type".to_string(), Value::String("media".to_string()));
    intent.insert("media".to_string(), Value::Object(media));
    intent.insert("fallbackText".to_string(), Value::String(fallback_text));
    if let Some(caption) = caption {
        intent.insert("caption".to_string(), Value::String(caption));
    }
    Ok(Value::Object(intent))
}

fn string_arg(args: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        args.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn nonnegative_u64_arg(args: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| match args.get(*key)? {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    })
}

fn filename_from_media_url(value: &str) -> Option<String> {
    if let Some(path) = local_path_from_media_url(value) {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToString::to_string)
            .filter(|name| !name.is_empty());
    }

    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let name = without_query.rsplit('/').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(percent_decode_file_url_path(name))
    }
}

fn guess_mime_type(filename: &str) -> Option<String> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "amr" => "audio/amr",
        "silk" => "audio/silk",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return None,
    };
    Some(mime.to_string())
}

fn normalize_tool_media_url(value: &str, cwd: Option<&str>) -> String {
    if is_http_url(value) || value.starts_with("file://") || Path::new(value).is_absolute() {
        return value.to_string();
    }
    let base = cwd
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(value).to_string_lossy().to_string()
}

fn read_mcp_message<R: BufRead>(reader: &mut R) -> Result<Option<McpMessage>, String> {
    let mut line = String::new();
    let mut content_length = None;
    loop {
        line.clear();
        let size = reader
            .read_line(&mut line)
            .map_err(|err| format!("failed to read MCP input: {}", err))?;
        if size == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.trim_start().starts_with('{') {
            let value = serde_json::from_str::<Value>(trimmed)
                .map_err(|err| format!("failed to parse MCP JSON line: {}", err))?;
            return Ok(Some(McpMessage {
                value,
                framing: McpMessageFraming::JsonLine,
            }));
        }
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|err| format!("invalid MCP Content-Length: {}", err))?,
                );
            }
        }
    }

    let content_length =
        content_length.ok_or_else(|| "MCP message missing Content-Length".to_string())?;
    let mut body = vec![0u8; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|err| format!("failed to read MCP body: {}", err))?;
    let value = serde_json::from_slice::<Value>(&body)
        .map_err(|err| format!("failed to parse MCP body: {}", err))?;
    Ok(Some(McpMessage {
        value,
        framing: McpMessageFraming::ContentLength,
    }))
}

fn write_mcp_message<W: Write>(
    writer: &mut W,
    value: &Value,
    framing: McpMessageFraming,
) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    match framing {
        McpMessageFraming::JsonLine => writer
            .write_all(&body)
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|err| format!("failed to write MCP JSON line: {}", err)),
        McpMessageFraming::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n", body.len())
                .map_err(|err| format!("failed to write MCP header: {}", err))?;
            writer
                .write_all(&body)
                .and_then(|_| writer.flush())
                .map_err(|err| format!("failed to write MCP body: {}", err))
        }
    }
}

pub fn start_weixin_qr_login_session(
    profile_name: &str,
    bot_config: &BotProfileConfig,
    force: bool,
) -> Result<(BotQrLoginStartInfo, BotQrLoginSession), String> {
    let mut bot_config = bot_config.clone();
    bot_config.normalize_for_profile(profile_name);
    if bot_config.auth_type != config::BOT_AUTH_QR_LOGIN {
        return Err("selected bot auth type does not use QR login".to_string());
    }
    let runtime = BotGatewayRuntimeConfig::from_profile(profile_name, &bot_config)?;
    if runtime.platform != config::BOT_PLATFORM_WEIXIN_ILINK {
        return Err("selected bot platform does not support Weixin QR login".to_string());
    }

    let mut bot = BotGatewayClient::start(&runtime.extension, runtime.state_dir.as_deref())?;
    let info = start_weixin_qr_login_with_client(profile_name, &runtime, &mut bot, force)?;
    Ok((
        info,
        BotQrLoginSession {
            runtime,
            client: Mutex::new(bot),
        },
    ))
}

pub fn configure_bot_integration(
    profile_name: &str,
    bot_config: &BotProfileConfig,
) -> Result<BotIntegrationConfigureInfo, String> {
    let mut bot_config = bot_config.clone();
    bot_config.normalize_for_profile(profile_name);
    if !bot_config.bridge_enabled() {
        return Err(format!("Bot is not enabled for workspace {}", profile_name));
    }
    if bot_config.auth_type == config::BOT_AUTH_QR_LOGIN {
        return Err(
            "QR login integrations must be configured through the QR login flow".to_string(),
        );
    }

    let runtime = BotGatewayRuntimeConfig::from_profile(profile_name, &bot_config)?;
    let (credentials, integration_config) = bot_gateway_integration_auth_payload(&bot_config);
    let mut bot = BotGatewayClient::start(&runtime.extension, runtime.state_dir.as_deref())?;
    let result = bot.request(
        "integrations.create",
        json!({
            "id": bot_config.integration_id.clone(),
            "tenantId": bot_config.tenant_id.clone(),
            "platform": bot_config.platform.clone(),
            "authType": bot_config.auth_type.clone(),
            "credentials": credentials,
            "config": integration_config,
            "status": "active",
        }),
    )?;
    let integration = result.get("integration").unwrap_or(&result);
    Ok(BotIntegrationConfigureInfo {
        profile_name: profile_name.to_string(),
        tenant_id: integration
            .get("tenantId")
            .and_then(Value::as_str)
            .unwrap_or(bot_config.tenant_id.as_str())
            .to_string(),
        integration_id: integration
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(bot_config.integration_id.as_str())
            .to_string(),
        platform: integration
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or(bot_config.platform.as_str())
            .to_string(),
        auth_type: integration
            .get("authType")
            .and_then(Value::as_str)
            .unwrap_or(bot_config.auth_type.as_str())
            .to_string(),
        status: integration
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active")
            .to_string(),
    })
}

pub fn scan_handoff_wifi_targets() -> Result<Vec<BotHandoffScanTarget>, String> {
    let output = command_stdout("arp", &["-a"])
        .ok_or_else(|| "failed to scan Wi-Fi/LAN neighbors with arp".to_string())?;
    Ok(parse_arp_scan_targets(&output))
}

pub fn scan_handoff_bluetooth_targets() -> Result<Vec<BotHandoffScanTarget>, String> {
    let mut targets = Vec::new();
    collect_bluetooth_scan_targets_from_commands(&mut targets);
    Ok(targets)
}

impl BotQrLoginSession {
    pub fn wait(&self, session_id: &str) -> Result<BotQrLoginWaitInfo, String> {
        let mut bot = self
            .client
            .lock()
            .map_err(|_| "Bot Gateway QR login mutex poisoned".to_string())?;
        wait_weixin_qr_login_with_client(&self.runtime, &mut bot, session_id)
    }
}

fn start_weixin_qr_login_with_client(
    profile_name: &str,
    runtime: &BotGatewayRuntimeConfig,
    bot: &mut BotGatewayClient,
    force: bool,
) -> Result<BotQrLoginStartInfo, String> {
    let result = bot.request(
        "auth.qr.start",
        json!({
            "platform": config::BOT_PLATFORM_WEIXIN_ILINK,
            "tenantId": runtime.tenant_id.clone(),
            "integrationId": runtime.integration_id.clone(),
            "force": force,
        }),
    )?;
    let auth = result.get("result").unwrap_or(&result);
    let session_id = auth
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Bot Gateway QR start response missing sessionId".to_string())?
        .to_string();

    Ok(BotQrLoginStartInfo {
        profile_name: profile_name.to_string(),
        tenant_id: runtime.tenant_id.clone(),
        integration_id: runtime.integration_id.clone(),
        session_id,
        qr_code_url: auth
            .get("qrCodeUrl")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        expires_at: auth
            .get("expiresAt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        message: auth
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Scan the QR code in Weixin.")
            .to_string(),
    })
}

fn wait_weixin_qr_login_with_client(
    runtime: &BotGatewayRuntimeConfig,
    bot: &mut BotGatewayClient,
    session_id: &str,
) -> Result<BotQrLoginWaitInfo, String> {
    let result = bot.request(
        "auth.qr.wait",
        json!({
            "platform": config::BOT_PLATFORM_WEIXIN_ILINK,
            "tenantId": runtime.tenant_id.clone(),
            "integrationId": runtime.integration_id.clone(),
            "sessionId": session_id,
            "timeoutMs": 5_000,
            "autoStart": true,
        }),
    )?;
    let auth = result.get("result").unwrap_or(&result);
    let status = auth
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending")
        .to_string();

    Ok(BotQrLoginWaitInfo {
        profile_name: runtime.profile_name.clone(),
        tenant_id: runtime.tenant_id.clone(),
        integration_id: runtime.integration_id.clone(),
        session_id: session_id.to_string(),
        confirmed: status == "confirmed",
        status,
        message: auth
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn run_bridge(
    config: BotBridgeConfig,
    app_stdin: SharedAppStdin,
    stdout_rx: mpsc::Receiver<Vec<u8>>,
) -> Result<(), String> {
    log_bridge(
        &config,
        &format!(
            "starting bridge plugin={} entry={} node={} state_dir={} platform={} tenant_id={} integration_id={} forward_all_codex_messages={}",
            config.extension.id,
            config.extension.entry_path.to_string_lossy(),
            config.extension.node.executable.to_string_lossy(),
            config
                .state_dir
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            config.platform,
            config.tenant_id,
            config.integration_id,
            config.forward_all_codex_messages
        ),
    );

    let _bridge_lease = match acquire_bot_bridge_lease(&config) {
        Ok(lease) => lease,
        Err(err) => {
            log_bridge(&config, &format!("bridge listener skipped: {}", err));
            return Ok(());
        }
    };

    if let Err(err) = migrate_legacy_bot_gateway_integration(&config) {
        log_bridge(
            &config,
            &format!("failed to migrate legacy Bot Gateway integration: {}", err),
        );
    }
    let mut bot = loop {
        match BotGatewayClient::start(&config.extension, config.state_dir.as_deref()) {
            Ok(bot) => break bot,
            Err(err) => {
                log_bridge(
                    &config,
                    &format!("Bot Gateway start failed; retrying: {}", err),
                );
                thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let dingtalk_rx = start_dingtalk_stream_listener(&config);
    let event_hub = CodexEventHub::new(stdout_rx);
    let idle_cursor = event_hub.cursor_now();
    let mut app = AppServerBridge {
        writer: app_stdin,
        event_hub,
        idle_cursor,
        dingtalk_rx,
        pending_dingtalk_events: VecDeque::new(),
        current_session_key: None,
        current_media_session_id: None,
        thread_id: None,
        selected_cwd: None,
        config: config.clone(),
        completed_events: BTreeMap::new(),
        handoff_active_threads: BTreeMap::new(),
        idle_handoff_turn_captures: BTreeMap::new(),
        idle_handoff_message_counter: 0,
    };
    let mut last_integration_start = Instant::now() - Duration::from_secs(60);

    loop {
        if last_integration_start.elapsed() >= Duration::from_secs(30) {
            ensure_integration_started(&mut bot, &config);
            last_integration_start = Instant::now();
        }

        let result = match bot.request("events.list", json!({ "limit": 20 })) {
            Ok(result) => result,
            Err(err) => {
                log_bridge(
                    &config,
                    &format!("Bot Gateway request failed; restarting child: {}", err),
                );
                bot.stop_child();
                match BotGatewayClient::start(&config.extension, config.state_dir.as_deref()) {
                    Ok(restarted) => {
                        bot = restarted;
                        last_integration_start = Instant::now() - Duration::from_secs(60);
                    }
                    Err(start_err) => {
                        log_bridge(
                            &config,
                            &format!("Bot Gateway restart failed; retrying: {}", start_err),
                        );
                        thread::sleep(Duration::from_secs(2));
                    }
                }
                continue;
            }
        };
        let events = result
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for queued in events {
            if let Err(err) = handle_queued_event(&mut bot, &mut app, &queued) {
                log_bridge(&config, &format!("event handling failed: {}", err));
            }
        }
        for queued in app.collect_dingtalk_stream_events() {
            if let Err(err) = handle_queued_event(&mut bot, &mut app, &queued) {
                log_bridge(
                    &config,
                    &format!("DingTalk stream event handling failed: {}", err),
                );
            }
        }
        app.process_idle_app_output(&mut bot);

        thread::sleep(config.poll_interval);
    }
}

fn acquire_bot_bridge_lease(config: &BotBridgeConfig) -> Result<Option<BotBridgeLease>, String> {
    let Some(state_dir) = config.state_dir.as_ref() else {
        log_bridge(config, "bridge lease not used: no state_dir configured");
        return Ok(None);
    };
    fs::create_dir_all(state_dir).map_err(|err| {
        format!(
            "failed to create bridge lease directory {}: {}",
            state_dir.to_string_lossy(),
            err
        )
    })?;
    let lock_path = state_dir.join(format!(
        ".bot-gateway-bridge-{}-{}.lock",
        sanitize_lock_component(&config.platform),
        sanitize_lock_component(&config.integration_id)
    ));
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|err| {
            format!(
                "failed to open bridge lease {}: {}",
                lock_path.to_string_lossy(),
                err
            )
        })?;
    if !try_lock_file_exclusive(&file).map_err(|err| {
        format!(
            "failed to acquire bridge lease {}: {}",
            lock_path.to_string_lossy(),
            err
        )
    })? {
        return Err(format!(
            "another Bot Gateway bridge is already active for platform={} tenant_id={} integration_id={} lock_path={}",
            config.platform,
            config.tenant_id,
            config.integration_id,
            lock_path.to_string_lossy()
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    file.set_len(0).map_err(|err| {
        format!(
            "failed to truncate bridge lease {}: {}",
            lock_path.to_string_lossy(),
            err
        )
    })?;
    file.write_all(
        format!(
            "pid={}\nplatform={}\ntenant_id={}\nintegration_id={}\nupdated_at={}\n",
            std::process::id(),
            config.platform,
            config.tenant_id,
            config.integration_id,
            now
        )
        .as_bytes(),
    )
    .map_err(|err| {
        format!(
            "failed to write bridge lease {}: {}",
            lock_path.to_string_lossy(),
            err
        )
    })?;
    let _ = file.sync_data();
    log_bridge(
        config,
        &format!(
            "bridge lease acquired lock_path={}",
            lock_path.to_string_lossy()
        ),
    );
    Ok(Some(BotBridgeLease { _file: file }))
}

#[cfg(unix)]
fn try_lock_file_exclusive(file: &File) -> Result<bool, String> {
    let result = unsafe { flock(file.as_raw_fd(), FLOCK_EXCLUSIVE | FLOCK_NONBLOCKING) };
    if result == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(err.to_string())
}

#[cfg(not(unix))]
fn try_lock_file_exclusive(_file: &File) -> Result<bool, String> {
    Ok(true)
}

fn sanitize_lock_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim_matches('_').is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

fn ensure_integration_started(bot: &mut BotGatewayClient, config: &BotBridgeConfig) {
    let Ok(result) = bot.request("integrations.list", json!({})) else {
        log_bridge(config, "integrations.list failed");
        return;
    };
    let Some(integrations) = result.get("integrations").and_then(Value::as_array) else {
        return;
    };
    let integration = integrations.iter().find(|integration| {
        integration
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == config.integration_id)
    });
    let Some(integration) = integration else {
        log_bridge(
            config,
            &format!(
                "integration {} not found; waiting for matching Bot Gateway integration",
                config.integration_id
            ),
        );
        return;
    };

    let platform = integration
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_startable_bot_gateway_platform(platform) {
        return;
    }

    if let Err(err) = bot.request(
        "integrations.start",
        json!({ "integrationId": config.integration_id }),
    ) {
        log_bridge(
            config,
            &format!(
                "integrations.start ignored for {}: {}",
                config.integration_id, err
            ),
        );
    }
}

fn bot_gateway_integration_auth_payload(bot_config: &BotProfileConfig) -> (Value, Value) {
    let mut credentials = Map::new();
    let mut integration_config = default_bot_gateway_integration_config(&bot_config.platform);

    for (key, value) in &bot_config.auth_fields {
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        if is_bot_gateway_config_field(&bot_config.platform, key) {
            integration_config.insert(key.to_string(), bot_gateway_config_value(key, value));
        } else {
            credentials.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    enforce_bot_gateway_integration_config(&bot_config.platform, &mut integration_config);

    (
        Value::Object(credentials),
        Value::Object(integration_config),
    )
}

fn enforce_bot_gateway_integration_config(
    platform: &str,
    integration_config: &mut Map<String, Value>,
) {
    if let Some(transport) = socket_first_bot_gateway_transport(platform) {
        integration_config.insert(
            "transport".to_string(),
            Value::String(transport.to_string()),
        );
    }
}

fn default_bot_gateway_integration_config(platform: &str) -> Map<String, Value> {
    let mut config = Map::new();
    config.insert("dryRun".to_string(), Value::Bool(false));
    if let Some(transport) = socket_first_bot_gateway_transport(platform) {
        config.insert(
            "transport".to_string(),
            Value::String(transport.to_string()),
        );
    }
    config
}

fn socket_first_bot_gateway_transport(platform: &str) -> Option<&'static str> {
    match platform {
        config::BOT_PLATFORM_SLACK => Some("socket"),
        config::BOT_PLATFORM_DISCORD => Some("websocket"),
        config::BOT_PLATFORM_TELEGRAM => Some("websocket"),
        config::BOT_PLATFORM_FEISHU => Some("websocket"),
        config::BOT_PLATFORM_DINGTALK => Some("websocket"),
        config::BOT_PLATFORM_LINE => Some("websocket"),
        config::BOT_PLATFORM_WECOM => Some("websocket"),
        config::BOT_PLATFORM_WEIXIN_ILINK => Some("long_polling"),
        _ => None,
    }
}

fn is_bot_gateway_config_field(platform: &str, key: &str) -> bool {
    matches!(
        key,
        "transport"
            | "dryRun"
            | "applicationId"
            | "publicKey"
            | "appId"
            | "appKey"
            | "corpId"
            | "agentId"
            | "robotCode"
    ) || (platform == config::BOT_PLATFORM_WEIXIN_ILINK
        && matches!(
            key,
            "baseUrl"
                | "cdnBaseUrl"
                | "accountId"
                | "userId"
                | "botAgent"
                | "routeTag"
                | "pollingIntervalMs"
                | "longPollTimeoutMs"
                | "cursorKey"
                | "downloadInboundMedia"
                | "inboundMediaDir"
                | "inboundMediaMaxBytes"
                | "outboundMediaTempDir"
        ))
        || (platform == config::BOT_PLATFORM_FEISHU
            && matches!(
                key,
                "domain" | "appType" | "receiveIdType" | "tenantKey" | "tenantAccessToken"
            ))
}

fn bot_gateway_config_value(key: &str, value: &str) -> Value {
    match key {
        "dryRun" | "downloadInboundMedia" => Value::Bool(matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )),
        "pollingIntervalMs" | "longPollTimeoutMs" | "inboundMediaMaxBytes" => value
            .trim()
            .parse::<u64>()
            .map(|number| json!(number))
            .unwrap_or_else(|_| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

fn start_dingtalk_stream_listener(config: &BotBridgeConfig) -> Option<mpsc::Receiver<Value>> {
    if config.platform != config::BOT_PLATFORM_DINGTALK {
        return None;
    }

    let auth = match load_dingtalk_integration_auth(config) {
        Ok(auth) => auth,
        Err(err) => {
            log_bridge(
                config,
                &format!("DingTalk stream listener disabled: {}", err),
            );
            return None;
        }
    };
    let (tx, rx) = mpsc::channel();
    let thread_config = config.clone();
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                log_bridge(
                    &thread_config,
                    &format!("DingTalk stream runtime failed: {}", err),
                );
                return;
            }
        };

        loop {
            let result = runtime.block_on(run_dingtalk_stream_once(
                thread_config.clone(),
                auth.clone(),
                tx.clone(),
            ));
            match result {
                Ok(()) => log_bridge(&thread_config, "DingTalk stream disconnected; reconnecting"),
                Err(err) => log_bridge(
                    &thread_config,
                    &format!("DingTalk stream failed; reconnecting: {}", err),
                ),
            }
            thread::sleep(Duration::from_secs(5));
        }
    });

    Some(rx)
}

async fn run_dingtalk_stream_once(
    config: BotBridgeConfig,
    auth: DingtalkIntegrationAuth,
    tx: mpsc::Sender<Value>,
) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};

    let (endpoint, ticket) = open_dingtalk_stream_connection(&auth).await?;
    let ws_url = dingtalk_websocket_url(&endpoint, &ticket);
    log_bridge(&config, "DingTalk stream connecting");
    let (socket, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|err| format!("failed to connect DingTalk stream websocket: {}", err))?;
    log_bridge(&config, "DingTalk stream connected");
    let (mut write, mut read) = socket.split();

    while let Some(message) = read.next().await {
        let message =
            message.map_err(|err| format!("DingTalk stream websocket read failed: {}", err))?;
        let text = match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => text,
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => String::from_utf8(bytes)
                .map_err(|err| format!("DingTalk stream binary message was not UTF-8: {}", err))?,
            tokio_tungstenite::tungstenite::Message::Close(_) => return Ok(()),
            tokio_tungstenite::tungstenite::Message::Ping(bytes) => {
                write
                    .send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                    .await
                    .map_err(|err| format!("DingTalk stream pong failed: {}", err))?;
                continue;
            }
            _ => continue,
        };

        let envelope = serde_json::from_str::<Value>(&text)
            .map_err(|err| format!("failed to parse DingTalk stream message: {}", err))?;
        let accepted = if let Some(queued) = dingtalk_queued_event_from_stream(&config, &envelope) {
            tx.send(queued).is_ok()
        } else {
            true
        };

        if let Some(response) = dingtalk_stream_response(&envelope, accepted) {
            write
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    response.to_string(),
                ))
                .await
                .map_err(|err| format!("DingTalk stream response failed: {}", err))?;
        }
    }

    Ok(())
}

async fn open_dingtalk_stream_connection(
    auth: &DingtalkIntegrationAuth,
) -> Result<(String, String), String> {
    let response = reqwest::Client::new()
        .post(DINGTALK_STREAM_OPEN_URL)
        .json(&json!({
            "clientId": auth.app_key,
            "clientSecret": auth.app_secret,
            "subscriptions": [{
                "topic": DINGTALK_ROBOT_TOPIC,
                "type": "CALLBACK"
            }],
            "ua": "codexl-bot-gateway/0.1.0"
        }))
        .send()
        .await
        .map_err(|err| format!("failed to open DingTalk stream connection: {}", err))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| format!("failed to read DingTalk stream open response: {}", err))?;
    if !status.is_success() {
        return Err(format!(
            "DingTalk stream open returned HTTP {}: {}",
            status, text
        ));
    }
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|err| format!("failed to parse DingTalk stream open response: {}", err))?;
    let endpoint = value
        .get("endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("DingTalk stream open response missing endpoint: {}", text))?;
    let ticket = value
        .get("ticket")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("DingTalk stream open response missing ticket: {}", text))?;

    Ok((endpoint.to_string(), ticket.to_string()))
}

fn dingtalk_websocket_url(endpoint: &str, ticket: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{}{}ticket={}", endpoint.trim(), separator, ticket.trim())
}

fn dingtalk_queued_event_from_stream(config: &BotBridgeConfig, envelope: &Value) -> Option<Value> {
    let headers = dingtalk_stream_headers(envelope)?;
    if envelope.get("type").and_then(Value::as_str) != Some("CALLBACK") {
        return None;
    }
    let topic = headers.get("topic").and_then(Value::as_str)?;
    if topic != DINGTALK_ROBOT_TOPIC {
        return None;
    }

    let data = envelope.get("data").and_then(Value::as_str)?;
    let body = serde_json::from_str::<Value>(data).ok()?;
    let event_id = dingtalk_string_field(&body, &["msgId", "messageId"])
        .or_else(|| dingtalk_string_map_field(headers, &["messageId"]))
        .unwrap_or_else(new_uuid_v4);
    let actor_id = dingtalk_string_field(&body, &["senderStaffId", "senderId"])
        .unwrap_or_else(|| "unknown".to_string());
    let conversation_id =
        dingtalk_string_field(&body, &["conversationId"]).unwrap_or_else(|| "unknown".to_string());
    let conversation_type = match dingtalk_string_field(&body, &["conversationType"]).as_deref() {
        Some("1") => "dm",
        _ => "group",
    };
    let message_text = dingtalk_message_text(&body).unwrap_or_default();
    let message_id = dingtalk_string_field(&body, &["msgId"]).unwrap_or_else(|| event_id.clone());
    let timestamp = dingtalk_string_field(&body, &["createAt"])
        .or_else(|| dingtalk_string_map_field(headers, &["time"]))
        .unwrap_or_default();
    let mut raw = body.as_object().cloned().unwrap_or_default();
    raw.insert("streamHeaders".to_string(), Value::Object(headers.clone()));
    if let Some(session_webhook) = dingtalk_string_field(&body, &["sessionWebhook"]) {
        raw.insert("context_token".to_string(), Value::String(session_webhook));
    }

    let event = json!({
        "id": event_id,
        "platform": config::BOT_PLATFORM_DINGTALK,
        "tenantId": config.tenant_id,
        "integrationId": config.integration_id,
        "type": "message.created",
        "actor": {
            "id": actor_id,
            "displayName": dingtalk_string_field(&body, &["senderNick"]),
            "isBot": false
        },
        "conversation": {
            "id": conversation_id,
            "type": conversation_type,
            "title": dingtalk_string_field(&body, &["conversationTitle"])
        },
        "message": {
            "id": message_id,
            "text": message_text
        },
        "timestamp": timestamp,
        "raw": Value::Object(raw)
    });
    let queue_id = format!(
        "{}:{}:{}:{}",
        config.tenant_id,
        config.integration_id,
        config::BOT_PLATFORM_DINGTALK,
        event.get("id").and_then(Value::as_str).unwrap_or("unknown")
    );

    Some(json!({
        "id": queue_id,
        "source": "dingtalk_stream",
        "event": event,
        "enqueuedAt": unix_seconds().to_string()
    }))
}

fn dingtalk_stream_response(envelope: &Value, success: bool) -> Option<Value> {
    let headers = dingtalk_stream_headers(envelope)?;
    let topic = headers
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if envelope.get("type").and_then(Value::as_str) == Some("SYSTEM") && topic == "disconnect" {
        return None;
    }

    let message_id = headers
        .get("messageId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let data = if topic == "ping" {
        envelope
            .get("data")
            .and_then(Value::as_str)
            .and_then(|data| serde_json::from_str::<Value>(data).ok())
            .unwrap_or_else(|| json!({}))
    } else if envelope.get("type").and_then(Value::as_str) == Some("EVENT") {
        json!({
            "status": if success { "SUCCESS" } else { "LATER" },
            "message": if success { "success" } else { "failed" }
        })
    } else {
        json!({ "response": null })
    };

    Some(json!({
        "code": if success { 200 } else { 500 },
        "headers": {
            "contentType": "application/json",
            "messageId": message_id
        },
        "message": if success { "OK" } else { "ERROR" },
        "data": serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
    }))
}

fn dingtalk_stream_headers(envelope: &Value) -> Option<&Map<String, Value>> {
    envelope
        .get("headers")
        .or_else(|| envelope.get("header"))
        .and_then(Value::as_object)
}

fn dingtalk_message_text(body: &Value) -> Option<String> {
    if let Some(text) = body
        .get("text")
        .and_then(|text| text.get("content"))
        .and_then(Value::as_str)
    {
        return Some(text.trim().to_string());
    }

    if let Some(content) = body.get("content").and_then(Value::as_object) {
        if let Some(text) = content.get("text").and_then(Value::as_str) {
            return Some(text.trim().to_string());
        }
        if let Some(rich_text) = content.get("richText").and_then(Value::as_array) {
            let text = rich_text
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if !text.trim().is_empty() {
                return Some(text.trim().to_string());
            }
        }
        if let Some(unknown) = content.get("unknownMsgType").and_then(Value::as_str) {
            return Some(unknown.trim().to_string());
        }
    }

    None
}

fn event_message_text(event: &Value) -> Option<String> {
    if let Some(text) = event
        .get("message")
        .and_then(|message| message.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    let platform = event
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if platform == config::BOT_PLATFORM_DINGTALK {
        if let Some(text) = event.get("raw").and_then(dingtalk_message_text) {
            if !text.trim().is_empty() {
                return Some(text.trim().to_string());
            }
        }
    }

    event_string_at_paths(
        event,
        &[
            "/message/content",
            "/raw/message/text",
            "/raw/message/content",
            "/raw/text/content",
            "/raw/content/text",
            "/raw/content",
        ],
    )
    .map(str::to_string)
}

fn dingtalk_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = value.get(*key) else {
            continue;
        };
        if let Some(value) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
        if value.is_number() || value.is_boolean() {
            return Some(value.to_string());
        }
    }
    None
}

fn dingtalk_string_map_field(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        if let Some(value) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
        if value.is_number() || value.is_boolean() {
            return Some(value.to_string());
        }
    }
    None
}

fn load_dingtalk_integration_auth(
    config: &BotBridgeConfig,
) -> Result<DingtalkIntegrationAuth, String> {
    let Some(state_dir) = config.state_dir.as_ref() else {
        return Err("no Bot Gateway state_dir configured".to_string());
    };
    let path = state_dir.join("integrations.json");
    let store = fs::read_to_string(&path).map_err(|err| {
        format!(
            "failed to read DingTalk integration store {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;
    let value = serde_json::from_str::<Value>(&store).map_err(|err| {
        format!(
            "failed to parse DingTalk integration store {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;
    let integration = find_integration(&value, &config.integration_id)
        .ok_or_else(|| format!("DingTalk integration {} not found", config.integration_id))?;
    if integration.get("platform").and_then(Value::as_str) != Some(config::BOT_PLATFORM_DINGTALK) {
        return Err(format!(
            "integration {} is not a DingTalk integration",
            config.integration_id
        ));
    }
    let config_object = integration.get("config").and_then(Value::as_object);
    let credentials = integration
        .get("encryptedCredentials")
        .or_else(|| integration.get("credentials"))
        .and_then(Value::as_object);
    let app_key = string_from_maps(&[config_object, credentials], &["appKey", "clientId"])
        .ok_or_else(|| "DingTalk appKey is missing".to_string())?;
    let app_secret = string_from_maps(
        &[credentials, config_object],
        &["appSecret", "clientSecret"],
    )
    .ok_or_else(|| "DingTalk appSecret is missing".to_string())?;

    Ok(DingtalkIntegrationAuth {
        app_key,
        app_secret,
    })
}

fn string_from_maps(maps: &[Option<&Map<String, Value>>], keys: &[&str]) -> Option<String> {
    for map in maps {
        let Some(map) = map else {
            continue;
        };
        for key in keys {
            if let Some(value) = map
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn handle_queued_event(
    bot: &mut BotGatewayClient,
    app: &mut AppServerBridge,
    queued: &Value,
) -> Result<(), String> {
    let event = queued
        .get("event")
        .ok_or_else(|| "queued event missing event".to_string())?;
    if event.get("integrationId").and_then(Value::as_str)
        != Some(app.config.integration_id.as_str())
    {
        return Ok(());
    }

    let event_id = queued
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| event.get("id").and_then(Value::as_str))
        .ok_or_else(|| "queued event missing id".to_string())?;
    let requires_gateway_ack = queued_event_requires_bot_gateway_ack(queued);

    if event
        .get("actor")
        .and_then(|actor| actor.get("isBot"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if requires_gateway_ack {
            let _ = bot.request("events.ack", json!({ "eventId": event_id }));
        }
        return Ok(());
    }

    let message_text = event_message_text(event).unwrap_or_default();
    let has_attachments =
        message_attachments(event).is_some_and(|attachments| !attachments.is_empty());
    if message_text.is_empty() && !has_attachments {
        if requires_gateway_ack {
            let _ = bot.request("events.ack", json!({ "eventId": event_id }));
        }
        return Ok(());
    }
    let message_text = if message_text.is_empty() {
        "Please review the attached media/file(s).".to_string()
    } else {
        message_text
    };

    if let Some(notice) = app.session_restore_notice(event) {
        send_bot_text_response(
            bot,
            &app.config,
            event,
            &format!("codexl:{}:session-switch", event_id),
            &notice,
        )?;
    }
    app.restore_session_for_event(event);

    log_bridge(
        &app.config,
        &format!(
            "received event event_id={} conversation={} actor={} text_len={}",
            event_id,
            event
                .get("conversation")
                .and_then(|conversation| conversation.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            event
                .get("actor")
                .and_then(|actor| actor.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            message_text.chars().count()
        ),
    );

    let event_key = event_id.to_string();
    let response = if let Some(response) = app.completed_events.get(&event_key) {
        log_bridge(
            &app.config,
            &format!(
                "retrying cached outbound response event_id={} text_len={} already_sent={}",
                event_id,
                response.response_text.chars().count(),
                response.already_sent
            ),
        );
        response.clone()
    } else {
        let action = match app.apply_bot_message(&message_text) {
            Ok(action) => action,
            Err(err) => BotMessageAction::Reply(err),
        };

        let response = match action {
            BotMessageAction::Reply(text) => CompletedEventResponse {
                response_text: text,
                already_sent: false,
            },
            BotMessageAction::Run(message_text) => {
                app.run_codex_turn(bot, &message_text, event, event_id)
            }
            BotMessageAction::SwitchProjectAndRun(project_switch) => {
                let notice = project_switch_notice(&project_switch.project, app.config.language);
                send_bot_text_response(
                    bot,
                    &app.config,
                    event,
                    &format!("codexl:{}:project-switch", event_id),
                    &notice,
                )?;
                app.apply_project_switch(&project_switch.project);
                app.run_codex_turn(bot, &project_switch.message_text, event, event_id)
            }
        };
        if let Err(err) = app.persist_current_session() {
            log_bridge(
                &app.config,
                &format!("failed to persist bot session state: {}", err),
            );
        }
        app.completed_events
            .insert(event_key.clone(), response.clone());
        response
    };

    if !response.already_sent {
        send_bot_text_response(
            bot,
            &app.config,
            event,
            &format!("codexl:{}", event_id),
            &response.response_text,
        )?;
        log_bridge(
            &app.config,
            &format!(
                "sent outbound response event_id={} conversation={} text_len={}",
                event_id,
                event
                    .get("conversation")
                    .and_then(|conversation| conversation.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                response.response_text.chars().count()
            ),
        );
        if let Some(cached) = app.completed_events.get_mut(&event_key) {
            cached.already_sent = true;
        }
    }
    if requires_gateway_ack {
        bot.request("events.ack", json!({ "eventId": event_id }))?;
        log_bridge(&app.config, &format!("acked event event_id={}", event_id));
    }
    app.completed_events.remove(&event_key);
    Ok(())
}

fn queued_event_requires_bot_gateway_ack(queued: &Value) -> bool {
    queued.get("source").and_then(Value::as_str) != Some("dingtalk_stream")
}

impl AppServerBridge {
    fn collect_dingtalk_stream_events(&mut self) -> Vec<Value> {
        let mut events: Vec<Value> = self.pending_dingtalk_events.drain(..).collect();
        if let Some(rx) = self.dingtalk_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(queued) => events.push(queued),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }
        events
    }

    fn defer_dingtalk_stream_events(&mut self, events: Vec<Value>) {
        self.pending_dingtalk_events.extend(events);
    }

    fn process_idle_app_output(&mut self, bot: &mut BotGatewayClient) {
        for _ in 0..100 {
            let event = match self.event_hub.try_next_event(&mut self.idle_cursor) {
                Ok(Some(event)) => event,
                Ok(None) => return,
                Err(CodexEventHubError::Disconnected) => return,
                Err(err) => {
                    log_bridge(
                        &self.config,
                        &format!("idle event cursor skipped: {}", err.message()),
                    );
                    self.idle_cursor = self.event_hub.cursor_now();
                    return;
                }
            };
            if let Err(err) = self.handle_idle_app_event(bot, &event) {
                log_bridge(
                    &self.config,
                    &format!("idle handoff output handling failed: {}", err),
                );
            }
        }
    }

    fn handle_idle_app_event(
        &mut self,
        bot: &mut BotGatewayClient,
        event: &CodexEvent,
    ) -> Result<(), String> {
        let Some(value) = event.value.as_ref() else {
            return Ok(());
        };
        let method = event.method.as_deref().unwrap_or("");
        let params = value.get("params").unwrap_or(&Value::Null);

        if is_bot_approval_request_method(method) {
            return self.handle_idle_handoff_approval_request(bot, &value, method, params);
        }

        if method == "item/agentMessage/delta" {
            if let Some(key) = idle_handoff_turn_key(params) {
                if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                    self.idle_handoff_turn_captures
                        .entry(key)
                        .or_default()
                        .fallback_text
                        .push_str(delta);
                }
            }
            return Ok(());
        }

        if method == "turn/completed" {
            return self.handle_idle_handoff_turn_completed(bot, params);
        }

        if method != "item/completed" {
            return Ok(());
        }

        if let Some(key) = idle_handoff_turn_key(params) {
            self.idle_handoff_turn_captures
                .entry(key)
                .or_default()
                .capture_completed_item(params);
        }

        let should_check_handoff_state =
            self.config.forward_all_codex_messages || !self.handoff_active_threads.is_empty();
        if !should_check_handoff_state {
            return Ok(());
        }

        let forward_decision = self.handoff_forward_decision();
        if forward_decision.handoff_presence.is_none() {
            if let Some(context) = self.idle_handoff_context(params) {
                self.ensure_handoff_deactivation_notice(
                    bot,
                    &context.event,
                    &context.thread_id,
                    &context.project,
                    forward_decision.handoff_evaluation.as_ref(),
                )?;
            }
        }

        let Some(text) = completed_agent_message_text(params) else {
            return Ok(());
        };

        if !self.config.forward_all_codex_messages {
            return Ok(());
        }
        if !forward_decision.should_forward {
            return Ok(());
        }
        let Some(context) = self.idle_handoff_context(params) else {
            log_bridge(
                &self.config,
                "handoff skipped: no recent bot conversation context",
            );
            return Ok(());
        };

        self.send_idle_handoff_text(
            bot,
            &context,
            forward_decision.handoff_presence.as_ref(),
            &text,
            "item",
        )
    }

    fn handle_idle_handoff_approval_request(
        &mut self,
        bot: &mut BotGatewayClient,
        value: &Value,
        method: &str,
        params: &Value,
    ) -> Result<(), String> {
        let forward_decision = self.handoff_forward_decision();
        let Some(context) = self.idle_handoff_context(params) else {
            log_bridge(
                &self.config,
                "handoff approval skipped: no recent bot conversation context",
            );
            return Ok(());
        };

        if forward_decision.handoff_presence.is_none() {
            self.ensure_handoff_deactivation_notice(
                bot,
                &context.event,
                &context.thread_id,
                &context.project,
                forward_decision.handoff_evaluation.as_ref(),
            )?;
        }

        if !forward_decision.should_forward {
            log_bridge(
                &self.config,
                &format!("handoff approval skipped: not away method={}", method),
            );
            return Ok(());
        }

        if let Some(presence) = forward_decision.handoff_presence.as_ref() {
            self.ensure_handoff_activation_notice(
                bot,
                &context.event,
                &context.thread_id,
                &context.project,
                presence,
            )?;
        }

        let request_id = value
            .get("id")
            .cloned()
            .ok_or_else(|| format!("approval request {} missing id", method))?;
        let request_key = request_id_key(&request_id).unwrap_or_else(|| {
            self.idle_handoff_message_counter = self.idle_handoff_message_counter.saturating_add(1);
            format!("idle-{}", self.idle_handoff_message_counter)
        });
        let event_id = format!(
            "idle-handoff:{}:approval:{}",
            context.thread_id, request_key
        );
        self.handle_bot_approval_request(
            bot,
            method,
            request_id,
            params.clone(),
            &context.event,
            &event_id,
            Instant::now() + self.config.turn_timeout,
        )
    }

    fn handle_idle_handoff_turn_completed(
        &mut self,
        bot: &mut BotGatewayClient,
        params: &Value,
    ) -> Result<(), String> {
        let key = idle_handoff_turn_key(params);
        let capture = key
            .as_ref()
            .and_then(|key| self.idle_handoff_turn_captures.remove(key))
            .unwrap_or_default();
        let error_text = turn_completed_error_message(params)
            .map(|message| format!("Codex turn failed: {}", message));
        let has_error = error_text.is_some();

        let mut checked_forward_decision = None;
        if !self.handoff_active_threads.is_empty() {
            let forward_decision = self.handoff_forward_decision();
            if forward_decision.handoff_presence.is_none() {
                if let Some(context) = self.idle_handoff_context(params) {
                    self.ensure_handoff_deactivation_notice(
                        bot,
                        &context.event,
                        &context.thread_id,
                        &context.project,
                        forward_decision.handoff_evaluation.as_ref(),
                    )?;
                }
            }
            checked_forward_decision = Some(forward_decision);
        }

        if self.config.forward_all_codex_messages && !has_error {
            return Ok(());
        }

        let text = error_text
            .or(capture.final_text)
            .unwrap_or(capture.fallback_text);
        if text.trim().is_empty() {
            return Ok(());
        }

        let forward_decision =
            checked_forward_decision.unwrap_or_else(|| self.handoff_forward_decision());
        let context = self.idle_handoff_context(params);
        if forward_decision.handoff_presence.is_none() {
            if let Some(context) = context.as_ref() {
                self.ensure_handoff_deactivation_notice(
                    bot,
                    &context.event,
                    &context.thread_id,
                    &context.project,
                    forward_decision.handoff_evaluation.as_ref(),
                )?;
            }
        }
        if !forward_decision.should_forward {
            return Ok(());
        }

        let Some(context) = context else {
            log_bridge(
                &self.config,
                "handoff turn completion skipped: no recent bot conversation context",
            );
            return Ok(());
        };

        self.send_idle_handoff_text(
            bot,
            &context,
            forward_decision.handoff_presence.as_ref(),
            &text,
            "turn-completed",
        )
    }

    fn idle_handoff_context(&self, params: &Value) -> Option<IdleHandoffContext> {
        let context = latest_bot_media_context(&self.config)?;
        let event = bot_event_from_media_context(&context);
        let thread_id = nested_param_id(params, "threadId", "thread")
            .or(context.thread_id.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let project = params
            .get("cwd")
            .and_then(Value::as_str)
            .or_else(|| {
                params
                    .get("thread")
                    .and_then(|thread| thread.get("cwd"))
                    .and_then(Value::as_str)
            })
            .or(context.cwd.as_deref())
            .unwrap_or(PROJECTLESS_PROJECT_LABEL)
            .to_string();
        Some(IdleHandoffContext {
            event,
            thread_id,
            project,
        })
    }

    fn send_idle_handoff_text(
        &mut self,
        bot: &mut BotGatewayClient,
        context: &IdleHandoffContext,
        presence: Option<&HandoffPresence>,
        text: &str,
        kind: &str,
    ) -> Result<(), String> {
        if let Some(presence) = presence {
            self.ensure_handoff_activation_notice(
                bot,
                &context.event,
                &context.thread_id,
                &context.project,
                presence,
            )?;
        }

        self.idle_handoff_message_counter = self.idle_handoff_message_counter.saturating_add(1);
        send_bot_text_response(
            bot,
            &self.config,
            &context.event,
            &format!(
                "codexl:idle-handoff:{}:{}",
                context.thread_id, self.idle_handoff_message_counter
            ),
            text,
        )?;
        log_bridge(
            &self.config,
            &format!(
                "forwarded idle Codex message kind={} thread={} text_len={}",
                kind,
                context.thread_id,
                text.chars().count()
            ),
        );
        Ok(())
    }

    fn ensure_handoff_activation_notice(
        &mut self,
        bot: &mut BotGatewayClient,
        event: &Value,
        thread_id: &str,
        project: &str,
        presence: &HandoffPresence,
    ) -> Result<(), String> {
        if self.handoff_active_threads.contains_key(thread_id) {
            return Ok(());
        }
        let notice = handoff_activation_notice_for_context(
            thread_id,
            project,
            presence,
            self.config.language,
        );
        self.idle_handoff_message_counter = self.idle_handoff_message_counter.saturating_add(1);
        send_bot_text_response(
            bot,
            &self.config,
            event,
            &format!(
                "codexl:idle-handoff:{}:on:{}",
                thread_id, self.idle_handoff_message_counter
            ),
            &notice,
        )?;
        self.handoff_active_threads
            .insert(thread_id.to_string(), unix_seconds());
        Ok(())
    }

    fn ensure_handoff_deactivation_notice(
        &mut self,
        bot: &mut BotGatewayClient,
        event: &Value,
        thread_id: &str,
        project: &str,
        presence: Option<&HandoffPresence>,
    ) -> Result<(), String> {
        if !self.handoff_active_threads.contains_key(thread_id) {
            return Ok(());
        }
        let notice = handoff_deactivation_notice_for_context(
            thread_id,
            project,
            presence,
            self.config.language,
        );
        self.idle_handoff_message_counter = self.idle_handoff_message_counter.saturating_add(1);
        send_bot_text_response(
            bot,
            &self.config,
            event,
            &format!(
                "codexl:idle-handoff:{}:off:{}",
                thread_id, self.idle_handoff_message_counter
            ),
            &notice,
        )?;
        self.handoff_active_threads.remove(thread_id);
        Ok(())
    }

    fn run_codex_turn(
        &mut self,
        bot: &mut BotGatewayClient,
        message_text: &str,
        event: &Value,
        event_id: &str,
    ) -> CompletedEventResponse {
        let turn_result = (|| -> Result<CodexTurnResult, String> {
            let thread_id = self.ensure_thread(message_text)?;
            self.persist_bot_media_context(event, event_id)?;
            let (turn_id, cursor) = self.start_turn(&thread_id, message_text, event)?;
            self.wait_turn_completed(bot, &thread_id, &turn_id, event, event_id, cursor)
        })();
        match turn_result {
            Ok(result) if !result.response_text.trim().is_empty() => CompletedEventResponse {
                already_sent: result.sent_messages > 0,
                response_text: result.response_text,
            },
            Ok(result) => CompletedEventResponse {
                already_sent: result.sent_messages > 0,
                response_text: "Codex completed the turn without a text response.".to_string(),
            },
            Err(err) => {
                log_bridge(
                    &self.config,
                    &format!("Codex turn for event {} failed: {}", event_id, err),
                );
                CompletedEventResponse {
                    response_text: format!("Codex turn failed: {}", err),
                    already_sent: false,
                }
            }
        }
    }
}

fn ensure_outbound_sent(result: &Value) -> Result<(), String> {
    let delivery = result.get("result").unwrap_or(result);
    let status = delivery
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "sent" {
        return Ok(());
    }

    let code = delivery
        .get("errorCode")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = delivery
        .get("errorMessage")
        .and_then(Value::as_str)
        .unwrap_or("Bot Gateway outbound did not report a sent delivery");
    Err(format!(
        "Bot Gateway outbound delivery status={} code={} message={}",
        status, code, message
    ))
}

fn send_dingtalk_text_response(
    config: &BotBridgeConfig,
    event: &Value,
    text: &str,
) -> Result<(), String> {
    let sender_staff_id = event
        .get("raw")
        .and_then(|raw| raw.get("senderStaffId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut body = json!({
        "msgtype": "text",
        "text": {
            "content": text
        }
    });
    if let Some(sender_staff_id) = sender_staff_id {
        body.as_object_mut().unwrap().insert(
            "at".to_string(),
            json!({
                "atUserIds": [sender_staff_id],
                "isAtAll": false
            }),
        );
    }

    send_dingtalk_session_webhook_message(config, event, &body)
}

fn send_dingtalk_approval_card(
    config: &BotBridgeConfig,
    event: &Value,
    idempotency_key: &str,
    prompt: &BotApprovalPrompt,
) -> Result<Value, String> {
    let body = dingtalk_approval_action_card(prompt);
    send_dingtalk_session_webhook_message(config, event, &body)?;
    Ok(json!({
        "result": {
            "id": idempotency_key,
            "status": "sent",
            "platform": config::BOT_PLATFORM_DINGTALK,
            "integrationId": event_integration_id(event, &config.integration_id)
        }
    }))
}

fn send_dingtalk_session_webhook_message(
    config: &BotBridgeConfig,
    event: &Value,
    body: &Value,
) -> Result<(), String> {
    let session_webhook = dingtalk_event_session_webhook(event)
        .ok_or_else(|| "DingTalk event is missing sessionWebhook".to_string())?;
    let auth = load_dingtalk_integration_auth(config)?;
    let access_token = dingtalk_access_token(&auth)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to create DingTalk send runtime: {}", err))?;
    runtime.block_on(async {
        let response = reqwest::Client::new()
            .post(session_webhook)
            .header("x-acs-dingtalk-access-token", access_token)
            .json(&body)
            .send()
            .await
            .map_err(|err| format!("DingTalk sessionWebhook send failed: {}", err))?;
        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|err| format!("failed to read DingTalk send response: {}", err))?;
        if !status.is_success() {
            return Err(format!(
                "DingTalk sessionWebhook returned HTTP {}: {}",
                status, response_text
            ));
        }
        ensure_dingtalk_send_response_ok(&response_text)
    })
}

fn dingtalk_approval_action_card(prompt: &BotApprovalPrompt) -> Value {
    let buttons: Vec<Value> = prompt
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let choice_number = index + 1;
            json!({
                "title": format!("{}. {}", choice_number, action.label),
                "actionURL": dingtalk_send_message_action_url(&choice_number.to_string())
            })
        })
        .collect();

    json!({
        "msgtype": "actionCard",
        "actionCard": {
            "title": prompt.title.clone(),
            "text": dingtalk_approval_action_card_text(prompt),
            "btnOrientation": if prompt.actions.len() <= 2 { "1" } else { "0" },
            "btns": buttons
        }
    })
}

fn dingtalk_approval_action_card_text(prompt: &BotApprovalPrompt) -> String {
    let mut lines = vec![
        format!("### {}", prompt.title),
        String::new(),
        prompt.body.clone(),
    ];
    for field in &prompt.fields {
        lines.push(String::new());
        lines.push(format!("**{}**", field.label));
        lines.push(field.value.clone());
    }
    if !prompt.actions.is_empty() {
        lines.push(String::new());
        lines.push("**Options**".to_string());
        for (index, action) in prompt.actions.iter().enumerate() {
            lines.push(format!("{}. {}", index + 1, action.label));
        }
        lines.push(String::new());
        lines.push("Click a button, or reply with an option number or label.".to_string());
    }
    lines.join("\n\n")
}

fn dingtalk_send_message_action_url(content: &str) -> String {
    format!(
        "dtmd://dingtalkclient/sendMessage?content={}",
        url_encode_query_component(content)
    )
}

fn url_encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

fn dingtalk_event_session_webhook(event: &Value) -> Option<&str> {
    event
        .get("raw")
        .and_then(|raw| raw.get("sessionWebhook"))
        .or_else(|| event.get("raw").and_then(|raw| raw.get("context_token")))
        .or_else(|| event.get("raw").and_then(|raw| raw.get("contextToken")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn ensure_dingtalk_send_response_ok(response_text: &str) -> Result<(), String> {
    let value = match serde_json::from_str::<Value>(response_text) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if let Some(errcode) = value.get("errcode").and_then(Value::as_i64) {
        if errcode != 0 {
            return Err(format!(
                "DingTalk send failed errcode={}: {}",
                errcode, response_text
            ));
        }
    }
    if let Some(errcode) = value
        .get("errcode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
    {
        return Err(format!(
            "DingTalk send failed errcode={}: {}",
            errcode, response_text
        ));
    }
    if let Some(code) = value
        .get("code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0" && *value != "OK")
    {
        let message = value
            .get("message")
            .or_else(|| value.get("errmsg"))
            .and_then(Value::as_str)
            .unwrap_or("DingTalk send failed");
        return Err(format!("DingTalk send failed code={}: {}", code, message));
    }
    Ok(())
}

fn dingtalk_access_token(auth: &DingtalkIntegrationAuth) -> Result<String, String> {
    let cache = DINGTALK_ACCESS_TOKEN_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let now = Instant::now();
    if let Ok(cache) = cache.lock() {
        if let Some(cached) = cache.get(&auth.app_key) {
            if cached.expires_at > now + Duration::from_secs(60) {
                return Ok(cached.token.clone());
            }
        }
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to create DingTalk token runtime: {}", err))?;
    let fetched = runtime.block_on(fetch_dingtalk_access_token(auth))?;
    if let Ok(mut cache) = cache.lock() {
        cache.insert(auth.app_key.clone(), fetched.clone());
    }
    Ok(fetched.token)
}

async fn fetch_dingtalk_access_token(
    auth: &DingtalkIntegrationAuth,
) -> Result<DingtalkAccessToken, String> {
    let response = reqwest::Client::new()
        .post(DINGTALK_ACCESS_TOKEN_URL)
        .json(&json!({
            "appKey": auth.app_key,
            "appSecret": auth.app_secret
        }))
        .send()
        .await
        .map_err(|err| format!("failed to request DingTalk access token: {}", err))?;
    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|err| format!("failed to read DingTalk access token response: {}", err))?;
    if !status.is_success() {
        return Err(format!(
            "DingTalk access token returned HTTP {}: {}",
            status, response_text
        ));
    }
    let value = serde_json::from_str::<Value>(&response_text)
        .map_err(|err| format!("failed to parse DingTalk access token response: {}", err))?;
    let token = value
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "DingTalk access token response missing token: {}",
                response_text
            )
        })?;
    let expire_in = value
        .get("expireIn")
        .and_then(Value::as_u64)
        .unwrap_or(7200);
    let cache_seconds = expire_in.saturating_sub(120).max(60);

    Ok(DingtalkAccessToken {
        token: token.to_string(),
        expires_at: Instant::now() + Duration::from_secs(cache_seconds),
    })
}

fn send_bot_text_response(
    bot: &mut BotGatewayClient,
    config: &BotBridgeConfig,
    event: &Value,
    idempotency_key: &str,
    text: &str,
) -> Result<(), String> {
    if event.get("platform").and_then(Value::as_str) == Some(config::BOT_PLATFORM_DINGTALK)
        && dingtalk_event_session_webhook(event).is_some()
    {
        return send_dingtalk_text_response(config, event, text);
    }

    let outbound_result =
        send_bot_text_response_with_result(bot, config, event, idempotency_key, text)?;
    ensure_outbound_sent(&outbound_result)
}

fn send_bot_text_response_with_result(
    bot: &mut BotGatewayClient,
    config: &BotBridgeConfig,
    event: &Value,
    idempotency_key: &str,
    text: &str,
) -> Result<Value, String> {
    let outbound = json!({
        "tenantId": event.get("tenantId").cloned().unwrap_or(Value::String("default".to_string())),
        "integrationId": event_integration_id(event, &config.integration_id),
        "conversationRef": conversation_ref(event),
        "intent": {
            "type": "text",
            "text": text,
        },
        "idempotencyKey": idempotency_key,
    });
    bot.request("outbound.send", outbound)
}

fn send_bot_approval_prompt(
    bot: &mut BotGatewayClient,
    config: &BotBridgeConfig,
    event: &Value,
    idempotency_key: &str,
    prompt: &BotApprovalPrompt,
) -> Result<Value, String> {
    if is_feishu_bot_event(event, config)
        || is_slack_bot_event(event, config)
        || is_discord_bot_event(event, config)
    {
        let card = if is_slack_bot_event(event, config) || is_discord_bot_event(event, config) {
            bot_approval_string_callback_card(prompt, None)
        } else {
            bot_approval_card(prompt, None)
        };
        let outbound = json!({
            "tenantId": event.get("tenantId").cloned().unwrap_or(Value::String("default".to_string())),
            "integrationId": event_integration_id(event, &config.integration_id),
            "conversationRef": conversation_ref(event),
            "intent": {
                "type": "card",
                "fallbackText": bot_approval_text(prompt),
                "card": card,
            },
            "idempotencyKey": idempotency_key,
        });
        let outbound_result = bot.request("outbound.send", outbound)?;
        ensure_outbound_sent(&outbound_result)?;
        return Ok(outbound_result);
    }
    if is_dingtalk_bot_event(event, config) {
        return send_dingtalk_approval_card(config, event, idempotency_key, prompt);
    }

    let outbound_result = send_bot_text_response_with_result(
        bot,
        config,
        event,
        idempotency_key,
        &bot_approval_text(prompt),
    )?;
    ensure_outbound_sent(&outbound_result)?;
    Ok(outbound_result)
}

fn update_bot_approval_card_status(
    bot: &mut BotGatewayClient,
    config: &BotBridgeConfig,
    event: &Value,
    message_id: &str,
    prompt: &BotApprovalPrompt,
    action: &BotApprovalAction,
) -> Result<(), String> {
    if !is_feishu_bot_event(event, config) {
        return Ok(());
    }

    let result = bot.request(
        "outbound.updateCard",
        json!({
            "tenantId": event.get("tenantId").cloned().unwrap_or(Value::String("default".to_string())),
            "integrationId": event_integration_id(event, &config.integration_id),
            "messageId": message_id,
            "card": bot_approval_card(prompt, Some(action)),
            "fallbackText": bot_approval_status_text(prompt, action),
        }),
    )?;
    ensure_outbound_sent(&result)
}

fn acknowledge_discord_approval_interaction(
    bot: &mut BotGatewayClient,
    config: &BotBridgeConfig,
    event: &Value,
) -> Result<(), String> {
    let Some(interaction_id) = event.pointer("/raw/id").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(interaction_token) = event.pointer("/raw/token").and_then(Value::as_str) else {
        return Ok(());
    };
    bot.request(
        "discord.interactions.callback",
        json!({
            "integrationId": event_integration_id(event, &config.integration_id),
            "interactionId": interaction_id,
            "interactionToken": interaction_token,
            "type": 6,
        }),
    )?;
    Ok(())
}

fn outbound_platform_message_id(result: &Value) -> Option<String> {
    result
        .get("result")
        .unwrap_or(result)
        .get("platformMessageId")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn bot_approval_card(prompt: &BotApprovalPrompt, selected: Option<&BotApprovalAction>) -> Value {
    let fields: Vec<Value> = prompt
        .fields
        .iter()
        .map(|field| {
            json!({
                "label": field.label.clone(),
                "value": field.value.clone(),
            })
        })
        .collect();
    let actions: Vec<Value> = match selected {
        Some(action) => vec![bot_approval_card_action(prompt, action, true)],
        None => prompt
            .actions
            .iter()
            .map(|action| bot_approval_card_action(prompt, action, false))
            .collect(),
    };
    let body = selected
        .map(|action| bot_approval_status_body(prompt, action))
        .unwrap_or_else(|| prompt.body.clone());

    json!({
        "title": prompt.title.clone(),
        "body": body,
        "fields": fields,
        "actions": actions,
    })
}

fn bot_approval_string_callback_card(
    prompt: &BotApprovalPrompt,
    selected: Option<&BotApprovalAction>,
) -> Value {
    let fields: Vec<Value> = prompt
        .fields
        .iter()
        .map(|field| {
            json!({
                "label": field.label.clone(),
                "value": field.value.clone(),
            })
        })
        .collect();
    let actions: Vec<Value> = match selected {
        Some(action) => vec![bot_approval_string_callback_card_action(
            prompt, action, true,
        )],
        None => prompt
            .actions
            .iter()
            .map(|action| bot_approval_string_callback_card_action(prompt, action, false))
            .collect(),
    };
    let body = selected
        .map(|action| bot_approval_status_body(prompt, action))
        .unwrap_or_else(|| prompt.body.clone());

    json!({
        "title": prompt.title.clone(),
        "body": body,
        "fields": fields,
        "actions": actions,
    })
}

fn bot_approval_card_action(
    prompt: &BotApprovalPrompt,
    action: &BotApprovalAction,
    disabled: bool,
) -> Value {
    let mut value = json!({
        "label": action.label.clone(),
        "value": {
            "kind": "codex_approval",
            "requestId": prompt.request_key.clone(),
            "choice": action.key.clone(),
        },
    });
    if disabled {
        value["disabled"] = Value::Bool(true);
    }
    value
}

fn bot_approval_string_callback_card_action(
    prompt: &BotApprovalPrompt,
    action: &BotApprovalAction,
    disabled: bool,
) -> Value {
    let payload = bot_approval_callback_payload_string(prompt, action);
    let mut value = json!({
        "label": action.label.clone(),
        "value": payload,
        "customId": payload,
    });
    if disabled {
        value["disabled"] = Value::Bool(true);
    }
    value
}

fn bot_approval_callback_payload_string(
    prompt: &BotApprovalPrompt,
    action: &BotApprovalAction,
) -> String {
    serde_json::to_string(&json!({
        "kind": "codex_approval",
        "requestId": prompt.request_key.clone(),
        "choice": action.key.clone(),
    }))
    .unwrap_or_else(|_| action.key.clone())
}

fn bot_approval_text(prompt: &BotApprovalPrompt) -> String {
    let mut lines = vec![prompt.title.clone(), String::new(), prompt.body.clone()];
    for field in &prompt.fields {
        lines.push(String::new());
        lines.push(format!("{}: {}", field.label, field.value));
    }
    if !prompt.actions.is_empty() {
        lines.push(String::new());
        lines.push("Options:".to_string());
        for (index, action) in prompt.actions.iter().enumerate() {
            lines.push(format!("{}. {}", index + 1, action.label));
        }
        lines.push(String::new());
        lines.push("Reply with an option number or label.".to_string());
    }
    lines.join("\n")
}

fn bot_approval_status_text(prompt: &BotApprovalPrompt, action: &BotApprovalAction) -> String {
    format!(
        "{}\n\n{}\n\nStatus: {}",
        prompt.title, prompt.body, action.label
    )
}

fn bot_approval_status_body(prompt: &BotApprovalPrompt, action: &BotApprovalAction) -> String {
    format!("{}\n\n**Status**\n{}", prompt.body, action.label)
}

fn is_bot_approval_request_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "mcpServer/elicitation/request"
            | "item/permissions/requestApproval"
    )
}

fn matches_approval_request_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    matches_thread_turn(params, thread_id, turn_id)
}

fn build_bot_approval_prompt(
    method: &str,
    request_key: &str,
    params: &Value,
) -> Result<BotApprovalPrompt, String> {
    match method {
        "item/commandExecution/requestApproval" => {
            Ok(build_command_approval_prompt(request_key, params))
        }
        "item/fileChange/requestApproval" => {
            Ok(build_file_change_approval_prompt(request_key, params))
        }
        "mcpServer/elicitation/request" => Ok(build_mcp_elicitation_prompt(request_key, params)),
        "item/permissions/requestApproval" => {
            Ok(build_permissions_approval_prompt(request_key, params))
        }
        _ => Err(format!("unsupported approval request method: {}", method)),
    }
}

fn build_command_approval_prompt(request_key: &str, params: &Value) -> BotApprovalPrompt {
    let has_network_context = params
        .get("networkApprovalContext")
        .is_some_and(|value| !value.is_null());
    let has_additional_permissions = params
        .get("additionalPermissions")
        .is_some_and(|value| !value.is_null());
    let mut fields = Vec::new();
    if let Some(command) = params.get("command").and_then(Value::as_str) {
        fields.push(approval_field("Command", command));
    }
    if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
        fields.push(approval_field("Working directory", cwd));
    }
    if let Some(reason) = params.get("reason").and_then(Value::as_str) {
        fields.push(approval_field("Reason", reason));
    }
    if let Some(network) = params
        .get("networkApprovalContext")
        .filter(|value| !value.is_null())
    {
        fields.push(approval_field("Network request", &compact_json(network)));
    }
    if let Some(permissions) = params
        .get("additionalPermissions")
        .filter(|value| !value.is_null())
    {
        fields.push(approval_field(
            "Additional permissions",
            &compact_json(permissions),
        ));
    }

    let decisions = params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .filter(|decisions| !decisions.is_empty())
        .cloned()
        .unwrap_or_else(|| vec![json!("accept"), json!("acceptForSession"), json!("cancel")]);
    let actions = decisions
        .iter()
        .enumerate()
        .map(|(index, decision)| BotApprovalAction {
            key: format!("command-{}", index + 1),
            label: command_decision_label(
                decision,
                has_network_context,
                has_additional_permissions,
            ),
            result: json!({ "decision": decision }),
        })
        .collect();

    BotApprovalPrompt {
        request_key: request_key.to_string(),
        title: "Codex permission request".to_string(),
        body: params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if has_network_context {
                    "Codex needs approval for network access.".to_string()
                } else {
                    "Codex wants to run a command.".to_string()
                }
            }),
        fields,
        actions,
    }
}

fn build_file_change_approval_prompt(request_key: &str, params: &Value) -> BotApprovalPrompt {
    let mut fields = Vec::new();
    if let Some(reason) = params.get("reason").and_then(Value::as_str) {
        fields.push(approval_field("Reason", reason));
    }
    if let Some(root) = params.get("grantRoot").and_then(Value::as_str) {
        fields.push(approval_field("Grant root", root));
    }

    BotApprovalPrompt {
        request_key: request_key.to_string(),
        title: "Codex file change approval".to_string(),
        body: params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "Codex wants to make file changes.".to_string()),
        fields,
        actions: vec![
            BotApprovalAction {
                key: "file-accept".to_string(),
                label: "Yes, proceed".to_string(),
                result: json!({ "decision": "accept" }),
            },
            BotApprovalAction {
                key: "file-session".to_string(),
                label: "Yes, and don't ask again for these files".to_string(),
                result: json!({ "decision": "acceptForSession" }),
            },
            BotApprovalAction {
                key: "file-cancel".to_string(),
                label: "No, and tell Codex what to do differently".to_string(),
                result: json!({ "decision": "cancel" }),
            },
        ],
    }
}

fn build_permissions_approval_prompt(request_key: &str, params: &Value) -> BotApprovalPrompt {
    let requested_permissions = params.get("permissions").unwrap_or(&Value::Null);
    let granted_permissions = granted_permissions_from_request(requested_permissions);
    let mut fields = Vec::new();
    if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
        fields.push(approval_field("Working directory", cwd));
    }
    if let Some(reason) = params.get("reason").and_then(Value::as_str) {
        fields.push(approval_field("Reason", reason));
    }
    fields.push(approval_field(
        "Requested permissions",
        &compact_json(requested_permissions),
    ));

    BotApprovalPrompt {
        request_key: request_key.to_string(),
        title: "Codex permission request".to_string(),
        body: params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "Codex requests additional permissions.".to_string()),
        fields,
        actions: vec![
            BotApprovalAction {
                key: "permissions-turn".to_string(),
                label: "Yes, grant these permissions for this turn".to_string(),
                result: json!({
                    "permissions": granted_permissions.clone(),
                    "scope": "turn",
                }),
            },
            BotApprovalAction {
                key: "permissions-strict".to_string(),
                label: "Yes, grant for this turn with strict auto review".to_string(),
                result: json!({
                    "permissions": granted_permissions.clone(),
                    "scope": "turn",
                    "strictAutoReview": true,
                }),
            },
            BotApprovalAction {
                key: "permissions-session".to_string(),
                label: "Yes, grant these permissions for this session".to_string(),
                result: json!({
                    "permissions": granted_permissions.clone(),
                    "scope": "session",
                }),
            },
            BotApprovalAction {
                key: "permissions-deny".to_string(),
                label: "No, continue without permissions".to_string(),
                result: json!({
                    "permissions": {},
                    "scope": "turn",
                }),
            },
        ],
    }
}

fn build_mcp_elicitation_prompt(request_key: &str, params: &Value) -> BotApprovalPrompt {
    let meta = params.get("_meta").filter(|value| !value.is_null());
    let is_tool_approval = meta
        .and_then(|meta| meta.get("codex_approval_kind"))
        .and_then(Value::as_str)
        == Some("mcp_tool_call");
    let is_message_only_schema = params
        .get("requestedSchema")
        .map(is_empty_object_schema)
        .unwrap_or(true);
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Codex requests approval.");

    let mut fields = Vec::new();
    if let Some(server_name) = params.get("serverName").and_then(Value::as_str) {
        fields.push(approval_field("MCP server", server_name));
    }
    if let Some(tool_description) = meta
        .and_then(|meta| meta.get("tool_description"))
        .and_then(Value::as_str)
    {
        fields.push(approval_field("Tool", tool_description));
    }
    if let Some(url) = params.get("url").and_then(Value::as_str) {
        fields.push(approval_field("URL", url));
    }
    append_tool_approval_display_fields(&mut fields, meta);

    let actions = if is_message_only_schema {
        let mut actions = vec![BotApprovalAction {
            key: "mcp-accept".to_string(),
            label: "Allow".to_string(),
            result: mcp_elicitation_result("accept", Value::Null),
        }];
        if approval_supports_persist_mode(meta, "session") {
            actions.push(BotApprovalAction {
                key: "mcp-accept-session".to_string(),
                label: "Allow for this session".to_string(),
                result: mcp_elicitation_result("accept", json!({ "persist": "session" })),
            });
        }
        if approval_supports_persist_mode(meta, "always") {
            actions.push(BotApprovalAction {
                key: "mcp-accept-always".to_string(),
                label: "Always allow".to_string(),
                result: mcp_elicitation_result("accept", json!({ "persist": "always" })),
            });
        }
        if is_tool_approval {
            actions.push(BotApprovalAction {
                key: "mcp-cancel".to_string(),
                label: "Cancel".to_string(),
                result: mcp_elicitation_result("cancel", Value::Null),
            });
        } else {
            actions.push(BotApprovalAction {
                key: "mcp-decline".to_string(),
                label: "Deny".to_string(),
                result: mcp_elicitation_result("decline", Value::Null),
            });
            actions.push(BotApprovalAction {
                key: "mcp-cancel".to_string(),
                label: "Cancel".to_string(),
                result: mcp_elicitation_result("cancel", Value::Null),
            });
        }
        actions
    } else {
        vec![BotApprovalAction {
            key: "mcp-cancel".to_string(),
            label: "Cancel".to_string(),
            result: mcp_elicitation_result("cancel", Value::Null),
        }]
    };

    BotApprovalPrompt {
        request_key: request_key.to_string(),
        title: if is_tool_approval {
            "Codex tool approval".to_string()
        } else {
            "Codex MCP request".to_string()
        },
        body: message.to_string(),
        fields,
        actions,
    }
}

fn command_decision_label(
    decision: &Value,
    has_network_context: bool,
    has_additional_permissions: bool,
) -> String {
    match decision.as_str() {
        Some("accept") => {
            if has_network_context {
                "Yes, just this once".to_string()
            } else {
                "Yes, proceed".to_string()
            }
        }
        Some("acceptForSession") => {
            if has_network_context {
                "Yes, and allow this host for this conversation".to_string()
            } else if has_additional_permissions {
                "Yes, and allow these permissions for this session".to_string()
            } else {
                "Yes, and don't ask again for this command in this session".to_string()
            }
        }
        Some("decline") => "No, continue without running it".to_string(),
        Some("cancel") => "No, and tell Codex what to do differently".to_string(),
        _ => {
            if let Some(prefix) = decision
                .get("acceptWithExecpolicyAmendment")
                .and_then(|value| value.get("execpolicy_amendment"))
                .and_then(|value| value.get("command"))
                .and_then(Value::as_str)
            {
                return format!(
                    "Yes, and don't ask again for commands that start with `{}`",
                    truncate_text(prefix, 96)
                );
            }
            if let Some(action) = decision
                .get("applyNetworkPolicyAmendment")
                .and_then(|value| value.get("network_policy_amendment"))
                .and_then(|value| value.get("action"))
                .and_then(Value::as_str)
            {
                return if action == "deny" {
                    "No, and block this host in the future".to_string()
                } else {
                    "Yes, and allow this host in the future".to_string()
                };
            }
            "Select this option".to_string()
        }
    }
}

fn approval_field(label: &str, value: &str) -> BotApprovalField {
    BotApprovalField {
        label: label.to_string(),
        value: truncate_text(value, 2_000),
    }
}

fn append_tool_approval_display_fields(fields: &mut Vec<BotApprovalField>, meta: Option<&Value>) {
    let Some(display) = meta
        .and_then(|meta| meta.get("tool_params_display"))
        .and_then(Value::as_array)
    else {
        return;
    };

    for item in display {
        let label = item
            .get("display_name")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("Parameter");
        let value = item
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                item.get("value")
                    .map(compact_json)
                    .unwrap_or_else(|| "null".to_string())
            });
        fields.push(approval_field(label, &value));
    }
}

fn mcp_elicitation_result(action: &str, meta: Value) -> Value {
    json!({
        "action": action,
        "content": Value::Null,
        "_meta": meta,
    })
}

fn granted_permissions_from_request(permissions: &Value) -> Value {
    let mut granted = Map::new();
    if let Some(network) = permissions.get("network").filter(|value| !value.is_null()) {
        granted.insert("network".to_string(), network.clone());
    }
    if let Some(file_system) = permissions
        .get("fileSystem")
        .filter(|value| !value.is_null())
    {
        granted.insert("fileSystem".to_string(), file_system.clone());
    }
    Value::Object(granted)
}

fn approval_supports_persist_mode(meta: Option<&Value>, expected_mode: &str) -> bool {
    let Some(persist) = meta.and_then(|meta| meta.get("persist")) else {
        return false;
    };
    persist.as_str() == Some(expected_mode)
        || persist.as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some(expected_mode))
        })
}

fn is_empty_object_schema(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("object")
        && value
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.is_empty())
}

fn same_approval_conversation(
    original_event: &Value,
    approval_event: &Value,
    platform_message_id: Option<&str>,
) -> bool {
    if let Some(message_id) = platform_message_id {
        if event_string_at_paths(
            approval_event,
            &[
                "/message/id",
                "/raw/message/id",
                "/raw/message/ts",
                "/raw/container/message_ts",
            ],
        ) == Some(message_id)
        {
            return true;
        }
    }

    let original_conversation_ids = approval_conversation_ids(original_event);
    let approval_conversation_ids = approval_conversation_ids(approval_event);
    if original_conversation_ids.iter().any(|original_id| {
        approval_conversation_ids
            .iter()
            .any(|approval_id| approval_id == original_id)
    }) {
        return true;
    }

    if is_dingtalk_event(original_event) && is_dingtalk_event(approval_event) {
        let original_actor_id = event_string_at_paths(original_event, &["/actor/id"]);
        let approval_actor_id = event_string_at_paths(approval_event, &["/actor/id"]);
        return original_actor_id.is_some() && original_actor_id == approval_actor_id;
    }

    false
}

fn approval_conversation_ids(event: &Value) -> Vec<String> {
    [
        "/conversation/id",
        "/raw/conversation/id",
        "/raw/conversationId",
        "/raw/openConversationId",
        "/raw/channel/id",
        "/raw/channel_id",
        "/raw/chat/id",
        "/raw/chatId",
        "/raw/chat_id",
        "/raw/group_id",
    ]
    .iter()
    .filter_map(|path| event.pointer(path))
    .filter_map(value_to_non_unknown_string)
    .collect()
}

fn value_to_non_unknown_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().map(str::trim) {
        if !text.is_empty() && text != "unknown" {
            return Some(text.to_string());
        }
    }
    if value.is_number() || value.is_boolean() {
        let text = value.to_string();
        if !text.is_empty() && text != "unknown" {
            return Some(text);
        }
    }
    None
}

fn is_dingtalk_event(event: &Value) -> bool {
    event.get("platform").and_then(Value::as_str) == Some(config::BOT_PLATFORM_DINGTALK)
}

fn event_string_at_paths<'a>(event: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths
        .iter()
        .filter_map(|path| event.pointer(path))
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|value| !value.is_empty() && *value != "unknown")
}

fn bot_approval_choice_from_event(
    event: &Value,
    request_key: &str,
    actions: &[BotApprovalAction],
) -> Option<String> {
    if let Some(payload) = bot_approval_payload_from_event(event) {
        if payload.get("kind").and_then(Value::as_str) == Some("codex_approval") {
            if payload
                .get("requestId")
                .and_then(Value::as_str)
                .is_some_and(|value| value != request_key)
            {
                return None;
            }
            return payload
                .get("choice")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
    }

    let text = event_message_text(event)?;
    bot_approval_choice_from_text(&text, actions)
}

fn bot_approval_payload_from_event(event: &Value) -> Option<Value> {
    [
        "/message/richText/value",
        "/message/richText/custom_id",
        "/message/richText",
        "/raw/data/custom_id",
        "/raw/actions/0/value",
        "/raw/actions/0",
        "/raw/event/action/value",
        "/raw/action/value",
        "/raw/action",
    ]
    .iter()
    .filter_map(|pointer| event.pointer(pointer))
    .find_map(normalize_bot_approval_payload)
}

fn normalize_bot_approval_payload(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| normalize_bot_approval_payload(&value)),
        Value::Object(map) => {
            if map.get("kind").and_then(Value::as_str) == Some("codex_approval") {
                return Some(Value::Object(map.clone()));
            }
            for key in ["value", "payload", "data"] {
                if let Some(nested) = map.get(key) {
                    if let Some(payload) = normalize_bot_approval_payload(nested) {
                        return Some(payload);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn bot_approval_choice_from_text(text: &str, actions: &[BotApprovalAction]) -> Option<String> {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    for (index, action) in actions.iter().enumerate() {
        let number = (index + 1).to_string();
        let label = action.label.to_ascii_lowercase();
        if normalized == number
            || normalized == format!("{}.", number)
            || normalized == action.key
            || normalized == label
        {
            return Some(action.key.clone());
        }
    }

    if normalized.contains("session") {
        if let Some(action) = actions
            .iter()
            .find(|action| action.label.to_ascii_lowercase().contains("session"))
        {
            return Some(action.key.clone());
        }
    }
    if matches!(
        normalized.as_str(),
        "yes" | "y" | "allow" | "approve" | "accept"
    ) {
        return actions
            .iter()
            .find(|action| {
                let label = action.label.to_ascii_lowercase();
                label.starts_with("yes") || label.starts_with("allow")
            })
            .map(|action| action.key.clone());
    }
    if matches!(normalized.as_str(), "deny" | "reject" | "no" | "n") {
        return actions
            .iter()
            .find(|action| {
                let label = action.label.to_ascii_lowercase();
                label.starts_with("no") || label.starts_with("deny")
            })
            .map(|action| action.key.clone());
    }
    if normalized == "cancel" || normalized == "stop" {
        return actions
            .iter()
            .find(|action| {
                action.label.eq_ignore_ascii_case("cancel") || action.key.contains("cancel")
            })
            .map(|action| action.key.clone());
    }
    None
}

fn request_id_key(request_id: &Value) -> Option<String> {
    match request_id {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn is_feishu_bot_event(event: &Value, config: &BotBridgeConfig) -> bool {
    event
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or(config.platform.as_str())
        == config::BOT_PLATFORM_FEISHU
}

fn is_dingtalk_bot_event(event: &Value, config: &BotBridgeConfig) -> bool {
    event
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or(config.platform.as_str())
        == config::BOT_PLATFORM_DINGTALK
}

fn is_slack_bot_event(event: &Value, config: &BotBridgeConfig) -> bool {
    event
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or(config.platform.as_str())
        == config::BOT_PLATFORM_SLACK
}

fn is_discord_bot_event(event: &Value, config: &BotBridgeConfig) -> bool {
    event
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or(config.platform.as_str())
        == config::BOT_PLATFORM_DISCORD
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| value.to_string())
        .trim()
        .to_string()
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn codex_input_from_bot_event(message_text: &str, event: &Value, bot_session_id: &str) -> Value {
    let mut items = Vec::new();
    let attachments = message_attachments(event).map(Vec::as_slice).unwrap_or(&[]);
    let text = codex_text_with_attachment_summary(message_text, attachments, bot_session_id);
    items.push(json!({
        "type": "text",
        "text": text,
        "text_elements": [],
    }));

    for attachment in attachments {
        if let Some(item) = attachment_to_codex_image_input(attachment) {
            items.push(item);
        }
    }

    Value::Array(items)
}

fn codex_text_with_attachment_summary(
    message_text: &str,
    attachments: &[Value],
    bot_session_id: &str,
) -> String {
    let media_hint = format!(
        "\n\nBot bridge session: botSessionId={}. When calling send_image, send_file, send_video, send_audio, or generic send_media, include this exact botSessionId so the media is sent to this same external bot conversation.",
        bot_session_id
    );
    if attachments.is_empty() {
        return format!("{}{}", message_text, media_hint);
    }

    let mut text = message_text.trim().to_string();
    if text.is_empty() {
        text.push_str("Please review the attached media/file(s).");
    }
    text.push_str("\n\nBot attachments:");
    for (index, attachment) in attachments.iter().enumerate() {
        text.push('\n');
        text.push_str(&attachment_summary_line(index + 1, attachment));
    }
    text.push_str(
        "\n\nImages with accessible local paths or remote URLs are also attached as image inputs. \
Use the listed paths/URLs for other files.",
    );
    text.push_str(&media_hint);
    text
}

fn attachment_summary_line(index: usize, attachment: &Value) -> String {
    let attachment_type = attachment
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("file");
    let name = attachment
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("attachment");
    let mime_type = attachment
        .get("mimeType")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown");
    let size = attachment
        .get("sizeBytes")
        .and_then(Value::as_u64)
        .map(|value| format!("{} bytes", value))
        .unwrap_or_else(|| "unknown size".to_string());
    let url = attachment_url(attachment).unwrap_or("no url/path");
    format!(
        "{}. type={} name={} mimeType={} size={} url={}",
        index, attachment_type, name, mime_type, size, url
    )
}

fn attachment_to_codex_image_input(attachment: &Value) -> Option<Value> {
    if !is_image_attachment(attachment) {
        return None;
    }
    let url = attachment_url(attachment)?;
    if is_http_url(url) {
        return Some(json!({
            "type": "image",
            "url": url,
        }));
    }
    let path = local_path_from_media_url(url)?;
    if path.is_file() {
        return Some(json!({
            "type": "localImage",
            "path": path.to_string_lossy().to_string(),
        }));
    }
    None
}

fn is_image_attachment(attachment: &Value) -> bool {
    attachment
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("image"))
        || attachment
            .get("mimeType")
            .and_then(Value::as_str)
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"))
        || attachment
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                let lower = value.to_ascii_lowercase();
                matches!(
                    Path::new(&lower).extension().and_then(|ext| ext.to_str()),
                    Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff")
                )
            })
}

fn attachment_url(attachment: &Value) -> Option<&str> {
    attachment
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| attachment.get("path").and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn message_attachments(event: &Value) -> Option<&Vec<Value>> {
    event
        .get("message")
        .and_then(|message| message.get("attachments"))
        .and_then(Value::as_array)
}

fn local_path_from_media_url(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode_file_url_path(path)));
    }
    if Path::new(url).is_absolute() {
        return Some(PathBuf::from(url));
    }
    None
}

fn percent_decode_file_url_path(path: &str) -> String {
    let mut output = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                output.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

impl AppServerBridge {
    fn session_restore_notice(&self, event: &Value) -> Option<String> {
        let key = bot_session_key(&self.config, event);
        let current_key = self.current_session_key.as_deref()?;
        if current_key == key {
            return None;
        }

        let state = load_bot_session_state(&self.config, &key).unwrap_or_default();
        let project = state
            .selected_cwd
            .as_deref()
            .unwrap_or(PROJECTLESS_PROJECT_LABEL);
        let session = state.thread_id.as_deref().map(short_id).unwrap_or("new");
        Some(format!(
            "接力：即将切换到当前 Bot 对话对应的 Codex 上下文。\n\n项目：{}\nSession：{}",
            project, session
        ))
    }

    fn restore_session_for_event(&mut self, event: &Value) {
        let key = bot_session_key(&self.config, event);
        if self.current_session_key.as_deref() == Some(key.as_str()) {
            if self.current_media_session_id.is_none() {
                self.current_media_session_id =
                    Some(resolve_bot_media_session_id(&self.config, &key, None));
            }
            return;
        }

        self.current_session_key = Some(key.clone());
        let persisted = load_bot_session_state(&self.config, &key);
        let had_persisted = persisted.is_some();
        let state = persisted.unwrap_or_default();
        self.current_media_session_id = Some(resolve_bot_media_session_id(
            &self.config,
            &key,
            state.media_session_id.as_deref(),
        ));
        if !had_persisted && self.restore_legacy_named_session(&key) {
            return;
        }
        self.thread_id = state
            .thread_id
            .filter(|thread_id| !thread_id.trim().is_empty());
        self.selected_cwd = state.selected_cwd.filter(|cwd| !cwd.trim().is_empty());

        let Some(thread_id) = self.thread_id.clone() else {
            return;
        };

        match self.resolve_thread(&thread_id) {
            Ok(thread) => {
                if self.selected_cwd.is_none() {
                    self.selected_cwd = thread.cwd.clone();
                }
                if thread.status.as_deref() == Some("notLoaded") {
                    if let Err(err) = self.resume_thread(&thread) {
                        log_bridge(
                            &self.config,
                            &format!(
                                "failed to resume persisted bot thread {}: {}",
                                thread_id, err
                            ),
                        );
                        self.thread_id = None;
                        let _ = self.persist_current_session();
                        return;
                    }
                }
                log_bridge(
                    &self.config,
                    &format!(
                        "restored bot conversation {} to Codex thread {}",
                        key, thread_id
                    ),
                );
            }
            Err(err) => {
                log_bridge(
                    &self.config,
                    &format!(
                        "persisted bot thread {} was not found in thread list: {}",
                        thread_id, err
                    ),
                );
                let fallback = ThreadSummary {
                    id: thread_id.clone(),
                    preview: String::new(),
                    cwd: self.selected_cwd.clone(),
                    path: None,
                    updated_at: 0,
                    status: None,
                };
                if let Err(err) = self.resume_thread(&fallback) {
                    log_bridge(
                        &self.config,
                        &format!(
                            "direct resume for persisted bot thread {} failed: {}",
                            thread_id, err
                        ),
                    );
                }
            }
        }
    }

    fn restore_legacy_named_session(&mut self, key: &str) -> bool {
        let names = legacy_bot_thread_names(&self.config);
        let Ok(mut threads) = self.list_threads(200) else {
            return false;
        };
        threads.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let Some(thread) = threads
            .into_iter()
            .find(|thread| names.iter().any(|name| thread.preview == *name))
        else {
            return false;
        };

        if thread.status.as_deref() == Some("notLoaded") {
            if let Err(err) = self.resume_thread(&thread) {
                log_bridge(
                    &self.config,
                    &format!(
                        "failed to resume legacy named bot thread {}: {}",
                        thread.id, err
                    ),
                );
                return false;
            }
        }

        self.thread_id = Some(thread.id.clone());
        self.selected_cwd = thread.cwd.clone();
        if let Err(err) = self.persist_current_session() {
            log_bridge(
                &self.config,
                &format!("failed to persist legacy bot session state: {}", err),
            );
        }
        log_bridge(
            &self.config,
            &format!(
                "migrated bot conversation {} to legacy Codex thread {}",
                key, thread.id
            ),
        );
        true
    }

    fn persist_current_session(&self) -> Result<(), String> {
        let Some(key) = self.current_session_key.as_ref() else {
            return Ok(());
        };
        persist_bot_session_state(
            &self.config,
            key,
            PersistedBotSessionState {
                thread_id: self.thread_id.clone(),
                selected_cwd: self.selected_cwd.clone(),
                media_session_id: self.current_media_session_id.clone(),
                updated_at: unix_seconds(),
            },
        )
    }

    fn apply_bot_message(&mut self, message_text: &str) -> Result<BotMessageAction, String> {
        let trimmed = message_text.trim();
        let command_lower = trimmed.to_ascii_lowercase();
        if trimmed.eq_ignore_ascii_case("ls") {
            return self.render_project_tree().map(BotMessageAction::Reply);
        }
        if trimmed.eq_ignore_ascii_case("reset") {
            self.thread_id = None;
            self.selected_cwd = None;
            return Ok(BotMessageAction::Reply(
                "Reset. Using a new projectless Codex session.".to_string(),
            ));
        }
        if command_lower.starts_with("select ") {
            let rest = &trimmed["select ".len()..];
            let thread = self.resolve_thread(rest.trim())?;
            if thread.status.as_deref() == Some("notLoaded") {
                self.resume_thread(&thread)?;
            }
            self.thread_id = Some(thread.id.clone());
            self.selected_cwd = thread.cwd.clone();
            return Ok(BotMessageAction::Reply(format!(
                "Selected thread {}: {}",
                short_id(&thread.id),
                display_thread_preview(&thread)
            )));
        }
        if command_lower.starts_with("use ") {
            let rest = &trimmed["use ".len()..];
            let (project_name, next_message) =
                if let Some((colon_index, colon_len)) = find_use_project_colon(rest) {
                    (
                        rest[..colon_index].trim(),
                        rest[colon_index + colon_len..].trim(),
                    )
                } else {
                    (rest.trim(), "")
                };
            if project_name.is_empty() {
                return Ok(BotMessageAction::Reply(
                    "Usage: use <project-name-or-[n]>: [message] or use [n]".to_string(),
                ));
            }
            let project = self.resolve_project(project_name)?;
            let projectless = project.cwd == PROJECTLESS_PROJECT_LABEL;
            if next_message.is_empty() {
                self.apply_project_switch(&project);
                if projectless {
                    return Ok(BotMessageAction::Reply(
                        "Using projectless Codex session mode.".to_string(),
                    ));
                }
                return Ok(BotMessageAction::Reply(format!(
                    "Using project {} ({})",
                    project.name, project.cwd
                )));
            }
            return Ok(BotMessageAction::SwitchProjectAndRun(BotProjectSwitch {
                project,
                message_text: next_message.to_string(),
            }));
        }

        Ok(BotMessageAction::Run(message_text.to_string()))
    }

    fn apply_project_switch(&mut self, project: &ProjectSummary) {
        self.thread_id = None;
        self.selected_cwd = if project.cwd == PROJECTLESS_PROJECT_LABEL {
            None
        } else {
            Some(project.cwd.clone())
        };
    }

    fn ensure_thread(&mut self, prompt_seed: &str) -> Result<String, String> {
        if let Some(thread_id) = self.thread_id.clone() {
            return Ok(thread_id);
        }

        let mut params = Map::new();
        if self.selected_cwd.is_none() {
            self.selected_cwd = Some(self.create_projectless_cwd(prompt_seed)?);
        }
        if let Some(cwd) = self.selected_cwd.as_ref() {
            params.insert("cwd".to_string(), Value::String(cwd.clone()));
        }
        params.insert(
            "serviceName".to_string(),
            Value::String("codexl_bot_gateway".to_string()),
        );
        params.insert(
            "threadSource".to_string(),
            Value::String("user".to_string()),
        );
        params.insert("ephemeral".to_string(), Value::Bool(false));
        params.insert(
            "personality".to_string(),
            Value::String("pragmatic".to_string()),
        );
        let result = self.request(
            "thread/start",
            Value::Object(params),
            Duration::from_secs(30),
        )?;
        let thread_id = result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "thread/start response missing thread.id".to_string())?
            .to_string();
        self.thread_id = Some(thread_id.clone());
        if let Err(err) = self.persist_current_session() {
            log_bridge(
                &self.config,
                &format!("failed to persist bot session state: {}", err),
            );
        }
        log_bridge(
            &self.config,
            &format!(
                "created Codex app thread {} for bot {}",
                thread_id, self.config.integration_id
            ),
        );
        Ok(thread_id)
    }

    fn start_turn(
        &mut self,
        thread_id: &str,
        message_text: &str,
        event: &Value,
    ) -> Result<(String, CodexEventCursor), String> {
        let mut params = Map::new();
        params.insert("threadId".to_string(), Value::String(thread_id.to_string()));
        if let Some(cwd) = self.selected_cwd.as_ref() {
            params.insert("cwd".to_string(), Value::String(cwd.clone()));
        }
        let session_key = self
            .current_session_key
            .clone()
            .unwrap_or_else(|| bot_session_key(&self.config, event));
        let bot_session_id = self
            .current_media_session_id
            .clone()
            .unwrap_or_else(|| resolve_bot_media_session_id(&self.config, &session_key, None));
        params.insert(
            "input".to_string(),
            codex_input_from_bot_event(message_text, event, &bot_session_id),
        );

        let turn_cursor = self.event_hub.cursor_now();
        let result = self.request("turn/start", Value::Object(params), Duration::from_secs(30))?;
        let turn_id = result
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| "turn/start response missing turn.id".to_string())?;
        log_bridge(
            &self.config,
            &format!("started Codex turn {} on thread {}", turn_id, thread_id),
        );
        Ok((turn_id, turn_cursor))
    }

    fn persist_bot_media_context(&self, event: &Value, event_id: &str) -> Result<(), String> {
        let session_key = self
            .current_session_key
            .clone()
            .unwrap_or_else(|| bot_session_key(&self.config, event));
        let session_id = self
            .current_media_session_id
            .clone()
            .unwrap_or_else(|| resolve_bot_media_session_id(&self.config, &session_key, None));
        persist_bot_media_context(
            &self.config,
            BotMediaMcpContext {
                session_id,
                session_key,
                thread_id: self.thread_id.clone(),
                tenant_id: event
                    .get("tenantId")
                    .and_then(Value::as_str)
                    .unwrap_or(self.config.tenant_id.as_str())
                    .to_string(),
                integration_id: event_integration_id(event, &self.config.integration_id),
                platform: event
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or(self.config.platform.as_str())
                    .to_string(),
                conversation_ref: conversation_ref(event),
                event_id: Some(event_id.to_string()),
                cwd: self.selected_cwd.clone(),
                updated_at: unix_seconds(),
            },
        )
    }

    fn create_projectless_cwd(&self, prompt_seed: &str) -> Result<String, String> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        let seconds = unix_seconds();
        let (year, month, day) = utc_date_from_unix_seconds(seconds);
        let date = format!("{:04}-{:02}-{:02}", year, month, day);
        let prompt_slug = sanitize_path_segment(prompt_seed);
        let slug = if prompt_slug.is_empty() {
            sanitize_path_segment(&format!("bot-{}", self.config.integration_id))
        } else {
            prompt_slug
        };
        let dir = PathBuf::from(home)
            .join("Documents")
            .join("Codex")
            .join(date)
            .join(format!("{}-{}", slug, seconds));
        fs::create_dir_all(&dir).map_err(|err| {
            format!(
                "failed to create projectless Codex session directory {}: {}",
                dir.to_string_lossy(),
                err
            )
        })?;
        Ok(dir.to_string_lossy().to_string())
    }

    fn render_project_tree(&mut self) -> Result<String, String> {
        let projects = self.sorted_projects()?;

        if projects.is_empty() {
            return Ok("No Codex projects or sessions found.".to_string());
        }

        let mut output = String::from(".\n");
        for (project_index, project) in projects.iter().enumerate() {
            let project_last = project_index + 1 == projects.len();
            let project_branch = if project_last { "`--" } else { "|--" };
            let project_prefix = if project_last { "   " } else { "|  " };
            let current_marker = if self.selected_cwd.as_deref() == Some(project.cwd.as_str()) {
                " [current]"
            } else {
                ""
            };
            output.push_str(&format!(
                "{} [{}] {} ({}){}\n",
                project_branch,
                project_index + 1,
                project.name,
                project.cwd,
                current_marker
            ));

            for (thread_index, thread) in project.threads.iter().enumerate() {
                let thread_last = thread_index + 1 == project.threads.len();
                let thread_branch = if thread_last { "`--" } else { "|--" };
                let current_thread_marker = if self.thread_id.as_deref() == Some(thread.id.as_str())
                {
                    " [selected]"
                } else {
                    ""
                };
                output.push_str(&format!(
                    "{}{} [{}.{}] {} {}{}\n",
                    project_prefix,
                    thread_branch,
                    project_index + 1,
                    thread_index + 1,
                    short_id(&thread.id),
                    display_thread_preview(thread),
                    current_thread_marker
                ));
            }
        }

        Ok(output.trim_end().to_string())
    }

    fn resolve_project(&mut self, query: &str) -> Result<ProjectSummary, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("Project name is empty.".to_string());
        }

        if let Some(project_index) = parse_project_index_selector(query) {
            let projects = self.sorted_projects()?;
            return projects.get(project_index).cloned().ok_or_else(|| {
                format!(
                    "Project selector '{}' was not found. Send 'ls' to list projects.",
                    query
                )
            });
        }

        if query.starts_with('/') || query.starts_with("~/") || query == "~" {
            let path = expand_home_path(query.to_string());
            if path.is_dir() {
                return Ok(ProjectSummary {
                    cwd: path.to_string_lossy().to_string(),
                    name: project_name(&path.to_string_lossy()),
                    threads: Vec::new(),
                    updated_at: 0,
                });
            }
        }

        let projects = self.sorted_projects()?;
        let query_lower = query.to_ascii_lowercase();
        let mut matches: Vec<ProjectSummary> = projects
            .into_iter()
            .filter(|project| {
                let name_lower = project.name.to_ascii_lowercase();
                let cwd_lower = project.cwd.to_ascii_lowercase();
                name_lower == query_lower
                    || cwd_lower == query_lower
                    || cwd_lower.ends_with(&format!("/{}", query_lower))
                    || name_lower.contains(&query_lower)
                    || cwd_lower.contains(&query_lower)
            })
            .collect();

        matches.sort_by(|left, right| {
            score_project_match(left, &query_lower)
                .cmp(&score_project_match(right, &query_lower))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });

        if matches.is_empty() {
            return Err(format!(
                "Project '{}' was not found. Send 'ls' to list projects.",
                query
            ));
        }

        let best_score = score_project_match(&matches[0], &query_lower);
        let same_score = matches
            .iter()
            .take_while(|project| score_project_match(project, &query_lower) == best_score)
            .count();
        if same_score > 1 && matches[0].cwd.to_ascii_lowercase() != query_lower {
            return Err(format!(
                "Project '{}' is ambiguous:\n{}",
                query,
                matches
                    .iter()
                    .take(5)
                    .map(|project| format!("- {} ({})", project.name, project.cwd))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        Ok(matches.remove(0))
    }

    fn resolve_thread(&mut self, query: &str) -> Result<ThreadSummary, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err("Thread selector is empty.".to_string());
        }

        if let Some((project_index, thread_index)) = parse_thread_index_selector(query) {
            let projects = self.sorted_projects()?;
            return projects
                .get(project_index)
                .and_then(|project| project.threads.get(thread_index))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "Thread selector '{}' was not found. Send 'ls' to list sessions.",
                        query
                    )
                });
        }

        let query_lower = query.to_ascii_lowercase();
        let mut matches: Vec<ThreadSummary> = self
            .list_threads(200)?
            .into_iter()
            .filter(|thread| {
                let id_lower = thread.id.to_ascii_lowercase();
                let preview_lower = thread.preview.to_ascii_lowercase();
                id_lower == query_lower
                    || id_lower.starts_with(&query_lower)
                    || preview_lower == query_lower
                    || preview_lower.contains(&query_lower)
            })
            .collect();

        matches.sort_by(|left, right| {
            score_thread_match(left, &query_lower)
                .cmp(&score_thread_match(right, &query_lower))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });

        if matches.is_empty() {
            return Err(format!(
                "Thread '{}' was not found. Send 'ls' to list sessions.",
                query
            ));
        }

        let best_score = score_thread_match(&matches[0], &query_lower);
        let same_score = matches
            .iter()
            .take_while(|thread| score_thread_match(thread, &query_lower) == best_score)
            .count();
        if same_score > 1 && matches[0].id.to_ascii_lowercase() != query_lower {
            return Err(format!(
                "Thread '{}' is ambiguous:\n{}",
                query,
                matches
                    .iter()
                    .take(5)
                    .map(|thread| format!(
                        "- {} {}",
                        short_id(&thread.id),
                        display_thread_preview(thread)
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        Ok(matches.remove(0))
    }

    fn sorted_projects(&mut self) -> Result<Vec<ProjectSummary>, String> {
        let mut projects = self.list_projects()?;
        projects.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.cwd.cmp(&right.cwd))
        });
        Ok(projects)
    }

    fn list_projects(&mut self) -> Result<Vec<ProjectSummary>, String> {
        let mut projects = BTreeMap::<String, ProjectSummary>::new();

        for cwd in self.list_config_project_paths().unwrap_or_default() {
            projects
                .entry(cwd.clone())
                .or_insert_with(|| ProjectSummary {
                    name: project_name(&cwd),
                    cwd,
                    threads: Vec::new(),
                    updated_at: 0,
                });
        }

        for thread in self.list_threads(200)? {
            let cwd = thread
                .cwd
                .clone()
                .unwrap_or_else(|| PROJECTLESS_PROJECT_LABEL.to_string());
            let project = projects
                .entry(cwd.clone())
                .or_insert_with(|| ProjectSummary {
                    name: project_name(&cwd),
                    cwd,
                    threads: Vec::new(),
                    updated_at: 0,
                });
            project.updated_at = project.updated_at.max(thread.updated_at);
            project.threads.push(thread);
        }

        for project in projects.values_mut() {
            project.threads.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.preview.cmp(&right.preview))
                    .then_with(|| left.id.cmp(&right.id))
            });
        }

        Ok(projects.into_values().collect())
    }

    fn list_config_project_paths(&mut self) -> Result<Vec<String>, String> {
        let result = self.request(
            "config/read",
            json!({
                "includeLayers": true,
                "cwd": Value::Null,
            }),
            Duration::from_secs(10),
        )?;
        let projects = result
            .get("config")
            .and_then(|config| config.get("projects"))
            .and_then(Value::as_object)
            .ok_or_else(|| "config/read response missing config.projects".to_string())?;
        Ok(projects.keys().cloned().collect())
    }

    fn list_threads(&mut self, limit: u64) -> Result<Vec<ThreadSummary>, String> {
        let result = self.request(
            "thread/list",
            json!({
                "limit": limit,
                "cursor": Value::Null,
                "sortKey": "updated_at",
                "modelProviders": Value::Null,
                "archived": false,
                "sourceKinds": [],
            }),
            Duration::from_secs(15),
        )?;
        let threads = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "thread/list response missing data".to_string())?;
        Ok(threads
            .iter()
            .filter_map(ThreadSummary::from_value)
            .collect())
    }

    fn resume_thread(&mut self, thread: &ThreadSummary) -> Result<(), String> {
        let mut params = Map::new();
        params.insert("threadId".to_string(), Value::String(thread.id.clone()));
        params.insert("history".to_string(), Value::Null);
        if let Some(path) = thread.path.as_ref() {
            params.insert("path".to_string(), Value::String(path.clone()));
        }
        if let Some(cwd) = thread.cwd.as_ref() {
            params.insert("cwd".to_string(), Value::String(cwd.clone()));
        }
        params.insert(
            "approvalPolicy".to_string(),
            Value::String("on-request".to_string()),
        );
        params.insert(
            "approvalsReviewer".to_string(),
            Value::String("user".to_string()),
        );
        params.insert(
            "sandbox".to_string(),
            Value::String("workspace-write".to_string()),
        );
        self.request(
            "thread/resume",
            Value::Object(params),
            Duration::from_secs(30),
        )?;
        Ok(())
    }

    fn wait_turn_completed(
        &mut self,
        bot: &mut BotGatewayClient,
        thread_id: &str,
        turn_id: &str,
        event: &Value,
        event_id: &str,
        mut cursor: CodexEventCursor,
    ) -> Result<CodexTurnResult, String> {
        let deadline = Instant::now() + self.config.turn_timeout;
        let mut capture = TurnCapture::default();
        let mut sent_messages = 0usize;

        loop {
            let now = Instant::now();
            if now >= deadline {
                log_bridge(
                    &self.config,
                    &format!(
                        "Codex turn {} on thread {} timed out after {}ms",
                        turn_id,
                        thread_id,
                        self.config.turn_timeout.as_millis()
                    ),
                );
                return Err("timed out waiting for Codex turn completion".to_string());
            }
            let timeout = std::cmp::min(Duration::from_millis(250), deadline - now);
            let event_line = match self.event_hub.next_event(&mut cursor, timeout) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(CodexEventHubError::Disconnected) => {
                    log_bridge(
                        &self.config,
                        &format!(
                            "Codex turn {} on thread {} lost app-server output channel",
                            turn_id, thread_id
                        ),
                    );
                    return Err("Codex app-server output channel closed".to_string());
                }
                Err(err) => {
                    log_bridge(
                        &self.config,
                        &format!(
                            "Codex turn {} on thread {} event cursor skipped: {}",
                            turn_id,
                            thread_id,
                            err.message()
                        ),
                    );
                    return Err("Codex app-server event cursor fell behind".to_string());
                }
            };
            let Some(value) = event_line.value.as_ref() else {
                continue;
            };
            let Some(method) = event_line.method.as_deref() else {
                continue;
            };
            let params = value.get("params").unwrap_or(&Value::Null);

            match method {
                method
                    if is_bot_approval_request_method(method)
                        && matches_approval_request_turn(params, thread_id, turn_id) =>
                {
                    let Some(request_id) = value.get("id").cloned() else {
                        continue;
                    };
                    self.handle_bot_approval_request(
                        bot,
                        method,
                        request_id,
                        params.clone(),
                        event,
                        event_id,
                        deadline,
                    )?;
                }
                "item/agentMessage/delta" if matches_thread_turn(params, thread_id, turn_id) => {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        capture.fallback_text.push_str(delta);
                    }
                }
                "item/completed" if matches_thread_turn(params, thread_id, turn_id) => {
                    capture.capture_completed_item(params);
                    if self.config.forward_all_codex_messages {
                        if let Some(text) = completed_agent_message_text(params) {
                            sent_messages += 1;
                            let idempotency_key =
                                format!("codexl:{}:codex-message:{}", event_id, sent_messages);
                            send_bot_text_response(
                                bot,
                                &self.config,
                                event,
                                &idempotency_key,
                                &text,
                            )?;
                            log_bridge(
                                &self.config,
                                &format!(
                                    "forwarded Codex message event_id={} turn_id={} index={} text_len={}",
                                    event_id,
                                    turn_id,
                                    sent_messages,
                                    text.chars().count()
                                ),
                            );
                        }
                    }
                }
                "turn/completed" if matches_thread_turn(params, thread_id, turn_id) => {
                    if let Some(message) = turn_completed_error_message(params) {
                        self.idle_cursor = cursor.clone();
                        log_bridge(
                            &self.config,
                            &format!(
                                "Codex turn {} on thread {} failed: {}",
                                turn_id, thread_id, message
                            ),
                        );
                        return Err(message);
                    }
                    let response_text = capture.final_text.unwrap_or(capture.fallback_text);
                    self.idle_cursor = cursor.clone();
                    log_bridge(
                        &self.config,
                        &format!(
                            "completed Codex turn {} on thread {} response_len={} forwarded_messages={}",
                            turn_id,
                            thread_id,
                            response_text.chars().count(),
                            sent_messages
                        ),
                    );
                    return Ok(CodexTurnResult {
                        response_text,
                        sent_messages,
                    });
                }
                _ => {}
            }
        }
    }

    fn handoff_forward_decision(&self) -> CodexForwardDecision {
        if self.config.handoff.enabled {
            let (presence, snapshot) =
                evaluate_handoff_presence_with_snapshot(&self.config.handoff);
            let should_forward = presence.away;
            self.log_handoff_decision(
                "idle-output",
                &snapshot,
                &presence,
                should_forward,
                self.config.forward_all_codex_messages,
            );
            return CodexForwardDecision {
                should_forward,
                handoff_presence: should_forward.then_some(presence.clone()),
                handoff_evaluation: Some(presence),
            };
        }

        CodexForwardDecision {
            should_forward: false,
            handoff_presence: None,
            handoff_evaluation: None,
        }
    }

    fn log_handoff_decision(
        &self,
        context: &str,
        snapshot: &HandoffSignalSnapshot,
        presence: &HandoffPresence,
        should_forward: bool,
        forward_all_considered: bool,
    ) {
        log_bridge(
            &self.config,
            &format!(
                "handoff decision context={} enabled={} forward_all_considered={} should_forward={} away={} screen_required=true screen_locked={} user_idle_enabled={} idle_threshold={} idle_seconds={} wifi_targets={} wifi_seen={} bluetooth_targets={} bluetooth_seen={} reasons=[{}] evidence=[{}] diagnostics=[{}]",
                context,
                self.config.handoff.enabled,
                forward_all_considered,
                should_forward,
                presence.away,
                fmt_option_bool(snapshot.signals.screen_locked),
                self.config.handoff.user_idle,
                self.config.handoff.idle_seconds,
                fmt_option_u64(snapshot.signals.idle_seconds),
                format_handoff_target_values(&self.config.handoff.phone_wifi_targets),
                fmt_option_bool(snapshot.signals.phone_wifi_seen),
                format_handoff_target_values(&self.config.handoff.phone_bluetooth_targets),
                fmt_option_bool(snapshot.signals.phone_bluetooth_seen),
                format_log_list(&presence.reasons),
                format_log_list(&presence.evidence),
                format_log_list(&snapshot.diagnostics)
            ),
        );
    }

    fn handle_bot_approval_request(
        &mut self,
        bot: &mut BotGatewayClient,
        method: &str,
        request_id: Value,
        params: Value,
        event: &Value,
        event_id: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        let request_key = request_id_key(&request_id)
            .ok_or_else(|| format!("approval request {} has invalid id", method))?;
        let prompt = build_bot_approval_prompt(method, &request_key, &params)?;

        log_bridge(
            &self.config,
            &format!(
                "forwarding approval request method={} request_id={} actions={}",
                method,
                request_key,
                prompt.actions.len()
            ),
        );

        let action = self.wait_for_bot_approval_action(
            bot,
            event,
            event_id,
            &request_key,
            &prompt,
            deadline,
        )?;
        write_json_line(
            &self.writer,
            &json!({
                "id": request_id,
                "result": action.result,
            }),
        )?;
        log_bridge(
            &self.config,
            &format!(
                "resolved approval request method={} request_id={} choice={}",
                method, request_key, action.key
            ),
        );
        Ok(())
    }

    fn wait_for_bot_approval_action(
        &mut self,
        bot: &mut BotGatewayClient,
        event: &Value,
        event_id: &str,
        request_key: &str,
        prompt: &BotApprovalPrompt,
        deadline: Instant,
    ) -> Result<BotApprovalAction, String> {
        let idempotency_key = format!("codexl:{}:approval:{}", event_id, request_key);
        let outbound_result =
            send_bot_approval_prompt(bot, &self.config, event, &idempotency_key, prompt)?;
        let platform_message_id = outbound_platform_message_id(&outbound_result);

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "timed out waiting for bot approval response request_id={}",
                    request_key
                ));
            }

            let result = bot.request("events.list", json!({ "limit": 20 }))?;
            let events = result
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            for queued in events {
                let Some(approval_event) = queued.get("event") else {
                    continue;
                };
                if event_integration_id(approval_event, &self.config.integration_id)
                    != self.config.integration_id
                {
                    continue;
                }
                if !same_approval_conversation(
                    event,
                    approval_event,
                    platform_message_id.as_deref(),
                ) {
                    continue;
                }

                let Some(choice_key) =
                    bot_approval_choice_from_event(approval_event, request_key, &prompt.actions)
                else {
                    continue;
                };
                let Some(action) = prompt
                    .actions
                    .iter()
                    .find(|action| action.key == choice_key)
                    .cloned()
                else {
                    continue;
                };

                if let Some(queued_event_id) = queued
                    .get("id")
                    .and_then(Value::as_str)
                    .or_else(|| approval_event.get("id").and_then(Value::as_str))
                {
                    let _ = bot.request("events.ack", json!({ "eventId": queued_event_id }));
                }

                if is_discord_bot_event(approval_event, &self.config) {
                    if let Err(err) =
                        acknowledge_discord_approval_interaction(bot, &self.config, approval_event)
                    {
                        log_bridge(
                            &self.config,
                            &format!(
                                "failed to acknowledge Discord approval interaction request_id={}: {}",
                                request_key, err
                            ),
                        );
                    }
                }

                if is_feishu_bot_event(event, &self.config) {
                    if let Some(message_id) = platform_message_id.as_deref() {
                        if let Err(err) = update_bot_approval_card_status(
                            bot,
                            &self.config,
                            event,
                            message_id,
                            prompt,
                            &action,
                        ) {
                            log_bridge(
                                &self.config,
                                &format!(
                                    "failed to update approval card request_id={} message_id={}: {}",
                                    request_key, message_id, err
                                ),
                            );
                        }
                    }
                }

                return Ok(action);
            }

            let mut deferred_dingtalk_events = Vec::new();
            for queued in self.collect_dingtalk_stream_events() {
                let Some(approval_event) = queued.get("event") else {
                    continue;
                };
                if event_integration_id(approval_event, &self.config.integration_id)
                    != self.config.integration_id
                {
                    deferred_dingtalk_events.push(queued);
                    continue;
                }
                if !same_approval_conversation(
                    event,
                    approval_event,
                    platform_message_id.as_deref(),
                ) {
                    deferred_dingtalk_events.push(queued);
                    continue;
                }

                let Some(choice_key) =
                    bot_approval_choice_from_event(approval_event, request_key, &prompt.actions)
                else {
                    deferred_dingtalk_events.push(queued);
                    continue;
                };
                let Some(action) = prompt
                    .actions
                    .iter()
                    .find(|action| action.key == choice_key)
                    .cloned()
                else {
                    deferred_dingtalk_events.push(queued);
                    continue;
                };

                self.defer_dingtalk_stream_events(deferred_dingtalk_events);
                return Ok(action);
            }
            self.defer_dingtalk_stream_events(deferred_dingtalk_events);

            let remaining = deadline.saturating_duration_since(Instant::now());
            let sleep_for = std::cmp::min(
                Duration::from_millis(BOT_APPROVAL_POLL_INTERVAL_MS),
                remaining,
            );
            if sleep_for.is_zero() {
                continue;
            }
            thread::sleep(sleep_for);
        }
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = next_app_request_id();
        let request = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        let mut cursor = self.event_hub.cursor_now();
        write_json_line(&self.writer, &request)?;

        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!("timed out waiting for {}", method));
            }
            let wait = std::cmp::min(Duration::from_millis(250), deadline - now);
            let event = match self.event_hub.next_event(&mut cursor, wait) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(err) => return Err(err.message()),
            };
            let Some(value) = event.value.as_ref() else {
                continue;
            };
            if value.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex app-server request failed")
                    .to_string());
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl TurnCapture {
    fn capture_completed_item(&mut self, params: &Value) {
        let Some(text) = completed_agent_message_text(params) else {
            return;
        };
        let Some(item) = params.get("item") else {
            return;
        };
        if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
            self.final_text = Some(text);
        } else if self.final_text.is_none() {
            self.fallback_text = text;
        }
    }
}

impl HandoffPresence {
    fn summary_for_language(&self, language: AppLanguage) -> String {
        let items = if self.reasons.is_empty() {
            &self.evidence
        } else {
            &self.reasons
        };
        if items.is_empty() {
            return localized_handoff_signal("presence signal unavailable", language);
        }
        let separator = match language {
            AppLanguage::Zh => "，",
            AppLanguage::En => ", ",
        };
        items
            .iter()
            .map(|item| localized_handoff_signal(item, language))
            .collect::<Vec<_>>()
            .join(separator)
    }
}

fn localized_handoff_signal(value: &str, language: AppLanguage) -> String {
    if language == AppLanguage::En {
        return value.to_string();
    }

    if let Some(seconds) = value
        .strip_prefix("idle for ")
        .and_then(|value| value.strip_suffix('s'))
    {
        return format!("空闲 {} 秒", seconds);
    }

    match value {
        "handoff disabled" => "接力未启用",
        "screen locked" => "屏幕已锁定",
        "screen unlocked" => "屏幕已解锁",
        "screen lock unknown" => "屏幕锁定状态未知",
        "idle time unknown" => "空闲时间未知",
        "wifi target seen" => "已检测到 Wi-Fi 目标",
        "wifi target missing" => "未检测到 Wi-Fi 目标",
        "wifi target unknown" => "Wi-Fi 目标状态未知",
        "bluetooth target seen" => "已检测到蓝牙目标",
        "bluetooth target missing" => "未检测到蓝牙目标",
        "bluetooth target unknown" => "蓝牙目标状态未知",
        "selected signal detected" => "已检测到选定信号",
        "selected signal not detected" => "未检测到选定信号",
        "presence signal unavailable" => "状态信号不可用",
        _ => value,
    }
    .to_string()
}

fn evaluate_handoff_presence_with_snapshot(
    config: &BotHandoffConfig,
) -> (HandoffPresence, HandoffSignalSnapshot) {
    let snapshot = collect_handoff_signal_snapshot(config);
    let presence = handoff_presence_from_signals(config, snapshot.signals.clone());
    (presence, snapshot)
}

fn collect_handoff_signal_snapshot(config: &BotHandoffConfig) -> HandoffSignalSnapshot {
    let mut diagnostics = Vec::new();
    let (screen_locked, screen_detail) = detect_screen_locked_with_detail();
    diagnostics.push(format!("screen_lock {}", screen_detail));

    let idle_seconds = config.user_idle.then(detect_user_idle_seconds).flatten();
    diagnostics.push(if config.user_idle {
        format!(
            "idle command result={} threshold={}",
            fmt_option_u64(idle_seconds),
            config.idle_seconds
        )
    } else {
        "idle disabled".to_string()
    });

    let (phone_wifi_seen, wifi_detail) = if config.phone_wifi_targets.is_empty() {
        (None, "wifi targets not configured".to_string())
    } else {
        detect_phone_wifi_targets_with_detail(&config.phone_wifi_targets)
    };
    diagnostics.push(format!("wifi {}", wifi_detail));

    let (phone_bluetooth_seen, bluetooth_detail) = if config.phone_bluetooth_targets.is_empty() {
        (None, "bluetooth targets not configured".to_string())
    } else {
        detect_phone_bluetooth_targets_with_detail(&config.phone_bluetooth_targets)
    };
    diagnostics.push(format!("bluetooth {}", bluetooth_detail));

    HandoffSignalSnapshot {
        signals: HandoffSignals {
            screen_locked,
            idle_seconds,
            phone_wifi_seen,
            phone_bluetooth_seen,
        },
        diagnostics,
    }
}

fn handoff_presence_from_signals(
    config: &BotHandoffConfig,
    signals: HandoffSignals,
) -> HandoffPresence {
    if !config.enabled {
        return HandoffPresence {
            away: false,
            reasons: Vec::new(),
            evidence: vec!["handoff disabled".to_string()],
        };
    }

    let mut reasons = Vec::new();
    let mut evidence = Vec::new();

    match signals.screen_locked {
        Some(true) => reasons.push("screen locked".to_string()),
        Some(false) => {
            return HandoffPresence {
                away: false,
                reasons,
                evidence: vec!["screen unlocked".to_string()],
            };
        }
        None => {
            return HandoffPresence {
                away: false,
                reasons,
                evidence: vec!["screen lock unknown".to_string()],
            };
        }
    }

    if config.user_idle {
        match signals.idle_seconds {
            Some(seconds) if seconds >= config.idle_seconds => {
                reasons.push(format!("idle for {}s", seconds));
            }
            Some(seconds) => evidence.push(format!("idle for {}s", seconds)),
            None => evidence.push("idle time unknown".to_string()),
        }
    }

    let mut target_configured = false;
    let mut target_seen = false;
    let mut target_missing = false;
    if !config.phone_wifi_targets.is_empty() {
        target_configured = true;
        match signals.phone_wifi_seen {
            Some(true) => {
                target_seen = true;
                evidence.push("wifi target seen".to_string());
            }
            Some(false) => {
                target_missing = true;
                evidence.push("wifi target missing".to_string());
            }
            None => {
                evidence.push("wifi target unknown".to_string());
            }
        }
    }
    if !config.phone_bluetooth_targets.is_empty() {
        target_configured = true;
        match signals.phone_bluetooth_seen {
            Some(true) => {
                target_seen = true;
                evidence.push("bluetooth target seen".to_string());
            }
            Some(false) => {
                target_missing = true;
                evidence.push("bluetooth target missing".to_string());
            }
            None => {
                evidence.push("bluetooth target unknown".to_string());
            }
        }
    }
    if target_configured {
        if target_seen {
            evidence.push("selected signal detected".to_string());
        } else if target_missing {
            reasons.push("selected signal not detected".to_string());
        }
    }

    HandoffPresence {
        away: !reasons.is_empty(),
        reasons,
        evidence,
    }
}

fn detect_screen_locked_with_detail() -> (Option<bool>, String) {
    let mut details = Vec::new();
    match command_stdout("/usr/sbin/ioreg", &["-r", "-k", "CGSSessionScreenIsLocked"]) {
        Some(output) => {
            if let Some(locked) = parse_screen_locked_from_ioreg_output(&output) {
                return (Some(locked), format!("CGSSessionScreenIsLocked={}", locked));
            }
            details.push("CGSSessionScreenIsLocked key missing".to_string());
        }
        None => {
            details.push("CGSSessionScreenIsLocked command failed".to_string());
        }
    }

    match command_stdout("/usr/sbin/ioreg", &["-n", "Root", "-d1"]) {
        Some(output) => {
            if let Some(locked) = parse_screen_locked_from_ioreg_output(&output) {
                details.push(format!("Root IOConsoleLocked={}", locked));
                return (Some(locked), details.join("; "));
            }
            if output.trim().is_empty() {
                details.push("Root ioreg output empty".to_string());
                (None, details.join("; "))
            } else {
                details.push("Root ioreg has no lock key, assuming unlocked".to_string());
                (Some(false), details.join("; "))
            }
        }
        None => {
            details.push("Root ioreg command failed".to_string());
            (None, details.join("; "))
        }
    }
}

fn parse_screen_locked_from_ioreg_output(output: &str) -> Option<bool> {
    for line in output.lines() {
        if !line.contains("CGSSessionScreenIsLocked") && !line.contains("IOConsoleLocked") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("yes") || lower.contains("true") || lower.contains("= 1") {
            return Some(true);
        }
        if lower.contains("no") || lower.contains("false") || lower.contains("= 0") {
            return Some(false);
        }
    }
    None
}

fn detect_user_idle_seconds() -> Option<u64> {
    let output = command_stdout("/usr/sbin/ioreg", &["-c", "IOHIDSystem"])?;
    output.lines().find_map(|line| {
        if !line.contains("HIDIdleTime") {
            return None;
        }
        let raw = line.split('=').nth(1)?.trim();
        let digits: String = raw.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        let nanos = digits.parse::<u64>().ok()?;
        Some(nanos / 1_000_000_000)
    })
}

fn detect_phone_wifi_targets_with_detail(targets: &[String]) -> (Option<bool>, String) {
    let arp_output = command_stdout("arp", &["-a"]);
    let mut checked = false;
    let mut checked_targets = Vec::new();
    for target in targets {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        checked = true;
        checked_targets.push(target.to_string());
        if arp_output
            .as_deref()
            .is_some_and(|output| arp_contains_target(output, target))
            || ping_target(target)
        {
            return (
                Some(true),
                format!(
                    "matched target={} arp_available={}",
                    target,
                    arp_output.is_some()
                ),
            );
        }
    }
    (
        checked.then_some(false),
        format!(
            "checked={} targets=[{}] arp_available={}",
            checked,
            checked_targets.join(", "),
            arp_output.is_some()
        ),
    )
}

fn detect_phone_bluetooth_targets_with_detail(targets: &[String]) -> (Option<bool>, String) {
    let targets: Vec<&str> = targets
        .iter()
        .map(|target| target.trim())
        .filter(|target| !target.is_empty())
        .collect();
    if targets.is_empty() {
        return (None, "no valid bluetooth targets".to_string());
    }
    let scan_targets = match scan_handoff_bluetooth_targets() {
        Ok(scan_targets) => scan_targets,
        Err(err) => return (None, format!("scan failed: {}", err)),
    };
    let seen = bluetooth_targets_seen_from_scan_targets(&targets, &scan_targets);
    let detail = match seen {
        Some(true) => {
            let matched = targets.iter().find_map(|target| {
                scan_targets
                    .iter()
                    .find(|scan_target| bluetooth_scan_target_matches(scan_target, target))
                    .map(|scan_target| format!("target={} matched={}", target, scan_target.label))
            });
            format!(
                "matched scan_count={} {} scanned=[{}]",
                scan_targets.len(),
                matched.unwrap_or_else(|| "matched target unknown".to_string()),
                format_scan_targets_for_log(&scan_targets)
            )
        }
        Some(false) => format!(
            "no match scan_count={} targets=[{}] scanned=[{}]",
            scan_targets.len(),
            targets.join(", "),
            format_scan_targets_for_log(&scan_targets)
        ),
        None => format!(
            "unknown scan_count={} targets=[{}]",
            scan_targets.len(),
            targets.join(", ")
        ),
    };
    (seen, detail)
}

fn bluetooth_targets_seen_from_scan_targets(
    targets: &[&str],
    scan_targets: &[BotHandoffScanTarget],
) -> Option<bool> {
    if targets.is_empty() || scan_targets.is_empty() {
        return None;
    }
    Some(targets.iter().any(|target| {
        scan_targets
            .iter()
            .any(|scan_target| bluetooth_scan_target_matches(scan_target, target))
    }))
}

fn parse_arp_scan_targets(output: &str) -> Vec<BotHandoffScanTarget> {
    let mut targets = Vec::new();
    for line in output.lines() {
        let Some(target) = parse_arp_scan_target(line) else {
            continue;
        };
        push_unique_scan_target(&mut targets, target);
    }
    targets
}

fn parse_arp_scan_target(line: &str) -> Option<BotHandoffScanTarget> {
    let line = line.trim();
    if line.is_empty() || line.contains("(incomplete)") {
        return None;
    }
    let open = line.find('(')?;
    let close = line[open + 1..].find(')')? + open + 1;
    let host = line[..open].trim().trim_end_matches('.');
    let ip = line[open + 1..close].trim();
    let after_at = line.split(" at ").nth(1).unwrap_or("").trim();
    let mac = after_at
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(',');
    let interface = line
        .split(" on ")
        .nth(1)
        .and_then(|value| value.split_whitespace().next())
        .unwrap_or("");
    let target = if !ip.is_empty() { ip } else { mac };
    if target.is_empty() {
        return None;
    }

    let mut detail_parts = Vec::new();
    if !mac.is_empty() && mac != "(incomplete)" {
        detail_parts.push(format!("MAC {}", mac));
    }
    if !interface.is_empty() {
        detail_parts.push(format!("interface {}", interface));
    }
    let label = if host.is_empty() || host == "?" {
        target.to_string()
    } else {
        format!("{} ({})", host, target)
    };
    Some(BotHandoffScanTarget {
        id: format!("wifi:{}", target),
        label,
        target: target.to_string(),
        detail: detail_parts.join(" / "),
        source: "wifi".to_string(),
    })
}

fn parse_bluetooth_scan_targets(output: &str) -> Vec<BotHandoffScanTarget> {
    let mut targets = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        collect_bluetooth_scan_targets(&value, &mut targets);
    }
    if targets.is_empty() {
        collect_bluetooth_scan_targets_from_text(output, &mut targets);
    }
    targets
}

fn collect_bluetooth_scan_targets_from_commands(targets: &mut Vec<BotHandoffScanTarget>) {
    let blueutil_arg_sets: &[&[&str]] = &[
        &["--format", "json", "--inquiry", "6"],
        &["--format", "json", "--connected"],
        &["--format", "json", "--paired"],
        &["--format", "json", "--recent"],
        &["--inquiry", "6"],
        &["--connected"],
        &["--paired"],
        &["--recent"],
    ];
    for args in blueutil_arg_sets {
        if let Some(output) = command_stdout("blueutil", args) {
            push_scan_targets_with_source(
                targets,
                parse_bluetooth_scan_targets(&output),
                &format!("blueutil {}", args.join(" ")),
            );
        }
    }

    if let Some(output) = command_stdin_stdout("/usr/bin/swift", &["-"], CORE_BLUETOOTH_SCAN_SWIFT)
    {
        push_scan_targets_with_source(
            targets,
            parse_bluetooth_scan_targets(&output),
            "core-bluetooth inquiry",
        );
    }

    if let Some(output) = command_stdout(
        "/usr/sbin/system_profiler",
        &["SPBluetoothDataType", "-json"],
    ) {
        push_scan_targets_with_source(
            targets,
            parse_bluetooth_scan_targets(&output),
            "system_profiler json",
        );
    }

    if let Some(output) = command_stdout("/usr/sbin/system_profiler", &["SPBluetoothDataType"]) {
        push_scan_targets_with_source(
            targets,
            parse_bluetooth_scan_targets(&output),
            "system_profiler text",
        );
    }

    if let Some(output) = command_stdout("ioreg", &["-r", "-c", "IOBluetoothDevice", "-l"]) {
        let mut ioreg_targets = Vec::new();
        collect_ioreg_bluetooth_scan_targets(&output, &mut ioreg_targets);
        push_scan_targets_with_source(targets, ioreg_targets, "ioreg IOBluetoothDevice");
    }
}

fn push_scan_targets_with_source(
    targets: &mut Vec<BotHandoffScanTarget>,
    parsed: Vec<BotHandoffScanTarget>,
    source: &str,
) {
    let source = source.trim();
    for mut target in parsed {
        if !source.is_empty() && !target.detail.contains("source ") {
            target.detail = if target.detail.trim().is_empty() {
                format!("source {}", source)
            } else {
                format!("{} / source {}", target.detail.trim(), source)
            };
        }
        push_unique_scan_target(targets, target);
    }
}

fn collect_bluetooth_scan_targets(value: &Value, targets: &mut Vec<BotHandoffScanTarget>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_bluetooth_scan_targets(value, targets);
            }
        }
        Value::Object(map) => {
            if let Some(target) = bluetooth_scan_target_from_object(map) {
                push_unique_scan_target(targets, target);
            }
            for value in map.values() {
                collect_bluetooth_scan_targets(value, targets);
            }
        }
        _ => {}
    }
}

fn bluetooth_scan_target_from_object(map: &Map<String, Value>) -> Option<BotHandoffScanTarget> {
    let name = first_string_field(
        map,
        &[
            "device_name",
            "device_title",
            "name",
            "_name",
            "displayName",
            "deviceName",
            "DeviceName",
            "localName",
            "Product",
        ],
    )
    .unwrap_or_default();
    let name = name.trim();
    let address = first_string_field(
        map,
        &[
            "device_address",
            "address",
            "bd_addr",
            "macAddress",
            "deviceAddress",
            "BD_ADDR",
            "BTAddress",
            "DeviceAddress",
        ],
    );
    let identifier = first_string_field(
        map,
        &["identifier", "id", "uuid", "UUID", "peripheralIdentifier"],
    );
    let has_device_marker = address.is_some()
        || identifier.is_some()
        || first_string_field(map, &["device_rssi", "rssi", "RSSI"]).is_some()
        || map
            .keys()
            .any(|key| key.to_ascii_lowercase().contains("device"));
    if !has_device_marker {
        return None;
    }
    if name.eq_ignore_ascii_case("bluetooth")
        || name.eq_ignore_ascii_case("Bluetooth-Incoming-Port")
    {
        return None;
    }
    let target = address
        .as_deref()
        .or(identifier.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(name)
        .trim();
    if target.is_empty() {
        return None;
    }
    let label = if name.is_empty() {
        bluetooth_fallback_label(target)
    } else {
        name.to_string()
    };
    let mut detail_parts = Vec::new();
    if let Some(address) = address.as_deref().filter(|value| !value.trim().is_empty()) {
        detail_parts.push(format!("address {}", address.trim()));
    }
    if let Some(identifier) = identifier
        .as_deref()
        .filter(|value| !value.trim().is_empty() && Some(*value) != address.as_deref())
    {
        detail_parts.push(format!("id {}", identifier.trim()));
    }
    if let Some(connected) = first_string_field(map, &["device_connected", "connected"]) {
        detail_parts.push(format!("connected {}", connected));
    }
    if let Some(rssi) = first_string_field(map, &["device_rssi", "rssi", "RSSI"]) {
        detail_parts.push(format!("RSSI {}", rssi));
    }

    Some(BotHandoffScanTarget {
        id: format!("bluetooth:{}", target),
        label,
        target: target.to_string(),
        detail: detail_parts.join(" / "),
        source: "bluetooth".to_string(),
    })
}

fn collect_bluetooth_scan_targets_from_text(output: &str, targets: &mut Vec<BotHandoffScanTarget>) {
    let mut current_name = String::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(target) = bluetooth_scan_target_from_key_value_line(trimmed) {
            push_unique_scan_target(targets, target);
            continue;
        }
        if let Some(name) = trimmed.strip_suffix(':') {
            let name = name.trim();
            if !name.eq_ignore_ascii_case("bluetooth") {
                current_name = name.to_string();
            }
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("address:") && !lower.starts_with("device address:") {
            continue;
        }
        let Some((_, address)) = trimmed.split_once(':') else {
            continue;
        };
        let address = address.trim();
        if current_name.is_empty() || address.is_empty() {
            continue;
        }
        push_unique_scan_target(
            targets,
            BotHandoffScanTarget {
                id: format!("bluetooth:{}", address),
                label: current_name.clone(),
                target: address.to_string(),
                detail: format!("address {}", address),
                source: "bluetooth".to_string(),
            },
        );
    }
}

fn bluetooth_scan_target_from_key_value_line(line: &str) -> Option<BotHandoffScanTarget> {
    if !line.contains(':') || !line.contains(',') {
        return None;
    }
    let mut map = Map::new();
    for part in line.split(',') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    bluetooth_scan_target_from_object(&map)
}

fn collect_ioreg_bluetooth_scan_targets(output: &str, targets: &mut Vec<BotHandoffScanTarget>) {
    let mut current: Option<Map<String, Value>> = None;
    for line in output.lines() {
        if line.contains("class IOBluetoothDevice") {
            if let Some(map) = current.take() {
                if let Some(target) = bluetooth_scan_target_from_ioreg_object(&map) {
                    push_unique_scan_target(targets, target);
                }
            }
            current = Some(Map::new());
            continue;
        }

        let Some(map) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = parse_ioreg_property_line(line) else {
            continue;
        };
        map.insert(key, Value::String(value));
    }
    if let Some(map) = current.take() {
        if let Some(target) = bluetooth_scan_target_from_ioreg_object(&map) {
            push_unique_scan_target(targets, target);
        }
    }
}

fn bluetooth_scan_target_from_ioreg_object(
    map: &Map<String, Value>,
) -> Option<BotHandoffScanTarget> {
    let device_type = first_string_field(map, &["DeviceType"]).unwrap_or_default();
    let tty_name = first_string_field(map, &["BTTTYName"]).unwrap_or_default();
    if device_type.eq_ignore_ascii_case("serial")
        && tty_name.eq_ignore_ascii_case("Bluetooth-Incoming-Port")
    {
        return None;
    }
    bluetooth_scan_target_from_object(map)
}

fn parse_ioreg_property_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let key_start = trimmed.find('"')? + 1;
    let key_end = trimmed[key_start..].find('"')? + key_start;
    let key = trimmed[key_start..key_end].trim();
    let (_, raw_value) = trimmed[key_end + 1..].split_once('=')?;
    let raw_value = raw_value.trim();
    let value = if let Some(hex_start) = raw_value.find('<') {
        let hex_end = raw_value[hex_start + 1..].find('>')? + hex_start + 1;
        format_mac_hex(&raw_value[hex_start + 1..hex_end])
            .unwrap_or_else(|| raw_value[hex_start + 1..hex_end].trim().to_string())
    } else if let Some(value_start) = raw_value.find('"') {
        let value_end = raw_value[value_start + 1..].find('"')? + value_start + 1;
        raw_value[value_start + 1..value_end].to_string()
    } else {
        raw_value
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(',')
            .to_string()
    };
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key.to_string(), value))
}

fn bluetooth_fallback_label(target: &str) -> String {
    let compact = target.trim();
    let short = compact.get(..8).unwrap_or(compact);
    format!("Bluetooth device {}", short)
}

fn bluetooth_scan_target_matches(scan_target: &BotHandoffScanTarget, target: &str) -> bool {
    let haystack = format!(
        "{}\n{}\n{}",
        scan_target.label, scan_target.target, scan_target.detail
    )
    .to_ascii_lowercase();
    bluetooth_target_match_candidates(target)
        .iter()
        .any(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return false;
            }
            let target_lower = candidate.to_ascii_lowercase();
            haystack.contains(&target_lower)
                || normalize_mac_address(candidate)
                    .as_deref()
                    .is_some_and(|target_mac| {
                        normalize_mac_address(&haystack)
                            .as_deref()
                            .is_some_and(|haystack_mac| haystack_mac.contains(target_mac))
                    })
        })
}

fn bluetooth_target_match_candidates(target: &str) -> Vec<String> {
    let target = target.trim();
    if target.is_empty() {
        return Vec::new();
    }
    let mut candidates = vec![target.to_string()];
    if let Some((name, raw_id)) = target.rsplit_once('(') {
        if let Some(id) = raw_id.strip_suffix(')') {
            let name = name.trim();
            let id = id.trim();
            if !id.is_empty() {
                candidates.push(id.to_string());
            }
            if !name.is_empty() {
                candidates.push(name.to_string());
            }
        }
    }
    candidates
}

fn fmt_option_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

fn fmt_option_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn format_log_list(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_string();
    }
    truncate_for_log(&values.join("; "), 600)
}

fn format_handoff_target_values(values: &[String]) -> String {
    let targets: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect();
    if targets.is_empty() {
        return "-".to_string();
    }
    truncate_for_log(&targets.join(", "), 300)
}

fn format_scan_targets_for_log(targets: &[BotHandoffScanTarget]) -> String {
    if targets.is_empty() {
        return "-".to_string();
    }
    let labels: Vec<String> = targets
        .iter()
        .take(8)
        .map(|target| {
            let detail = if target.detail.trim().is_empty() {
                String::new()
            } else {
                format!(" {}", target.detail.trim())
            };
            format!("{}({}){}", target.label, target.target, detail)
        })
        .collect();
    let suffix = if targets.len() > labels.len() {
        format!("; +{} more", targets.len() - labels.len())
    } else {
        String::new()
    };
    truncate_for_log(&format!("{}{}", labels.join("; "), suffix), 600)
}

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("...");
    truncated
}

fn first_string_field(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        match value {
            Value::String(text) if !text.trim().is_empty() => return Some(text.trim().to_string()),
            Value::Bool(value) => return Some(value.to_string()),
            Value::Number(value) => return Some(value.to_string()),
            _ => {}
        }
    }
    None
}

fn push_unique_scan_target(targets: &mut Vec<BotHandoffScanTarget>, target: BotHandoffScanTarget) {
    if target.target.trim().is_empty()
        || targets.iter().any(|existing| {
            existing.source == target.source
                && existing.target.eq_ignore_ascii_case(target.target.as_str())
        })
    {
        return;
    }
    targets.push(target);
}

fn ping_target(target: &str) -> bool {
    Command::new("ping")
        .args(["-c", "1", "-W", "1", target])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn arp_contains_target(output: &str, target: &str) -> bool {
    if let Some(mac) = normalize_mac_address(target) {
        return normalize_mac_address(output)
            .as_deref()
            .is_some_and(|output_mac| output_mac.contains(&mac));
    }
    output
        .to_ascii_lowercase()
        .contains(&target.to_ascii_lowercase())
}

fn normalize_mac_address(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    (hex.len() >= 12).then(|| hex.to_ascii_lowercase())
}

fn format_mac_hex(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    let mut parts = Vec::new();
    for index in (0..12).step_by(2) {
        parts.push(hex[index..index + 2].to_ascii_lowercase());
    }
    Some(parts.join(":"))
}

fn command_stdout(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn command_stdin_stdout(command: &str, args: &[&str], input: &str) -> Option<String> {
    let swift_module_cache = std::env::temp_dir().join("codexl-swift-module-cache");
    let _ = fs::create_dir_all(&swift_module_cache);
    let mut child = Command::new(command)
        .args(args)
        .env("CLANG_MODULE_CACHE_PATH", swift_module_cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input.as_bytes()).ok()?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn completed_agent_message_text(params: &Value) -> Option<String> {
    let item = params.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return None;
    }
    let text = item
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn turn_completed_error_message(params: &Value) -> Option<String> {
    params
        .get("turn")
        .and_then(|turn| turn.get("error"))
        .filter(|error| !error.is_null())
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown turn error")
                .to_string()
        })
}

fn idle_handoff_turn_key(params: &Value) -> Option<String> {
    let thread_id = nested_param_id(params, "threadId", "thread")?;
    let turn_id = nested_param_id(params, "turnId", "turn").unwrap_or("unknown");
    Some(format!("{}:{}", thread_id, turn_id))
}

impl ThreadSummary {
    fn from_value(value: &Value) -> Option<Self> {
        let id = value.get("id").and_then(Value::as_str)?.to_string();
        let preview = value
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .or_else(|| value.get("preview").and_then(Value::as_str))
            .unwrap_or("Untitled")
            .trim()
            .to_string();
        let cwd = value
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .map(ToString::to_string);
        let path = value
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(ToString::to_string);
        let updated_at = value.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
        let status = value
            .get("status")
            .and_then(|status| status.get("type"))
            .and_then(Value::as_str)
            .map(ToString::to_string);

        Some(Self {
            id,
            preview,
            cwd,
            path,
            updated_at,
            status,
        })
    }
}

fn project_name(cwd: &str) -> String {
    if cwd == PROJECTLESS_PROJECT_LABEL {
        return cwd.to_string();
    }
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(cwd)
        .to_string()
}

fn display_thread_preview(thread: &ThreadSummary) -> String {
    thread
        .preview
        .lines()
        .next()
        .unwrap_or("Untitled")
        .trim()
        .chars()
        .take(80)
        .collect()
}

fn project_switch_notice(project: &ProjectSummary, language: AppLanguage) -> String {
    if project.cwd == PROJECTLESS_PROJECT_LABEL {
        return match language {
            AppLanguage::Zh => "接力：即将切换到无项目 Codex 会话。".to_string(),
            AppLanguage::En => "Handoff: switching to a projectless Codex session.".to_string(),
        };
    }

    match language {
        AppLanguage::Zh => format!("接力：即将切换到项目 {} ({})。", project.name, project.cwd),
        AppLanguage::En => format!(
            "Handoff: switching to project {} ({}).",
            project.name, project.cwd
        ),
    }
}

fn handoff_activation_notice_for_context(
    thread_id: &str,
    project: &str,
    presence: &HandoffPresence,
    language: AppLanguage,
) -> String {
    let reason = presence.summary_for_language(language);
    match language {
        AppLanguage::Zh => format!(
            "接力模式已开启：检测到你可能不在电脑旁（{}）。接下来会把此 Codex 会话的消息转发到 Bot。\n\n项目：{}\nSession：{}",
            reason,
            project,
            short_id(thread_id)
        ),
        AppLanguage::En => format!(
            "Handoff is now on: Codex detected that you may be away from your computer ({}). Messages from this Codex session will be forwarded to Bot.\n\nProject: {}\nSession: {}",
            reason,
            project,
            short_id(thread_id)
        ),
    }
}

fn handoff_deactivation_notice_for_context(
    thread_id: &str,
    project: &str,
    presence: Option<&HandoffPresence>,
    language: AppLanguage,
) -> String {
    let reason = presence
        .map(|presence| presence.summary_for_language(language))
        .unwrap_or_else(|| localized_handoff_signal("presence signal unavailable", language));
    match language {
        AppLanguage::Zh => format!(
            "接力模式已关闭：当前接力条件不再满足（{}）。后续消息不会再因接力转发到 Bot。\n\n项目：{}\nSession：{}",
            reason,
            project,
            short_id(thread_id)
        ),
        AppLanguage::En => format!(
            "Handoff is now off: the current handoff conditions are no longer met ({}). Future messages will no longer be forwarded to Bot because of handoff.\n\nProject: {}\nSession: {}",
            reason,
            project,
            short_id(thread_id)
        ),
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn score_project_match(project: &ProjectSummary, query_lower: &str) -> u8 {
    let name_lower = project.name.to_ascii_lowercase();
    let cwd_lower = project.cwd.to_ascii_lowercase();
    if name_lower == query_lower || cwd_lower == query_lower {
        0
    } else if cwd_lower.ends_with(&format!("/{}", query_lower)) {
        1
    } else if name_lower.contains(query_lower) {
        2
    } else {
        3
    }
}

fn score_thread_match(thread: &ThreadSummary, query_lower: &str) -> u8 {
    let id_lower = thread.id.to_ascii_lowercase();
    let preview_lower = thread.preview.to_ascii_lowercase();
    if id_lower == query_lower {
        0
    } else if id_lower.starts_with(query_lower) {
        1
    } else if preview_lower == query_lower {
        2
    } else {
        3
    }
}

fn parse_project_index_selector(query: &str) -> Option<usize> {
    let inner = bracketed_selector(query)?;
    if inner.contains('.') {
        return None;
    }
    let index = inner.parse::<usize>().ok()?;
    index.checked_sub(1)
}

fn parse_thread_index_selector(query: &str) -> Option<(usize, usize)> {
    let inner = bracketed_selector(query)?;
    let (project, thread) = inner.split_once('.')?;
    if thread.contains('.') {
        return None;
    }
    let project_index = project.trim().parse::<usize>().ok()?.checked_sub(1)?;
    let thread_index = thread.trim().parse::<usize>().ok()?.checked_sub(1)?;
    Some((project_index, thread_index))
}

fn bracketed_selector(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

fn find_use_project_colon(text: &str) -> Option<(usize, usize)> {
    text.char_indices()
        .find(|(_, ch)| *ch == ':' || *ch == '：')
        .map(|(index, ch)| (index, ch.len_utf8()))
}

fn sanitize_path_segment(text: &str) -> String {
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

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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

fn resolve_bot_gateway_state_dir(value: Option<&str>, profile_name: &str) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(value.to_string()))
        .or_else(|| {
            std::env::var(STATE_DIR_ENV)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(expand_home_path)
        })
        .or_else(|| Some(config::generated_bot_gateway_state_dir(profile_name)))
}

fn bot_session_store_path(config: &BotBridgeConfig) -> PathBuf {
    config
        .state_dir
        .clone()
        .unwrap_or_else(legacy_bot_session_store_dir)
        .join("codex-sessions.json")
}

fn bot_media_context_path(config: &BotBridgeConfig) -> PathBuf {
    config
        .state_dir
        .clone()
        .unwrap_or_else(legacy_bot_session_store_dir)
        .join(BOT_MEDIA_CONTEXT_FILE)
}

fn legacy_bot_session_store_dir() -> PathBuf {
    codexl_home_dir().join("bot-gateway-bridge")
}

fn load_bot_media_context_for_tool(
    config: &BotBridgeConfig,
    args: &Map<String, Value>,
) -> Result<BotMediaMcpContext, String> {
    let requested_session_id = string_arg(
        args,
        &["botSessionId", "botSessionKey", "sessionId", "sessionKey"],
    )
    .ok_or_else(|| {
        "Bot media MCP calls require botSessionId from the current Bot bridge prompt".to_string()
    })?;
    let store = load_bot_media_context_store(config)?;
    if let Some(context) = store.sessions.get(&requested_session_id) {
        return Ok(context.clone());
    }
    if let Some(context) = store.sessions.values().find(|context| {
        context.session_key == requested_session_id
            || context.thread_id.as_deref() == Some(&requested_session_id)
    }) {
        return Ok(context.clone());
    }

    Err(format!(
        "Bot media session {} was not found; use the botSessionId from the current Bot bridge prompt",
        requested_session_id
    ))
}

fn load_bot_media_context_store(
    config: &BotBridgeConfig,
) -> Result<BotMediaMcpContextStore, String> {
    let path = bot_media_context_path(config);
    let content = fs::read_to_string(&path).map_err(|err| {
        format!(
            "failed to read bot media context {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;
    let value = serde_json::from_str::<Value>(&content).map_err(|err| {
        format!(
            "failed to parse bot media context {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;

    if value.get("sessions").is_some() {
        return serde_json::from_value::<BotMediaMcpContextStore>(value).map_err(|err| {
            format!(
                "failed to parse bot media context {}: {}",
                path.to_string_lossy(),
                err
            )
        });
    }

    let mut context = serde_json::from_value::<BotMediaMcpContext>(value).map_err(|err| {
        format!(
            "failed to parse bot media context {}: {}",
            path.to_string_lossy(),
            err
        )
    })?;
    if context.session_key.is_empty() {
        context.session_key = "legacy".to_string();
    }
    if context.session_id.is_empty() {
        context.session_id = new_uuid_v4();
    }
    let mut store = BotMediaMcpContextStore {
        latest_session_id: Some(context.session_id.clone()),
        sessions: BTreeMap::new(),
    };
    store.sessions.insert(context.session_id.clone(), context);
    Ok(store)
}

fn latest_bot_media_context(config: &BotBridgeConfig) -> Option<BotMediaMcpContext> {
    let store = load_bot_media_context_store(config).ok()?;
    store
        .latest_session_id
        .as_deref()
        .and_then(|session_id| store.sessions.get(session_id))
        .cloned()
        .or_else(|| {
            store
                .sessions
                .values()
                .max_by_key(|context| context.updated_at)
                .cloned()
        })
}

fn bot_event_from_media_context(context: &BotMediaMcpContext) -> Value {
    let conversation = json!({
        "id": context
            .conversation_ref
            .get("platformConversationId")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        "type": context
            .conversation_ref
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("dm"),
    });
    let mut message = Map::new();
    if let Some(thread_id) = context
        .conversation_ref
        .get("threadId")
        .and_then(Value::as_str)
        .or(context.thread_id.as_deref())
    {
        message.insert("threadId".to_string(), Value::String(thread_id.to_string()));
    }
    let mut raw = Map::new();
    if let Some(context_token) = context
        .conversation_ref
        .get("contextToken")
        .and_then(Value::as_str)
    {
        raw.insert(
            "context_token".to_string(),
            Value::String(context_token.to_string()),
        );
    }

    json!({
        "tenantId": context.tenant_id.clone(),
        "integrationId": context.integration_id.clone(),
        "platform": context.platform.clone(),
        "conversation": conversation,
        "message": Value::Object(message),
        "raw": Value::Object(raw),
    })
}

fn persist_bot_media_context(
    config: &BotBridgeConfig,
    context: BotMediaMcpContext,
) -> Result<(), String> {
    let path = bot_media_context_path(config);
    let mut store = match load_bot_media_context_store(config) {
        Ok(store) => store,
        Err(_) => BotMediaMcpContextStore::default(),
    };
    let mut context = context;
    if context.session_id.is_empty() {
        context.session_id = new_uuid_v4();
    }
    store.latest_session_id = Some(context.session_id.clone());
    store.sessions.insert(context.session_id.clone(), context);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content = serde_json::to_string_pretty(&store).map_err(|err| err.to_string())?;
    fs::write(&path, content).map_err(|err| {
        format!(
            "failed to write bot media context {}: {}",
            path.to_string_lossy(),
            err
        )
    })
}

fn load_bot_session_state(config: &BotBridgeConfig, key: &str) -> Option<PersistedBotSessionState> {
    load_bot_session_store(&bot_session_store_path(config))
        .sessions
        .get(key)
        .cloned()
        .or_else(|| {
            let legacy_path = legacy_bot_session_store_dir().join("codex-sessions.json");
            if legacy_path == bot_session_store_path(config) {
                return None;
            }
            load_bot_session_store(&legacy_path)
                .sessions
                .get(key)
                .cloned()
        })
}

fn bot_session_key(config: &BotBridgeConfig, event: &Value) -> String {
    let tenant_id = event
        .get("tenantId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(config.tenant_id.as_str());
    let platform = event
        .get("platform")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(config.platform.as_str());
    let conversation_id = event
        .get("conversation")
        .and_then(|conversation| conversation.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown");

    serde_json::to_string(&json!([
        tenant_id,
        config.integration_id.as_str(),
        platform,
        conversation_id
    ]))
    .unwrap_or_else(|_| {
        format!(
            "{}:{}:{}:{}",
            tenant_id, config.integration_id, platform, conversation_id
        )
    })
}

fn resolve_bot_media_session_id(
    config: &BotBridgeConfig,
    session_key: &str,
    persisted_session_id: Option<&str>,
) -> String {
    if let Some(session_id) = normalize_uuid_like(persisted_session_id) {
        return session_id;
    }

    load_bot_media_context_store(config)
        .ok()
        .and_then(|store| {
            store
                .sessions
                .values()
                .find(|context| context.session_key == session_key)
                .and_then(|context| normalize_uuid_like(Some(&context.session_id)))
        })
        .unwrap_or_else(new_uuid_v4)
}

fn normalize_uuid_like(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    is_uuid_like(value).then(|| value.to_ascii_lowercase())
}

fn is_uuid_like(value: &str) -> bool {
    let value = value.trim();
    if value.len() != 36 {
        return false;
    }
    for (index, ch) in value.chars().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if ch != '-' {
                    return false;
                }
            }
            _ => {
                if !ch.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

fn new_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let counter = BOT_MEDIA_SESSION_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let fallback = nanos ^ ((std::process::id() as u128) << 64) ^ counter;
        bytes = fallback.to_be_bytes();
    }

    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn legacy_bot_thread_names(config: &BotBridgeConfig) -> Vec<String> {
    let mut names = vec![format!("Bot: {}", config.integration_id)];
    if config.tenant_id != config.integration_id {
        names.push(format!("Bot: {}", config.tenant_id));
    }
    names
}

fn load_bot_session_store(path: &Path) -> PersistedBotSessionStore {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<PersistedBotSessionStore>(&content).ok())
        .unwrap_or_default()
}

fn persist_bot_session_state(
    config: &BotBridgeConfig,
    key: &str,
    state: PersistedBotSessionState,
) -> Result<(), String> {
    let path = bot_session_store_path(config);
    let mut store = load_bot_session_store(&path);
    if state.thread_id.is_some() || state.selected_cwd.is_some() || state.media_session_id.is_some()
    {
        store.sessions.insert(key.to_string(), state);
    } else {
        store.sessions.remove(key);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content = serde_json::to_string_pretty(&store).map_err(|err| err.to_string())?;
    fs::write(&path, content).map_err(|err| {
        format!(
            "failed to write bot session store {}: {}",
            path.to_string_lossy(),
            err
        )
    })
}

fn migrate_legacy_bot_gateway_integration(config: &BotBridgeConfig) -> Result<(), String> {
    let Some(target_dir) = config.state_dir.as_ref() else {
        return Ok(());
    };
    let target_path = target_dir.join("integrations.json");
    let mut target_store = read_bot_gateway_integration_store(&target_path)
        .unwrap_or_else(|| json!({ "integrations": [] }));

    if integration_store_contains(&target_store, &config.integration_id) {
        return Ok(());
    }

    let Some(integration) = legacy_bot_gateway_state_dirs(config)
        .into_iter()
        .filter(|dir| dir != target_dir)
        .find_map(|dir| {
            read_bot_gateway_integration_store(&dir.join("integrations.json"))
                .and_then(|store| find_integration(&store, &config.integration_id))
        })
    else {
        return Ok(());
    };

    let integrations = target_store
        .as_object_mut()
        .and_then(|store| store.get_mut("integrations"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            format!(
                "Bot Gateway integration store {} does not contain an integrations array",
                target_path.to_string_lossy()
            )
        })?;
    integrations.push(integration);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content = serde_json::to_string_pretty(&target_store).map_err(|err| err.to_string())?;
    fs::write(&target_path, content).map_err(|err| {
        format!(
            "failed to write Bot Gateway integration store {}: {}",
            target_path.to_string_lossy(),
            err
        )
    })?;
    log_bridge(
        config,
        &format!(
            "migrated Bot Gateway integration {} to {}",
            config.integration_id,
            target_dir.to_string_lossy()
        ),
    );
    Ok(())
}

fn legacy_bot_gateway_state_dirs(config: &BotBridgeConfig) -> Vec<PathBuf> {
    vec![
        config.extension.root_dir.join(".bot-gateway-state"),
        codexl_home_dir()
            .join("bot-gateway")
            .join(".bot-gateway-state"),
    ]
}

fn read_bot_gateway_integration_store(path: &Path) -> Option<Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
}

fn integration_store_contains(store: &Value, integration_id: &str) -> bool {
    find_integration(store, integration_id).is_some()
}

fn find_integration(store: &Value, integration_id: &str) -> Option<Value> {
    store
        .get("integrations")
        .and_then(Value::as_array)?
        .iter()
        .find(|integration| {
            integration
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == integration_id)
        })
        .cloned()
}

impl BotGatewayClient {
    fn start(extension: &BuiltinNodeExtension, state_dir: Option<&Path>) -> Result<Self, String> {
        let mut command = Command::new(&extension.node.executable);
        command.arg(&extension.entry_path);
        if let Some(state_dir) = state_dir {
            command.env(BOT_GATEWAY_STATE_DIR_ENV, state_dir);
            if let Some(proxy_url) = bot_gateway_proxy_url_from_state_dir(state_dir) {
                command.env(BOT_GATEWAY_PROXY_URL_ENV, &proxy_url);
                for key in ["http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY"] {
                    if std::env::var_os(key).is_none() {
                        command.env(key, &proxy_url);
                    }
                }
            }
        }
        let mut child = command
            .current_dir(&extension.root_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start Bot Gateway stdio: {}", err))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open Bot Gateway stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open Bot Gateway stdout".to_string())?;
        if let Some(stderr) = child.stderr.take() {
            let log_path = log_path();
            thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    let size = match reader.read_line(&mut line) {
                        Ok(size) => size,
                        Err(_) => break,
                    };
                    if size == 0 {
                        break;
                    }
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        log_bridge_path(&log_path, &format!("bot-gateway stderr: {}", trimmed));
                    }
                }
            });
        }
        let (response_tx, response_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                let size = match reader.read_line(&mut line) {
                    Ok(size) => size,
                    Err(_) => break,
                };
                if size == 0 {
                    break;
                }
                let Ok(value) = serde_json::from_str::<Value>(line.trim_end()) else {
                    continue;
                };
                if response_tx.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin,
            response_rx,
            next_id: 1,
        };
        let _ = client.request_with_timeout(
            "health",
            json!({}),
            Duration::from_secs(BOT_GATEWAY_HEALTH_TIMEOUT_SECS),
        )?;
        Ok(client)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_timeout(
            method,
            params,
            Duration::from_secs(BOT_GATEWAY_DEFAULT_REQUEST_TIMEOUT_SECS),
        )
    }

    fn request_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = format!("{}{}", BOT_REQUEST_ID_PREFIX, self.next_id);
        self.next_id += 1;
        let request = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_vec(&request).map_err(|err| err.to_string())?;
        self.stdin
            .write_all(&line)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|err| format!("failed to write Bot Gateway request: {}", err))?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.stop_child();
                return Err(format!(
                    "Bot Gateway request timed out after {}s: {}",
                    timeout.as_secs(),
                    method
                ));
            }
            let value = match self.response_rx.recv_timeout(remaining) {
                Ok(value) => value,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.stop_child();
                    return Err(format!(
                        "Bot Gateway request timed out after {}s: {}",
                        timeout.as_secs(),
                        method
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Bot Gateway stdio exited".to_string());
                }
            };
            if value.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            let message = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Bot Gateway request failed");
            return Err(message.to_string());
        }
    }

    fn stop_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bot_gateway_proxy_url_from_state_dir(state_dir: &Path) -> Option<String> {
    let path = state_dir.join("integrations.json");
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    let integrations = value.get("integrations")?.as_array()?;
    for integration in integrations {
        let Some(config) = integration.get("config").and_then(Value::as_object) else {
            continue;
        };
        for key in ["proxyUrl", "httpsProxy", "httpProxy", "proxy"] {
            let value = config
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(value) = value {
                return Some(value.to_string());
            }
        }
    }
    None
}

impl Drop for BotGatewayClient {
    fn drop(&mut self) {
        self.stop_child();
    }
}

impl BotGatewayRuntimeConfig {
    fn from_profile(profile_name: &str, bot_config: &BotProfileConfig) -> Result<Self, String> {
        let mut bot_config = bot_config.clone();
        bot_config.normalize_for_profile(profile_name);
        if !bot_config.bridge_enabled() {
            return Err(format!("Bot is not enabled for workspace {}", profile_name));
        }

        Ok(Self {
            profile_name: profile_name.to_string(),
            extension: extensions::resolve_builtin_bot_gateway_extension()?,
            state_dir: resolve_bot_gateway_state_dir(Some(&bot_config.state_dir), profile_name),
            platform: bot_config.platform,
            tenant_id: bot_config.tenant_id,
            integration_id: bot_config.integration_id,
        })
    }
}

impl BotBridgeConfig {
    fn from_env() -> Option<Self> {
        if !env_truthy(ENABLED_ENV) {
            return None;
        }

        let extension = match extensions::resolve_builtin_bot_gateway_extension() {
            Ok(extension) => extension,
            Err(err) => {
                log_bridge_path(&log_path(), &format!("bridge disabled: {}", err));
                return None;
            }
        };
        let platform = std::env::var(PLATFORM_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| config::BOT_PLATFORM_WEIXIN_ILINK.to_string());
        let profile_name = std::env::var(PROFILE_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let tenant_id = std::env::var(TENANT_ID_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| profile_name.clone())
            .unwrap_or_else(|| config::DEFAULT_BOT_TENANT_ID.to_string());
        let state_dir =
            resolve_bot_gateway_state_dir(None, profile_name.as_deref().unwrap_or(&tenant_id));
        let integration_id = std::env::var(INTEGRATION_ID_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "default".to_string());
        let mut handoff = BotHandoffConfig {
            enabled: env_truthy(HANDOFF_ENABLED_ENV),
            idle_seconds: env_u64(HANDOFF_IDLE_SECONDS_ENV, 30),
            screen_lock: env_bool_with_default(HANDOFF_SCREEN_LOCK_ENV, true),
            user_idle: env_bool_with_default(HANDOFF_USER_IDLE_ENV, true),
            phone_wifi_targets: env_list(HANDOFF_PHONE_WIFI_TARGETS_ENV),
            phone_bluetooth_targets: env_list(HANDOFF_PHONE_BLUETOOTH_TARGETS_ENV),
        };
        handoff.normalize();

        Some(Self {
            extension,
            state_dir,
            platform,
            tenant_id,
            integration_id,
            poll_interval: Duration::from_millis(env_u64(POLL_INTERVAL_ENV, 1500)),
            turn_timeout: Duration::from_millis(env_u64(TURN_TIMEOUT_ENV, 600_000)),
            forward_all_codex_messages: env_truthy(FORWARD_ALL_CODEX_MESSAGES_ENV),
            handoff,
            language: AppLanguage::from_value(
                &std::env::var(LANGUAGE_ENV).unwrap_or_else(|_| "en".to_string()),
            ),
            log_path: log_path(),
        })
    }
}

fn conversation_ref(event: &Value) -> Value {
    let mut ref_object = Map::new();
    if let Some(conversation_id) = event
        .get("conversation")
        .and_then(|conversation| conversation.get("id"))
        .and_then(Value::as_str)
    {
        ref_object.insert(
            "platformConversationId".to_string(),
            Value::String(conversation_id.to_string()),
        );
    }
    if let Some(conversation_type) = event
        .get("conversation")
        .and_then(|conversation| conversation.get("type"))
        .and_then(Value::as_str)
    {
        ref_object.insert(
            "type".to_string(),
            Value::String(conversation_type.to_string()),
        );
    }
    if let Some(thread_id) = event
        .get("message")
        .and_then(|message| message.get("threadId"))
        .and_then(Value::as_str)
    {
        ref_object.insert("threadId".to_string(), Value::String(thread_id.to_string()));
    }
    if let Some(context_token) = event
        .get("raw")
        .and_then(|raw| raw.get("context_token"))
        .or_else(|| event.get("raw").and_then(|raw| raw.get("sessionWebhook")))
        .or_else(|| event.get("raw").and_then(|raw| raw.get("contextToken")))
        .and_then(Value::as_str)
    {
        ref_object.insert(
            "contextToken".to_string(),
            Value::String(context_token.to_string()),
        );
    }
    Value::Object(ref_object)
}

fn event_integration_id(event: &Value, fallback: &str) -> String {
    event
        .get("integrationId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn is_startable_bot_gateway_platform(platform: &str) -> bool {
    matches!(
        platform,
        config::BOT_PLATFORM_WEIXIN_ILINK
            | config::BOT_PLATFORM_FEISHU
            | config::BOT_PLATFORM_DISCORD
            | config::BOT_PLATFORM_SLACK
            | config::BOT_PLATFORM_TELEGRAM
    )
}

fn matches_thread_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    if !matches_param_id(params, "threadId", "thread", thread_id) {
        return false;
    }

    match nested_param_id(params, "turnId", "turn") {
        Some(actual_turn_id) => actual_turn_id == turn_id,
        None => true,
    }
}

fn matches_param_id(params: &Value, top_level_key: &str, nested_key: &str, expected: &str) -> bool {
    nested_param_id(params, top_level_key, nested_key).is_some_and(|actual| actual == expected)
}

fn nested_param_id<'a>(
    params: &'a Value,
    top_level_key: &str,
    nested_key: &str,
) -> Option<&'a str> {
    params
        .get(top_level_key)
        .and_then(Value::as_str)
        .or_else(|| {
            params
                .get(nested_key)
                .and_then(|nested| nested.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| params.get(nested_key).and_then(Value::as_str))
}

fn write_json_line(writer: &SharedAppStdin, value: &Value) -> Result<(), String> {
    let line = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    let mut writer = writer
        .lock()
        .map_err(|_| "Codex app-server stdin mutex poisoned".to_string())?;
    writer
        .write_all(&line)
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|err| format!("failed to write Codex app-server request: {}", err))
}

fn next_app_request_id() -> String {
    format!(
        "{}{}",
        APP_REQUEST_ID_PREFIX,
        APP_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn trim_json_line(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .unwrap_or(line)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_bool_with_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_list(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            let separator = if value.contains('\n') { '\n' } else { ',' };
            value
                .split(separator)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn log_path() -> PathBuf {
    std::env::var(LOG_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(expand_home_path)
        .unwrap_or_else(|| codexl_home_dir().join("bot-gateway-bridge.log"))
}

fn log_bridge(config: &BotBridgeConfig, message: &str) {
    log_bridge_path(&config.log_path, message);
}

fn log_bridge_path(path: &Path, message: &str) {
    let Some(mut file) = open_log_file(path) else {
        return;
    };
    let _ = writeln!(file, "[{}] {}", timestamp_seconds(), message);
}

fn open_log_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn expand_home_path(path: String) -> PathBuf {
    super::expand_home_path(path)
}

fn codexl_home_dir() -> PathBuf {
    super::codexl_home_dir()
}

fn timestamp_seconds() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{}", seconds)
}

#[cfg(test)]
mod tests;
