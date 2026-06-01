use super::cdp::{cdp_send, connect_target, list_targets};
use super::*;
use crate::config::{generated_codex_home, AppConfig, ProviderProfile};
use crate::extensions::builtins::gateway::config as gateway_config;
use rand::rngs::OsRng;
use rand::RngCore;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Component;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Instant;

const CODEXL_PLUGIN_BRIDGE_PATH: &str = "/plugin/_bridge";
const CODEXL_PLUGIN_CDP_BINDING_NAME: &str = "__codexlPluginBridge";
const CODEXL_PLUGIN_INJECT_TIMEOUT_MS: u64 = 20_000;
const CODEXL_PLUGIN_INJECT_RETRY_MS: u64 = 150;
const CODEXL_PLUGIN_RUNTIME_VERSION: &str = "0.1.18";
const CODEXL_RENDERER_PLUGIN_ENTRY_LIMIT_BYTES: u64 = 2 * 1024 * 1024;
const CODEXL_CORE_PLUGIN_ID: &str = "codexl.core";
const CODEXL_PLUGIN_REMOTE_WEB_BRIDGE_URL: &str =
    "ws://127.0.0.1:0/plugin/_bridge?token=remote-web-bridge";
const CODEXL_PLUGIN_WEB_BRIDGE_MESSAGE_TYPE: &str = "codexl-plugin-bridge";
pub(crate) const SHOW_ALL_SESSIONS_KEY: &str = "showAllSessions";

static CODEXL_PLUGIN_BRIDGE_TOKENS: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RendererPluginManifest {
    id: String,
    name: String,
    version: String,
    entry: String,
    enabled: bool,
}

impl Default for RendererPluginManifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            entry: "index.js".to_string(),
            enabled: true,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RendererPluginPayload {
    id: String,
    name: String,
    version: String,
    entry: String,
    source: String,
    source_url: String,
}

pub fn spawn_codex_plugin_injector(
    cdp_host: String,
    cdp_port: u16,
    http_host: String,
    http_port: u16,
) {
    let bridge_token = register_plugin_bridge_token();
    let bridge_url = plugin_bridge_url(&http_host, http_port, &bridge_token);

    tokio::spawn(async move {
        if let Err(err) = inject_codex_plugin_runtime(cdp_host, cdp_port, bridge_url).await {
            eprintln!("[codex-plugin] failed to inject runtime: {}", err);
        }
    });
}

pub fn plugin_bridge_token_valid(query: Option<&str>) -> bool {
    let Some(token) = query.and_then(query_token) else {
        return false;
    };
    plugin_bridge_tokens()
        .lock()
        .map(|tokens| tokens.contains(token))
        .unwrap_or(false)
}

pub async fn handle_plugin_bridge_websocket<S>(
    mut websocket: WebSocketStream<S>,
    token: Option<String>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if !token
        .as_deref()
        .map(|token| {
            plugin_bridge_tokens()
                .lock()
                .map(|tokens| tokens.contains(token))
                .unwrap_or(false)
        })
        .unwrap_or(false)
    {
        let _ = websocket
            .send(Message::Text(
                json!({
                    "type": "error",
                    "error": "invalid plugin bridge token",
                })
                .to_string(),
            ))
            .await;
        let _ = websocket.close(None).await;
        return Ok(());
    }

    eprintln!("[codex-plugin] bridge websocket opened");
    while let Some(message) = websocket.next().await {
        let message = message.map_err(|e| e.to_string())?;
        let Message::Text(raw) = message else {
            continue;
        };
        let response = plugin_bridge_socket_response(&raw).await;
        websocket
            .send(Message::Text(response.to_string()))
            .await
            .map_err(|e| e.to_string())?;
    }
    eprintln!("[codex-plugin] bridge websocket closed");
    Ok(())
}

pub(super) fn web_plugin_runtime_script_response() -> WebResourceResponse {
    WebResourceResponse {
        status: StatusCode::OK,
        content_type: "application/javascript; charset=utf-8".to_string(),
        body: Bytes::from(web_plugin_runtime_script()),
    }
}

pub(super) fn web_plugin_runtime_version() -> &'static str {
    CODEXL_PLUGIN_RUNTIME_VERSION
}

pub(super) fn web_plugin_runtime_script() -> String {
    codex_plugin_bootstrap_script(CODEXL_PLUGIN_REMOTE_WEB_BRIDGE_URL)
}

pub(crate) fn is_plugin_bridge_message(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(CODEXL_PLUGIN_WEB_BRIDGE_MESSAGE_TYPE)
}

pub(crate) async fn dispatch_plugin_bridge_message(message: Value) -> Result<Value, String> {
    let plugin_request = message
        .get("pluginRequest")
        .or_else(|| message.get("request"))
        .or_else(|| message.get("payload"))
        .cloned()
        .ok_or_else(|| "missing CodexL plugin bridge request".to_string())?;
    let raw = serde_json::to_string(&plugin_request).map_err(|err| err.to_string())?;
    let response = plugin_bridge_socket_response(&raw).await;
    Ok(json!({
        "messages": [],
        "codexlPluginResponse": response,
    }))
}

async fn inject_codex_plugin_runtime(
    cdp_host: String,
    cdp_port: u16,
    bridge_url: String,
) -> Result<(), String> {
    let source = codex_plugin_bootstrap_script(&bridge_url);
    let started_at = Instant::now();

    loop {
        match inject_selected_page_target(&cdp_host, cdp_port, &source).await {
            Ok(target_label) => {
                eprintln!("[codex-plugin] runtime injected into {}", target_label);
                return Ok(());
            }
            Err(err) => {
                if started_at.elapsed() >= Duration::from_millis(CODEXL_PLUGIN_INJECT_TIMEOUT_MS) {
                    return Err(err);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(CODEXL_PLUGIN_INJECT_RETRY_MS)).await;
    }
}

async fn inject_selected_page_target(
    cdp_host: &str,
    cdp_port: u16,
    source: &str,
) -> Result<String, String> {
    let targets = list_targets(cdp_host, cdp_port).await?;
    let target = select_plugin_target(&targets)
        .ok_or_else(|| "no page CDP target with websocket debugger URL".to_string())?;
    let mut socket = connect_target(&target).await?;
    let mut next_id = 1_u64;
    let target_label = plugin_target_label(&target);

    let _ = cdp_send(&mut socket, &mut next_id, "Page.enable", json!({})).await;
    let _ = cdp_send(&mut socket, &mut next_id, "Runtime.enable", json!({})).await;
    let _ = cdp_send(
        &mut socket,
        &mut next_id,
        "Runtime.addBinding",
        json!({
            "name": CODEXL_PLUGIN_CDP_BINDING_NAME,
        }),
    )
    .await;
    let _ = cdp_send(
        &mut socket,
        &mut next_id,
        "Page.setBypassCSP",
        json!({
            "enabled": true,
        }),
    )
    .await;
    cdp_send(
        &mut socket,
        &mut next_id,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({
            "source": source,
        }),
    )
    .await?;
    let evaluate_id = next_id;
    next_id += 1;
    socket
        .send(Message::Text(
            json!({
                "id": evaluate_id,
                "method": "Runtime.evaluate",
                "params": {
                    "awaitPromise": false,
                    "expression": source,
                    "returnByValue": true,
                },
            })
            .to_string(),
        ))
        .await
        .map_err(|err| err.to_string())?;

    tokio::spawn(run_plugin_cdp_binding_bridge(
        socket,
        next_id,
        target_label.clone(),
    ));

    Ok(target_label)
}

async fn run_plugin_cdp_binding_bridge(
    mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    mut next_id: u64,
    target_label: String,
) {
    let close_reason = loop {
        let message = match socket.next().await {
            Some(Ok(message)) => message,
            Some(Err(err)) => break err.to_string(),
            None => break "CDP socket closed".to_string(),
        };
        let Message::Text(text) = message else {
            continue;
        };
        let value = match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") {
            continue;
        }
        let params = value.get("params").unwrap_or(&Value::Null);
        if params.get("name").and_then(Value::as_str) != Some(CODEXL_PLUGIN_CDP_BINDING_NAME) {
            continue;
        }
        let Some(payload) = params.get("payload").and_then(Value::as_str) else {
            continue;
        };
        let response = plugin_bridge_socket_response(payload).await;
        let expression = plugin_cdp_response_expression(&response);
        let id = next_id;
        next_id += 1;
        let command = json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "awaitPromise": false,
                "expression": expression,
                "returnByValue": false,
            },
        });
        if let Err(err) = socket.send(Message::Text(command.to_string())).await {
            break err.to_string();
        }
    };
    eprintln!(
        "[codex-plugin] CDP binding bridge closed: target={} reason={}",
        target_label, close_reason
    );
}

fn select_plugin_target(targets: &[CdpTarget]) -> Option<CdpTarget> {
    let page_targets = targets
        .iter()
        .filter(|target| {
            target.target_type == "page" && !target.web_socket_debugger_url.trim().is_empty()
        })
        .collect::<Vec<_>>();

    page_targets
        .iter()
        .find(|target| {
            format!("{} {}", target.title, target.url)
                .to_lowercase()
                .contains("codex")
        })
        .copied()
        .cloned()
        .or_else(|| page_targets.first().copied().cloned())
}

fn plugin_target_label(target: &CdpTarget) -> String {
    let title = target.title.trim();
    let url = target.url.trim();
    if !title.is_empty() && !url.is_empty() {
        format!("{} ({})", title, url)
    } else if !title.is_empty() {
        title.to_string()
    } else if !url.is_empty() {
        url.to_string()
    } else {
        target.id.clone()
    }
}

async fn plugin_bridge_socket_response(raw: &str) -> Value {
    let request = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(err) => {
            return json!({
                "ok": false,
                "error": format!("invalid JSON: {}", err),
            });
        }
    };
    let request_id = request.get("id").cloned();
    let message_type = request
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");

    let result = match message_type {
        "hello" => {
            let location = request
                .get("location")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            eprintln!("[codex-plugin] renderer connected: {}", location);
            Ok(json!({
                "type": "hello",
                "runtimeVersion": CODEXL_PLUGIN_RUNTIME_VERSION,
                "capabilities": [
                    "log",
                    "ping",
                    "renderer-ui",
                    "react-hook"
                ],
            }))
        }
        "log" => {
            let level = request
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("info");
            let message = request.get("message").and_then(Value::as_str).unwrap_or("");
            eprintln!("[codex-plugin][renderer][{}] {}", level, message);
            Ok(json!({ "type": "log-ack" }))
        }
        "ping" | "bridge-heartbeat" => Ok(json!({
            "type": "pong",
            "ts": timestamp_millis(),
        })),
        "plugin:list" => load_renderer_plugins().map(|plugins| {
            json!({
                "type": "plugin-list",
                "plugins": plugins,
            })
        }),
        "session:context-usage" => renderer_session_context_usage(&request),
        "models:list-for-host" => plugin_list_models_for_host_response(&request),
        "transcribe:fetch" => plugin_transcribe_fetch_response(&request).await,
        "storage:get" => renderer_plugin_storage_get(&request),
        "storage:set" => renderer_plugin_storage_set(&request),
        "storage:remove" => renderer_plugin_storage_remove(&request),
        _ => Err(format!(
            "unsupported plugin bridge message type: {}",
            message_type
        )),
    };

    match result {
        Ok(value) => json!({
            "id": request_id,
            "ok": true,
            "value": value,
        }),
        Err(error) => json!({
            "id": request_id,
            "ok": false,
            "error": error,
        }),
    }
}

fn plugin_list_models_for_host_response(request: &Value) -> Result<Value, String> {
    let config = AppConfig::load();
    let Some(profile) = config.provider_profile(&config.active_provider) else {
        return Ok(json!({
            "type": "models-list-for-host",
            "handled": false,
        }));
    };
    let Some(catalog_path) = codex_model_catalog_path_for_profile(&profile) else {
        return Ok(json!({
            "type": "models-list-for-host",
            "handled": false,
        }));
    };
    let content = fs::read_to_string(&catalog_path)
        .map_err(|err| format!("Failed to read model catalog {}: {}", catalog_path, err))?;
    let catalog = serde_json::from_str::<Value>(&content)
        .map_err(|err| format!("Failed to parse model catalog {}: {}", catalog_path, err))?;
    let response = list_models_for_host_from_catalog(&catalog, &profile.model, request)?;
    Ok(json!({
        "type": "models-list-for-host",
        "handled": true,
        "response": response,
    }))
}

fn codex_model_catalog_path_for_profile(profile: &ProviderProfile) -> Option<String> {
    if profile.provider_name.trim() == gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME {
        if let Ok(path) = gateway_config::write_codex_model_catalog(&profile.model) {
            return Some(path);
        }
    }

    let config_path = generated_codex_home(profile).join("config.toml");
    let content = fs::read_to_string(config_path).ok()?;
    top_level_toml_string(&content, "model_catalog_json")
}

fn list_models_for_host_from_catalog(
    catalog: &Value,
    default_model: &str,
    request: &Value,
) -> Result<Value, String> {
    let include_hidden = request
        .get("includeHidden")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let offset = request
        .get("cursor")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .unwrap_or(0) as usize;
    let limit = request
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .filter(|value| *value > 0)
        .unwrap_or(usize::MAX);
    let models = catalog
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "Model catalog missing models array".to_string())?;
    let items = models
        .iter()
        .filter_map(|item| codex_model_catalog_item_for_host(item, default_model))
        .filter(|item| {
            include_hidden
                || item
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .map(|hidden| !hidden)
                    .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    let end = offset.saturating_add(limit).min(items.len());
    let data = if offset < items.len() {
        items[offset..end].to_vec()
    } else {
        Vec::new()
    };
    let next_cursor = if end < items.len() {
        Value::String(end.to_string())
    } else {
        Value::Null
    };

    Ok(json!({
        "data": data,
        "nextCursor": next_cursor,
    }))
}

fn codex_model_catalog_item_for_host(item: &Value, default_model: &str) -> Option<Value> {
    let model = catalog_string_field(item, &["model", "id", "slug", "display_name", "name"])?;
    let display_name = catalog_string_field(item, &["displayName", "display_name", "name"])
        .unwrap_or_else(|| model.clone());
    let description = catalog_string_field(item, &["description"]).unwrap_or_default();
    let hidden = catalog_model_is_hidden(item);
    Some(json!({
        "id": model,
        "model": model,
        "upgrade": catalog_field_or_null(item, &["upgrade"]),
        "upgradeInfo": catalog_field_or_null(item, &["upgradeInfo", "upgrade_info"]),
        "availabilityNux": catalog_field_or_null(item, &["availabilityNux", "availability_nux"]),
        "displayName": display_name,
        "description": description,
        "hidden": hidden,
        "supportedReasoningEfforts": catalog_supported_reasoning_efforts(item),
        "defaultReasoningEffort": catalog_field_or_null(item, &["defaultReasoningEffort", "default_reasoning_level"]),
        "inputModalities": catalog_field_or_array(item, &["inputModalities", "input_modalities"]),
        "supportsPersonality": catalog_field_or_bool(item, &["supportsPersonality", "supports_personality"], false),
        "additionalSpeedTiers": catalog_field_or_array(item, &["additionalSpeedTiers", "additional_speed_tiers"]),
        "serviceTiers": catalog_field_or_array(item, &["serviceTiers", "service_tiers"]),
        "defaultServiceTier": catalog_field_or_null(item, &["defaultServiceTier", "default_service_tier"]),
        "isDefault": !default_model.trim().is_empty() && model == default_model.trim(),
    }))
}

fn catalog_model_is_hidden(item: &Value) -> bool {
    if item.get("hidden").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let visibility = catalog_string_field(item, &["visibility"]).unwrap_or_default();
    !visibility.is_empty() && visibility != "list"
}

fn catalog_supported_reasoning_efforts(item: &Value) -> Value {
    let Some(levels) = item
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .or_else(|| {
            item.get("supported_reasoning_levels")
                .and_then(Value::as_array)
        })
    else {
        return json!([]);
    };

    Value::Array(
        levels
            .iter()
            .filter_map(|level| {
                let effort = catalog_string_field(level, &["reasoningEffort", "effort"])?;
                Some(json!({
                    "reasoningEffort": effort,
                    "description": catalog_string_field(level, &["description"]).unwrap_or_default(),
                }))
            })
            .collect(),
    )
}

fn catalog_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(ToString::to_string)
}

fn catalog_field_or_null(item: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| item.get(*key).cloned())
        .unwrap_or(Value::Null)
}

fn catalog_field_or_array(item: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(Value::as_array).cloned())
        .map(Value::Array)
        .unwrap_or_else(|| json!([]))
}

fn catalog_field_or_bool(item: &Value, keys: &[&str], default_value: bool) -> Value {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(Value::as_bool))
        .map(Value::Bool)
        .unwrap_or(Value::Bool(default_value))
}

fn top_level_toml_string(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return None;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return parse_basic_toml_string(value.trim());
        }
    }
    None
}

fn parse_basic_toml_string(value: &str) -> Option<String> {
    let mut chars = value.trim_start().chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut output = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            output.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }
        if quote == '"' && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            return Some(output);
        }
        output.push(ch);
    }
    None
}

async fn plugin_transcribe_fetch_response(request: &Value) -> Result<Value, String> {
    let request_id = request
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("codexl-plugin-transcribe");
    let message = json!({
        "type": "fetch",
        "requestId": request_id,
        "method": request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("POST"),
        "url": request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("/transcribe"),
        "headers": request
            .get("headers")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "body": request
            .get("body")
            .cloned()
            .unwrap_or_else(|| Value::String(String::new())),
    });
    let config = AppConfig::load();
    let Some(response) =
        super::super::custom_transcribe_fetch_response_for_config(&message, &config, "in-app")
            .await
    else {
        return Ok(json!({
            "type": "transcribe-fetch",
            "handled": false,
        }));
    };
    Ok(json!({
        "type": "transcribe-fetch",
        "handled": true,
        "response": response,
    }))
}

fn plugin_cdp_response_expression(response: &Value) -> String {
    format!(
        "(() => {{ const runtime = window.__codexlPluginRuntime; if (runtime && typeof runtime.acceptCdpResponse === \"function\") {{ runtime.acceptCdpResponse({}); }} }})()",
        response
    )
}

fn codex_plugin_bootstrap_script(bridge_url: &str) -> String {
    let bridge_url_json = serde_json::to_string(bridge_url).unwrap_or_else(|_| "\"\"".to_string());
    CODEXL_PLUGIN_BOOTSTRAP
        .replace("\"__CODEXL_PLUGIN_BRIDGE_URL__\"", &bridge_url_json)
        .replace(
            "__CODEXL_PLUGIN_RUNTIME_VERSION__",
            CODEXL_PLUGIN_RUNTIME_VERSION,
        )
}

fn plugin_bridge_url(http_host: &str, http_port: u16, token: &str) -> String {
    let host = browser_loopback_host(http_host);
    format!(
        "ws://{}:{}{}?token={}",
        host, http_port, CODEXL_PLUGIN_BRIDGE_PATH, token
    )
}

fn browser_loopback_host(http_host: &str) -> String {
    match http_host.trim() {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1".to_string(),
        host if host.contains(':') && !host.starts_with('[') => format!("[{}]", host),
        host => host.to_string(),
    }
}

fn register_plugin_bridge_token() -> String {
    let token = random_token();
    if let Ok(mut tokens) = plugin_bridge_tokens().lock() {
        tokens.insert(token.clone());
    }
    token
}

fn plugin_bridge_tokens() -> &'static StdMutex<HashSet<String>> {
    CODEXL_PLUGIN_BRIDGE_TOKENS.get_or_init(|| StdMutex::new(HashSet::new()))
}

fn query_token(query: &str) -> Option<&str> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key == "token" && !value.is_empty() {
            Some(value)
        } else {
            None
        }
    })
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push_str(&format!("{:02x}", byte));
    }
    token
}

fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn load_renderer_plugins() -> Result<Vec<RendererPluginPayload>, String> {
    let root = renderer_plugin_root();
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut plugins = Vec::new();
    let entries = fs::read_dir(&root).map_err(|err| {
        format!(
            "failed to read renderer plugin dir {}: {}",
            root.display(),
            err
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let manifest_path = plugin_dir.join("plugin.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest_content = fs::read_to_string(&manifest_path)
            .map_err(|err| format!("failed to read {}: {}", manifest_path.display(), err))?;
        let manifest = serde_json::from_str::<RendererPluginManifest>(&manifest_content)
            .map_err(|err| format!("failed to parse {}: {}", manifest_path.display(), err))?;
        if !manifest.enabled {
            continue;
        }

        let fallback_id = plugin_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("plugin")
            .to_string();
        let id = if manifest.id.trim().is_empty() {
            fallback_id
        } else {
            manifest.id.trim().to_string()
        };
        validate_plugin_id(&id)?;

        let entry_path = safe_relative_path(&manifest.entry).ok_or_else(|| {
            format!(
                "renderer plugin {} entry is not safe: {}",
                id, manifest.entry
            )
        })?;
        let entry_file = plugin_dir.join(&entry_path);
        let metadata = fs::metadata(&entry_file)
            .map_err(|err| format!("failed to inspect {}: {}", entry_file.display(), err))?;
        if metadata.len() > CODEXL_RENDERER_PLUGIN_ENTRY_LIMIT_BYTES {
            return Err(format!(
                "renderer plugin {} entry exceeds {} bytes",
                id, CODEXL_RENDERER_PLUGIN_ENTRY_LIMIT_BYTES
            ));
        }
        let source = fs::read_to_string(&entry_file)
            .map_err(|err| format!("failed to read {}: {}", entry_file.display(), err))?;
        let name = if manifest.name.trim().is_empty() {
            id.clone()
        } else {
            manifest.name.trim().to_string()
        };
        plugins.push(RendererPluginPayload {
            entry: manifest.entry,
            id: id.clone(),
            name,
            source,
            source_url: format!("codexl-renderer-plugin://{}/{}", id, entry_path.display()),
            version: manifest.version,
        });
    }
    plugins.sort_by(|first, second| first.id.cmp(&second.id));
    Ok(plugins)
}

fn renderer_plugin_storage_get(request: &Value) -> Result<Value, String> {
    let plugin_id = request_plugin_id(request)?;
    let key = request_storage_key(request)?;
    let data = read_renderer_plugin_storage(&plugin_id)?;
    Ok(json!({
        "type": "storage-value",
        "value": data.get(&key).cloned().unwrap_or(Value::Null),
    }))
}

pub(crate) fn renderer_core_plugin_bool_setting(key: &str) -> bool {
    read_renderer_plugin_storage(CODEXL_CORE_PLUGIN_ID)
        .ok()
        .and_then(|data| data.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn renderer_session_context_usage(request: &Value) -> Result<Value, String> {
    let thread_id = request
        .get("threadId")
        .or_else(|| request.get("conversationId"))
        .or_else(|| request.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let requested_path = request
        .get("path")
        .or_else(|| request.get("rolloutPath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if thread_id.is_none() && requested_path.is_none() {
        return Err("session context usage requires threadId or path".to_string());
    }

    let roots = session_roots();
    if roots.is_empty() {
        return Err("no Codex session roots were found".to_string());
    }

    let path = if let Some(path) = requested_path.as_deref() {
        validate_requested_session_read_path(path, &roots)?
    } else {
        find_session_file(&roots, thread_id.as_deref().unwrap_or(""))
            .ok_or_else(|| "session file was not found".to_string())?
    };

    let resolved_thread_id = thread_id
        .or_else(|| session_file_thread_id(&path))
        .unwrap_or_default();
    Ok(json!({
        "type": "session-context-usage",
        "threadId": resolved_thread_id,
        "path": path.to_string_lossy().to_string(),
        "tokenUsage": latest_session_context_usage(&path).unwrap_or(Value::Null),
    }))
}

fn session_roots() -> Vec<PathBuf> {
    let config = AppConfig::load();
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    push_session_root(&mut roots, &mut seen, config.codex_home.clone());
    push_session_root(&mut roots, &mut seen, crate::config::default_codex_home());
    if let Ok(value) = std::env::var("CODEX_HOME") {
        push_session_root(&mut roots, &mut seen, value);
    }
    if let Ok(value) = std::env::var("CODEXL_CODEX_HOME") {
        push_session_root(&mut roots, &mut seen, value);
    }

    for profile in &config.codex_home_profiles {
        push_session_root(&mut roots, &mut seen, profile.path.clone());
    }
    for profile in &config.provider_profiles {
        push_session_root(&mut roots, &mut seen, profile.codex_home.clone());
        push_session_root(
            &mut roots,
            &mut seen,
            crate::config::generated_codex_home(profile)
                .to_string_lossy()
                .to_string(),
        );
    }
    for root in discovered_session_roots() {
        push_session_root(&mut roots, &mut seen, root.to_string_lossy().to_string());
    }

    roots
}

fn push_session_root(roots: &mut Vec<PathBuf>, seen: &mut HashSet<String>, value: String) {
    let normalized = crate::config::normalize_home_path(&value);
    if normalized.trim().is_empty() {
        return;
    }
    let path = PathBuf::from(normalized);
    if !path.join("sessions").is_dir() {
        return;
    }
    let key = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone())
        .to_string_lossy()
        .to_string();
    if seen.insert(key) {
        roots.push(path);
    }
}

fn discovered_session_roots() -> Vec<PathBuf> {
    let root = crate::extensions::builtins::codexl_home_dir().join("codex-homes");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("sessions").is_dir())
        .collect()
}

fn validate_requested_session_read_path(value: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    validate_requested_session_path(value, roots, "read")
}

fn validate_requested_session_path(
    value: &str,
    roots: &[PathBuf],
    action: &str,
) -> Result<PathBuf, String> {
    let path = PathBuf::from(crate::config::normalize_home_path(value));
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|err| format!("failed to inspect session {}: {}", path.display(), err))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing to {} a symlinked session file", action));
    }
    if !metadata.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return Err(format!("session {} target must be a .jsonl file", action));
    }
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("failed to resolve session {}: {}", path.display(), err))?;
    if roots
        .iter()
        .any(|root| path_is_under(&canonical, &root.join("sessions")))
    {
        Ok(canonical)
    } else {
        Err(format!(
            "session {} target is outside known Codex session directories",
            action
        ))
    }
}

fn find_session_file(roots: &[PathBuf], thread_id: &str) -> Option<PathBuf> {
    if thread_id.trim().is_empty() {
        return None;
    }
    for root in roots {
        let mut files = Vec::new();
        collect_session_jsonl_files(&root.join("sessions"), &mut files);
        for path in files {
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            if session_file_thread_id(&path)
                .as_deref()
                .is_some_and(|session_thread_id| {
                    session_thread_id_matches(session_thread_id, thread_id)
                })
            {
                return path.canonicalize().ok().or(Some(path));
            }
        }
    }
    None
}

fn session_thread_id_matches(session_thread_id: &str, requested_thread_id: &str) -> bool {
    let session_thread_id = session_thread_id.trim();
    let requested_thread_id = requested_thread_id.trim();
    if session_thread_id.is_empty() || requested_thread_id.is_empty() {
        return false;
    }
    session_thread_id == requested_thread_id
        || requested_thread_id
            .strip_prefix("local:")
            .is_some_and(|id| id == session_thread_id)
        || session_thread_id
            .strip_prefix("local:")
            .is_some_and(|id| id == requested_thread_id)
}

fn collect_session_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn session_file_thread_id(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let meta: Value = serde_json::from_str(line.trim_end()).ok()?;
    if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    meta.get("payload")
        .and_then(|payload| payload.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn latest_session_context_usage(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(line.trim_end()) else {
            continue;
        };
        if let Some(info) = session_event_token_count_info(&event) {
            latest = Some(info.clone());
        }
    }
    latest
}

fn session_event_token_count_info(event: &Value) -> Option<&Value> {
    if event.get("type").and_then(Value::as_str) == Some("token_count") {
        return event.get("info").filter(|info| info.is_object());
    }
    let payload = event.get("payload")?;
    if payload.get("type").and_then(Value::as_str) == Some("token_count") {
        payload.get("info").filter(|info| info.is_object())
    } else {
        None
    }
}

fn path_is_under(path: &Path, root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(root).is_ok()
}

fn renderer_plugin_storage_set(request: &Value) -> Result<Value, String> {
    let plugin_id = request_plugin_id(request)?;
    let key = request_storage_key(request)?;
    let value = request.get("value").cloned().unwrap_or(Value::Null);
    let mut data = read_renderer_plugin_storage(&plugin_id)?;
    data.insert(key, value);
    write_renderer_plugin_storage(&plugin_id, &data)?;
    Ok(json!({ "type": "storage-set" }))
}

fn renderer_plugin_storage_remove(request: &Value) -> Result<Value, String> {
    let plugin_id = request_plugin_id(request)?;
    let key = request_storage_key(request)?;
    let mut data = read_renderer_plugin_storage(&plugin_id)?;
    data.remove(&key);
    write_renderer_plugin_storage(&plugin_id, &data)?;
    Ok(json!({ "type": "storage-removed" }))
}

fn request_plugin_id(request: &Value) -> Result<String, String> {
    let plugin_id = request
        .get("pluginId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "pluginId is required".to_string())?
        .to_string();
    validate_plugin_id(&plugin_id)?;
    Ok(plugin_id)
}

fn request_storage_key(request: &Value) -> Result<String, String> {
    let key = request
        .get("key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "storage key is required".to_string())?;
    if key.len() > 256 {
        return Err("storage key is too long".to_string());
    }
    Ok(key.to_string())
}

fn read_renderer_plugin_storage(plugin_id: &str) -> Result<BTreeMap<String, Value>, String> {
    let path = renderer_plugin_storage_path(plugin_id)?;
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    serde_json::from_str::<BTreeMap<String, Value>>(&content)
        .map_err(|err| format!("failed to parse {}: {}", path.display(), err))
}

fn write_renderer_plugin_storage(
    plugin_id: &str,
    data: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let path = renderer_plugin_storage_path(plugin_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
    }
    let content = serde_json::to_string_pretty(data).map_err(|err| err.to_string())?;
    fs::write(&path, content).map_err(|err| format!("failed to write {}: {}", path.display(), err))
}

fn renderer_plugin_storage_path(plugin_id: &str) -> Result<PathBuf, String> {
    validate_plugin_id(plugin_id)?;
    Ok(crate::extensions::builtins::codexl_home_dir()
        .join("renderer-plugin-data")
        .join(format!("{}.json", plugin_id)))
}

fn renderer_plugin_root() -> PathBuf {
    std::env::var("CODEXL_RENDERER_PLUGIN_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::extensions::builtins::codexl_home_dir().join("renderer-plugins"))
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), String> {
    let valid = !plugin_id.is_empty()
        && plugin_id.len() <= 128
        && plugin_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@'));
    if valid {
        Ok(())
    } else {
        Err(format!("invalid renderer plugin id: {}", plugin_id))
    }
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            _ => return None,
        }
    }
    Some(safe)
}

const CODEXL_PLUGIN_BOOTSTRAP: &str = r#"(() => {
  const RUNTIME_VERSION = "__CODEXL_PLUGIN_RUNTIME_VERSION__";
  const RUNTIME_BUILD = "settings-context-18";
  const BRIDGE_URL = "__CODEXL_PLUGIN_BRIDGE_URL__";
  const ROOT_ID = "codexl-plugin-runtime-root";
  const CORE_PLUGIN_ID = "codexl.core";
  const SHOW_CONTEXT_INDICATORS_KEY = "showContextIndicators";
  const SHOW_ALL_SESSIONS_KEY = "showAllSessions";
  const SETTINGS_STYLE_ID = "codexl-plugin-global-style";
  const SETTINGS_NAV_ATTR = "data-codexl-settings-nav";
  const SETTINGS_PANEL_ATTR = "data-codexl-settings-panel";
  const CONTEXT_INDICATOR_ID = "codexl-context-indicator";
  const CONTEXT_TOOLTIP_ID = "codexl-context-indicator-tooltip";
  const SETTINGS_REFRESH_BURST_DELAYS_MS = [0, 120, 360, 900, 1800];
  const SETTINGS_INTERACTION_SCAN_WINDOW_MS = 4500;
  const CONTEXT_INDICATOR_BURST_DELAYS_MS = [0, 150, 500, 1200];
  const existing = window.__codexlPluginRuntime;
  if (existing && existing.version === RUNTIME_VERSION && existing.build === RUNTIME_BUILD) {
    existing.reconfigure?.(BRIDGE_URL);
    existing.mount?.();
    existing.connect?.();
    return { ok: true, alreadyInstalled: true, build: RUNTIME_BUILD, version: RUNTIME_VERSION };
  }
  if (existing) {
    try {
      existing.teardown?.();
    } catch {}
    try {
      document.getElementById(ROOT_ID)?.remove();
    } catch {}
  }

  const runtime = {
    activeThreadId: null,
    bridgeUrl: BRIDGE_URL,
    build: RUNTIME_BUILD,
    cleanup: [],
    codexlSettings: {
      showAllSessions: false,
      showContextIndicators: false,
    },
    contextIndicatorCleanup: [],
    contextUsageByThread: new Map(),
    contextUsageSessionRefreshByThread: new Map(),
    latestContextUsage: null,
    lastContextLocationKey: "",
    loadedPlugins: new Map(),
    pending: new Map(),
    panels: new Map(),
    renderers: new Map(),
    socket: null,
    status: "booting",
    statusDetail: "",
    nextId: 1,
    settingsInteractionUntil: 0,
    settingsDiagnostics: [],
    settingsRefreshTimers: [],
    version: RUNTIME_VERSION,
  };
  window.__codexlPluginRuntime = runtime;
  installClipboardFallback();

  function log(level, message, detail) {
    try {
      console[level === "error" ? "error" : "info"]("[codex-plugin]", message, detail || "");
    } catch {}
    send({ type: "log", level, message, detail });
  }

  function installClipboardFallback() {
    if (window.isSecureContext) {
      return;
    }

    const marker = "__codexlClipboardFallbackInstalled";
    if (window[marker]) {
      return;
    }
    window[marker] = true;

    const shim = {
      async writeText(text) {
        await fallbackCopyText(String(text ?? ""));
      },
    };

    try {
      const existing = navigator.clipboard;
      if (existing && typeof existing.writeText === "function") {
        existing.writeText = shim.writeText.bind(shim);
        return;
      }
    } catch {}

    try {
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: shim,
      });
      return;
    } catch {}

    try {
      Object.defineProperty(Navigator.prototype, "clipboard", {
        configurable: true,
        get() {
          return shim;
        },
      });
    } catch {}
  }

  async function fallbackCopyText(text) {
    if (!text) {
      return;
    }
    if (!document.body) {
      return;
    }

    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.top = "-9999px";
    textarea.style.left = "-9999px";
    textarea.style.opacity = "0";

    const selection = document.getSelection?.();
    const previousRange = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    const activeElement = document.activeElement;

    document.body.appendChild(textarea);
    textarea.focus({ preventScroll: true });
    textarea.select();
    textarea.setSelectionRange(0, textarea.value.length);

    try {
      document.execCommand("copy");
    } catch {}

    textarea.remove();

    try {
      if (activeElement && typeof activeElement.focus === "function") {
        activeElement.focus({ preventScroll: true });
      }
    } catch {}

    try {
      if (selection && previousRange) {
        selection.removeAllRanges();
        selection.addRange(previousRange);
      }
    } catch {}
  }

  function installReactHook() {
    const originalHook = window.__REACT_DEVTOOLS_GLOBAL_HOOK__;
    if (originalHook && originalHook.__codexlPluginPatched) {
      runtime.hook = originalHook;
      rememberExistingRenderers(originalHook);
      return originalHook;
    }

    const hook = originalHook || {};
    const originalInject = typeof hook.inject === "function" ? hook.inject.bind(hook) : null;
    const originalOnCommitFiberRoot =
      typeof hook.onCommitFiberRoot === "function" ? hook.onCommitFiberRoot.bind(hook) : null;
    const originalOnCommitFiberUnmount =
      typeof hook.onCommitFiberUnmount === "function" ? hook.onCommitFiberUnmount.bind(hook) : null;
    let fallbackRendererId = 1;

    hook.supportsFiber = true;
    hook.renderers = hook.renderers || new Map();
    rememberExistingRenderers(hook);
    hook.inject = function inject(renderer) {
      let id;
      if (originalInject) {
        try {
          id = originalInject(renderer);
        } catch (error) {
          log("error", "React hook inject failed", String(error));
        }
      }
      if (id == null) {
        id = fallbackRendererId++;
      }
      try {
        hook.renderers.set(id, renderer);
      } catch {}
      runtime.renderers.set(id, renderer);
      updateUi();
      return id;
    };
    hook.onCommitFiberRoot = function onCommitFiberRoot(id, root, priorityLevel, didError) {
      runtime.lastFiberRoot = root;
      updateUi();
      scheduleGatewayModelQuerySelectorRepair();
      if (originalOnCommitFiberRoot) {
        return originalOnCommitFiberRoot(id, root, priorityLevel, didError);
      }
    };
    hook.onCommitFiberUnmount = function onCommitFiberUnmount(id, fiber) {
      if (originalOnCommitFiberUnmount) {
        return originalOnCommitFiberUnmount(id, fiber);
      }
    };

    try {
      Object.defineProperty(hook, "__codexlPluginPatched", {
        configurable: true,
        value: true,
      });
      Object.defineProperty(window, "__REACT_DEVTOOLS_GLOBAL_HOOK__", {
        configurable: true,
        value: hook,
        writable: true,
      });
    } catch {
      window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = hook;
    }

    runtime.hook = hook;
    return hook;
  }

  function rememberExistingRenderers(hook) {
    try {
      if (hook && hook.renderers && typeof hook.renderers.forEach === "function") {
        hook.renderers.forEach((renderer, id) => runtime.renderers.set(id, renderer));
      }
    } catch {}
  }

  function getFiber(node) {
    if (!node || typeof node !== "object") {
      return null;
    }
    for (const key of Object.keys(node)) {
      if (
        key.startsWith("__reactFiber$") ||
        key.startsWith("__reactInternalInstance$")
      ) {
        return node[key] || null;
      }
    }
    return null;
  }

  function fiberName(fiber) {
    const type = fiber && (fiber.elementType || fiber.type);
    if (!type) {
      return "";
    }
    if (typeof type === "string") {
      return type;
    }
    return type.displayName || type.name || "";
  }

  function findOwnerByName(start, name) {
    let fiber = start && start.nodeType ? getFiber(start) : start;
    while (fiber) {
      if (fiberName(fiber) === name) {
        return fiber;
      }
      fiber = fiber.return || null;
    }
    return null;
  }

  function request(type, payload = {}) {
    const id = String(runtime.nextId++);
    const message = { ...payload, id, type };
    return new Promise((resolve, reject) => {
      runtime.pending.set(id, { reject, resolve });
      send(message);
      window.setTimeout(() => {
        if (!runtime.pending.has(id)) {
          return;
        }
        runtime.pending.delete(id);
        reject(new Error(`Plugin bridge request timed out: ${type}`));
      }, 30000);
    });
  }

  function desktopApiFetchEndpoint(message) {
    if (message?.type !== "fetch" || typeof message.url !== "string") {
      return "";
    }
    try {
      const url = new URL(message.url, window.location.href);
      if (url.protocol !== "vscode:" || url.hostname !== "codex") {
        return "";
      }
      return url.pathname.replace(/^\/+/, "");
    } catch {
      const prefix = "vscode://codex/";
      return message.url.startsWith(prefix) ? message.url.slice(prefix.length).split(/[?#]/, 1)[0] : "";
    }
  }

  function desktopApiFetchParams(message) {
    let body = message?.body;
    if (typeof body === "string") {
      if (!body.trim()) {
        body = {};
      } else {
        try {
          body = JSON.parse(body);
        } catch {
          body = {};
        }
      }
    }
    if (!body || typeof body !== "object") {
      return {};
    }
    return body.params && typeof body.params === "object" ? body.params : body;
  }

  function dispatchDesktopApiFetchMessage(message, response) {
    if (!message?.requestId) {
      return;
    }
    window.dispatchEvent(
      new MessageEvent("message", {
        data: response,
        origin: window.location.origin,
        source: window,
      }),
    );
  }

  function dispatchDesktopApiFetchSuccess(message, body) {
    dispatchDesktopApiFetchMessage(message, {
      type: "fetch-response",
      requestId: message.requestId,
      responseType: "success",
      status: 200,
      headers: { "content-type": "application/json" },
      bodyJsonString: JSON.stringify(body ?? null),
    });
  }

  function dispatchDesktopApiFetchError(message, status, error) {
    dispatchDesktopApiFetchMessage(message, {
      type: "fetch-response",
      requestId: message.requestId,
      responseType: "error",
      status,
      error: String(error || "CodexL desktop API request failed"),
    });
  }

  function isListModelsForHostDesktopApiRequest(message) {
    if (String(message?.method || "POST").toUpperCase() !== "POST") {
      return false;
    }
    return desktopApiFetchEndpoint(message) === "list-models-for-host";
  }

  function isIdeContextDesktopApiRequest(message) {
    if (String(message?.method || "POST").toUpperCase() !== "POST") {
      return false;
    }
    return desktopApiFetchEndpoint(message) === "ide-context";
  }

  async function maybeHandleListModelsForHostDesktopApiRequest(message) {
    if (!isListModelsForHostDesktopApiRequest(message) || !message?.requestId) {
      return false;
    }
    const result = await request("models:list-for-host", {
      ...desktopApiFetchParams(message),
      method: message.method || "POST",
      requestId: message.requestId,
      url: message.url,
    });
    if (!result?.handled) {
      return false;
    }
    dispatchDesktopApiFetchSuccess(message, result.response);
    return true;
  }

  function maybeHandleIdeContextDesktopApiRequest(message) {
    if (!isIdeContextDesktopApiRequest(message) || !message?.requestId) {
      return false;
    }
    dispatchDesktopApiFetchSuccess(message, { ideContext: null });
    return true;
  }

  function installDesktopApiHostModelListEventInterceptor() {
    if (runtime.desktopApiHostModelListEventInterceptorInstalled) {
      return;
    }
    runtime.desktopApiHostModelListEventInterceptorInstalled = true;
    const pendingRequestIds = new Set();
    runtime.desktopApiHostModelListPendingRequestIds = pendingRequestIds;
    const handleRequest = async (message) => {
      if (!message?.requestId) {
        return;
      }
      const handlesListModels = isListModelsForHostDesktopApiRequest(message);
      const handlesIdeContext = isIdeContextDesktopApiRequest(message);
      if (!handlesListModels && !handlesIdeContext) {
        return;
      }
      pendingRequestIds.add(message.requestId);
      try {
        if (handlesIdeContext) {
          maybeHandleIdeContextDesktopApiRequest(message);
        } else {
          await maybeHandleListModelsForHostDesktopApiRequest(message);
        }
      } catch (error) {
        log("error", "CodexL desktop API model list failed", String(error));
        dispatchDesktopApiFetchError(message, 500, error?.message || error);
      } finally {
        window.setTimeout(() => pendingRequestIds.delete(message.requestId), 1000);
      }
    };
    const onMessageFromView = (event) => {
      handleRequest(event?.detail);
    };
    const onNativeFetchResponse = (event) => {
      const data = event?.data;
      if (
        data?.type === "fetch-response" &&
        data?.responseType === "error" &&
        data?.status === 432 &&
        pendingRequestIds.has(data.requestId)
      ) {
        event.stopImmediatePropagation();
        event.preventDefault?.();
      }
    };
    window.addEventListener("codex-message-from-view", onMessageFromView, true);
    window.addEventListener("message", onNativeFetchResponse, true);
    runtime.cleanup.push(() => {
      window.removeEventListener("codex-message-from-view", onMessageFromView, true);
      window.removeEventListener("message", onNativeFetchResponse, true);
    });
    log("info", "CodexL desktop API model list event interceptor installed");
  }

  function maybeQueryClientFromCandidate(candidate) {
    if (!candidate || typeof candidate !== "object") {
      return null;
    }
    if (candidate.queryClient?.getQueryCache) {
      return candidate.queryClient;
    }
    if (candidate.getQueryCache && candidate.getQueryData) {
      return candidate;
    }
    return null;
  }

  function findQueryClientFromFiber(fiber) {
    let owner = fiber;
    for (let depth = 0; owner && depth < 100; depth += 1, owner = owner.return || null) {
      let hook = owner.memoizedState;
      for (let index = 0; hook && index < 160; index += 1, hook = hook.next || null) {
        const value = hook.memoizedState;
        const queryClient =
          maybeQueryClientFromCandidate(value) ||
          maybeQueryClientFromCandidate(value?.current) ||
          maybeQueryClientFromCandidate(value?.value) ||
          maybeQueryClientFromCandidate(value?.state) ||
          maybeQueryClientFromCandidate(value?.data);
        if (queryClient) {
          return queryClient;
        }
      }
    }
    return null;
  }

  function findComposerModelQueryClient() {
    const trigger = document.querySelector("[data-codex-intelligence-trigger]");
    return findQueryClientFromFiber(getFiber(trigger));
  }

  function modelId(value) {
    return String(value?.model || value?.id || "").trim();
  }

  function looksLikeGatewayModelList(models) {
    return models.some((model) => {
      const id = modelId(model);
      return id.includes("/") || id.includes(":") || id.includes(" ");
    });
  }

  function gatewayModelSelectResult(rawData) {
    const rawModels = Array.isArray(rawData?.data) ? rawData.data : [];
    const models = rawModels.filter((model) => model && !model.hidden && modelId(model));
    if (!models.length || !looksLikeGatewayModelList(models)) {
      return null;
    }
    return {
      defaultModel: models.find((model) => model.isDefault) || models[0] || null,
      models,
    };
  }

  function selectedModelsLength(value) {
    return Array.isArray(value?.models) ? value.models.length : null;
  }

  function repairGatewayModelObserverSelect(query, observer, replacement) {
    if (!observer?.options || typeof observer.setOptions !== "function") {
      return false;
    }
    let selected = null;
    try {
      selected = observer.options.select?.(query.state.data);
    } catch {}
    if (selectedModelsLength(selected) !== 0) {
      return false;
    }
    const patchedSelect = () => replacement;
    Object.defineProperty(patchedSelect, "__codexlGatewayModelSelectPatch", {
      configurable: true,
      value: true,
    });
    observer.setOptions({
      ...observer.options,
      select: patchedSelect,
    });
    try {
      observer.updateResult?.();
    } catch {}
    return true;
  }

  function repairGatewayModelQuerySelectors() {
    const queryClient = findComposerModelQueryClient();
    if (!queryClient?.getQueryCache) {
      return 0;
    }
    let patched = 0;
    try {
      for (const query of queryClient.getQueryCache().getAll()) {
        if (!JSON.stringify(query.queryKey || []).includes("models")) {
          continue;
        }
        const replacement = gatewayModelSelectResult(query.state?.data);
        if (!replacement) {
          continue;
        }
        for (const observer of query.observers || []) {
          if (repairGatewayModelObserverSelect(query, observer, replacement)) {
            patched += 1;
          }
        }
        if (patched > 0) {
          try {
            query.setData?.(query.state.data);
          } catch {}
        }
      }
    } catch (error) {
      runtime.gatewayModelQuerySelectorRepairError = String(error);
    }
    if (patched > 0 && !runtime.gatewayModelQuerySelectorRepairLogged) {
      runtime.gatewayModelQuerySelectorRepairLogged = true;
      log("info", "CodexL gateway model query selector repaired", String(patched));
    }
    return patched;
  }

  function scheduleGatewayModelQuerySelectorRepair() {
    if (runtime.gatewayModelQuerySelectorRepairTimer) {
      return;
    }
    runtime.gatewayModelQuerySelectorRepairTimer = window.setTimeout(() => {
      runtime.gatewayModelQuerySelectorRepairTimer = null;
      repairGatewayModelQuerySelectors();
    }, 50);
  }

  function installGatewayModelQuerySelectorRepair() {
    if (runtime.gatewayModelQuerySelectorRepairInstalled) {
      return;
    }
    runtime.gatewayModelQuerySelectorRepairInstalled = true;
    const burstDelays = [0, 120, 360, 900, 1800, 3600, 7200];
    for (const delay of burstDelays) {
      const timer = window.setTimeout(repairGatewayModelQuerySelectors, delay);
      runtime.cleanup.push(() => window.clearTimeout(timer));
    }
    const interval = window.setInterval(repairGatewayModelQuerySelectors, 1500);
    runtime.cleanup.push(() => window.clearInterval(interval));
  }

  function isTranscribeFetchUrl(value) {
    if (typeof value !== "string" || !value) {
      return false;
    }
    try {
      const url = new URL(value, window.location.href);
      return url.pathname === "/transcribe";
    } catch {
      return value.trim() === "/transcribe";
    }
  }

  function headersToObject(headers) {
    const output = {};
    try {
      for (const [key, value] of headers.entries()) {
        output[key] = value;
      }
    } catch {}
    return output;
  }

  function headerValue(headers, name) {
    const target = name.toLowerCase();
    for (const [key, value] of Object.entries(headers || {})) {
      if (String(key).toLowerCase() === target) {
        return String(value);
      }
    }
    return "";
  }

  function setHeaderValue(headers, name, value) {
    const target = name.toLowerCase();
    for (const key of Object.keys(headers || {})) {
      if (String(key).toLowerCase() === target) {
        headers[key] = value;
        return;
      }
    }
    headers[name] = value;
  }

  function bytesToBase64(bytes) {
    let binary = "";
    const chunkSize = 0x8000;
    for (let index = 0; index < bytes.length; index += chunkSize) {
      binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
    }
    return btoa(binary);
  }

  function responseHeadersFromBridge(response) {
    const headers = new Headers();
    const source = response?.headers && typeof response.headers === "object" ? response.headers : {};
    for (const [key, value] of Object.entries(source)) {
      if (typeof value === "string") {
        try {
          headers.set(key, value);
        } catch {}
      }
    }
    return headers;
  }

  function responseBodyFromBridge(response) {
    if (typeof response?.bodyJsonString === "string") {
      return response.bodyJsonString;
    }
    if (typeof response?.body === "string") {
      return response.body;
    }
    if (response?.body != null) {
      try {
        return JSON.stringify(response.body);
      } catch {}
    }
    if (response?.error) {
      return JSON.stringify({ error: String(response.error) });
    }
    return "";
  }

  function fetchResponseFromBridge(response) {
    const status = Number(response?.status) || 500;
    if (response?.responseType === "error") {
      return new Response(JSON.stringify({ error: response.error || "Unable to transcribe audio" }), {
        headers: { "content-type": "application/json" },
        status,
      });
    }
    const headers = responseHeadersFromBridge(response);
    if (!headers.has("content-type")) {
      headers.set("content-type", "application/json");
    }
    return new Response(responseBodyFromBridge(response), { headers, status });
  }

  function installTranscribeFetchInterceptor() {
    if (runtime.transcribeFetchInstalled || typeof window.fetch !== "function") {
      return;
    }
    runtime.transcribeFetchInstalled = true;
    const originalFetch = window.fetch.bind(window);
    runtime.originalFetch = originalFetch;
    window.fetch = async function codexlFetch(input, init) {
      let fetchRequest;
      try {
        fetchRequest = new Request(input, init);
      } catch {
        return originalFetch(input, init);
      }
      if (
        String(fetchRequest.method || "GET").toUpperCase() !== "POST" ||
        !isTranscribeFetchUrl(fetchRequest.url)
      ) {
        return originalFetch(input, init);
      }
      try {
        const headers = headersToObject(fetchRequest.headers);
        let body = "";
        if (headerValue(headers, "x-codex-base64") === "1") {
          body = await fetchRequest.clone().text();
        } else {
          const bytes = new Uint8Array(await fetchRequest.clone().arrayBuffer());
          body = bytesToBase64(bytes);
          setHeaderValue(headers, "X-Codex-Base64", "1");
        }
        const result = await request("transcribe:fetch", {
          body,
          headers,
          method: fetchRequest.method,
          requestId: `codexl-in-app-transcribe-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          url: fetchRequest.url,
        });
        if (!result?.handled) {
          return originalFetch(input, init);
        }
        return fetchResponseFromBridge(result.response);
      } catch (error) {
        log("error", "CodexL transcribe fetch failed", String(error));
        return originalFetch(input, init);
      }
    };
    runtime.cleanup.push(() => {
      if (window.fetch === runtime.originalFetch || window.fetch?.name === "codexlFetch") {
        window.fetch = originalFetch;
      }
    });
  }

  function installDesktopApiHostModelListInterceptor() {
    if (runtime.desktopApiHostModelListInstallStarted) {
      return;
    }
    runtime.desktopApiHostModelListInstallStarted = true;
    installDesktopApiHostModelListEventInterceptor();
    let attempts = 0;
    const patchBridge = () => {
      const bridge = window.electronBridge;
      if (!bridge || typeof bridge.sendMessageFromView !== "function") {
        return false;
      }
      const descriptor = Object.getOwnPropertyDescriptor(bridge, "sendMessageFromView");
      if (descriptor?.get?.__codexlHostModelListDescriptor) {
        runtime.desktopApiHostModelListPatched = true;
        return true;
      }
      if (bridge.sendMessageFromView.__codexlHostModelListPatch) {
        runtime.desktopApiHostModelListPatched = true;
        return true;
      }
      let originalSendMessageFromView = bridge.sendMessageFromView;
      let exposedSendMessageFromView = null;
      const expose = (value) => {
        originalSendMessageFromView = value;
        if (typeof value !== "function") {
          exposedSendMessageFromView = value;
          return;
        }
        if (value.__codexlHostModelListPatch) {
          exposedSendMessageFromView = value;
          return;
        }
        const patchedSendMessageFromView = async function codexlSendMessageFromView(message) {
          if (isListModelsForHostDesktopApiRequest(message)) {
            try {
              if (await maybeHandleListModelsForHostDesktopApiRequest(message)) {
                return;
              }
            } catch (error) {
              log("error", "CodexL desktop API model list failed", String(error));
              dispatchDesktopApiFetchError(message, 500, error?.message || error);
              return;
            }
          }
          return value.call(this, message);
        };
        Object.defineProperty(patchedSendMessageFromView, "__codexlHostModelListPatch", {
          configurable: true,
          value: true,
        });
        exposedSendMessageFromView = patchedSendMessageFromView;
      };
      expose(originalSendMessageFromView);
      const getter = function codexlHostModelListSendMessageGetter() {
        return exposedSendMessageFromView;
      };
      Object.defineProperty(getter, "__codexlHostModelListDescriptor", {
        configurable: true,
        value: true,
      });
      Object.defineProperty(bridge, "sendMessageFromView", {
        configurable: true,
        get: getter,
        set: expose,
      });
      runtime.cleanup.push(() => {
        const currentDescriptor = Object.getOwnPropertyDescriptor(bridge, "sendMessageFromView");
        if (currentDescriptor?.get?.__codexlHostModelListDescriptor) {
          Object.defineProperty(bridge, "sendMessageFromView", {
            configurable: true,
            value: originalSendMessageFromView,
            writable: true,
          });
        }
      });
      runtime.desktopApiHostModelListPatched = true;
      if (!runtime.desktopApiHostModelListPatchedLogged) {
        runtime.desktopApiHostModelListPatchedLogged = true;
        log("info", "CodexL desktop API model list interceptor installed");
      }
      return true;
    };
    const tryInstall = () => {
      attempts += 1;
      try {
        patchBridge();
      } catch (error) {
        runtime.desktopApiHostModelListPatchError = String(error);
      }
      if (attempts < 80) {
        runtime.desktopApiHostModelListInstallTimer = window.setTimeout(tryInstall, 250);
      } else if (!window.electronBridge?.sendMessageFromView?.__codexlHostModelListPatch) {
        log("info", "CodexL desktop API model list bridge patch unavailable; event interceptor active");
      }
    };
    runtime.cleanup.push(() => {
      if (runtime.desktopApiHostModelListInstallTimer) {
        window.clearTimeout(runtime.desktopApiHostModelListInstallTimer);
      }
    });
    tryInstall();
  }

  function isTranscribeDesktopApiRequest(method, url) {
    if (String(method || "GET").toUpperCase() !== "POST") {
      return false;
    }
    return isTranscribeFetchUrl(String(url || ""));
  }

  function desktopApiModuleUrls() {
    const urls = [];
    const remember = (value) => {
      if (
        typeof value === "string" &&
        value.includes("/assets/setting-storage-") &&
        value.endsWith(".js") &&
        !urls.includes(value)
      ) {
        urls.push(value);
      }
    };
    try {
      for (const link of Array.from(document.querySelectorAll("link[href]"))) {
        remember(link.href);
      }
    } catch {}
    try {
      for (const entry of performance.getEntriesByType("resource")) {
        remember(entry.name);
      }
    } catch {}
    return urls;
  }

  function desktopApiClientClassFromModule(module) {
    for (const value of Object.values(module || {})) {
      if (!value || typeof value.getInstance !== "function") {
        continue;
      }
      let instance;
      try {
        instance = value.getInstance();
      } catch {
        continue;
      }
      if (
        instance &&
        typeof instance.post === "function" &&
        typeof instance.sendRequest === "function" &&
        instance.pendingRequests instanceof Map
      ) {
        return value;
      }
    }
    return null;
  }

  async function sendCustomTranscribeDesktopApiRequest(method, url, options = {}) {
    const response = await window.fetch(url, {
      body: options?.body,
      headers: options?.headers,
      method,
      signal: options?.signal,
    });
    const text = await response.text();
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      body = { text };
    }
    if (!response.ok) {
      throw new Error(text || "Unable to transcribe audio");
    }
    return {
      body,
      headers: Object.fromEntries(response.headers.entries()),
      status: response.status,
    };
  }

  async function patchDesktopApiTranscribeModule(moduleUrl) {
    let module;
    try {
      module = await import(moduleUrl);
    } catch {
      return false;
    }
    const ClientClass = desktopApiClientClassFromModule(module);
    if (!ClientClass) {
      return false;
    }
    const client = ClientClass.getInstance();
    const prototype = Object.getPrototypeOf(client);
    if (!prototype || prototype.__codexlDesktopApiTranscribePatched) {
      return true;
    }
    const originalSendRequest = prototype.sendRequest;
    if (typeof originalSendRequest !== "function") {
      return false;
    }
    const patchedSendRequest = async function codexlDesktopApiSendRequest(method, url, options = {}) {
      if (isTranscribeDesktopApiRequest(method, url)) {
        return sendCustomTranscribeDesktopApiRequest(method, url, options);
      }
      return originalSendRequest.call(this, method, url, options);
    };
    Object.defineProperty(patchedSendRequest, "__codexlDesktopApiTranscribePatch", {
      configurable: true,
      value: true,
    });
    prototype.sendRequest = patchedSendRequest;
    Object.defineProperty(prototype, "__codexlDesktopApiTranscribePatched", {
      configurable: true,
      value: true,
    });
    Object.defineProperty(prototype, "__codexlOriginalDesktopApiSendRequest", {
      configurable: true,
      value: originalSendRequest,
    });
    runtime.cleanup.push(() => {
      if (prototype.sendRequest?.__codexlDesktopApiTranscribePatch) {
        prototype.sendRequest = originalSendRequest;
      }
      try {
        delete prototype.__codexlDesktopApiTranscribePatched;
        delete prototype.__codexlOriginalDesktopApiSendRequest;
      } catch {}
    });
    runtime.desktopApiTranscribePatched = true;
    log("info", "CodexL desktop API transcribe interceptor installed", moduleUrl);
    return true;
  }

  function installDesktopApiTranscribeInterceptor() {
    if (runtime.desktopApiTranscribeInstallStarted) {
      return;
    }
    runtime.desktopApiTranscribeInstallStarted = true;
    let attempts = 0;
    const tryInstall = async () => {
      attempts += 1;
      for (const moduleUrl of desktopApiModuleUrls()) {
        if (await patchDesktopApiTranscribeModule(moduleUrl)) {
          return;
        }
      }
      if (attempts < 80) {
        runtime.desktopApiTranscribeInstallTimer = window.setTimeout(tryInstall, 250);
      } else {
        log("error", "CodexL desktop API transcribe interceptor not installed");
      }
    };
    runtime.cleanup.push(() => {
      if (runtime.desktopApiTranscribeInstallTimer) {
        window.clearTimeout(runtime.desktopApiTranscribeInstallTimer);
      }
    });
    void tryInstall();
  }

  function hasCdpBindingBridge() {
    return typeof window.__codexlPluginBridge === "function";
  }

  function handleBridgeMessage(message) {
    if (message && message.id != null && runtime.pending.has(String(message.id))) {
      const pending = runtime.pending.get(String(message.id));
      runtime.pending.delete(String(message.id));
      if (message.ok) {
        pending.resolve(message.value);
      } else {
        pending.reject(new Error(message.error || "Plugin bridge request failed"));
      }
    }
  }

  function send(message) {
    if (hasCdpBindingBridge()) {
      try {
        window.__codexlPluginBridge(JSON.stringify(message));
        return true;
      } catch (error) {
        runtime.status = "error";
        runtime.statusDetail = `cdp binding error: ${String(error)}`;
        updateUi();
        return false;
      }
    }
    const socket = runtime.socket;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return false;
    }
    try {
      socket.send(JSON.stringify(message));
      return true;
    } catch {
      return false;
    }
  }

  function afterBridgeConnected() {
    send({
      type: "hello",
      location: window.location.href,
      runtimeVersion: RUNTIME_VERSION,
      reactHookInstalled: !!runtime.hook,
    });
    loadCoreSettings().catch((error) =>
      log("error", "CodexL settings load failed", String(error))
    );
    loadPlugins().catch((error) => log("error", "Plugin load failed", String(error)));
  }

  function connect() {
    if (hasCdpBindingBridge()) {
      runtime.status = "connected";
      runtime.statusDetail = "cdp binding";
      updateUi();
      afterBridgeConnected();
      return;
    }
    if (
      runtime.socket &&
      (runtime.socket.readyState === WebSocket.OPEN ||
        runtime.socket.readyState === WebSocket.CONNECTING)
    ) {
      return;
    }
    try {
      runtime.status = "connecting";
      runtime.statusDetail = runtime.bridgeUrl.replace(/token=[^&]+/, "token=<redacted>");
      updateUi();
      const socket = new WebSocket(runtime.bridgeUrl);
      runtime.socket = socket;
      socket.addEventListener("open", () => {
        runtime.status = "connected";
        runtime.statusDetail = "";
        updateUi();
        afterBridgeConnected();
      });
      socket.addEventListener("message", (event) => {
        let message;
        try {
          message = JSON.parse(event.data);
        } catch {
          return;
        }
        handleBridgeMessage(message);
      });
      socket.addEventListener("close", (event) => {
        runtime.status = "disconnected";
        runtime.statusDetail = `close ${event.code}${event.reason ? `: ${event.reason}` : ""}`;
        updateUi();
        window.setTimeout(connect, 1000);
      });
      socket.addEventListener("error", () => {
        runtime.status = "error";
        runtime.statusDetail = "websocket error";
        updateUi();
        diagnoseBridge().catch(() => {});
      });
    } catch (error) {
      runtime.status = "error";
      runtime.statusDetail = String(error);
      updateUi();
      window.setTimeout(connect, 1000);
    }
  }

  function registerPanel(id, title, render) {
    if (!id || typeof render !== "function") {
      throw new Error("registerPanel requires id and render function");
    }
    runtime.panels.set(id, { render, title: title || id });
    updatePanels();
  }

  function reconfigure(bridgeUrl) {
    if (!bridgeUrl || runtime.bridgeUrl === bridgeUrl) {
      return;
    }
    runtime.bridgeUrl = bridgeUrl;
    runtime.status = "booting";
    runtime.statusDetail = "bridge URL updated";
    runtime.pluginsLoaded = false;
    runtime.loadedPlugins.clear();
    try {
      runtime.socket?.close();
    } catch {}
    runtime.socket = null;
    updateUi();
  }

  function acceptCdpResponse(message) {
    handleBridgeMessage(message);
  }

  function teardown() {
    try {
      runtime.socket?.close();
    } catch {}
    for (const cleanup of runtime.cleanup.splice(0)) {
      try {
        cleanup();
      } catch {}
    }
    for (const pending of runtime.pending.values()) {
      try {
        pending.reject(new Error("Plugin runtime was replaced"));
      } catch {}
    }
    runtime.pending.clear();
    try {
      uninstallContextIndicator();
    } catch {}
    try {
      removeCodexLSettingsInjection();
    } catch {}
    try {
      document.getElementById(SETTINGS_STYLE_ID)?.remove();
    } catch {}
    try {
      runtime.ui?.host?.remove();
    } catch {}
    runtime.ui = null;
  }

  function bridgeHttpUrl(path) {
    const url = new URL(runtime.bridgeUrl);
    url.protocol = url.protocol === "wss:" ? "https:" : "http:";
    url.pathname = path;
    return url.toString();
  }

  async function diagnoseBridge() {
    try {
      const probe = await fetch(bridgeHttpUrl("/plugin/_bridge"), { cache: "no-store" });
      let detail = "";
      try {
        const body = await probe.json();
        detail = body && typeof body.error === "string" ? ` ${body.error}` : "";
      } catch {}
      runtime.statusDetail = `websocket error; bridge probe ${probe.status}${detail}`;
      updateUi();
      return;
    } catch (error) {
      runtime.statusDetail = `websocket error; ${String(error)}`;
      updateUi();
    }
  }

  function isElementVisible(element) {
    if (!(element instanceof Element)) {
      return false;
    }
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      return false;
    }
    const style = window.getComputedStyle(element);
    return (
      style.visibility !== "hidden" &&
      style.display !== "none" &&
      Number(style.opacity || "1") !== 0
    );
  }

  function normalizedText(element) {
    return (element?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function elementDebugSummary(element) {
    if (!(element instanceof Element)) {
      return null;
    }
    const rect = element.getBoundingClientRect();
    const fiberNames = [];
    try {
      let fiber = getFiber(element);
      let depth = 0;
      while (fiber && depth < 8) {
        const name = fiberName(fiber);
        if (name) {
          fiberNames.push(name);
        }
        fiber = fiber.return || null;
        depth += 1;
      }
    } catch {}
    return {
      ariaLabel: element.getAttribute("aria-label") || "",
      className: String(element.className || "").slice(0, 120),
      dataCodexlNav: element.getAttribute(SETTINGS_NAV_ATTR) || "",
      fiberNames,
      role: element.getAttribute("role") || "",
      tag: element.tagName.toLowerCase(),
      text: normalizedText(element).slice(0, 160),
      rect: {
        bottom: Math.round(rect.bottom),
        height: Math.round(rect.height),
        left: Math.round(rect.left),
        right: Math.round(rect.right),
        top: Math.round(rect.top),
        width: Math.round(rect.width),
      },
    };
  }

  function settingsDebugLog(message, detail = {}, level = "info", options = {}) {
    const now = Date.now();
    const key = `${level}:${message}`;
    runtime.settingsDebugSeen ||= Object.create(null);
    if (runtime.settingsDebugSeen[key] && now - runtime.settingsDebugSeen[key] < 2000) {
      return;
    }
    runtime.settingsDebugSeen[key] = now;
    const entry = {
      build: RUNTIME_BUILD,
      detail,
      href: window.location.href,
      message,
      ts: now,
      version: RUNTIME_VERSION,
    };
    runtime.settingsDiagnostics.push(entry);
    runtime.settingsDiagnostics.splice(0, Math.max(0, runtime.settingsDiagnostics.length - 50));
    if (options.emit === false) {
      return;
    }
    try {
      const method = level === "warn" ? "warn" : level === "error" ? "error" : "info";
      console[method]("[codexl-settings]", message, detail);
    } catch {}
    try {
      send({
        type: "log",
        level,
        message: `[settings] ${message}`,
        detail: JSON.stringify(entry).slice(0, 8000),
      });
    } catch {}
  }

  function installGlobalStyle() {
    if (document.getElementById(SETTINGS_STYLE_ID)) {
      return;
    }
    const style = document.createElement("style");
    style.id = SETTINGS_STYLE_ID;
    style.textContent = `
      [${SETTINGS_NAV_ATTR}="1"] {
        cursor: pointer;
      }
      .codexl-settings-panel {
        background: var(--color-main-surface-primary, Canvas);
        box-sizing: border-box;
        color: inherit;
        inset: 0;
        overflow-y: auto;
        position: absolute;
        z-index: 20;
      }
      #${CONTEXT_INDICATOR_ID} {
        align-items: center;
        background: transparent;
        border: 0;
        border-radius: 999px;
        box-sizing: border-box;
        color: rgb(16, 163, 127);
        cursor: help;
        display: inline-flex;
        flex: 0 0 auto;
        height: 28px;
        justify-content: center;
        margin: 0;
        padding: 0;
        pointer-events: auto;
        position: static;
        width: 28px;
      }
      #${CONTEXT_INDICATOR_ID}[data-state="medium"] {
        color: rgb(194, 120, 3);
      }
      #${CONTEXT_INDICATOR_ID}[data-state="high"] {
        color: rgb(209, 67, 67);
      }
      #${CONTEXT_INDICATOR_ID}[data-state="unknown"] {
        color: color-mix(in srgb, currentColor 46%, transparent);
      }
      #${CONTEXT_INDICATOR_ID} svg {
        display: block;
        height: 14px;
        width: 14px;
      }
      #${CONTEXT_TOOLTIP_ID} {
        background: var(--color-main-surface-primary, Canvas);
        border: 1px solid var(--color-border-default, rgba(120, 120, 120, .28));
        border-radius: 6px;
        box-shadow: 0 8px 24px rgba(0, 0, 0, .18);
        box-sizing: border-box;
        color: var(--color-text-primary, CanvasText);
        font: 12px/1.35 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        max-width: min(360px, calc(100vw - 24px));
        overflow: hidden;
        padding: 5px 7px;
        pointer-events: none;
        position: fixed;
        text-overflow: ellipsis;
        white-space: nowrap;
        z-index: 2147483647;
      }
    `;
    (document.head || document.documentElement).appendChild(style);
  }

  async function loadCoreSettings() {
    if (runtime.coreSettingsLoaded) {
      return;
    }
    runtime.coreSettingsLoaded = true;
    try {
      const [showContextResponse, showAllSessionsResponse] = await Promise.all([
        request("storage:get", {
          key: SHOW_CONTEXT_INDICATORS_KEY,
          pluginId: CORE_PLUGIN_ID,
        }),
        request("storage:get", {
          key: SHOW_ALL_SESSIONS_KEY,
          pluginId: CORE_PLUGIN_ID,
        }),
      ]);
      runtime.codexlSettings.showContextIndicators = showContextResponse?.value === true;
      runtime.codexlSettings.showAllSessions = showAllSessionsResponse?.value === true;
    } catch (error) {
      runtime.coreSettingsLoaded = false;
      throw error;
    } finally {
      updateSettingsUi();
      syncContextIndicator();
    }
  }

  function setShowContextIndicators(enabled) {
    runtime.codexlSettings.showContextIndicators = !!enabled;
    updateSettingsUi();
    syncContextIndicator();
    request("storage:set", {
      key: SHOW_CONTEXT_INDICATORS_KEY,
      pluginId: CORE_PLUGIN_ID,
      value: runtime.codexlSettings.showContextIndicators,
    }).catch((error) =>
      log("error", "CodexL settings save failed", String(error))
    );
  }

  function setShowAllSessions(enabled) {
    runtime.codexlSettings.showAllSessions = !!enabled;
    updateSettingsUi();
    request("storage:set", {
      key: SHOW_ALL_SESSIONS_KEY,
      pluginId: CORE_PLUGIN_ID,
      value: runtime.codexlSettings.showAllSessions,
    }).catch((error) =>
      log("error", "CodexL settings save failed", String(error))
    );
  }

  const SETTINGS_NAV_LABELS = new Set([
    "Account",
    "Advanced",
    "Appearance",
    "Appshots",
    "Apps",
    "Archived chats",
    "Back to app",
    "Browser",
    "Computer use",
    "Configuration",
    "Connections",
    "Data controls",
    "Developer",
    "Environments",
    "Git",
    "General",
    "Hooks",
    "Keyboard shortcuts",
    "MCP",
    "MCP servers",
    "Memory",
    "Model",
    "Notifications",
    "Personalization",
    "Plugins",
    "Privacy",
    "Profile",
    "Settings",
    "Usage",
    "Worktrees",
    "\u5e38\u89c4",
    "\u8d26\u6237",
    "\u5916\u89c2",
    "\u901a\u77e5",
    "\u6570\u636e\u63a7\u4ef6",
    "\u5f00\u53d1\u8005",
    "\u9ad8\u7ea7",
    "\u4f7f\u7528\u60c5\u51b5",
    "\u6a21\u578b",
    "\u5e94\u7528",
    "\u63d2\u4ef6",
    "\u9690\u79c1",
    "\u8bbe\u7f6e",
    "\u8bb0\u5fc6",
  ]);

  const SETTINGS_LABELS_BY_LOWERCASE = new Map(
    Array.from(SETTINGS_NAV_LABELS).map((label) => [label.toLowerCase(), label])
  );

  function isSettingsNavText(text) {
    const value = text.replace(/\s+/g, " ").trim();
    return settingsLabelForText(value) != null;
  }

  function settingsLabelForText(text) {
    const value = (text || "").replace(/\s+/g, " ").trim();
    if (!value) {
      return null;
    }
    const lowercaseValue = value.toLowerCase();
    const exactCaseInsensitive = SETTINGS_LABELS_BY_LOWERCASE.get(lowercaseValue);
    if (exactCaseInsensitive) {
      return exactCaseInsensitive;
    }
    if (SETTINGS_NAV_LABELS.has(value)) {
      return value;
    }
    for (const label of SETTINGS_NAV_LABELS) {
      const lowercaseLabel = label.toLowerCase();
      if (
        value.length <= label.length + 32 &&
        (lowercaseValue.startsWith(`${lowercaseLabel} `) ||
          lowercaseValue.endsWith(` ${lowercaseLabel}`))
      ) {
        return label;
      }
    }
    return null;
  }

  function isSettingsRoute() {
    const locationText = `${window.location.pathname} ${window.location.search} ${window.location.hash}`.toLowerCase();
    return locationText.includes("settings");
  }

  function isSettingsHeadingText(text) {
    const value = (text || "").replace(/\s+/g, " ").trim();
    return value === "Settings" || value === "\u8bbe\u7f6e";
  }

  function hasSettingsHeading(root) {
    return Array.from(root.querySelectorAll('h1, h2, [role="heading"]'))
      .filter(isElementVisible)
      .some((element) => isSettingsHeadingText(normalizedText(element)));
  }

  function settingsInteractiveItems(root) {
    return Array.from(
      root.querySelectorAll(
        'button, a, [role="button"], [role="menuitem"], [role="tab"]'
      )
    ).filter((element) => isElementVisible(element));
  }

  function labelCountInText(text) {
    const value = (text || "").replace(/\s+/g, " ").trim();
    let count = 0;
    for (const label of SETTINGS_NAV_LABELS) {
      if (value.includes(label)) {
        count += 1;
      }
    }
    return count;
  }

  function settingsNavRowForElement(element, root) {
    let row = element;
    let node = element;
    let depth = 0;
    while (node && node !== root && node !== document.body && depth < 5) {
      const text = normalizedText(node);
      const rect = node.getBoundingClientRect();
      if (
        settingsLabelForText(text) &&
        labelCountInText(text) <= 1 &&
        rect.height <= 72 &&
        rect.width >= 40
      ) {
        row = node;
      }
      node = node.parentElement;
      depth += 1;
    }
    const closest = element.closest(
      'button, a, [role="button"], [role="menuitem"], [role="tab"]'
    );
    if (
      closest &&
      root.contains(closest) &&
      isElementVisible(closest) &&
      labelCountInText(normalizedText(closest)) <= 1
    ) {
      return closest;
    }
    return row;
  }

  function settingsNavItemElements(root) {
    const seen = new Set();
    const elements = Array.from(
      root.querySelectorAll(
        'button, a, [role="button"], [role="menuitem"], [role="tab"], [tabindex], div, span'
      )
    );
    const items = [];
    for (const element of elements) {
      if (!isElementVisible(element) || !settingsLabelForText(normalizedText(element))) {
        continue;
      }
      const item = settingsNavRowForElement(element, root);
      if (seen.has(item) || !isElementVisible(item)) {
        continue;
      }
      seen.add(item);
      items.push(item);
    }
    return items;
  }

  const SETTINGS_PAGE_SIGNATURE_LABELS = new Set([
    "Appshots",
    "Archived chats",
    "Configuration",
    "Connections",
    "Environments",
    "Git",
    "Hooks",
    "MCP servers",
    "Personalization",
    "Worktrees",
  ]);

  function settingsLabelSet(root) {
    return new Set(
      settingsNavItemElements(root)
        .map((element) => settingsLabelForText(normalizedText(element)))
        .filter(Boolean)
    );
  }

  function settingsMenuSignatureScore(labels) {
    let score = 0;
    for (const label of SETTINGS_PAGE_SIGNATURE_LABELS) {
      if (labels.has(label)) {
        score += 1;
      }
    }
    return score;
  }

  function isPlausibleSettingsNavContainer(candidate) {
    if (!(candidate instanceof Element) || !isElementVisible(candidate)) {
      return false;
    }
    const rect = candidate.getBoundingClientRect();
    const labels = settingsLabelSet(candidate);
    if (labels.size < 8 || settingsMenuSignatureScore(labels) < 5) {
      return false;
    }
    const leftSideLimit = Math.max(360, window.innerWidth * 0.35);
    if (rect.left > leftSideLimit) {
      return false;
    }
    if (rect.width > 420 || rect.height < 180) {
      return false;
    }
    return true;
  }

  function settingsNavCandidateScore(candidate) {
    if (!isPlausibleSettingsNavContainer(candidate)) {
      return 0;
    }
    const rect = candidate.getBoundingClientRect();
    const labels = settingsLabelSet(candidate);
    const signatureScore = settingsMenuSignatureScore(labels);
    const semanticBonus = candidate.matches('nav, aside, [role="tablist"], [role="menu"]')
      ? 200
      : 0;
    const leftBonus = Math.max(0, 80 - Math.round(rect.left));
    const widthBonus = Math.max(0, 420 - Math.round(rect.width));
    const heightPenalty = Math.max(0, Math.round(rect.height) - 650);
    return signatureScore * 1000 + labels.size * 20 + semanticBonus + leftBonus + widthBonus - heightPenalty;
  }

  function findBestSettingsNavCandidate(root) {
    const candidates = new Set();
    for (const item of settingsNavItemElements(root)) {
      let node = item;
      let depth = 0;
      while (node && node !== document.body && depth < 10) {
        if (node instanceof Element && isElementVisible(node)) {
          candidates.add(node);
        }
        node = node.parentElement;
        depth += 1;
      }
    }
    root
      .querySelectorAll?.('nav, aside, [role="tablist"], [role="menu"], div')
      .forEach((element) => {
        if (isElementVisible(element)) {
          candidates.add(element);
        }
      });

    let best = null;
    let bestScore = 0;
    let bestArea = Number.POSITIVE_INFINITY;
    const scored = [];
    for (const candidate of candidates) {
      const score = settingsNavCandidateScore(candidate);
      if (!score) {
        continue;
      }
      const rect = candidate.getBoundingClientRect();
      const area = rect.width * rect.height;
      scored.push({
        area: Math.round(area),
        labels: Array.from(settingsLabelSet(candidate)),
        score,
        summary: elementDebugSummary(candidate),
      });
      if (score > bestScore || (score === bestScore && area < bestArea)) {
        best = candidate;
        bestScore = score;
        bestArea = area;
      }
    }
    runtime.lastSettingsCandidateDiagnostics = scored
      .sort((first, second) => second.score - first.score || first.area - second.area)
      .slice(0, 8);
    return best;
  }

  function findSettingsShell() {
    const routeLooksLikeSettings = isSettingsRoute();
    const documentSettingsLabels = settingsLabelSet(document);
    const documentLooksLikeSettingsMenu =
      settingsMenuSignatureScore(documentSettingsLabels) >= 5;
    if (!routeLooksLikeSettings && !hasSettingsHeading(document) && !documentLooksLikeSettingsMenu) {
      settingsDebugLog(
        "settings shell rejected at document gate",
        {
          documentLabels: Array.from(documentSettingsLabels),
          documentSignatureScore: settingsMenuSignatureScore(documentSettingsLabels),
          hasHeading: hasSettingsHeading(document),
          routeLooksLikeSettings,
        },
        "info",
        { emit: false }
      );
      return null;
    }
    const candidates = Array.from(
      document.querySelectorAll(
        '[role="dialog"], [aria-modal="true"], [data-radix-dialog-content], main, section, [class*="settings"], div'
      )
    ).filter((element) => {
      if (!isElementVisible(element) || element.closest(`#${ROOT_ID}`)) {
        return false;
      }
      if (element.matches('button, a, [role="button"], [role="menu"], [role="menuitem"]')) {
        return false;
      }
      if (element.closest('[role="menu"]')) {
        return false;
      }
      const rect = element.getBoundingClientRect();
      if (rect.width < 480 || rect.height < 320) {
        return false;
      }
      const labels = settingsLabelSet(element);
      const labelCount = labels.size;
      return (
        labelCount >= 3 &&
        (routeLooksLikeSettings ||
          hasSettingsHeading(element) ||
          settingsMenuSignatureScore(labels) >= 5)
      );
    });
    runtime.lastSettingsShellCandidateDiagnostics = candidates.slice(0, 12).map((candidate) => ({
      labels: Array.from(settingsLabelSet(candidate)),
      signatureScore: settingsMenuSignatureScore(settingsLabelSet(candidate)),
      summary: elementDebugSummary(candidate),
    }));

    candidates.sort((first, second) => {
      const firstRect = first.getBoundingClientRect();
      const secondRect = second.getBoundingClientRect();
      return firstRect.width * firstRect.height - secondRect.width * secondRect.height;
    });
    if (!candidates[0]) {
      settingsDebugLog(
        "settings shell not found after candidate scan",
        {
          documentLabels: Array.from(documentSettingsLabels),
          documentSignatureScore: settingsMenuSignatureScore(documentSettingsLabels),
          routeLooksLikeSettings,
        },
        "info",
        { emit: false }
      );
    }
    return candidates[0] || null;
  }

  function findSettingsNav(shell) {
    const directCandidate = findBestSettingsNavCandidate(shell);
    if (directCandidate) {
      return directCandidate;
    }
    const candidates = Array.from(
      shell.querySelectorAll('nav, aside, [role="tablist"], [role="menu"], div')
    ).filter(isElementVisible);
    let best = null;
    let bestScore = 0;
    for (const candidate of candidates) {
      const score = settingsLabelSet(candidate).size;
      if (score > bestScore) {
        best = candidate;
        bestScore = score;
      }
    }
    return bestScore >= 3 ? best : null;
  }

  function findSettingsShellForNav(nav) {
    const navRect = nav.getBoundingClientRect();
    let node = nav.parentElement;
    let best = null;
    let depth = 0;
    while (node && node !== document.body && depth < 12) {
      if (isElementVisible(node)) {
        const rect = node.getBoundingClientRect();
        if (
          rect.width >= Math.max(480, navRect.width + 220) &&
          rect.height >= 320 &&
          rect.left <= navRect.left + 24 &&
          rect.right >= navRect.right + 220
        ) {
          best = node;
        }
      }
      node = node.parentElement;
      depth += 1;
    }
    return best;
  }

  function removeDuplicateIds(root) {
    if (!(root instanceof Element)) {
      return;
    }
    root.removeAttribute("id");
    root.querySelectorAll("[id]").forEach((element) => element.removeAttribute("id"));
  }

  function createCodexLSettingsIcon(referenceItem) {
    const referenceIcon = referenceItem?.querySelector?.("svg");
    const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    icon.setAttribute("width", referenceIcon?.getAttribute("width") || "20");
    icon.setAttribute("height", referenceIcon?.getAttribute("height") || "20");
    icon.setAttribute("viewBox", "0 0 20 20");
    icon.setAttribute("fill", "none");
    icon.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    const className =
      referenceIcon?.getAttribute("class") ||
      "icon-sm inline-block align-middle text-token-icon-foreground";
    icon.setAttribute("class", className);
    icon.innerHTML = `
      <path d="M7.25 6.25 3.75 10l3.5 3.75" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path>
      <path d="M12.75 6.25 16.25 10l-3.5 3.75" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"></path>
      <path d="M11.25 4.25 8.75 15.75" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"></path>
    `;
    return icon;
  }

  function replaceSettingsNavIcon(item, referenceItem) {
    const icon = createCodexLSettingsIcon(referenceItem);
    const existingIcon = item.querySelector("svg");
    if (existingIcon) {
      existingIcon.replaceWith(icon);
      return;
    }
    const content = item.querySelector("div") || item;
    content.insertBefore(icon, content.firstChild);
  }

  function replaceSettingsNavText(item, label) {
    const walker = document.createTreeWalker(
      item,
      NodeFilter.SHOW_TEXT,
      {
        acceptNode(node) {
          return node.nodeValue?.trim()
            ? NodeFilter.FILTER_ACCEPT
            : NodeFilter.FILTER_REJECT;
        },
      }
    );
    const textNodes = [];
    let node = walker.nextNode();
    while (node) {
      textNodes.push(node);
      node = walker.nextNode();
    }
    if (textNodes.length > 0) {
      textNodes[0].nodeValue = label;
      for (const extra of textNodes.slice(1)) {
        extra.nodeValue = "";
      }
      return;
    }
    const span = document.createElement("span");
    span.className = "truncate";
    span.textContent = label;
    (item.querySelector("div") || item).appendChild(span);
  }

  function setCodexLSettingsNavActive(item, active) {
    if (!(item instanceof Element)) {
      return;
    }
    item.classList.toggle("codexl-settings-nav-active", active);
    item.classList.toggle("bg-token-list-hover-background", active);
    item.classList.toggle("hover:bg-token-list-hover-background", !active);
    if (active) {
      item.setAttribute("aria-current", "page");
      item.setAttribute("aria-selected", "true");
    } else {
      item.removeAttribute("aria-current");
      item.removeAttribute("aria-selected");
    }
    const content = item.querySelector("div") || item;
    content.classList.toggle("text-token-list-active-selection-foreground", active);
    content.classList.toggle("text-token-foreground", !active);
    const icon = item.querySelector("svg");
    if (icon) {
      icon.classList.toggle("text-token-list-active-selection-icon-foreground", active);
    }
  }

  function storeSettingsNavVisualState(item) {
    if (!(item instanceof Element) || item.__codexlSettingsNavVisualState) {
      return;
    }
    const classElements = [item, ...item.querySelectorAll("[class]")];
    item.__codexlSettingsNavVisualState = {
      ariaCurrent: item.getAttribute("aria-current"),
      ariaSelected: item.getAttribute("aria-selected"),
      classes: classElements.map((element) => [
        element,
        element.getAttribute("class") || "",
      ]),
    };
  }

  function restoreSettingsNavVisualState(item) {
    const state = item?.__codexlSettingsNavVisualState;
    if (!state) {
      return;
    }
    for (const [element, className] of state.classes) {
      element.setAttribute("class", className);
    }
    if (state.ariaCurrent == null) {
      item.removeAttribute("aria-current");
    } else {
      item.setAttribute("aria-current", state.ariaCurrent);
    }
    if (state.ariaSelected == null) {
      item.removeAttribute("aria-selected");
    } else {
      item.setAttribute("aria-selected", state.ariaSelected);
    }
    delete item.__codexlSettingsNavVisualState;
  }

  function suppressNativeSettingsNavSelection(nav, codexlItem) {
    runtime.suppressedSettingsNavItems ||= [];
    for (const item of runtime.suppressedSettingsNavItems.splice(0)) {
      restoreSettingsNavVisualState(item);
    }
    const nativeSelected = Array.from(
      nav.querySelectorAll('[aria-current="page"], [aria-selected="true"], .bg-token-list-hover-background')
    ).filter((item) => item !== codexlItem && !item.hasAttribute(SETTINGS_NAV_ATTR));
    for (const item of nativeSelected) {
      storeSettingsNavVisualState(item);
      item.removeAttribute("aria-current");
      item.removeAttribute("aria-selected");
      item.classList.remove("bg-token-list-hover-background");
      item.classList.add("hover:bg-token-list-hover-background");
      const content = item.querySelector("div") || item;
      content.classList.remove("text-token-list-active-selection-foreground");
      content.classList.add("text-token-foreground");
      item
        .querySelectorAll("svg")
        .forEach((icon) =>
          icon.classList.remove("text-token-list-active-selection-icon-foreground")
        );
      runtime.suppressedSettingsNavItems.push(item);
    }
  }

  function restoreNativeSettingsNavSelection() {
    for (const item of runtime.suppressedSettingsNavItems || []) {
      restoreSettingsNavVisualState(item);
    }
    runtime.suppressedSettingsNavItems = [];
  }

  function createSettingsNavItem(referenceItem) {
    let item;
    if (referenceItem instanceof HTMLButtonElement) {
      item = referenceItem.cloneNode(true);
      item.type = "button";
    } else if (referenceItem instanceof HTMLAnchorElement) {
      item = document.createElement("button");
      item.type = "button";
      item.className = referenceItem.className;
      item.innerHTML = referenceItem.innerHTML;
    } else {
      item = referenceItem.cloneNode(true);
      item.setAttribute("role", "button");
      item.tabIndex = 0;
    }
    removeDuplicateIds(item);
    replaceSettingsNavIcon(item, referenceItem);
    replaceSettingsNavText(item, "CodexL");
    item.setAttribute(SETTINGS_NAV_ATTR, "1");
    item.setAttribute("aria-label", "CodexL");
    item.removeAttribute("disabled");
    item.removeAttribute("href");
    item.removeAttribute("data-state");
    item.removeAttribute("aria-current");
    item.removeAttribute("aria-selected");
    setCodexLSettingsNavActive(item, false);
    item.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      showCodexLSettings();
    });
    item.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }
      event.preventDefault();
      showCodexLSettings();
    });
    return item;
  }

  function ensureSettingsNavItem(nav) {
    let item = nav.querySelector(`[${SETTINGS_NAV_ATTR}="1"]`);
    if (item) {
      if (!item.querySelector("svg")) {
        item.remove();
        item = null;
      }
    }
    if (item) {
      settingsDebugLog("CodexL settings nav item already present", {
        item: elementDebugSummary(item),
        nav: elementDebugSummary(nav),
      }, "info", { emit: false });
      return item;
    }
    const items = settingsNavItemElements(nav);
    const referenceItem = items[items.length - 1] || settingsInteractiveItems(nav)[0];
    if (!referenceItem) {
      settingsDebugLog(
        "CodexL settings nav insert failed: no reference item",
        {
          labels: Array.from(settingsLabelSet(nav)),
          nav: elementDebugSummary(nav),
        },
        "warn"
      );
      return null;
    }
    const referenceParent = referenceItem.parentElement;
    if (!referenceParent) {
      settingsDebugLog(
        "CodexL settings nav insert failed: reference item has no parent",
        {
          labels: Array.from(settingsLabelSet(nav)),
          nav: elementDebugSummary(nav),
          referenceItem: elementDebugSummary(referenceItem),
        },
        "warn"
      );
      return null;
    }
    item = createSettingsNavItem(referenceItem);
    referenceParent.insertBefore(item, referenceItem.nextSibling);
    settingsDebugLog("CodexL settings nav item inserted", {
      item: elementDebugSummary(item),
      labels: Array.from(settingsLabelSet(nav)),
      nav: elementDebugSummary(nav),
      referenceItem: elementDebugSummary(referenceItem),
    });
    for (const nativeItem of items) {
      nativeItem.addEventListener("click", () => hideCodexLSettings(), true);
    }
    return item;
  }

  function removeMisplacedSettingsNavItems(nav) {
    document.querySelectorAll(`[${SETTINGS_NAV_ATTR}="1"]`).forEach((item) => {
      if (nav.contains(item)) {
        return;
      }
      settingsDebugLog("removing misplaced CodexL settings nav item", {
        item: elementDebugSummary(item),
        selectedNav: elementDebugSummary(nav),
      });
      item.remove();
    });
  }

  function findSettingsContent(shell, nav) {
    const navRect = nav.getBoundingClientRect();
    const candidates = Array.from(
      shell.querySelectorAll('main, [role="tabpanel"], section, [class*="content"], [class*="pane"], div')
    ).filter((element) => {
      if (!isElementVisible(element) || element === nav || element.contains(nav) || nav.contains(element)) {
        return false;
      }
      const rect = element.getBoundingClientRect();
      return rect.width >= 220 && rect.height >= 120 && rect.left >= navRect.left;
    });
    candidates.sort((first, second) => {
      const firstRect = first.getBoundingClientRect();
      const secondRect = second.getBoundingClientRect();
      const firstScore =
        firstRect.width * firstRect.height + (firstRect.left >= navRect.right - 24 ? 1000000 : 0);
      const secondScore =
        secondRect.width * secondRect.height + (secondRect.left >= navRect.right - 24 ? 1000000 : 0);
      return secondScore - firstScore;
    });
    return candidates[0] || null;
  }

  function renderCodexLSettingsPanel(panel) {
    panel.className = "codexl-settings-panel scrollbar-stable p-panel";
    panel.setAttribute(SETTINGS_PANEL_ATTR, "1");
    panel.innerHTML = `
      <div class="mx-auto flex w-full flex-col max-w-2xl electron:min-w-[calc(320px*var(--codex-window-zoom))]">
        <div class="flex items-center justify-between gap-3 pb-panel">
          <div class="flex min-w-0 flex-1 flex-col gap-1.5 pb-panel">
            <div class="electron:heading-lg heading-base truncate">CodexL</div>
          </div>
        </div>
        <div class="flex flex-col gap-[var(--padding-panel)]">
          <section class="flex flex-col gap-2">
            <div class="flex flex-col divide-y-[0.5px] divide-token-border overflow-hidden rounded-lg border border-token-border" style="background-color: var(--color-background-panel, var(--color-token-bg-fog));">
              <div class="flex items-center justify-between gap-4 p-3">
                <div class="flex min-w-0 items-center gap-3">
                  <div class="flex min-w-0 flex-col gap-1">
                    <div class="min-w-0 text-sm text-token-text-primary">Show Context Indicators</div>
                  </div>
                </div>
                <div class="flex shrink-0 items-center gap-2">
                  <button class="codexl-settings-switch inline-flex items-center text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-token-focus-border focus-visible:rounded-full cursor-interaction" type="button" role="switch" aria-label="Show Context Indicators" aria-checked="false" data-state="unchecked" data-codexl-setting="showContextIndicators">
                    <span class="codexl-settings-switch-track relative inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out bg-token-foreground/10 h-5 w-8" data-state="unchecked">
                      <span class="codexl-settings-switch-thumb rounded-full border border-[color:var(--gray-0)] bg-[color:var(--gray-0)] shadow-sm transition-transform duration-200 ease-out data-[state=unchecked]:translate-x-[2px] data-[state=checked]:translate-x-[14px] h-4 w-4" data-state="unchecked"></span>
                    </span>
                  </button>
                </div>
              </div>
              <div class="flex items-center justify-between gap-4 p-3">
                <div class="flex min-w-0 items-center gap-3">
                  <div class="flex min-w-0 flex-col gap-1">
                    <div class="min-w-0 text-sm text-token-text-primary">Show All Sessions</div>
                  </div>
                </div>
                <div class="flex shrink-0 items-center gap-2">
                  <button class="codexl-settings-switch inline-flex items-center text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-token-focus-border focus-visible:rounded-full cursor-interaction" type="button" role="switch" aria-label="Show All Sessions" aria-checked="false" data-state="unchecked" data-codexl-setting="showAllSessions">
                    <span class="codexl-settings-switch-track relative inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out bg-token-foreground/10 h-5 w-8" data-state="unchecked">
                      <span class="codexl-settings-switch-thumb rounded-full border border-[color:var(--gray-0)] bg-[color:var(--gray-0)] shadow-sm transition-transform duration-200 ease-out data-[state=unchecked]:translate-x-[2px] data-[state=checked]:translate-x-[14px] h-4 w-4" data-state="unchecked"></span>
                    </span>
                  </button>
                </div>
              </div>
            </div>
          </section>
        </div>
      </div>
    `;
    panel
      .querySelector('[data-codexl-setting="showContextIndicators"]')
      ?.addEventListener("click", () => {
        setShowContextIndicators(!runtime.codexlSettings.showContextIndicators);
      });
    panel
      .querySelector('[data-codexl-setting="showAllSessions"]')
      ?.addEventListener("click", () => {
        setShowAllSessions(!runtime.codexlSettings.showAllSessions);
      });
    updateSettingsUi();
  }

  function showCodexLSettings() {
    const shell = findSettingsShell();
    const nav = (shell && findSettingsNav(shell)) || findBestSettingsNavCandidate(document);
    if (!nav) {
      return;
    }
    const effectiveShell = shell || findSettingsShellForNav(nav);
    if (!effectiveShell) {
      return;
    }
    removeMisplacedSettingsNavItems(nav);
    const navItem = ensureSettingsNavItem(nav);
    const content = findSettingsContent(effectiveShell, nav);
    if (!navItem || !content) {
      return;
    }
    suppressNativeSettingsNavSelection(nav, navItem);
    let panel = content.querySelector(`[${SETTINGS_PANEL_ATTR}="1"]`);
    if (!panel) {
      panel = document.createElement("section");
      renderCodexLSettingsPanel(panel);
      content.appendChild(panel);
    }
    if (window.getComputedStyle(content).position === "static") {
      content.dataset.codexlSettingsPosition = "1";
      content.dataset.codexlOriginalPosition = content.style.position || "";
      content.style.position = "relative";
    }
    panel.hidden = false;
    setCodexLSettingsNavActive(navItem, true);
    updateSettingsUi();
  }

  function hideCodexLSettings() {
    restoreNativeSettingsNavSelection();
    document
      .querySelectorAll(`[${SETTINGS_PANEL_ATTR}="1"]`)
      .forEach((panel) => {
        panel.hidden = true;
      });
    document
      .querySelectorAll(`[${SETTINGS_NAV_ATTR}="1"]`)
      .forEach((item) => {
        setCodexLSettingsNavActive(item, false);
      });
  }

  function updateSettingsUi() {
    document
      .querySelectorAll(".codexl-settings-switch")
      .forEach((toggle) => {
        const setting = toggle.getAttribute("data-codexl-setting") || "showContextIndicators";
        const enabledBySetting = {
          [SHOW_ALL_SESSIONS_KEY]: runtime.codexlSettings.showAllSessions,
          [SHOW_CONTEXT_INDICATORS_KEY]: runtime.codexlSettings.showContextIndicators,
        };
        const enabled = enabledBySetting[setting] === true;
        const state = enabled ? "checked" : "unchecked";
        toggle.setAttribute("aria-checked", state === "checked" ? "true" : "false");
        toggle.setAttribute("data-state", state);
        toggle
          .querySelectorAll(".codexl-settings-switch-track, .codexl-settings-switch-thumb")
          .forEach((element) => element.setAttribute("data-state", state));
        const track = toggle.querySelector(".codexl-settings-switch-track");
        if (track) {
          track.classList.toggle("bg-token-charts-blue", state === "checked");
          track.classList.toggle("bg-token-foreground/10", state !== "checked");
        }
      });
  }

  function removeCodexLSettingsInjection() {
    const existingItems = document.querySelectorAll(`[${SETTINGS_NAV_ATTR}="1"]`).length;
    const existingPanels = document.querySelectorAll(`[${SETTINGS_PANEL_ATTR}="1"]`).length;
    if (existingItems || existingPanels) {
      settingsDebugLog("removing CodexL settings injection", {
        existingItems,
        existingPanels,
      });
    }
    document
      .querySelectorAll(`[${SETTINGS_NAV_ATTR}="1"]`)
      .forEach((item) => item.remove());
    document
      .querySelectorAll(`[${SETTINGS_PANEL_ATTR}="1"]`)
      .forEach((panel) => panel.remove());
    document
      .querySelectorAll('[data-codexl-settings-position="1"]')
      .forEach((content) => {
        content.style.position = content.dataset.codexlOriginalPosition || "";
        delete content.dataset.codexlSettingsPosition;
        delete content.dataset.codexlOriginalPosition;
      });
  }

  function visibleSettingsTextSamples() {
    const seen = new Set();
    const samples = [];
    const elements = Array.from(
      document.querySelectorAll(
        'button, a, [role="button"], [role="menuitem"], [role="tab"], [tabindex], li, div, span, h1, h2, [role="heading"]'
      )
    );
    for (const element of elements) {
      if (!isElementVisible(element)) {
        continue;
      }
      const text = normalizedText(element);
      if (!text || text.length > 120 || seen.has(text)) {
        continue;
      }
      seen.add(text);
      samples.push(text);
      if (samples.length >= 80) {
        break;
      }
    }
    return samples;
  }

  function getSettingsDiagnostics({ shell = null, nav = null } = {}) {
    const documentLabels = settingsLabelSet(document);
    const navLabels = nav ? settingsLabelSet(nav) : new Set();
    return {
      build: RUNTIME_BUILD,
      documentLabels: Array.from(documentLabels),
      documentSignatureScore: settingsMenuSignatureScore(documentLabels),
      hasSettingsHeading: hasSettingsHeading(document),
      href: window.location.href,
      matchedItems: settingsNavItemElements(document).slice(0, 40).map((item) => ({
        label: settingsLabelForText(normalizedText(item)),
        summary: elementDebugSummary(item),
      })),
      nav: elementDebugSummary(nav),
      navCandidateDiagnostics: runtime.lastSettingsCandidateDiagnostics || [],
      navLabels: Array.from(navLabels),
      routeLooksLikeSettings: isSettingsRoute(),
      shell: elementDebugSummary(shell),
      shellCandidateDiagnostics: runtime.lastSettingsShellCandidateDiagnostics || [],
      textSamples: visibleSettingsTextSamples(),
      version: RUNTIME_VERSION,
    };
  }

  function settingsLocationKey() {
    return `${window.location.pathname}${window.location.search}${window.location.hash}`;
  }

  function hasCodexLSettingsInjection() {
    return !!document.querySelector(
      `[${SETTINGS_NAV_ATTR}="1"], [${SETTINGS_PANEL_ATTR}="1"]`
    );
  }

  function settingsScanWindowActive() {
    return Date.now() < (runtime.settingsInteractionUntil || 0);
  }

  function shouldScanSettingsDom(force = false) {
    return (
      force ||
      isSettingsRoute() ||
      settingsScanWindowActive() ||
      hasCodexLSettingsInjection()
    );
  }

  function clearSettingsRefreshTimers() {
    for (const timer of runtime.settingsRefreshTimers.splice(0)) {
      window.clearTimeout(timer);
    }
  }

  function scheduleSettingsRefresh({ delay = 0, diagnostics = false, force = false } = {}) {
    if (!document.body) {
      return;
    }
    if (delay > 0) {
      const timer = window.setTimeout(() => {
        scheduleSettingsRefresh({ diagnostics, force });
      }, delay);
      runtime.settingsRefreshTimers.push(timer);
      return;
    }
    if (runtime.settingsRefreshScheduled) {
      return;
    }
    runtime.settingsRefreshScheduled = true;
    window.requestAnimationFrame(() => {
      runtime.settingsRefreshScheduled = false;
      refreshCodexLSettingsNav({ diagnostics, force });
    });
  }

  function scheduleSettingsRefreshBurst(reason = "unknown") {
    runtime.settingsInteractionUntil = Math.max(
      runtime.settingsInteractionUntil || 0,
      Date.now() + SETTINGS_INTERACTION_SCAN_WINDOW_MS
    );
    clearSettingsRefreshTimers();
    settingsDebugLog("CodexL settings refresh scheduled", { reason }, "info", { emit: false });
    for (const delay of SETTINGS_REFRESH_BURST_DELAYS_MS) {
      scheduleSettingsRefresh({ delay });
    }
  }

  function settingsTriggerText(target) {
    const element =
      target instanceof Element
        ? target.closest('button, a, [role="button"], [role="menuitem"], [role="tab"], [aria-label], [title]')
        : null;
    if (!element || element.closest(`#${ROOT_ID}`)) {
      return "";
    }
    return [
      element.getAttribute("aria-label"),
      element.getAttribute("title"),
      normalizedText(element).slice(0, 120),
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
  }

  function targetLooksLikeSettingsTrigger(target) {
    const text = settingsTriggerText(target);
    return /\bsettings\b|\u8bbe\u7f6e/.test(text);
  }

  function refreshCodexLSettingsNav({ diagnostics = false, force = false } = {}) {
    if (!shouldScanSettingsDom(force)) {
      return false;
    }
    const shell = findSettingsShell();
    const nav = (shell && findSettingsNav(shell)) || findBestSettingsNavCandidate(document);
    if (!nav) {
      if (diagnostics) {
        settingsDebugLog(
          "CodexL settings nav not found",
          getSettingsDiagnostics({ shell, nav }),
          "info",
          { emit: false }
        );
      }
      if (hasCodexLSettingsInjection()) {
        removeCodexLSettingsInjection();
      }
      return false;
    }
    removeMisplacedSettingsNavItems(nav);
    const item = ensureSettingsNavItem(nav);
    settingsDebugLog("CodexL settings refresh result", {
      hasItem: !!item,
      labels: Array.from(settingsLabelSet(nav)),
      nav: elementDebugSummary(nav),
      shell: elementDebugSummary(shell),
    }, "info", { emit: false });
    updateSettingsUi();
    return true;
  }

  function installCodexLSettingsInjector() {
    if (runtime.settingsInjectorInstalled || !document.body) {
      return;
    }
    runtime.settingsInjectorInstalled = true;
    settingsDebugLog("CodexL settings injector installed", {
      build: RUNTIME_BUILD,
      href: window.location.href,
      version: RUNTIME_VERSION,
    });
    const onPotentialSettingsOpen = (event) => {
      if (targetLooksLikeSettingsTrigger(event.target)) {
        scheduleSettingsRefreshBurst("settings-interaction");
      }
    };
    const onPotentialSettingsKey = (event) => {
      if (
        (event.key === "Enter" || event.key === " ") &&
        targetLooksLikeSettingsTrigger(event.target)
      ) {
        scheduleSettingsRefreshBurst("settings-keyboard");
      }
    };
    const onLocationChange = () => {
      const key = settingsLocationKey();
      if (runtime.settingsLastLocationKey === key) {
        return;
      }
      runtime.settingsLastLocationKey = key;
      if (isSettingsRoute()) {
        scheduleSettingsRefreshBurst("settings-route");
      } else if (!settingsScanWindowActive() && hasCodexLSettingsInjection()) {
        removeCodexLSettingsInjection();
      }
    };
    const originalPushState = history.pushState;
    const originalReplaceState = history.replaceState;
    const dispatchLocationChange = () => {
      window.dispatchEvent(new Event("codexl-location-change"));
    };
    const wrappedPushState = function codexlPushState(...args) {
      const result = originalPushState.apply(this, args);
      window.setTimeout(dispatchLocationChange, 0);
      return result;
    };
    const wrappedReplaceState = function codexlReplaceState(...args) {
      const result = originalReplaceState.apply(this, args);
      window.setTimeout(dispatchLocationChange, 0);
      return result;
    };
    try {
      history.pushState = wrappedPushState;
      history.replaceState = wrappedReplaceState;
    } catch {}
    document.addEventListener("click", onPotentialSettingsOpen, true);
    document.addEventListener("keydown", onPotentialSettingsKey, true);
    window.addEventListener("codexl-location-change", onLocationChange, true);
    window.addEventListener("hashchange", onLocationChange, true);
    window.addEventListener("popstate", onLocationChange, true);
    runtime.cleanup.push(() => {
      clearSettingsRefreshTimers();
      document.removeEventListener("click", onPotentialSettingsOpen, true);
      document.removeEventListener("keydown", onPotentialSettingsKey, true);
      window.removeEventListener("codexl-location-change", onLocationChange, true);
      window.removeEventListener("hashchange", onLocationChange, true);
      window.removeEventListener("popstate", onLocationChange, true);
      if (history.pushState === wrappedPushState) {
        history.pushState = originalPushState;
      }
      if (history.replaceState === wrappedReplaceState) {
        history.replaceState = originalReplaceState;
      }
    });
    runtime.settingsLastLocationKey = "";
    onLocationChange();
  }

  function threadIdFromUrlLike(value) {
    if (typeof value !== "string" || !value.trim() || value.trim().startsWith('#')) {
      return null;
    }
    try {
      const url = new URL(value, window.location.href);
      if (url.origin !== window.location.origin) {
        return null;
      }
      const queryThreadId =
        url.searchParams.get("threadId") ||
        url.searchParams.get("conversationId") ||
        url.searchParams.get("sessionId") ||
        url.searchParams.get("id");
      if (queryThreadId) {
        return normalizeThreadId(queryThreadId);
      }
      const routeNames = new Set([
        "c",
        "chat",
        "conversation",
        "conversations",
        "local",
        "no-project",
        "no_project",
        "projectless",
        "projectless-session",
        "projectless-sessions",
        "remote",
        "session",
        "sessions",
        "thread",
        "threads",
      ]);
      const parts = url.pathname.split("/").filter(Boolean);
      for (let index = 0; index < parts.length - 1; index += 1) {
        if (!routeNames.has(parts[index])) {
          continue;
        }
        const decoded = decodeURIComponent(parts[index + 1]);
        const threadId = normalizeThreadId(decoded);
        if (threadId) {
          return threadId;
        }
      }
    } catch {}
    return null;
  }

  function toFiniteNumber(value) {
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    if (typeof value !== "string") {
      return null;
    }
    const normalized = value.replace(/,/g, "").trim();
    if (!normalized) {
      return null;
    }
    const parsed = Number(normalized);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function firstFiniteNumber(...values) {
    for (const value of values) {
      const number = toFiniteNumber(value);
      if (number != null) {
        return number;
      }
    }
    return null;
  }

  function firstObject(...values) {
    for (const value of values) {
      if (value && typeof value === "object") {
        return value;
      }
    }
    return null;
  }

  function clampPercent(value) {
    if (!Number.isFinite(value)) {
      return null;
    }
    return Math.max(0, Math.min(100, value));
  }

  function compactTokenCount(value) {
    const number = toFiniteNumber(value);
    if (number == null) {
      return null;
    }
    if (number >= 1000000) {
      return `${Math.round(number / 1000000)}M`;
    }
    if (number >= 1000) {
      return `${Math.round(number / 1000)}K`;
    }
    return String(Math.round(number));
  }

  function formatTokenCount(value) {
    const number = toFiniteNumber(value);
    if (number == null) {
      return null;
    }
    return Math.round(number).toLocaleString("en-US");
  }

  function makeContextUsage({ detail, percent, score = 1, source, total, used }) {
    const normalizedPercent = clampPercent(percent);
    if (normalizedPercent == null) {
      return null;
    }
    let text = detail;
    if (!text) {
      const usedText = formatTokenCount(used);
      const totalText = formatTokenCount(total);
      const remainingPercent = Math.max(0, 100 - normalizedPercent);
      text =
        usedText && totalText
          ? `Context ${Math.round(remainingPercent)}% left (${usedText} used / ${totalText} total)`
          : `Context ${Math.round(remainingPercent)}% left`;
    }
    return {
      detail: text,
      percent: normalizedPercent,
      score,
      source: source || "unknown",
      total,
      used,
    };
  }

  function normalizeThreadId(value) {
    if (typeof value === "string") {
      const trimmed = value.trim();
      return trimmed ? trimmed : null;
    }
    if (value && typeof value === "object") {
      return normalizeThreadId(value.threadId ?? value.conversationId ?? value.id);
    }
    return null;
  }

  function clearActiveContextUsage(reason = "unknown") {
    runtime.activeThreadId = null;
    runtime.latestContextUsage = null;
    runtime.lastContextClearReason = reason;
    scheduleContextIndicatorUpdate();
  }

  function rememberActiveThreadId(value, { clearWhenMissing = false } = {}) {
    const threadId = normalizeThreadId(value);
    if (threadId) {
      runtime.activeThreadId = threadId;
      scheduleContextIndicatorUpdate();
    } else if (clearWhenMissing) {
      clearActiveContextUsage("missing-thread-id");
    }
    return threadId;
  }

  function tokenUsageTotalTokens(usage) {
    if (!usage || typeof usage !== "object") {
      return null;
    }
    const direct = firstFiniteNumber(usage.totalTokens, usage.total_tokens, usage.total);
    if (direct != null) {
      return direct;
    }
    const input = firstFiniteNumber(
      usage.inputTokens,
      usage.input_tokens,
      usage.promptTokens,
      usage.prompt_tokens
    );
    const output = firstFiniteNumber(
      usage.outputTokens,
      usage.output_tokens,
      usage.completionTokens,
      usage.completion_tokens
    );
    if (input != null || output != null) {
      return (input || 0) + (output || 0);
    }
    return null;
  }

  function hasTokenUsageShape(value) {
    if (!value || typeof value !== "object") {
      return false;
    }
    return (
      firstFiniteNumber(
        value.modelContextWindow,
        value.model_context_window,
        value.contextWindow,
        value.context_window,
        value.maxContextWindow,
        value.max_context_window
      ) != null ||
      firstObject(
        value.last,
        value.lastTokenUsage,
        value.last_token_usage,
        value.totalTokenUsage,
        value.total_token_usage
      ) != null ||
      tokenUsageTotalTokens(value) != null
    );
  }

  function tokenUsageFromValue(value) {
    if (!value || typeof value !== "object") {
      return null;
    }
    if (value.type === "token_count" && value.info && typeof value.info === "object") {
      return value.info;
    }
    if (
      value.payload?.type === "token_count" &&
      value.payload.info &&
      typeof value.payload.info === "object"
    ) {
      return value.payload.info;
    }
    const nested = firstObject(
      value.latestTokenUsageInfo,
      value.latest_token_usage_info,
      value.tokenUsageInfo,
      value.token_usage_info,
      value.tokenUsage,
      value.token_usage,
      value.usageInfo,
      value.usage_info,
      value.usage
    );
    if (nested) {
      return nested;
    }
    return hasTokenUsageShape(value) ? value : null;
  }

  function contextUsageFromTokenUsage(tokenUsage, source) {
    const normalizedTokenUsage = tokenUsageFromValue(tokenUsage);
    if (!normalizedTokenUsage || typeof normalizedTokenUsage !== "object") {
      return null;
    }
    const last =
      firstObject(
        normalizedTokenUsage.last,
        normalizedTokenUsage.lastTokenUsage,
        normalizedTokenUsage.last_token_usage
      ) ||
      firstObject(
        normalizedTokenUsage.totalTokenUsage,
        normalizedTokenUsage.total_token_usage
      ) || normalizedTokenUsage;
    const contextWindow = firstFiniteNumber(
      normalizedTokenUsage.modelContextWindow,
      normalizedTokenUsage.model_context_window,
      normalizedTokenUsage.contextWindow,
      normalizedTokenUsage.context_window,
      normalizedTokenUsage.maxContextWindow,
      normalizedTokenUsage.max_context_window
    );
    const totalTokens = tokenUsageTotalTokens(last);
    if (contextWindow == null || contextWindow <= 0 || totalTokens == null || totalTokens < 0) {
      return null;
    }
    const used = Math.min(totalTokens, contextWindow);
    return makeContextUsage({
      percent: (used / contextWindow) * 100,
      score: 10,
      source,
      total: contextWindow,
      used,
    });
  }

  function rememberContextUsageForThread(threadId, tokenUsage, source) {
    const usage = contextUsageFromTokenUsage(tokenUsage, source);
    if (!usage) {
      return false;
    }
    const normalizedThreadId = normalizeThreadId(threadId);
    if (normalizedThreadId) {
      runtime.contextUsageByThread.set(normalizedThreadId, usage);
      if (threadIdFromLocation() === normalizedThreadId) {
        runtime.activeThreadId = normalizedThreadId;
      }
    }
    runtime.latestContextUsage = usage;
    scheduleContextIndicatorUpdate();
    return true;
  }

  function tokenUsageFromConversationState(conversationState) {
    if (!conversationState || typeof conversationState !== "object") {
      return null;
    }
    return tokenUsageFromValue(conversationState);
  }

  function threadStreamStateParams(message) {
    if (!message || typeof message !== "object") {
      return null;
    }
    if (message.type === "thread-stream-state-changed") {
      return message;
    }
    if (
      message.type === "ipc-broadcast" &&
      message.method === "thread-stream-state-changed" &&
      message.params &&
      typeof message.params === "object"
    ) {
      return message.params;
    }
    if (
      message.type === "mcp-notification" &&
      message.method === "thread-stream-state-changed" &&
      message.params &&
      typeof message.params === "object"
    ) {
      return message.params;
    }
    return null;
  }

  function scanAppServerTokenUsage(value, threadId, source, depth = 0, seen = new WeakSet()) {
    if (!value || typeof value !== "object" || depth > 4) {
      return false;
    }
    if (seen.has(value)) {
      return false;
    }
    seen.add(value);
    let found = false;
    const currentThreadId = normalizeThreadId(value.threadId ?? value.conversationId ?? value.id) || threadId;
    const ownTokenUsage = tokenUsageFromValue(value);
    if (ownTokenUsage) {
      found = rememberContextUsageForThread(currentThreadId, ownTokenUsage, source) || found;
    }
    if (Array.isArray(value)) {
      for (const item of value.slice(0, 50)) {
        found = scanAppServerTokenUsage(item, currentThreadId, source, depth + 1, seen) || found;
      }
      return found;
    }
    for (const key of Object.keys(value).slice(0, 80)) {
      let child;
      try {
        child = value[key];
      } catch {
        continue;
      }
      if (
        key === "latestTokenUsageInfo" ||
        key === "latest_token_usage_info" ||
        key === "tokenUsageInfo" ||
        key === "token_usage_info" ||
        key === "tokenUsage" ||
        key === "token_usage" ||
        key === "usage"
      ) {
        found = rememberContextUsageForThread(currentThreadId, child, source) || found;
        continue;
      }
      if (child && typeof child === "object") {
        found = scanAppServerTokenUsage(child, currentThreadId, source, depth + 1, seen) || found;
      }
    }
    return found;
  }

  function handleThreadStreamStateChange(message) {
    const params = threadStreamStateParams(message);
    if (!params) {
      return false;
    }
    const threadId = normalizeThreadId(params.conversationId ?? params.threadId);
    const change = params.change;
    if (!change || typeof change !== "object") {
      return true;
    }
    if (change.type === "snapshot") {
      const conversationState = change.conversationState;
      const snapshotThreadId = normalizeThreadId(conversationState?.id ?? threadId);
      if (snapshotThreadId) {
        rememberActiveThreadId(snapshotThreadId);
      }
      const tokenUsage = tokenUsageFromConversationState(conversationState);
      if (tokenUsage) {
        rememberContextUsageForThread(
          snapshotThreadId ?? threadId,
          tokenUsage,
          "app-server:thread-stream-state-changed"
        );
      }
      return true;
    }
    if (threadId) {
      rememberActiveThreadId(threadId);
    }
    scanAppServerTokenUsage(change, threadId, "app-server:thread-stream-state-changed");
    return true;
  }

  function handleMcpNotification(message) {
    if (!message || message.type !== "mcp-notification") {
      return false;
    }
    const method = message.method || "";
    const params = message.params && typeof message.params === "object" ? message.params : {};
    if (method === "thread/tokenUsage/updated") {
      rememberContextUsageForThread(
        params.threadId ?? params.conversationId,
        tokenUsageFromValue(params),
        "app-server:thread/tokenUsage/updated"
      );
      return true;
    }
    if (method === "thread/started") {
      rememberActiveThreadId(params.thread ?? params.threadId ?? params.conversationId, {
        clearWhenMissing: true,
      });
      return true;
    }
    return false;
  }

  function handleMcpRequestOrResponse(message) {
    if (!message || typeof message !== "object") {
      return false;
    }
    if (message.type === "mcp-request") {
      const request = message.request;
      const params = request?.params;
      if (request?.method === "thread/start") {
        const threadId = rememberActiveThreadId(params?.threadId ?? params?.conversationId);
        if (!threadId) {
          clearActiveContextUsage("thread-start");
        }
      } else if (request?.method === "thread/resume") {
        rememberActiveThreadId(params?.threadId ?? params?.conversationId, {
          clearWhenMissing: true,
        });
      }
      return false;
    }
    if (message.type === "mcp-response") {
      const result = message.message?.result;
      rememberActiveThreadId(result?.thread?.id ?? result?.threadId);
      return false;
    }
    return false;
  }

  function handleCodexAppServerMessage(message) {
    if (!message || typeof message !== "object") {
      return;
    }
    if (handleThreadStreamStateChange(message)) {
      return;
    }
    if (handleMcpNotification(message)) {
      return;
    }
    if (
      scanAppServerTokenUsage(
        message,
        normalizeThreadId(message.threadId ?? message.conversationId ?? message.id),
        "app-server:message"
      )
    ) {
      return;
    }
    handleMcpRequestOrResponse(message);
  }

  function replayBufferedCodexAppServerMessages() {
    try {
      const queued = window.__codexWebBridgeNotifications?.messages;
      if (Array.isArray(queued)) {
        for (const entry of queued.slice(-256)) {
          handleCodexAppServerMessage(entry?.message ?? entry);
        }
      }
    } catch {}
    try {
      const snapshots = window.__codexWebBridgeLatestThreadStreamSnapshots;
      if (snapshots && typeof snapshots === "object") {
        for (const snapshot of Object.values(snapshots)) {
          handleCodexAppServerMessage(snapshot);
        }
      }
    } catch {}
  }

  function installCodexAppServerContextBridge() {
    if (runtime.contextMessageBridgeInstalled) {
      return;
    }
    runtime.contextMessageBridgeInstalled = true;
    const onWindowMessage = (event) => {
      handleCodexAppServerMessage(event?.data);
    };
    const onMessageFromView = (event) => {
      handleCodexAppServerMessage(event?.detail);
    };
    window.addEventListener("message", onWindowMessage, true);
    window.addEventListener("codex-message-from-view", onMessageFromView, true);
    runtime.cleanup.push(() => window.removeEventListener("message", onWindowMessage, true));
    runtime.cleanup.push(() =>
      window.removeEventListener("codex-message-from-view", onMessageFromView, true)
    );
    replayBufferedCodexAppServerMessages();
  }

  function threadIdFromLocation() {
    return threadIdFromUrlLike(window.location.href);
  }

  function isNewThreadLocation() {
    const pathname = window.location.pathname.replace(/\/+$/, "") || "/";
    return pathname === "/" || pathname === "/local" || pathname === "/remote";
  }

  function syncActiveThreadFromLocation() {
    const locationKey = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (runtime.lastContextLocationKey === locationKey) {
      return;
    }
    runtime.lastContextLocationKey = locationKey;
    const locationThreadId = threadIdFromLocation();
    if (locationThreadId) {
      runtime.activeThreadId = locationThreadId;
      return;
    }
    if (isNewThreadLocation()) {
      runtime.activeThreadId = null;
    }
  }

  function refreshContextUsageFromSession(threadId) {
    const normalizedThreadId = normalizeThreadId(threadId);
    if (!normalizedThreadId) {
      return;
    }
    const now = Date.now();
    const state = runtime.contextUsageSessionRefreshByThread.get(normalizedThreadId) || {};
    if (state.pending || now - (state.lastAttempt || 0) < 5000) {
      return;
    }
    runtime.contextUsageSessionRefreshByThread.set(normalizedThreadId, {
      ...state,
      lastAttempt: now,
      pending: true,
    });
    request("session:context-usage", { threadId: normalizedThreadId })
      .then((response) => {
        const tokenUsage = tokenUsageFromValue(response);
        if (tokenUsage) {
          rememberContextUsageForThread(
            response?.threadId ?? normalizedThreadId,
            tokenUsage,
            "session:context-usage"
          );
        }
      })
      .catch(() => {})
      .finally(() => {
        const latest = runtime.contextUsageSessionRefreshByThread.get(normalizedThreadId) || {};
        runtime.contextUsageSessionRefreshByThread.set(normalizedThreadId, {
          ...latest,
          pending: false,
        });
      });
  }

  function currentContextUsage() {
    syncActiveThreadFromLocation();
    const threadId = normalizeThreadId(runtime.activeThreadId);
    if (threadId) {
      const usage = runtime.contextUsageByThread.get(threadId) || null;
      if (!usage) {
        refreshContextUsageFromSession(threadId);
      }
      return usage;
    }
    return null;
  }

  function findComposerInput() {
    const selector = [
      'textarea:not([disabled]):not([readonly])',
      '[contenteditable="true"]',
      '[role="textbox"]',
      'input[type="text"]:not([disabled]):not([readonly])',
    ].join(",");
    const inputs = Array.from(document.querySelectorAll(selector)).filter((element) => {
      if (!isElementVisible(element) || element.closest(`#${ROOT_ID}`)) {
        return false;
      }
      const rect = element.getBoundingClientRect();
      return rect.width >= 160 && rect.height >= 18 && rect.bottom >= window.innerHeight * 0.45;
    });
    inputs.sort((first, second) => {
      const firstRect = first.getBoundingClientRect();
      const secondRect = second.getBoundingClientRect();
      return secondRect.bottom - firstRect.bottom || secondRect.width - firstRect.width;
    });
    return inputs[0] || null;
  }

  function findComposerContainer(input) {
    if (!input) {
      return null;
    }
    const inputRect = input.getBoundingClientRect();
    let node = input.parentElement;
    let best = node;
    let depth = 0;
    while (node && node !== document.body && depth < 8) {
      const rect = node.getBoundingClientRect();
      const nearInput = rect.bottom >= inputRect.bottom - 24 && rect.top <= inputRect.top + 24;
      const plausibleSize = rect.width >= inputRect.width && rect.height <= 260;
      if (nearInput && plausibleSize) {
        best = node;
      }
      if (
        node.matches?.('form, [role="form"], [data-testid*="composer"], [class*="composer"]')
      ) {
        return node;
      }
      node = node.parentElement;
      depth += 1;
    }
    return best;
  }

  function composerSendButtonScore(button, input) {
    if (!(button instanceof HTMLButtonElement) || !isElementVisible(button)) {
      return 0;
    }
    if (button.closest(`#${ROOT_ID}`) || button.closest(`[${SETTINGS_PANEL_ATTR}="1"]`)) {
      return 0;
    }
    const rect = button.getBoundingClientRect();
    if (
      rect.width < 22 ||
      rect.width > 48 ||
      rect.height < 22 ||
      rect.height > 48 ||
      rect.bottom < window.innerHeight * 0.45
    ) {
      return 0;
    }
    const accessibleText = [
      button.getAttribute("aria-label"),
      button.getAttribute("title"),
      normalizedText(button),
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    if (
      /add|attach|file|dictate|microphone|voice|settings|archive|pin|filter|new chat/.test(
        accessibleText
      )
    ) {
      return 0;
    }
    const className = String(button.className || "");
    let score = 1;
    if (className.includes("size-token-button-composer")) {
      score += 50;
    }
    if (className.includes("bg-token-foreground")) {
      score += 80;
    }
    if (className.includes("rounded-full")) {
      score += 10;
    }
    if (!button.getAttribute("aria-label")) {
      score += 10;
    }
    if (button.querySelector("svg")) {
      score += 10;
    }
    if (input) {
      const inputRect = input.getBoundingClientRect();
      if (Math.abs(rect.bottom - inputRect.bottom) <= 48 || rect.top >= inputRect.top) {
        score += 25;
      }
      if (rect.left >= inputRect.left) {
        score += 10;
      }
    }
    return score;
  }

  function findComposerSendButton(input) {
    if (!input) {
      return null;
    }
    const roots = [findComposerContainer(input), document].filter(Boolean);
    const seen = new Set();
    const candidates = [];
    for (const root of roots) {
      for (const button of Array.from(root.querySelectorAll("button"))) {
        if (seen.has(button)) {
          continue;
        }
        seen.add(button);
        const score = composerSendButtonScore(button, input);
        if (score > 0) {
          candidates.push({ button, score });
        }
      }
    }
    candidates.sort((first, second) => {
      if (second.score !== first.score) {
        return second.score - first.score;
      }
      return (
        second.button.getBoundingClientRect().right -
        first.button.getBoundingClientRect().right
      );
    });
    return candidates[0]?.score >= 80 ? candidates[0].button : null;
  }

  function restoreContextParentPosition(parent) {
    if (!parent || parent.dataset.codexlContextPosition !== "1") {
      return;
    }
    parent.style.position = parent.dataset.codexlOriginalPosition || "";
    delete parent.dataset.codexlContextPosition;
    delete parent.dataset.codexlOriginalPosition;
  }

  function removeContextIndicator() {
    const indicator = document.getElementById(CONTEXT_INDICATOR_ID);
    if (!indicator) {
      return;
    }
    const parent = indicator.parentElement;
    indicator.remove();
    restoreContextParentPosition(parent);
    removeContextTooltip();
  }

  function ensureContextIndicator(sendButton) {
    const container = sendButton?.parentElement;
    if (!container) {
      return null;
    }
    let indicator = document.getElementById(CONTEXT_INDICATOR_ID);
    if (!indicator) {
      indicator = document.createElement("div");
      indicator.id = CONTEXT_INDICATOR_ID;
      indicator.innerHTML = `
        <svg aria-hidden="true" viewBox="0 0 14 14">
          <title data-codexl-context-title>Context usage unavailable</title>
          <circle cx="7" cy="7" r="5" stroke="currentColor" stroke-width="2" fill="none" opacity=".16"></circle>
          <circle data-codexl-context-progress cx="7" cy="7" r="5" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" transform="rotate(-90 7 7)"></circle>
        </svg>
      `;
    }
    bindContextIndicatorTooltip(indicator);
    if (indicator.parentElement !== container || indicator.nextElementSibling !== sendButton) {
      restoreContextParentPosition(indicator.parentElement);
      container.insertBefore(indicator, sendButton);
    }
    return indicator;
  }

  function removeContextTooltip() {
    document.getElementById(CONTEXT_TOOLTIP_ID)?.remove();
  }

  function positionContextTooltip(indicator, tooltip) {
    const indicatorRect = indicator.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    const gap = 8;
    const left = Math.max(
      8,
      Math.min(
        window.innerWidth - tooltipRect.width - 8,
        indicatorRect.left + indicatorRect.width / 2 - tooltipRect.width / 2
      )
    );
    const top =
      indicatorRect.top - tooltipRect.height - gap >= 8
        ? indicatorRect.top - tooltipRect.height - gap
        : indicatorRect.bottom + gap;
    tooltip.style.left = `${Math.round(left)}px`;
    tooltip.style.top = `${Math.round(top)}px`;
  }

  function showContextTooltip(indicator) {
    const detail = indicator.dataset.contextDetail || indicator.title || "";
    if (!detail) {
      return;
    }
    let tooltip = document.getElementById(CONTEXT_TOOLTIP_ID);
    if (!tooltip) {
      tooltip = document.createElement("div");
      tooltip.id = CONTEXT_TOOLTIP_ID;
      document.body.appendChild(tooltip);
    }
    tooltip.textContent = detail;
    positionContextTooltip(indicator, tooltip);
  }

  function bindContextIndicatorTooltip(indicator) {
    if (indicator.dataset.codexlTooltipBound === "1") {
      return;
    }
    indicator.dataset.codexlTooltipBound = "1";
    indicator.addEventListener("mouseenter", () => showContextTooltip(indicator));
    indicator.addEventListener("mousemove", () => showContextTooltip(indicator));
    indicator.addEventListener("mouseleave", removeContextTooltip);
    indicator.addEventListener("blur", removeContextTooltip);
  }

  function setContextIndicatorTitle(indicator, title) {
    indicator.title = title;
    indicator.dataset.contextDetail = title;
    indicator.setAttribute("aria-label", title);
    const svg = indicator.querySelector("svg");
    svg?.setAttribute("aria-label", title);
    const svgTitle = indicator.querySelector("[data-codexl-context-title]");
    if (svgTitle) {
      svgTitle.textContent = title;
    }
  }

  function renderContextIndicator(indicator, usage) {
    const progress = indicator.querySelector("[data-codexl-context-progress]");
    const circumference = 2 * Math.PI * 5;
    progress.setAttribute("stroke-dasharray", String(circumference));
    if (!usage || usage.percent == null) {
      progress.setAttribute("stroke-dashoffset", String(circumference));
      indicator.dataset.state = "unknown";
      setContextIndicatorTitle(indicator, "Context usage unavailable");
      return;
    }

    const percent = clampPercent(usage.percent) ?? 0;
    progress.setAttribute(
      "stroke-dashoffset",
      String(circumference * (1 - percent / 100))
    );
    const state = percent >= 85 ? "high" : percent >= 60 ? "medium" : "low";
    indicator.dataset.state = state;
    setContextIndicatorTitle(
      indicator,
      usage.detail || `Context ${Math.round(Math.max(0, 100 - percent))}% left`
    );
  }

  function updateContextIndicator() {
    if (!runtime.codexlSettings.showContextIndicators || !document.body) {
      removeContextIndicator();
      return;
    }
    const input = findComposerInput();
    const sendButton = findComposerSendButton(input);
    if (!sendButton) {
      removeContextIndicator();
      return;
    }
    const usage = currentContextUsage();
    if (!usage) {
      removeContextIndicator();
      return;
    }
    const indicator = ensureContextIndicator(sendButton);
    if (indicator) {
      renderContextIndicator(indicator, usage);
    }
  }

  function scheduleContextIndicatorUpdate() {
    if (!runtime.codexlSettings.showContextIndicators || !document.body) {
      removeContextIndicator();
      return;
    }
    if (runtime.contextIndicatorUpdateScheduled) {
      return;
    }
    runtime.contextIndicatorUpdateScheduled = true;
    window.requestAnimationFrame(() => {
      runtime.contextIndicatorUpdateScheduled = false;
      updateContextIndicator();
    });
  }

  function scheduleContextIndicatorBurst() {
    for (const timer of runtime.contextIndicatorTimers || []) {
      window.clearTimeout(timer);
    }
    runtime.contextIndicatorTimers = [];
    for (const delay of CONTEXT_INDICATOR_BURST_DELAYS_MS) {
      const timer = window.setTimeout(scheduleContextIndicatorUpdate, delay);
      runtime.contextIndicatorTimers.push(timer);
    }
  }

  function uninstallContextIndicator() {
    for (const timer of runtime.contextIndicatorTimers || []) {
      window.clearTimeout(timer);
    }
    runtime.contextIndicatorTimers = [];
    for (const cleanup of runtime.contextIndicatorCleanup.splice(0)) {
      try {
        cleanup();
      } catch {}
    }
    runtime.contextIndicatorInstalled = false;
    runtime.contextIndicatorUpdateScheduled = false;
    removeContextIndicator();
  }

  function installContextIndicator() {
    if (!runtime.codexlSettings.showContextIndicators || !document.body) {
      uninstallContextIndicator();
      return;
    }
    if (runtime.contextIndicatorInstalled) {
      scheduleContextIndicatorBurst();
      return;
    }
    runtime.contextIndicatorInstalled = true;
    const onLightweightUiChange = () => scheduleContextIndicatorUpdate();
    const onRouteOrViewportChange = () => scheduleContextIndicatorBurst();
    document.addEventListener("focusin", onLightweightUiChange, true);
    document.addEventListener("input", onLightweightUiChange, true);
    window.addEventListener("resize", onRouteOrViewportChange, true);
    window.addEventListener("codexl-location-change", onRouteOrViewportChange, true);
    window.addEventListener("hashchange", onRouteOrViewportChange, true);
    window.addEventListener("popstate", onRouteOrViewportChange, true);
    runtime.contextIndicatorCleanup.push(() => {
      document.removeEventListener("focusin", onLightweightUiChange, true);
      document.removeEventListener("input", onLightweightUiChange, true);
      window.removeEventListener("resize", onRouteOrViewportChange, true);
      window.removeEventListener("codexl-location-change", onRouteOrViewportChange, true);
      window.removeEventListener("hashchange", onRouteOrViewportChange, true);
      window.removeEventListener("popstate", onRouteOrViewportChange, true);
    });
    scheduleContextIndicatorBurst();
  }

  function syncContextIndicator() {
    if (runtime.codexlSettings.showContextIndicators) {
      installContextIndicator();
    } else {
      uninstallContextIndicator();
    }
  }

  function mount() {
    if (!document.body) {
      window.setTimeout(mount, 50);
      return;
    }
    installGlobalStyle();
    installCodexAppServerContextBridge();
    installCodexLSettingsInjector();
    installContextIndicator();
    if (document.getElementById(ROOT_ID)) {
      updateUi();
      return;
    }
    const host = document.createElement("div");
    host.id = ROOT_ID;
    host.style.position = "fixed";
    host.style.zIndex = "2147483647";
    host.style.right = "16px";
    host.style.bottom = "16px";
    document.body.appendChild(host);
    const shadow = host.attachShadow({ mode: "open" });
    shadow.innerHTML = `
      <style>
        :host { color-scheme: light dark; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
        button { font: inherit; }
        .panel {
          display: none; width: 320px; margin-bottom: 8px; border: 1px solid rgba(120, 120, 120, .28); border-radius: 8px;
          background: color-mix(in srgb, Canvas 94%, transparent); color: CanvasText; box-shadow: 0 18px 42px rgba(0, 0, 0, .28);
          overflow: hidden; backdrop-filter: blur(16px);
        }
        .panel.open { display: block; }
        .header { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 10px 12px; border-bottom: 1px solid rgba(120, 120, 120, .22); }
        .title { font-size: 13px; font-weight: 650; }
        .close { border: 0; background: transparent; color: inherit; cursor: pointer; font-size: 18px; line-height: 1; }
        .body { padding: 10px 12px; display: grid; gap: 8px; font-size: 12px; }
        .row { display: flex; justify-content: space-between; gap: 12px; }
        .label { color: color-mix(in srgb, CanvasText 62%, transparent); }
        .value { font-variant-numeric: tabular-nums; text-align: right; }
        .actions { display: flex; gap: 8px; padding-top: 2px; }
        .action { border: 1px solid rgba(120, 120, 120, .35); border-radius: 6px; background: transparent; color: inherit; cursor: pointer; padding: 5px 8px; }
        .panels { display: grid; gap: 8px; }
      </style>
      <section class="panel" part="panel">
        <div class="header">
          <div class="title">CodexL Plugins</div>
          <button class="close" title="Close" aria-label="Close">&times;</button>
        </div>
        <div class="body">
          <div class="row"><span class="label">Bridge</span><span class="value" data-field="status">booting</span></div>
          <div class="row"><span class="label">Detail</span><span class="value" data-field="detail"></span></div>
          <div class="row"><span class="label">Plugins</span><span class="value" data-field="plugins">0</span></div>
          <div class="row"><span class="label">React renderers</span><span class="value" data-field="renderers">0</span></div>
          <div class="row"><span class="label">Fiber fallback</span><span class="value" data-field="fiber">ready</span></div>
          <div class="actions">
            <button class="action" data-action="ping">Ping</button>
            <button class="action" data-action="reload">Reload</button>
          </div>
          <div class="panels" data-panels></div>
        </div>
      </section>
    `;
    runtime.ui = {
      host,
      panel: shadow.querySelector(".panel"),
      detail: shadow.querySelector('[data-field="detail"]'),
      panels: shadow.querySelector("[data-panels]"),
      plugins: shadow.querySelector('[data-field="plugins"]'),
      renderers: shadow.querySelector('[data-field="renderers"]'),
      status: shadow.querySelector('[data-field="status"]'),
    };
    shadow.querySelector(".close").addEventListener("click", () => {
      runtime.ui.panel.classList.remove("open");
    });
    shadow.querySelector('[data-action="reload"]').addEventListener("click", () => {
      window.location.reload();
    });
    shadow.querySelector('[data-action="ping"]').addEventListener("click", () => {
      request("ping").catch((error) => log("error", "Ping failed", String(error)));
    });
    updateUi();
    updatePanels();
  }

  function updatePanels() {
    if (!runtime.ui || !runtime.ui.panels) {
      return;
    }
    runtime.ui.panels.textContent = "";
    for (const [id, panel] of runtime.panels) {
      const container = document.createElement("div");
      container.setAttribute("data-panel-id", id);
      try {
        panel.render(container, runtime);
      } catch (error) {
        container.textContent = `${panel.title}: ${String(error)}`;
      }
      runtime.ui.panels.appendChild(container);
    }
  }

  function updateUi() {
    if (!runtime.ui) {
      return;
    }
    runtime.ui.status.textContent = runtime.status;
    runtime.ui.detail.textContent = runtime.statusDetail || "-";
    runtime.ui.plugins.textContent = String(runtime.loadedPlugins.size);
    runtime.ui.renderers.textContent = String(runtime.renderers.size);
  }

  async function loadPlugins() {
    if (runtime.pluginsLoaded) {
      return;
    }
    runtime.pluginsLoaded = true;
    const response = await request("plugin:list");
    const plugins = Array.isArray(response?.plugins) ? response.plugins : [];
    for (const plugin of plugins) {
      try {
        await startPlugin(plugin);
      } catch (error) {
        log("error", `Failed to start plugin ${plugin?.id || "<unknown>"}`, String(error));
      }
    }
    updateUi();
  }

  async function startPlugin(plugin) {
    if (!plugin || !plugin.id || runtime.loadedPlugins.has(plugin.id)) {
      return;
    }
    const api = createPluginApi(plugin);
    const module = { exports: {} };
    const exports = module.exports;
    const sourceUrl = plugin.sourceUrl || `codexl-renderer-plugin://${plugin.id}/index.js`;
    const source = `${plugin.source || ""}\n//# sourceURL=${sourceUrl}`;
    const factory = new Function("api", "module", "exports", source);
    const returned = factory(api, module, exports);
    const exported = returned || module.exports.default || module.exports;
    if (exported && typeof exported.start === "function") {
      await exported.start(api);
    } else if (typeof exported === "function") {
      await exported(api);
    }
    runtime.loadedPlugins.set(plugin.id, {
      api,
      exports: exported,
      manifest: plugin,
    });
    log("info", `Loaded plugin ${plugin.id}`);
  }

  function createPluginApi(plugin) {
    const pluginId = plugin.id;
    return {
      bridge: runtime.bridge,
      id: pluginId,
      log: {
        error: (message, detail) => log("error", `[${pluginId}] ${message}`, detail),
        info: (message, detail) => log("info", `[${pluginId}] ${message}`, detail),
      },
      manifest: plugin,
      react: runtime.react,
      registerPanel: (id, title, render) => {
        registerPanel(`${pluginId}:${id}`, title, (container) => render(container, api));
      },
      storage: {
        get: async (key) => {
          const response = await request("storage:get", { key, pluginId });
          return response?.value ?? null;
        },
        remove: (key) => request("storage:remove", { key, pluginId }),
        set: (key, value) => request("storage:set", { key, pluginId, value }),
      },
      ui: {
        close: () => runtime.ui?.panel?.classList.remove("open"),
        open: () => runtime.ui?.panel?.classList.add("open"),
      },
    };
  }

  runtime.bridge = { request, send, status: () => runtime.status };
  runtime.connect = connect;
  runtime.mount = mount;
  runtime.acceptCdpResponse = acceptCdpResponse;
  runtime.reconfigure = reconfigure;
  runtime.teardown = teardown;
  runtime.react = {
    fiberName,
    findOwnerByName,
    getFiber,
    renderers: () => Array.from(runtime.renderers.keys()),
  };
  runtime.registerPanel = registerPanel;
  Object.defineProperty(window, "codexlPlugins", {
    configurable: true,
    value: {
      bridge: runtime.bridge,
      react: runtime.react,
      registerPanel,
      runtime,
      settings: {
        debugSettings: () => {
          const diagnostics = getSettingsDiagnostics({
            nav: findBestSettingsNavCandidate(document),
            shell: findSettingsShell(),
          });
          settingsDebugLog("manual settings diagnostics", diagnostics);
          return diagnostics;
        },
        setShowContextIndicators,
        values: runtime.codexlSettings,
      },
      version: RUNTIME_VERSION,
    },
  });

  installTranscribeFetchInterceptor();
  installDesktopApiHostModelListInterceptor();
  installGatewayModelQuerySelectorRepair();
  installDesktopApiTranscribeInterceptor();
  installReactHook();
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mount, { once: true });
  } else {
    mount();
  }
  connect();

  return { ok: true, installed: true, version: RUNTIME_VERSION };
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_bootstrap_installs_react_hook_and_bridge() {
        let script = codex_plugin_bootstrap_script("ws://127.0.0.1:14588/plugin/_bridge?token=t");

        assert!(script.contains("__REACT_DEVTOOLS_GLOBAL_HOOK__"));
        assert!(script.contains("__codexlPluginBridge"));
        assert!(script.contains("acceptCdpResponse"));
        assert!(script.contains("codexlPlugins"));
        assert!(script.contains("plugin:list"));
        assert!(script.contains("storage:get"));
        assert!(script.contains("transcribe:fetch"));
        assert!(script.contains("models:list-for-host"));
        assert!(script.contains("ide-context"));
        assert!(script.contains("installTranscribeFetchInterceptor"));
        assert!(script.contains("installDesktopApiHostModelListInterceptor"));
        assert!(script.contains("installGatewayModelQuerySelectorRepair"));
        assert!(script.contains("codex-message-from-view"));
        assert!(script.contains("installDesktopApiTranscribeInterceptor"));
        assert!(script.contains("installClipboardFallback"));
        assert!(script.contains("fallbackCopyText"));
        assert!(script.contains("setting-storage-"));
        assert!(script.contains("new WebSocket"));
        assert!(script.contains("ws://127.0.0.1:14588/plugin/_bridge?token=t"));
        assert!(!script.contains("__CODEXL_PLUGIN_BRIDGE_URL__"));
    }

    #[test]
    fn plugin_bootstrap_injects_codexl_settings_and_context_indicator() {
        let script = codex_plugin_bootstrap_script("ws://127.0.0.1:14588/plugin/_bridge?token=t");

        assert!(script.contains("CodexL"));
        assert!(script.contains("Show Context Indicators"));
        assert!(script.contains("Show All Sessions"));
        assert!(script.contains("showContextIndicators"));
        assert!(script.contains("showAllSessions"));
        assert!(script.contains("codexl-context-indicator"));
        assert!(script.contains("findComposerSendButton"));
        assert!(script.contains("container.insertBefore(indicator, sendButton)"));
        assert!(script.contains("session:context-usage"));
        assert!(script.contains("refreshContextUsageFromSession"));
        assert!(script.contains("token_count"));
        assert!(script.contains("thread/tokenUsage/updated"));
        assert!(script.contains("modelContextWindow"));
        assert!(script.contains("model_context_window"));
        assert!(script.contains("tokenUsageTotalTokens"));
        assert!(script.contains("last_token_usage"));
        assert!(script.contains("latestTokenUsageInfo"));
        assert!(script.contains("tokenUsageInfo"));
        assert!(script.contains("threadIdFromLocation"));
        assert!(script.contains("scheduleSettingsRefreshBurst"));
        assert!(script.contains("syncContextIndicator"));
        assert!(script.contains("toLocaleString(\"en-US\")"));
        assert!(script.contains("data-codexl-context-title"));
        assert!(script.contains("codexl-context-indicator-tooltip"));
        assert!(script.contains("bindContextIndicatorTooltip"));
        assert!(script.contains("clearActiveContextUsage"));
        assert!(script.contains("clearActiveContextUsage(\"thread-start\")"));
        assert!(script.contains("const usage = runtime.contextUsageByThread.get(threadId) || null"));
        assert!(script.contains("if (!usage)"));
        assert!(!script.contains("new MutationObserver"));
        assert!(!script.contains("setInterval(refreshCodexLSettingsNav"));
        assert!(!script.contains("setInterval(updateContextIndicator"));
        assert!(script.contains("background: transparent"));
        assert!(script.contains("border: 0"));
        assert!(!script.contains("right: 48px"));
        assert!(!script.contains("bottom: 12px"));
        assert!(!script.contains("contextWindow ??"));
        assert!(!script.contains("lastTotalTokens"));
        assert!(!script.contains("if (!runtime.activeThreadId)"));
        assert!(!script.contains("return runtime.latestContextUsage || null"));
        assert!(
            !script.contains("border: 1px solid color-mix(in srgb, currentColor 18%, transparent)")
        );
        assert!(!script.contains("collectDomContextUsage"));
        assert!(!script.contains("parseContextUsageText"));
    }

    #[test]
    fn wildcard_http_host_maps_to_loopback_for_browser_bridge() {
        assert_eq!(
            plugin_bridge_url("0.0.0.0", 14588, "abc"),
            "ws://127.0.0.1:14588/plugin/_bridge?token=abc"
        );
        assert_eq!(
            plugin_bridge_url("::1", 14588, "abc"),
            "ws://[::1]:14588/plugin/_bridge?token=abc"
        );
    }

    #[test]
    fn plugin_bridge_rejects_missing_or_unknown_token() {
        assert!(!plugin_bridge_token_valid(None));
        assert!(!plugin_bridge_token_valid(Some("token=missing")));
    }

    #[test]
    fn plugin_bridge_accepts_registered_token() {
        let token = register_plugin_bridge_token();
        assert!(plugin_bridge_token_valid(Some(&format!("token={}", token))));
    }

    #[test]
    fn latest_session_context_usage_reads_token_count_events() {
        let dir = std::env::temp_dir().join(format!("codexl-context-{}", random_token()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("session.jsonl");
        fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"thread-1"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":42},"model_context_window":1000}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":50,"output_tokens":7},"model_context_window":2000}}}
"#,
        )
        .expect("write session");

        let usage = latest_session_context_usage(&path).expect("token usage");

        assert_eq!(
            usage
                .pointer("/last_token_usage/input_tokens")
                .and_then(Value::as_i64),
            Some(50)
        );
        assert_eq!(
            usage.get("model_context_window").and_then(Value::as_i64),
            Some(2000)
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn session_file_matches_local_prefixed_thread_ids() {
        assert!(session_thread_id_matches(
            "0123456789abcdef",
            "local:0123456789abcdef"
        ));
        assert!(session_thread_id_matches(
            "local:0123456789abcdef",
            "0123456789abcdef"
        ));
        assert!(!session_thread_id_matches(
            "0123456789abcdef",
            "local:fedcba9876543210"
        ));
    }

    #[test]
    fn renderer_plugin_entry_paths_must_be_relative_and_safe() {
        assert_eq!(
            safe_relative_path("index.js"),
            Some(PathBuf::from("index.js"))
        );
        assert_eq!(
            safe_relative_path("dist/plugin.js"),
            Some(PathBuf::from("dist").join("plugin.js"))
        );
        assert_eq!(safe_relative_path("../plugin.js"), None);
        assert_eq!(safe_relative_path("/tmp/plugin.js"), None);
    }

    #[test]
    fn renderer_plugin_ids_are_constrained_for_storage_paths() {
        assert!(validate_plugin_id("plugin.one").is_ok());
        assert!(validate_plugin_id("@scope-plugin").is_ok());
        assert!(validate_plugin_id("../plugin").is_err());
        assert!(validate_plugin_id("plugin/slash").is_err());
    }

    #[test]
    fn cdp_response_expression_dispatches_to_runtime() {
        let expression = plugin_cdp_response_expression(&json!({
            "id": "1",
            "ok": true,
            "value": { "type": "pong" },
        }));

        assert!(expression.contains("__codexlPluginRuntime"));
        assert!(expression.contains("acceptCdpResponse"));
        assert!(expression.contains("\"pong\""));
    }

    #[test]
    fn list_models_for_host_from_catalog_matches_desktop_shape() {
        let catalog = json!({
            "models": [
                {
                    "slug": "Big model/glm-5.1",
                    "display_name": "Big model/glm-5.1",
                    "description": "NextAI Gateway model Big model/glm-5.1",
                    "visibility": "list",
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [
                        { "effort": "low", "description": "Low reasoning" },
                        { "effort": "xhigh", "description": "Extra high reasoning" }
                    ],
                    "input_modalities": ["text", "image"],
                    "additional_speed_tiers": [],
                    "service_tiers": []
                },
                {
                    "slug": "hidden-model",
                    "visibility": "hidden"
                }
            ]
        });

        let response = list_models_for_host_from_catalog(
            &catalog,
            "Big model/glm-5.1",
            &json!({ "includeHidden": false }),
        )
        .expect("model list");
        let data = response
            .get("data")
            .and_then(Value::as_array)
            .expect("data array");

        assert_eq!(data.len(), 1);
        assert_eq!(
            data[0].get("model").and_then(Value::as_str),
            Some("Big model/glm-5.1")
        );
        assert_eq!(
            data[0].get("isDefault").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            data[0]
                .pointer("/supportedReasoningEfforts/1/reasoningEffort")
                .and_then(Value::as_str),
            Some("xhigh")
        );
        assert_eq!(
            data[0]
                .get("defaultReasoningEffort")
                .and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            data[0]
                .pointer("/inputModalities/1")
                .and_then(Value::as_str),
            Some("image")
        );
    }

    #[test]
    fn top_level_toml_string_stops_before_sections() {
        let content = r#"model_catalog_json = "/tmp/catalog.json"
[profiles.nextai]
model_catalog_json = "/tmp/profile-catalog.json"
"#;

        assert_eq!(
            top_level_toml_string(content, "model_catalog_json").as_deref(),
            Some("/tmp/catalog.json")
        );
        assert_eq!(top_level_toml_string(content, "missing"), None);
    }
}
