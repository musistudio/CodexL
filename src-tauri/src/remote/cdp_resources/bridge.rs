use super::cdp::{connect_target, list_targets, select_target};
use super::file_picker::{dispatch_web_file_picker_message, is_web_file_picker_message};
use super::resource::log_web_resource_targets;
use super::*;
use crate::remote::crypto::RemoteCrypto;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

const WEB_BRIDGE_TARGET_CACHE_TTL_MS: u64 = 30_000;
const WEB_BRIDGE_CDP_PENDING_PRUNE_LIMIT: usize = 1024;
const WEB_BRIDGE_HEARTBEAT_TYPE: &str = "bridge-heartbeat";
const WEB_BRIDGE_SNAPSHOT_REQUEST_TYPE: &str = "codex-web-bridge-request-snapshot";
const WEB_BRIDGE_STREAM_IDLE_TIMEOUT_MS: u64 = 120_000;
const WEB_BRIDGE_STREAM_MAX_DURATION_MS: u64 = 10 * 60_000;
const WEB_BRIDGE_STREAM_POLL_INTERVAL_MS: u64 = 25;
const WEB_BRIDGE_STREAM_POLL_LIMIT: usize = 64;
const WEB_BRIDGE_NOTIFICATION_POLL_INTERVAL_MS: u64 = 50;
const WEB_BRIDGE_NOTIFICATION_POLL_LIMIT: usize = 256;
const WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES: usize = 512 * 1024;
const WEB_BRIDGE_NOTIFICATION_IDLE_TIMEOUT_MS: u64 = 10 * 60_000;
const WEB_BRIDGE_EVENT_HUB_CAPACITY: usize = 8192;

static WEB_BRIDGE_TARGET_CACHE: OnceLock<StdMutex<Option<CachedWebBridgeTarget>>> = OnceLock::new();
static WEB_BRIDGE_CDP_CLIENT_CACHE: OnceLock<StdMutex<Option<CachedWebBridgeCdpClient>>> =
    OnceLock::new();
static WEB_BRIDGE_EVENT_HUBS: OnceLock<StdMutex<HashMap<String, Arc<WebBridgeEventHub>>>> =
    OnceLock::new();

struct CachedWebBridgeTarget {
    cdp_host: String,
    cdp_port: u16,
    expires_at: Instant,
    target: CdpTarget,
}

struct CachedWebBridgeCdpClient {
    cdp_host: String,
    cdp_port: u16,
    target_id: String,
    target_ws_url: String,
    client: Arc<WebBridgeCdpClient>,
}

struct WebBridgeCdpClient {
    open: Arc<AtomicBool>,
    sender: mpsc::UnboundedSender<WebBridgeCdpCommand>,
}

struct WebBridgeCdpCommand {
    method: String,
    params: Value,
    response: tokio::sync::oneshot::Sender<Result<Value, String>>,
}

struct WebBridgeCdpPending {
    method: String,
    response: tokio::sync::oneshot::Sender<Result<Value, String>>,
}

#[derive(Debug, Clone)]
struct WebBridgeEvent {
    seq: u64,
    message: Value,
    bytes: usize,
    #[allow(dead_code)]
    received_at: Instant,
}

#[derive(Debug, Clone)]
struct WebBridgeEventCursor {
    next_seq: u64,
}

struct WebBridgeEventHub {
    cdp_host: String,
    cdp_port: u16,
    state: StdMutex<WebBridgeEventHubState>,
    notify: tokio::sync::Notify,
}

struct WebBridgeEventHubState {
    events: VecDeque<WebBridgeEvent>,
    first_seq: u64,
    next_seq: u64,
    producer_running: bool,
    producer_retry_after: Option<Instant>,
    consumers: usize,
}

struct WebBridgeEventConsumer {
    hub: Arc<WebBridgeEventHub>,
    cursor: WebBridgeEventCursor,
}

impl WebBridgeEventHub {
    fn new(cdp_host: String, cdp_port: u16) -> Self {
        Self {
            cdp_host,
            cdp_port,
            state: StdMutex::new(WebBridgeEventHubState {
                events: VecDeque::new(),
                first_seq: 1,
                next_seq: 1,
                producer_running: false,
                producer_retry_after: None,
                consumers: 0,
            }),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn subscribe(self: &Arc<Self>) -> WebBridgeEventConsumer {
        let cursor = {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            state.consumers = state.consumers.saturating_add(1);
            WebBridgeEventCursor {
                next_seq: state.next_seq,
            }
        };
        self.ensure_producer();
        WebBridgeEventConsumer {
            hub: self.clone(),
            cursor,
        }
    }

    #[cfg(test)]
    fn subscribe_without_producer_for_test(self: &Arc<Self>) -> WebBridgeEventConsumer {
        let cursor = {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            state.consumers = state.consumers.saturating_add(1);
            WebBridgeEventCursor {
                next_seq: state.next_seq,
            }
        };
        WebBridgeEventConsumer {
            hub: self.clone(),
            cursor,
        }
    }

    fn unregister_consumer(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.consumers = state.consumers.saturating_sub(1);
        }
        self.notify.notify_waiters();
    }

    fn has_consumers(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.consumers > 0)
            .unwrap_or(false)
    }

    fn ensure_producer(self: &Arc<Self>) {
        let should_start = {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            if state
                .producer_retry_after
                .map(|retry_after| retry_after > Instant::now())
                .unwrap_or(false)
            {
                return;
            }
            if state.producer_running {
                false
            } else {
                state.producer_running = true;
                state.producer_retry_after = None;
                true
            }
        };
        if !should_start {
            return;
        }

        let hub = self.clone();
        tokio::spawn(async move {
            let result = run_web_bridge_notification_producer(hub.clone()).await;
            let failed = result.is_err();
            hub.mark_producer_stopped(failed);
            if let Err(err) = result {
                eprintln!("[codex-web] bridge notification producer stopped: {}", err);
            }
        });
    }

    fn mark_producer_stopped(&self, failed: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.producer_running = false;
            state.producer_retry_after = if failed {
                Some(Instant::now() + Duration::from_secs(1))
            } else {
                None
            };
        }
        self.notify.notify_waiters();
    }

    fn publish_messages(&self, messages: Vec<Value>) {
        if messages.is_empty() {
            return;
        }

        {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            for message in messages {
                let seq = state.next_seq;
                state.next_seq = state.next_seq.saturating_add(1);
                let bytes = serde_json::to_vec(&message)
                    .map(|raw| raw.len())
                    .unwrap_or(0);
                state.events.push_back(WebBridgeEvent {
                    seq,
                    message,
                    bytes,
                    received_at: Instant::now(),
                });
            }
            while state.events.len() > WEB_BRIDGE_EVENT_HUB_CAPACITY {
                state.events.pop_front();
            }
            state.first_seq = state
                .events
                .front()
                .map(|event| event.seq)
                .unwrap_or(state.next_seq);
        }
        self.notify.notify_waiters();
    }

    fn next_batch(
        &self,
        cursor: &mut WebBridgeEventCursor,
        limit: usize,
        max_bytes: usize,
    ) -> Vec<Value> {
        let state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        let mut messages = Vec::new();
        let limit = limit.max(1);
        let mut bytes = 0usize;
        let mut next_seq = cursor.next_seq;

        if next_seq < state.first_seq {
            let requested = next_seq;
            let first = state.first_seq;
            next_seq = first;
            messages.push(json!({
                "type": "codex-web-bridge-notification-gap",
                "droppedCount": first.saturating_sub(requested),
                "reason": "rust-event-hub-overflow",
                "requestedSeq": requested,
                "firstSeq": first,
                "queued": state.events.len(),
            }));
        }

        for event in state.events.iter() {
            if event.seq < next_seq {
                continue;
            }
            if messages.len() >= limit {
                break;
            }
            if !messages.is_empty() && bytes.saturating_add(event.bytes) > max_bytes {
                break;
            }
            messages.push(event.message.clone());
            bytes = bytes.saturating_add(event.bytes);
            next_seq = event.seq.saturating_add(1);
        }
        cursor.next_seq = next_seq;

        messages
    }
}

impl Drop for WebBridgeEventConsumer {
    fn drop(&mut self) {
        self.hub.unregister_consumer();
    }
}

impl WebBridgeEventConsumer {
    fn next_batch(&mut self, limit: usize, max_bytes: usize) -> Vec<Value> {
        self.hub.next_batch(&mut self.cursor, limit, max_bytes)
    }
}

#[cfg(test)]
pub(super) fn web_bridge_event_hub_test_fanout_messages() -> (Vec<Value>, Vec<Value>) {
    let hub = Arc::new(WebBridgeEventHub::new("127.0.0.1".to_string(), 0));
    let mut first = hub.subscribe_without_producer_for_test();
    let mut second = hub.subscribe_without_producer_for_test();
    hub.publish_messages(vec![
        json!({ "type": "mcp-notification", "method": "one" }),
        json!({ "type": "ipc-broadcast", "method": "thread-stream-state-changed" }),
    ]);
    (
        first.next_batch(16, WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES),
        second.next_batch(16, WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES),
    )
}

#[cfg(test)]
pub(super) fn web_bridge_event_hub_test_gap_message() -> Vec<Value> {
    let hub = Arc::new(WebBridgeEventHub::new("127.0.0.1".to_string(), 0));
    let mut consumer = hub.subscribe_without_producer_for_test();
    let messages = (0..(WEB_BRIDGE_EVENT_HUB_CAPACITY + 2))
        .map(|index| json!({ "type": "mcp-notification", "index": index }))
        .collect::<Vec<_>>();
    hub.publish_messages(messages);
    consumer.next_batch(16, WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES)
}

pub async fn dispatch_web_bridge_message(
    cdp_host: &str,
    cdp_port: u16,
    message: Value,
) -> Result<Value, String> {
    let message_type = message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("<missing>")
        .to_string();
    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let url = message
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let mcp_method = message
        .get("request")
        .and_then(|request| request.get("method"))
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let mcp_id = message
        .get("request")
        .and_then(|request| request.get("id"))
        .map(|id| match id {
            Value::String(value) => value.clone(),
            _ => id.to_string(),
        })
        .unwrap_or_else(|| "<none>".to_string());
    eprintln!(
        "[codex-web] bridge request: cdp=http://{}:{} type={} requestId={} url={} mcpMethod={} mcpId={}",
        cdp_host, cdp_port, message_type, request_id, url, mcp_method, mcp_id
    );

    if message.get("type").and_then(Value::as_str) == Some(WEB_BRIDGE_SNAPSHOT_REQUEST_TYPE) {
        let reason = message
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("remote-request");
        let client_id = message
            .get("clientId")
            .and_then(Value::as_str)
            .unwrap_or("codex-web-bridge");
        let cdp_client = web_bridge_cdp_client(cdp_host, cdp_port).await?;
        let result = cdp_client
            .send(
                "Runtime.evaluate",
                json!({
                    "awaitPromise": true,
                    "expression": web_bridge_snapshot_request_expression(reason, client_id),
                    "returnByValue": true,
                }),
            )
            .await?;
        return web_bridge_runtime_value(&result);
    }

    if is_web_file_picker_message(&message) {
        let value = dispatch_web_file_picker_message(message)?;
        let entry_count = value
            .get("value")
            .and_then(|value| value.get("entries"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        eprintln!(
            "[codex-web] bridge response: type={} entries={}",
            message_type, entry_count
        );
        return Ok(value);
    }

    let cdp_client = web_bridge_cdp_client(cdp_host, cdp_port).await?;
    let result = match cdp_client
        .send(
            "Runtime.evaluate",
            json!({
                "awaitPromise": true,
                "expression": web_bridge_dispatch_expression(&message),
                "returnByValue": true,
            }),
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            if !cdp_client.is_open() {
                clear_cached_web_bridge_cdp_client(cdp_host, cdp_port);
            }
            return Err(err);
        }
    };
    let value = web_bridge_runtime_value(&result)?;
    let message_count = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let message_types = value
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    message
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "<none>".to_string());
    eprintln!(
        "[codex-web] bridge response: type={} requestId={} messages={} messageTypes={} timedOut={}",
        message_type,
        request_id,
        message_count,
        message_types,
        value
            .get("timedOut")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    );
    Ok(value)
}

async fn web_bridge_cdp_client(
    cdp_host: &str,
    cdp_port: u16,
) -> Result<Arc<WebBridgeCdpClient>, String> {
    let target = web_bridge_target(cdp_host, cdp_port).await?;
    match web_bridge_cdp_client_for_target(cdp_host, cdp_port, &target).await {
        Ok(client) => Ok(client),
        Err(err) => {
            clear_cached_web_bridge_target(cdp_host, cdp_port);
            eprintln!("[codex-web] bridge cached CDP target failed: {}", err);
            let target = load_web_bridge_target(cdp_host, cdp_port).await?;
            web_bridge_cdp_client_for_target(cdp_host, cdp_port, &target).await
        }
    }
}

async fn web_bridge_cdp_client_for_target(
    cdp_host: &str,
    cdp_port: u16,
    target: &CdpTarget,
) -> Result<Arc<WebBridgeCdpClient>, String> {
    if let Some(client) = cached_web_bridge_cdp_client(cdp_host, cdp_port, target) {
        return Ok(client);
    }

    let client = WebBridgeCdpClient::connect(target).await?;
    if let Err(err) = client.send("Runtime.enable", json!({})).await {
        client.close();
        return Err(err);
    }
    let client = store_or_reuse_web_bridge_cdp_client(cdp_host, cdp_port, target, client);
    Ok(client)
}

impl WebBridgeCdpClient {
    async fn connect(target: &CdpTarget) -> Result<Arc<Self>, String> {
        let socket = connect_target(target).await?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let open = Arc::new(AtomicBool::new(true));
        let client = Arc::new(Self {
            open: open.clone(),
            sender,
        });
        let target_label = web_bridge_target_label(target);
        tokio::spawn(run_web_bridge_cdp_socket(
            socket,
            receiver,
            open,
            target_label,
        ));
        Ok(client)
    }

    fn close(&self) {
        self.open.store(false, Ordering::Relaxed);
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed) && !self.sender.is_closed()
    }

    async fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.is_open() {
            return Err("CDP bridge websocket is closed".to_string());
        }
        let (response, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(WebBridgeCdpCommand {
                method: method.to_string(),
                params,
                response,
            })
            .map_err(|_| "CDP bridge websocket is closed".to_string())?;
        tokio::time::timeout(Duration::from_millis(CDP_COMMAND_TIMEOUT_MS), receiver)
            .await
            .map_err(|_| format!("CDP command timed out: {}", method))?
            .map_err(|_| "CDP bridge websocket closed before response".to_string())?
    }
}

async fn run_web_bridge_cdp_socket(
    mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    mut receiver: mpsc::UnboundedReceiver<WebBridgeCdpCommand>,
    open: Arc<AtomicBool>,
    target_label: String,
) {
    let mut next_id = 1_u64;
    let mut pending = HashMap::<u64, WebBridgeCdpPending>::new();
    let close_reason = loop {
        tokio::select! {
            command = receiver.recv() => {
                let Some(command) = command else {
                    break "CDP bridge command channel closed".to_string();
                };
                prune_closed_web_bridge_cdp_pending(&mut pending);
                let WebBridgeCdpCommand {
                    method,
                    params,
                    response,
                } = command;
                let id = next_id;
                next_id += 1;
                let payload = json!({
                    "id": id,
                    "method": method.clone(),
                    "params": params,
                })
                .to_string();
                if let Err(err) = socket.send(Message::Text(payload)).await {
                    let error = err.to_string();
                    let _ = response.send(Err(error.clone()));
                    break error;
                }
                pending.insert(id, WebBridgeCdpPending {
                    method,
                    response,
                });
            }
            message = socket.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(err)) => break err.to_string(),
                    None => break "CDP bridge websocket closed".to_string(),
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let value = match serde_json::from_str::<Value>(&text) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                if let Some(pending_command) = pending.remove(&id) {
                    let result = web_bridge_cdp_result(&value, &pending_command.method);
                    let _ = pending_command.response.send(result);
                }
            }
        }
    };
    open.store(false, Ordering::Relaxed);
    for (_, pending_command) in pending.drain() {
        let _ = pending_command.response.send(Err(close_reason.clone()));
    }
    eprintln!(
        "[codex-web] bridge CDP websocket closed: target={} reason={}",
        target_label, close_reason
    );
}

fn web_bridge_cdp_result(value: &Value, method: &str) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(method)
            .to_string();
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        let data = error
            .get("data")
            .map(|data| data.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        return Err(format!("{} (code={}, data={})", message, code, data));
    }
    Ok(value.get("result").cloned().unwrap_or_else(|| json!({})))
}

fn web_bridge_runtime_value(result: &Value) -> Result<Value, String> {
    result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| "Runtime.evaluate returned no bridge value".to_string())
}

fn prune_closed_web_bridge_cdp_pending(pending: &mut HashMap<u64, WebBridgeCdpPending>) {
    if pending.len() < WEB_BRIDGE_CDP_PENDING_PRUNE_LIMIT {
        return;
    }
    pending.retain(|_, pending_command| !pending_command.response.is_closed());
}

fn cached_web_bridge_cdp_client(
    cdp_host: &str,
    cdp_port: u16,
    target: &CdpTarget,
) -> Option<Arc<WebBridgeCdpClient>> {
    let cache = WEB_BRIDGE_CDP_CLIENT_CACHE.get_or_init(|| StdMutex::new(None));
    let mut guard = cache.lock().ok()?;
    let Some(cached) = guard.as_ref() else {
        return None;
    };
    let matches = cached.cdp_host == cdp_host
        && cached.cdp_port == cdp_port
        && cached.target_id == target.id
        && cached.target_ws_url == target.web_socket_debugger_url;
    if matches && cached.client.is_open() {
        return Some(cached.client.clone());
    }
    if matches {
        let previous = guard.take();
        drop(guard);
        if let Some(previous) = previous {
            previous.client.close();
        }
    }
    None
}

fn store_or_reuse_web_bridge_cdp_client(
    cdp_host: &str,
    cdp_port: u16,
    target: &CdpTarget,
    client: Arc<WebBridgeCdpClient>,
) -> Arc<WebBridgeCdpClient> {
    let cache = WEB_BRIDGE_CDP_CLIENT_CACHE.get_or_init(|| StdMutex::new(None));
    let mut previous_to_close = None;
    let mut new_client_to_close = None;
    let selected = match cache.lock() {
        Ok(mut guard) => {
            if let Some(cached) = guard.as_ref() {
                let matches = cached.cdp_host == cdp_host
                    && cached.cdp_port == cdp_port
                    && cached.target_id == target.id
                    && cached.target_ws_url == target.web_socket_debugger_url;
                if matches && cached.client.is_open() {
                    new_client_to_close = Some(client);
                    cached.client.clone()
                } else {
                    previous_to_close = guard.take();
                    *guard = Some(CachedWebBridgeCdpClient {
                        cdp_host: cdp_host.to_string(),
                        cdp_port,
                        target_id: target.id.clone(),
                        target_ws_url: target.web_socket_debugger_url.clone(),
                        client: client.clone(),
                    });
                    eprintln!(
                        "[codex-web] bridge CDP websocket opened: target={}",
                        web_bridge_target_label(target)
                    );
                    client
                }
            } else {
                *guard = Some(CachedWebBridgeCdpClient {
                    cdp_host: cdp_host.to_string(),
                    cdp_port,
                    target_id: target.id.clone(),
                    target_ws_url: target.web_socket_debugger_url.clone(),
                    client: client.clone(),
                });
                eprintln!(
                    "[codex-web] bridge CDP websocket opened: target={}",
                    web_bridge_target_label(target)
                );
                client
            }
        }
        Err(_) => client,
    };
    if let Some(previous) = previous_to_close {
        previous.client.close();
    }
    if let Some(new_client) = new_client_to_close {
        new_client.close();
    }
    selected
}

fn clear_cached_web_bridge_cdp_client(cdp_host: &str, cdp_port: u16) {
    let cache = WEB_BRIDGE_CDP_CLIENT_CACHE.get_or_init(|| StdMutex::new(None));
    let previous = match cache.lock() {
        Ok(mut guard) => {
            if guard
                .as_ref()
                .map(|cached| cached.cdp_host == cdp_host && cached.cdp_port == cdp_port)
                .unwrap_or(false)
            {
                guard.take()
            } else {
                None
            }
        }
        Err(_) => None,
    };
    if let Some(previous) = previous {
        previous.client.close();
    }
}

fn web_bridge_target_label(target: &CdpTarget) -> String {
    format!(
        "id={} type={} title={} url={}",
        if target.id.is_empty() {
            "<empty>"
        } else {
            &target.id
        },
        if target.target_type.is_empty() {
            "<empty>"
        } else {
            &target.target_type
        },
        if target.title.is_empty() {
            "<empty>"
        } else {
            &target.title
        },
        if target.url.is_empty() {
            "<empty>"
        } else {
            &target.url
        }
    )
}

async fn web_bridge_target(cdp_host: &str, cdp_port: u16) -> Result<CdpTarget, String> {
    if let Some(target) = cached_web_bridge_target(cdp_host, cdp_port) {
        return Ok(target);
    }
    load_web_bridge_target(cdp_host, cdp_port).await
}

async fn load_web_bridge_target(cdp_host: &str, cdp_port: u16) -> Result<CdpTarget, String> {
    let targets = list_targets(cdp_host, cdp_port).await?;
    let target = select_target(&targets)
        .ok_or_else(|| "no page target with webSocketDebuggerUrl".to_string())?;
    log_web_resource_targets(&targets, &target);
    store_web_bridge_target(cdp_host, cdp_port, target.clone());
    Ok(target)
}

fn cached_web_bridge_target(cdp_host: &str, cdp_port: u16) -> Option<CdpTarget> {
    let cache = WEB_BRIDGE_TARGET_CACHE.get_or_init(|| StdMutex::new(None));
    let mut guard = cache.lock().ok()?;
    let Some(cached) = guard.as_ref() else {
        return None;
    };
    if cached.cdp_host == cdp_host && cached.cdp_port == cdp_port {
        if cached.expires_at > Instant::now() {
            return Some(cached.target.clone());
        }
        *guard = None;
    }
    None
}

fn store_web_bridge_target(cdp_host: &str, cdp_port: u16, target: CdpTarget) {
    let cache = WEB_BRIDGE_TARGET_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedWebBridgeTarget {
            cdp_host: cdp_host.to_string(),
            cdp_port,
            expires_at: Instant::now() + Duration::from_millis(WEB_BRIDGE_TARGET_CACHE_TTL_MS),
            target,
        });
    }
}

fn clear_cached_web_bridge_target(cdp_host: &str, cdp_port: u16) {
    let cache = WEB_BRIDGE_TARGET_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        if guard
            .as_ref()
            .map(|cached| cached.cdp_host == cdp_host && cached.cdp_port == cdp_port)
            .unwrap_or(false)
        {
            *guard = None;
        }
    }
    clear_cached_web_bridge_cdp_client(cdp_host, cdp_port);
}

pub async fn handle_web_bridge_websocket<S>(
    websocket: WebSocketStream<S>,
    cdp_host: String,
    cdp_port: u16,
    crypto: Option<Arc<RemoteCrypto>>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    eprintln!(
        "[codex-web] bridge websocket opened: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    let (mut write, mut read) = websocket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let pump_tx = tx.clone();
    let pump_crypto = crypto.clone();
    let pump_active = Arc::new(AtomicBool::new(true));
    spawn_web_bridge_notification_pump_for_connection(
        cdp_host.clone(),
        cdp_port,
        pump_active.clone(),
        move |partial| {
            if let Some(text) =
                encrypt_bridge_socket_text(pump_crypto.as_deref(), partial.to_string())
            {
                let _ = pump_tx.send(Message::Text(text));
            }
        },
    );

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
                    handle_web_bridge_socket_text(
                        &tx,
                        cdp_host.clone(),
                        cdp_port,
                        raw,
                        crypto.clone(),
                    );
                }
                Message::Binary(bytes) => match String::from_utf8(bytes) {
                    Ok(raw) => {
                        handle_web_bridge_socket_text(
                            &tx,
                            cdp_host.clone(),
                            cdp_port,
                            raw,
                            crypto.clone(),
                        );
                    }
                    Err(err) => {
                        let response = web_bridge_socket_response(None, Err(err.to_string()));
                        if let Some(text) =
                            encrypt_bridge_socket_text(crypto.as_deref(), response.to_string())
                        {
                            let _ = tx.send(Message::Text(text));
                        }
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
    pump_active.store(false, Ordering::Relaxed);
    eprintln!(
        "[codex-web] bridge websocket closed: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    result
}

pub fn spawn_web_bridge_notification_pump<F>(cdp_host: String, cdp_port: u16, emit: F)
where
    F: Fn(Value) + Send + Sync + 'static,
{
    spawn_web_bridge_notification_pump_with_options(
        cdp_host,
        cdp_port,
        None,
        Some(Duration::from_millis(
            WEB_BRIDGE_NOTIFICATION_IDLE_TIMEOUT_MS,
        )),
        emit,
    );
}

fn spawn_web_bridge_notification_pump_for_connection<F>(
    cdp_host: String,
    cdp_port: u16,
    active: Arc<AtomicBool>,
    emit: F,
) where
    F: Fn(Value) + Send + Sync + 'static,
{
    spawn_web_bridge_notification_pump_with_options(cdp_host, cdp_port, Some(active), None, emit);
}

fn spawn_web_bridge_notification_pump_with_options<F>(
    cdp_host: String,
    cdp_port: u16,
    active: Option<Arc<AtomicBool>>,
    idle_timeout: Option<Duration>,
    emit: F,
) where
    F: Fn(Value) + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let hub = web_bridge_event_hub(cdp_host, cdp_port);
        let consumer = hub.subscribe();
        if let Err(err) =
            run_web_bridge_notification_consumer(consumer, active, idle_timeout, emit).await
        {
            eprintln!("[codex-web] bridge notification consumer stopped: {}", err);
        }
    });
}

fn web_bridge_event_hub(cdp_host: String, cdp_port: u16) -> Arc<WebBridgeEventHub> {
    let key = format!("{}:{}", cdp_host, cdp_port);
    let hubs = WEB_BRIDGE_EVENT_HUBS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = hubs.lock().unwrap_or_else(|err| err.into_inner());
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(WebBridgeEventHub::new(cdp_host, cdp_port)))
        .clone()
}

async fn run_web_bridge_notification_consumer<F>(
    mut consumer: WebBridgeEventConsumer,
    active: Option<Arc<AtomicBool>>,
    idle_timeout: Option<Duration>,
    emit: F,
) -> Result<(), String>
where
    F: Fn(Value) + Send + Sync,
{
    let mut last_message_at = Instant::now();
    loop {
        if active
            .as_ref()
            .map(|active| !active.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            return Ok(());
        }

        consumer.hub.ensure_producer();
        let messages = consumer.next_batch(
            WEB_BRIDGE_NOTIFICATION_POLL_LIMIT,
            WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES,
        );
        if messages.is_empty() {
            if idle_timeout
                .map(|timeout| last_message_at.elapsed() > timeout)
                .unwrap_or(false)
            {
                return Ok(());
            }
        } else {
            last_message_at = Instant::now();
            emit(json!({ "messages": messages }));
            continue;
        }

        let _ = tokio::time::timeout(
            Duration::from_millis(WEB_BRIDGE_NOTIFICATION_POLL_INTERVAL_MS),
            consumer.hub.notify.notified(),
        )
        .await;
    }
}

async fn run_web_bridge_notification_producer(hub: Arc<WebBridgeEventHub>) -> Result<(), String> {
    let cdp_client = web_bridge_cdp_client(&hub.cdp_host, hub.cdp_port).await?;
    cdp_client
        .send(
            "Runtime.evaluate",
            json!({
                "awaitPromise": true,
                "expression": web_bridge_notification_install_expression(),
                "returnByValue": true,
            }),
        )
        .await?;

    loop {
        if !hub.has_consumers() {
            return Ok(());
        }

        let result = match cdp_client
            .send(
                "Runtime.evaluate",
                json!({
                    "awaitPromise": true,
                    "expression": web_bridge_notification_poll_expression(
                        WEB_BRIDGE_NOTIFICATION_POLL_LIMIT,
                    ),
                    "returnByValue": true,
                }),
            )
            .await
        {
            Ok(result) => result,
            Err(err) => {
                if !cdp_client.is_open() {
                    clear_cached_web_bridge_cdp_client(&hub.cdp_host, hub.cdp_port);
                }
                return Err(err);
            }
        };
        let value = web_bridge_runtime_value(&result)?;
        let messages = value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        hub.publish_messages(messages);

        tokio::time::sleep(Duration::from_millis(
            WEB_BRIDGE_NOTIFICATION_POLL_INTERVAL_MS,
        ))
        .await;
    }
}

pub async fn dispatch_web_bridge_socket_payload_with_emitter<F>(
    cdp_host: &str,
    cdp_port: u16,
    raw: &str,
    emit: F,
) -> Value
where
    F: Fn(Value) + Send + Sync,
{
    let (id, message) = parse_web_bridge_socket_message(raw);
    eprintln!(
        "[codex-web] bridge socket message: id={} parseOk={}",
        id.as_deref().unwrap_or("<none>"),
        message.is_ok()
    );
    let result = match message {
        Ok(message) if is_web_bridge_socket_heartbeat(&message) => {
            Ok(json!({ "type": "bridge-heartbeat-ack" }))
        }
        Ok(message) if is_web_bridge_fetch_stream_message(&message) => {
            dispatch_web_bridge_stream_message(cdp_host, cdp_port, message, &emit).await
        }
        Ok(message) => dispatch_web_bridge_message(cdp_host, cdp_port, message).await,
        Err(err) => Err(err),
    };
    web_bridge_socket_response(id, result)
}

pub(super) fn is_web_bridge_socket_heartbeat(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WEB_BRIDGE_HEARTBEAT_TYPE)
}

fn is_web_bridge_fetch_stream_message(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("fetch-stream")
        && message.get("requestId").and_then(Value::as_str).is_some()
}

async fn dispatch_web_bridge_stream_message<F>(
    cdp_host: &str,
    cdp_port: u16,
    message: Value,
    emit: &F,
) -> Result<Value, String>
where
    F: Fn(Value) + Send + Sync,
{
    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cdp_client = match web_bridge_cdp_client(cdp_host, cdp_port).await {
        Ok(client) => client,
        Err(err) => {
            return Ok(json!({
                "messages": [web_bridge_fetch_stream_error(&request_id, &err)],
            }));
        }
    };
    let start_result = match cdp_client
        .send(
            "Runtime.evaluate",
            json!({
                "awaitPromise": true,
                "expression": web_bridge_stream_start_expression(&message),
                "returnByValue": true,
            }),
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            if !cdp_client.is_open() {
                clear_cached_web_bridge_cdp_client(cdp_host, cdp_port);
            }
            return Ok(json!({
                "messages": [web_bridge_fetch_stream_error(&request_id, &err)],
            }));
        }
    };
    let start_value = match web_bridge_runtime_value(&start_result) {
        Ok(value) => value,
        Err(err) => {
            return Ok(json!({
                "messages": [web_bridge_fetch_stream_error(&request_id, &err)],
            }));
        }
    };
    let Some(stream_key) = start_value
        .get("streamKey")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(start_value);
    };

    let started_at = Instant::now();
    let mut last_activity_at = Instant::now();
    let mut emitted_count = 0_usize;
    let mut saw_terminal = false;
    loop {
        let poll_result = match cdp_client
            .send(
                "Runtime.evaluate",
                json!({
                    "awaitPromise": true,
                    "expression": web_bridge_stream_poll_expression(
                        &stream_key,
                        WEB_BRIDGE_STREAM_POLL_LIMIT,
                    ),
                    "returnByValue": true,
                }),
            )
            .await
        {
            Ok(result) => result,
            Err(err) => {
                if !cdp_client.is_open() {
                    clear_cached_web_bridge_cdp_client(cdp_host, cdp_port);
                }
                emit(json!({
                    "messages": [web_bridge_fetch_stream_error(&request_id, &err)],
                }));
                return Ok(json!({ "messages": [], "timedOut": false }));
            }
        };
        let poll_value = match web_bridge_runtime_value(&poll_result) {
            Ok(value) => value,
            Err(err) => {
                emit(json!({
                    "messages": [web_bridge_fetch_stream_error(&request_id, &err)],
                }));
                return Ok(json!({ "messages": [], "timedOut": false }));
            }
        };
        let messages = poll_value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !messages.is_empty() {
            emitted_count += messages.len();
            last_activity_at = Instant::now();
            saw_terminal = saw_terminal
                || messages
                    .iter()
                    .any(web_bridge_fetch_stream_message_is_terminal);
            emit(json!({ "messages": messages }));
        }

        let complete = poll_value
            .get("complete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if complete {
            let timed_out = poll_value
                .get("timedOut")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if timed_out && !saw_terminal {
                emit(json!({
                    "messages": [web_bridge_fetch_stream_error(
                        &request_id,
                        "Timed out waiting for Codex host fetch stream",
                    )],
                }));
            }
            eprintln!(
                "[codex-web] bridge stream response: requestId={} messages={} timedOut={}",
                request_id, emitted_count, timed_out
            );
            return Ok(json!({ "messages": [], "timedOut": timed_out }));
        }

        if last_activity_at.elapsed() > Duration::from_millis(WEB_BRIDGE_STREAM_IDLE_TIMEOUT_MS) {
            let _ = cdp_client
                .send(
                    "Runtime.evaluate",
                    json!({
                        "awaitPromise": true,
                        "expression": web_bridge_stream_cleanup_expression(&stream_key),
                        "returnByValue": true,
                    }),
                )
                .await;
            emit(json!({
                "messages": [web_bridge_fetch_stream_error(
                    &request_id,
                    "Timed out waiting for Codex host fetch stream activity",
                )],
            }));
            return Ok(json!({ "messages": [], "timedOut": true }));
        }

        if started_at.elapsed() > Duration::from_millis(WEB_BRIDGE_STREAM_MAX_DURATION_MS) {
            let _ = cdp_client
                .send(
                    "Runtime.evaluate",
                    json!({
                        "awaitPromise": true,
                        "expression": web_bridge_stream_cleanup_expression(&stream_key),
                        "returnByValue": true,
                    }),
                )
                .await;
            emit(json!({
                "messages": [web_bridge_fetch_stream_error(
                    &request_id,
                    "Timed out waiting for Codex host fetch stream to finish",
                )],
            }));
            return Ok(json!({ "messages": [], "timedOut": true }));
        }

        tokio::time::sleep(Duration::from_millis(WEB_BRIDGE_STREAM_POLL_INTERVAL_MS)).await;
    }
}

fn web_bridge_fetch_stream_message_is_terminal(message: &Value) -> bool {
    matches!(
        message.get("type").and_then(Value::as_str),
        Some("fetch-stream-error") | Some("fetch-stream-complete")
    )
}

fn web_bridge_fetch_stream_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "fetch-stream-error",
        "requestId": request_id,
        "error": error,
    })
}

fn handle_web_bridge_socket_text(
    tx: &mpsc::UnboundedSender<Message>,
    cdp_host: String,
    cdp_port: u16,
    raw: String,
    crypto: Option<Arc<RemoteCrypto>>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let raw = match decrypt_bridge_socket_text(crypto.as_deref(), &raw) {
            Ok(raw) => raw,
            Err(err) => {
                let response = web_bridge_socket_response(None, Err(err));
                if let Some(text) =
                    encrypt_bridge_socket_text(crypto.as_deref(), response.to_string())
                {
                    let _ = tx.send(Message::Text(text));
                }
                return;
            }
        };
        let partial_tx = tx.clone();
        let partial_crypto = crypto.clone();
        let response = dispatch_web_bridge_socket_payload_with_emitter(
            &cdp_host,
            cdp_port,
            &raw,
            move |partial| {
                if let Some(text) =
                    encrypt_bridge_socket_text(partial_crypto.as_deref(), partial.to_string())
                {
                    let _ = partial_tx.send(Message::Text(text));
                }
            },
        )
        .await;
        if let Some(text) = encrypt_bridge_socket_text(crypto.as_deref(), response.to_string()) {
            let _ = tx.send(Message::Text(text));
        }
    });
}

fn encrypt_bridge_socket_text(crypto: Option<&RemoteCrypto>, raw: String) -> Option<String> {
    match crypto {
        Some(crypto) => match crypto.encrypt_text(&raw) {
            Ok(encrypted) => Some(encrypted),
            Err(err) => {
                eprintln!("[codex-web] bridge payload encryption failed: {}", err);
                None
            }
        },
        None => Some(raw),
    }
}

fn decrypt_bridge_socket_text(crypto: Option<&RemoteCrypto>, raw: &str) -> Result<String, String> {
    match crypto {
        Some(crypto) => crypto.decrypt_text(raw),
        None => Ok(raw.to_string()),
    }
}

pub(super) fn parse_web_bridge_socket_message(
    raw: &str,
) -> (Option<String>, Result<Value, String>) {
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(err) => return (None, Err(err.to_string())),
    };
    let id = value.get("id").and_then(web_bridge_id_to_string);
    if let Some(message) = value.get("message") {
        return (id, Ok(message.clone()));
    }
    if value.get("type").is_some() {
        return (id, Ok(value));
    }
    (id, Err("missing bridge message".to_string()))
}

pub(super) fn web_bridge_id_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn web_bridge_socket_response(
    id: Option<String>,
    result: Result<Value, String>,
) -> Value {
    let mut response = match result {
        Ok(Value::Object(map)) => Value::Object(map),
        Ok(value) => json!({ "messages": [], "value": value }),
        Err(error) => json!({ "messages": [], "error": error }),
    };
    if let Value::Object(map) = &mut response {
        if let Some(id) = id {
            map.insert("id".to_string(), Value::String(id));
        }
        map.entry("messages".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    response
}
pub(super) fn web_bridge_dispatch_expression(message: &Value) -> String {
    let message = serde_json::to_string(message).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"(async () => {{
            const input = {message};
            const timeoutMs = input?.type === "mcp-request" ? 5 * 60 * 1000 : 30000;
            const originalRequestId =
              input && typeof input.requestId === "string" ? input.requestId : null;
            const originalId = input && typeof input.id === "string" ? input.id : null;
            const bridgeId =
              originalRequestId || originalId
                ? `codex-web-${{Date.now()}}-${{Math.random().toString(36).slice(2)}}`
                : null;
            const message = {{ ...input }};
            if (originalRequestId) {{
              message.requestId = bridgeId;
            }}
            if (!originalRequestId && originalId) {{
              message.id = bridgeId;
            }}

            const clone = (value) => {{
              if (value === undefined) {{
                return undefined;
              }}
              return JSON.parse(JSON.stringify(value));
            }};
            const restoreIds = (value) => {{
              const restored = clone(value);
              if (restored && originalRequestId && restored.requestId === bridgeId) {{
                restored.requestId = originalRequestId;
              }}
              if (restored && !originalRequestId && originalId && restored.id === bridgeId) {{
                restored.id = originalId;
              }}
              return restored;
            }};
            const markBridgeForwarded = (value) => {{
              if (!value || typeof value !== "object") {{
                return;
              }}
              try {{
                Object.defineProperty(value, "__codexWebBridgeNotificationForwarded", {{
                  configurable: true,
                  value: true,
                }});
              }} catch {{
                value.__codexWebBridgeNotificationForwarded = true;
              }}
            }};
            const dispatchLocalCodexAppMessage = (value) => {{
              markBridgeForwarded(value);
              window.dispatchEvent(
                new MessageEvent("message", {{
                  data: value,
                  origin: window.location.origin,
                  source: window,
                }}),
              );
            }};
            const threadStreamStateParams = (value) => {{
              if (!value || typeof value !== "object") {{
                return null;
              }}
              if (value.type === "thread-stream-state-changed") {{
                return value;
              }}
              if (
                value.type === "ipc-broadcast" &&
                value.method === "thread-stream-state-changed" &&
                value.params &&
                typeof value.params === "object"
              ) {{
                return value.params;
              }}
              return null;
            }};
            const reflectRemoteThreadStreamStateToCodexApp = (value) => {{
              const params = threadStreamStateParams(value);
              if (!params || !params.conversationId || !params.hostId || !params.change) {{
                return;
              }}
              const reflected = {{
                type: "ipc-broadcast",
                method: "thread-stream-state-changed",
                sourceClientId:
                  value.sourceClientId || value.clientId || "codex-web-remote-bridge",
                version: Number.isFinite(value.version)
                  ? value.version
                  : Number.isFinite(params.version)
                    ? params.version
                    : 6,
                params: clone(params),
              }};
              if (params.change?.type === "snapshot") {{
                const snapshots =
                  (window.__codexWebBridgeLatestThreadStreamSnapshots ||= Object.create(null));
                snapshots[params.conversationId] = clone(reflected);
              }}
              dispatchLocalCodexAppMessage(reflected);
            }};
            const reflectThreadStartResponseToCodexApp = (response) => {{
              if (input?.type !== "mcp-request" || input.request?.method !== "thread/start") {{
                return;
              }}
              const thread = response?.message?.result?.thread;
              if (!thread || typeof thread !== "object") {{
                return;
              }}
              // Remote-created threads did not run Codex App's local createConversation()
              // path, so seed local conversation state before turn stream notifications arrive.
              dispatchLocalCodexAppMessage({{
                type: "mcp-notification",
                hostId: input.hostId,
                method: "thread/started",
                params: {{ thread: clone(thread) }},
              }});
            }};
            const sendToHost = async () => {{
              if (window.electronBridge?.sendMessageFromView) {{
                await window.electronBridge.sendMessageFromView(message);
              }} else {{
                window.dispatchEvent(
                  new CustomEvent("codex-message-from-view", {{ detail: message }}),
                );
              }}
            }};
            const waitForMessage = (predicate) =>
              new Promise((resolve) => {{
                const timer = window.setTimeout(() => {{
                  cleanup();
                  resolve({{ timedOut: true }});
                }}, timeoutMs);
                const cleanup = () => {{
                  window.clearTimeout(timer);
                  window.removeEventListener("message", onMessage);
                }};
                const onMessage = (event) => {{
                  const data = event && event.data;
                  if (!predicate(data)) {{
                    return;
                  }}
                  cleanup();
                  resolve({{ message: restoreIds(data), timedOut: false }});
                }};
                window.addEventListener("message", onMessage);
              }});

            const waitForMessages = (types, shouldResolve) =>
              new Promise((resolve) => {{
                const messages = [];
                const timer = window.setTimeout(() => {{
                  cleanup();
                  resolve({{ messages, timedOut: true }});
                }}, timeoutMs);
                const cleanup = () => {{
                  window.clearTimeout(timer);
                  window.removeEventListener("message", onMessage);
                }};
                const onMessage = (event) => {{
                  const data = event && event.data;
                  if (!data || !types.includes(data.type)) {{
                    return;
                  }}
                  messages.push(clone(data));
                  if (shouldResolve(data, messages)) {{
                    cleanup();
                    resolve({{ messages, timedOut: false }});
                  }}
                }};
                window.addEventListener("message", onMessage);
              }});

            if (input?.type === "persisted-atom-sync-request") {{
              const pending = waitForMessage((data) => data?.type === "persisted-atom-sync");
              await sendToHost();
              const result = await pending;
              if (result.timedOut) {{
                return {{
                  messages: [{{ type: "persisted-atom-sync", state: {{}} }}],
                  timedOut: true,
                }};
              }}
              return {{ messages: [result.message] }};
            }}

            if (input?.type === "shared-object-subscribe") {{
              await sendToHost();
              const value = window.electronBridge?.getSharedObjectSnapshotValue?.(input.key);
              return {{
                messages: [{{ type: "shared-object-updated", key: input.key, value: clone(value) }}],
              }};
            }}

            if (input?.type === "electron-add-new-workspace-root-option" && input.root) {{
              const pending = waitForMessages(
                [
                  "workspace-root-options-updated",
                  "active-workspace-roots-updated",
                  "workspace-root-option-added",
                  "navigate-to-route",
                ],
                (data) => data?.type === "navigate-to-route",
              );
              await sendToHost();
              return await pending;
            }}

            if (input?.type === "electron-set-active-workspace-root") {{
              const pending = waitForMessages(
                ["active-workspace-roots-updated"],
                (data) => data?.type === "active-workspace-roots-updated",
              );
              await sendToHost();
              return await pending;
            }}

            if (input?.type === "electron-update-workspace-root-options") {{
              const pending = waitForMessages(
                ["workspace-root-options-updated"],
                (data) => data?.type === "workspace-root-options-updated",
              );
              await sendToHost();
              return await pending;
            }}

            if (input?.type === "fetch" && originalRequestId) {{
              const pending = waitForMessage(
                (data) => data?.type === "fetch-response" && data.requestId === bridgeId,
              );
              await sendToHost();
              const result = await pending;
              if (result.timedOut) {{
                return {{
                  messages: [
                    {{
                      type: "fetch-response",
                      requestId: originalRequestId,
                      responseType: "error",
                      status: 504,
                      error: "Timed out waiting for Codex host fetch response",
                    }},
                  ],
                  timedOut: true,
                }};
              }}
              return {{ messages: [result.message] }};
            }}

            if (input?.type === "fetch-stream" && originalRequestId) {{
              const messages = [];
              const pending = new Promise((resolve) => {{
                const timer = window.setTimeout(() => {{
                  cleanup();
                  resolve({{ timedOut: true }});
                }}, timeoutMs);
                const cleanup = () => {{
                  window.clearTimeout(timer);
                  window.removeEventListener("message", onMessage);
                }};
                const onMessage = (event) => {{
                  const data = event && event.data;
                  if (!data || data.requestId !== bridgeId) {{
                    return;
                  }}
                  if (
                    data.type !== "fetch-stream-event" &&
                    data.type !== "fetch-stream-error" &&
                    data.type !== "fetch-stream-complete"
                  ) {{
                    return;
                  }}
                  messages.push(restoreIds(data));
                  if (data.type === "fetch-stream-error" || data.type === "fetch-stream-complete") {{
                    cleanup();
                    resolve({{ timedOut: false }});
                  }}
                }};
                window.addEventListener("message", onMessage);
              }});
              await sendToHost();
              const result = await pending;
              return {{ messages, timedOut: result.timedOut }};
            }}

            if (input?.type === "mcp-request" && input.request?.id != null) {{
              const mcpRequestId = input.request.id;
              const pending = waitForMessage(
                (data) =>
                  data?.type === "mcp-response" &&
                  data.hostId === input.hostId &&
                  data.message?.id === mcpRequestId,
              );
              await sendToHost();
              const result = await pending;
              if (result.timedOut) {{
                return {{
                  messages: [
                    {{
                      type: "mcp-response",
                      hostId: input.hostId,
                      message: {{
                        id: mcpRequestId,
                        error: {{
                          code: -32000,
                          message: "Timed out waiting for Codex host MCP response",
                        }},
                      }},
                    }},
                  ],
                  timedOut: true,
                }};
              }}
              reflectThreadStartResponseToCodexApp(result.message);
              return {{ messages: [result.message] }};
            }}

            if (threadStreamStateParams(input)) {{
              await sendToHost();
              reflectRemoteThreadStreamStateToCodexApp(input);
              return {{ messages: [] }};
            }}

            if (bridgeId) {{
              const pending = waitForMessage(
                (data) => data?.requestId === bridgeId || data?.id === bridgeId,
              );
              await sendToHost();
              const result = await pending;
              return result.timedOut
                ? {{ messages: [], timedOut: true }}
                : {{ messages: [result.message] }};
            }}

            await sendToHost();
            return {{ messages: [] }};
          }})()"#
    )
}

pub(super) fn web_bridge_stream_start_expression(message: &Value) -> String {
    let message = serde_json::to_string(message).unwrap_or_else(|_| "{}".to_string());
    let idle_timeout_ms = WEB_BRIDGE_STREAM_IDLE_TIMEOUT_MS.to_string();
    r#"(async () => {
      const input = __CODEX_WEB_BRIDGE_STREAM_MESSAGE__;
      const idleTimeoutMs = __CODEX_WEB_BRIDGE_STREAM_IDLE_TIMEOUT_MS__;
      const originalRequestId =
        input && typeof input.requestId === "string" ? input.requestId : null;
      if (!originalRequestId) {
        return { messages: [] };
      }
      const bridgeId = `codex-web-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      if (!window.__codexWebBridgeStreams) {
        window.__codexWebBridgeStreams = Object.create(null);
      }
      const streams = window.__codexWebBridgeStreams;
      const clone = (value) => {
        if (value === undefined) {
          return undefined;
        }
        return JSON.parse(JSON.stringify(value));
      };
      const restoreIds = (value) => {
        const restored = clone(value);
        if (restored && restored.requestId === bridgeId) {
          restored.requestId = originalRequestId;
        }
        return restored;
      };
      const stream = {
        complete: false,
        messages: [],
        onMessage: null,
        timedOut: false,
        timer: null,
      };
      const startIdleTimer = () => {
        if (stream.timer) {
          window.clearTimeout(stream.timer);
        }
        stream.timer = window.setTimeout(() => {
          stream.timedOut = true;
          stream.complete = true;
          cleanup();
        }, idleTimeoutMs);
      };
      const cleanup = () => {
        if (stream.timer) {
          window.clearTimeout(stream.timer);
        }
        if (stream.onMessage) {
          window.removeEventListener("message", stream.onMessage);
        }
        stream.onMessage = null;
        stream.timer = null;
      };
      stream.onMessage = (event) => {
        const data = event && event.data;
        if (!data || data.requestId !== bridgeId) {
          return;
        }
        if (
          data.type !== "fetch-stream-event" &&
          data.type !== "fetch-stream-error" &&
          data.type !== "fetch-stream-complete"
        ) {
          return;
        }
        stream.messages.push(restoreIds(data));
        startIdleTimer();
        if (data.type === "fetch-stream-error" || data.type === "fetch-stream-complete") {
          stream.complete = true;
          cleanup();
        }
      };
      startIdleTimer();
      streams[bridgeId] = stream;
      window.addEventListener("message", stream.onMessage);

      const message = { ...input, requestId: bridgeId };
      const sendToHost = async () => {
        if (window.electronBridge?.sendMessageFromView) {
          await window.electronBridge.sendMessageFromView(message);
        } else {
          window.dispatchEvent(
            new CustomEvent("codex-message-from-view", { detail: message }),
          );
        }
      };
      try {
        await sendToHost();
      } catch (error) {
        const text = error && error.message ? error.message : String(error);
        cleanup();
        delete streams[bridgeId];
        return {
          messages: [
            {
              type: "fetch-stream-error",
              requestId: originalRequestId,
              error: text,
            },
          ],
        };
      }
      return { messages: [], streamKey: bridgeId };
    })()"#
        .replace("__CODEX_WEB_BRIDGE_STREAM_MESSAGE__", &message)
        .replace(
            "__CODEX_WEB_BRIDGE_STREAM_IDLE_TIMEOUT_MS__",
            &idle_timeout_ms,
        )
}

pub(super) fn web_bridge_stream_poll_expression(stream_key: &str, limit: usize) -> String {
    let stream_key = serde_json::to_string(stream_key).unwrap_or_else(|_| "\"\"".to_string());
    r#"(async () => {
      const streamKey = __CODEX_WEB_BRIDGE_STREAM_KEY__;
      const limit = __CODEX_WEB_BRIDGE_STREAM_LIMIT__;
      const streams = window.__codexWebBridgeStreams;
      const stream = streams && streams[streamKey];
      if (!stream) {
        return { messages: [], complete: true, timedOut: false };
      }
      const messages = stream.messages.splice(0, limit);
      const complete = Boolean(stream.complete) && stream.messages.length === 0;
      const timedOut = Boolean(stream.timedOut);
      if (complete) {
        delete streams[streamKey];
      }
      return { messages, complete, timedOut };
    })()"#
        .replace("__CODEX_WEB_BRIDGE_STREAM_KEY__", &stream_key)
        .replace(
            "__CODEX_WEB_BRIDGE_STREAM_LIMIT__",
            &limit.max(1).to_string(),
        )
}

fn web_bridge_stream_cleanup_expression(stream_key: &str) -> String {
    let stream_key = serde_json::to_string(stream_key).unwrap_or_else(|_| "\"\"".to_string());
    r#"(async () => {
      const streamKey = __CODEX_WEB_BRIDGE_STREAM_KEY__;
      const streams = window.__codexWebBridgeStreams;
      const stream = streams && streams[streamKey];
      if (stream) {
        if (stream.timer) {
          window.clearTimeout(stream.timer);
        }
        if (stream.onMessage) {
          window.removeEventListener("message", stream.onMessage);
        }
        delete streams[streamKey];
      }
      return { messages: [], complete: true };
    })()"#
        .replace("__CODEX_WEB_BRIDGE_STREAM_KEY__", &stream_key)
}

pub(super) fn web_bridge_notification_install_expression() -> &'static str {
    r#"(async () => {
      const MAX_QUEUE_MESSAGES = 4096;
      const MAX_QUEUE_BYTES = 8 * 1024 * 1024;
      const state = (window.__codexWebBridgeNotifications ||= {
        installed: false,
        messages: [],
      });
      const FOLLOWER_HOST_RESPONSE_METHODS = new Set([
        "thread-follower-command-approval-decision",
        "thread-follower-file-approval-decision",
        "thread-follower-permissions-request-approval-response",
        "thread-follower-submit-mcp-server-elicitation-response",
        "thread-follower-submit-user-input",
      ]);
      const THREAD_HYDRATION_METHODS = new Set([
        "thread/list",
        "thread/read",
        "thread/search",
        "thread/turns/list",
        "turn/list",
      ]);
      const THREAD_HYDRATION_STREAM_REPLAY_DELAY_MS = 100;
      state.messages = Array.isArray(state.messages) ? state.messages : [];
      state.nextSeq = Number.isFinite(state.nextSeq) ? state.nextSeq : 1;
      state.queuedBytes = Number.isFinite(state.queuedBytes) ? state.queuedBytes : 0;
      state.droppedCount = Number.isFinite(state.droppedCount) ? state.droppedCount : 0;
      state.threadHydrationRequests =
        state.threadHydrationRequests && typeof state.threadHydrationRequests === "object"
          ? state.threadHydrationRequests
          : Object.create(null);
      if (!state.clientId) {
        state.clientId = `codex-web-bridge-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      }
      const serialize = (value) => {
        if (value === undefined) {
          return undefined;
        }
        try {
          const raw = JSON.stringify(value);
          if (raw === undefined) {
            return undefined;
          }
          return { bytes: raw.length, value: JSON.parse(raw) };
        } catch {
          return undefined;
        }
      };
      const notificationMessage = (entry) => entry && entry.message ? entry.message : entry;
      const notificationBytes = (entry) => Number.isFinite(entry && entry.bytes) ? entry.bytes : 0;
      const notificationMethod = (message) =>
        message && message.type === "ipc-broadcast" ? message.method : message?.type;
      const notificationParams = (message) =>
        message && message.type === "ipc-broadcast" ? message.params : message;
      const isTurnStreamMessage = (message) =>
        notificationMethod(message) === "thread-stream-state-changed";
      const isTurnStreamPatch = (message) =>
        isTurnStreamMessage(message) && notificationParams(message)?.change?.type === "patches";
      const isTurnStreamSnapshot = (message) =>
        isTurnStreamMessage(message) && notificationParams(message)?.change?.type === "snapshot";
      const shouldForward = (data) => {
        if (!data || typeof data !== "object" || Array.isArray(data)) {
          return false;
        }
        if (typeof data.type !== "string" || data.type.length === 0) {
          return false;
        }
        if (data.__codexWebBridgeNotificationForwarded) {
          return false;
        }
        return true;
      };
      const ipcRequestBody = (data) => {
        if (!data || typeof data !== "object" || data.type !== "fetch") {
          return null;
        }
        if (typeof data.url !== "string") {
          return null;
        }
        let url;
        try {
          url = new URL(data.url, window.location.href);
        } catch {
          return null;
        }
        if (url.protocol !== "vscode:" || url.hostname !== "codex" || url.pathname !== "/ipc-request") {
          return null;
        }
        if (typeof data.body === "string") {
          try {
            return JSON.parse(data.body);
          } catch {
            return null;
          }
        }
        if (data.body && typeof data.body === "object") {
          return data.body;
        }
        return null;
      };
      const isFollowerHostResponseFetch = (data) => {
        const body = ipcRequestBody(data);
        if (!body || !FOLLOWER_HOST_RESPONSE_METHODS.has(body.method)) {
          return false;
        }
        const targetClientId = typeof body.targetClientId === "string" ? body.targetClientId : "";
        if (!targetClientId.includes("codex-web")) {
          return false;
        }
        const params = body.params && typeof body.params === "object" ? body.params : null;
        return Boolean(params && params.requestId != null);
      };
      const acknowledgeFollowerHostResponseFetch = (data) => {
        if (!isFollowerHostResponseFetch(data) || typeof data.requestId !== "string") {
          return;
        }
        window.dispatchEvent(
          new MessageEvent("message", {
            data: {
              type: "fetch-response",
              requestId: data.requestId,
              responseType: "success",
              status: 200,
              headers: {},
              bodyJsonString: JSON.stringify({
                requestId: "",
                result: { ok: true },
                resultType: "success",
                type: "response",
              }),
            },
            origin: window.location.origin,
            source: window,
          }),
        );
      };
      const mcpRequestMethod = (data) =>
        data?.type === "mcp-request" && typeof data.request?.method === "string"
          ? data.request.method
          : null;
      const mcpRequestId = (data) =>
        data?.type === "mcp-request" && data.request?.id != null
          ? String(data.request.id)
          : null;
      const mcpResponseId = (data) =>
        data?.type === "mcp-response" && data.message?.id != null
          ? String(data.message.id)
          : null;
      const hydrationConversationId = (data) => {
        const params = data?.request?.params;
        if (!params || typeof params !== "object") {
          return null;
        }
        if (typeof params.conversationId === "string") {
          return params.conversationId;
        }
        if (typeof params.threadId === "string") {
          return params.threadId;
        }
        return null;
      };
      const hydrationKey = (hostId, requestId) => `${hostId || ""}:${requestId}`;
      const markBridgeForwarded = (value) => {
        if (!value || typeof value !== "object") {
          return;
        }
        try {
          Object.defineProperty(value, "__codexWebBridgeNotificationForwarded", {
            configurable: true,
            value: true,
          });
        } catch {
          value.__codexWebBridgeNotificationForwarded = true;
        }
      };
      const replayLatestThreadStreamSnapshot = (conversationId) => {
        if (!conversationId) {
          return;
        }
        window.setTimeout(() => {
          const snapshots = window.__codexWebBridgeLatestThreadStreamSnapshots;
          const snapshot = snapshots && snapshots[conversationId];
          if (!snapshot) {
            return;
          }
          const message = serialize(snapshot)?.value;
          if (!message) {
            return;
          }
          markBridgeForwarded(message);
          window.dispatchEvent(
            new MessageEvent("message", {
              data: message,
              origin: window.location.origin,
              source: window,
            }),
          );
        }, THREAD_HYDRATION_STREAM_REPLAY_DELAY_MS);
      };
      const trackThreadHydrationMessage = (data, channel) => {
        if (channel === "codex-message-from-view" && THREAD_HYDRATION_METHODS.has(mcpRequestMethod(data))) {
          const requestId = mcpRequestId(data);
          const conversationId = hydrationConversationId(data);
          if (requestId && conversationId) {
            state.threadHydrationRequests[hydrationKey(data.hostId, requestId)] = {
              conversationId,
              ts: Date.now(),
            };
          }
          return;
        }
        if (channel !== "message") {
          return;
        }
        const responseId = mcpResponseId(data);
        if (!responseId) {
          return;
        }
        const key = hydrationKey(data.hostId, responseId);
        const pending = state.threadHydrationRequests[key];
        if (!pending) {
          return;
        }
        delete state.threadHydrationRequests[key];
        replayLatestThreadStreamSnapshot(pending.conversationId);
      };
      const queueOverLimit = () =>
        state.messages.length > MAX_QUEUE_MESSAGES ||
        (state.queuedBytes > MAX_QUEUE_BYTES && state.messages.length > 1);
      const dropEntryAt = (index) => {
        const entry = state.messages[index];
        state.messages.splice(index, 1);
        state.queuedBytes = Math.max(0, state.queuedBytes - notificationBytes(entry));
        state.droppedCount += 1;
      };
      const trimQueue = () => {
        const beforeDropped = state.droppedCount;
        while (queueOverLimit()) {
          const index = state.messages.findIndex(
            (entry) => !isTurnStreamMessage(notificationMessage(entry)),
          );
          if (index < 0) {
            break;
          }
          dropEntryAt(index);
        }
        while (queueOverLimit()) {
          let index = state.messages.findIndex((entry) =>
            isTurnStreamPatch(notificationMessage(entry)),
          );
          if (index < 0) {
            index = state.messages.findIndex(
              (entry) => !isTurnStreamSnapshot(notificationMessage(entry)),
            );
          }
          if (index < 0) {
            break;
          }
          dropEntryAt(index);
        }
        if (state.droppedCount !== beforeDropped) {
          state.pendingGap = {
            type: "codex-web-bridge-notification-gap",
            droppedCount: state.droppedCount,
            reason: "queue-overflow",
            queued: state.messages.length,
            queuedBytes: state.queuedBytes,
            ts: Date.now(),
          };
        }
      };
      const queueBridgeNotification = (data, channel) => {
        if (!shouldForward(data)) {
          return;
        }
        const serialized = serialize(data);
        if (serialized && serialized.value) {
          const message = serialized.value;
          message.__codexWebBridgeNotificationForwarded = true;
          message.__codexWebBridgeNotificationSeq = state.nextSeq++;
          message.__codexWebBridgeNotificationQueuedAt = Date.now();
          message.__codexEventSource = "cdp-webview";
          message.__codexEventChannel =
            message.type === "ipc-broadcast" ? "ipc-broadcast" : channel;
          message.__codexEventMethod =
            message.type === "ipc-broadcast" ? message.method : message.type;
          state.messages.push({ bytes: serialized.bytes, message });
          state.queuedBytes += serialized.bytes;
          trimQueue();
          if (channel === "codex-message-from-view") {
            acknowledgeFollowerHostResponseFetch(data);
          }
          trackThreadHydrationMessage(data, channel);
        }
      };
      if (!state.installed) {
        window.addEventListener(
          "message",
          (event) => queueBridgeNotification(event && event.data, "message"),
          true,
        );
        window.addEventListener(
          "codex-message-from-view",
          (event) => queueBridgeNotification(event && event.detail, "codex-message-from-view"),
          true,
        );
        state.installed = true;
      }
      return { messages: [] };
    })()"#
}

pub(super) fn web_bridge_notification_poll_expression(limit: usize) -> String {
    r#"(async () => {
      const state = window.__codexWebBridgeNotifications;
      if (!state || !Array.isArray(state.messages)) {
        return { messages: [] };
      }
      const limit = __CODEX_WEB_BRIDGE_NOTIFICATION_LIMIT__;
      const maxBytes = __CODEX_WEB_BRIDGE_NOTIFICATION_MAX_BYTES__;
      const messages = [];
      let bytes = 0;
      const notificationMessage = (entry) => entry && entry.message ? entry.message : entry;
      const notificationBytes = (entry) => Number.isFinite(entry && entry.bytes) ? entry.bytes : 0;
      if (state.pendingGap && messages.length < limit) {
        messages.push(state.pendingGap);
        state.pendingGap = null;
      }
      while (state.messages.length > 0 && messages.length < limit) {
        const entry = state.messages[0];
        const entryBytes = notificationBytes(entry);
        if (messages.length > 0 && bytes + entryBytes > maxBytes) {
          break;
        }
        state.messages.shift();
        state.queuedBytes = Math.max(0, (state.queuedBytes || 0) - entryBytes);
        messages.push(notificationMessage(entry));
        bytes += entryBytes;
      }
      return {
        messages,
        queued: state.messages.length,
        queuedBytes: state.queuedBytes || 0,
        droppedCount: state.droppedCount || 0,
        nextSeq: state.nextSeq || 0,
      };
    })()"#
        .replace(
            "__CODEX_WEB_BRIDGE_NOTIFICATION_LIMIT__",
            &limit.max(1).to_string(),
        )
        .replace(
            "__CODEX_WEB_BRIDGE_NOTIFICATION_MAX_BYTES__",
            &WEB_BRIDGE_NOTIFICATION_POLL_MAX_BYTES.to_string(),
        )
}

pub(super) fn web_bridge_snapshot_request_expression(reason: &str, client_id: &str) -> String {
    let reason = serde_json::to_string(reason).unwrap_or_else(|_| "\"remote-request\"".to_string());
    let client_id =
        serde_json::to_string(client_id).unwrap_or_else(|_| "\"codex-web-bridge\"".to_string());
    r#"(async () => {
      const reason = __CODEX_WEB_BRIDGE_SNAPSHOT_REASON__;
      const clientId = __CODEX_WEB_BRIDGE_SNAPSHOT_CLIENT_ID__;
      const messages = [];
      const clone = (value) => {
        if (value === undefined) {
          return undefined;
        }
        return JSON.parse(JSON.stringify(value));
      };
      const markBridgeForwarded = (value) => {
        if (!value || typeof value !== "object") {
          return;
        }
        try {
          Object.defineProperty(value, "__codexWebBridgeNotificationForwarded", {
            configurable: true,
            value: true,
          });
        } catch {
          value.__codexWebBridgeNotificationForwarded = true;
        }
      };
      const toIpcBroadcast = (detail) => {
        if (
          !detail ||
          typeof detail !== "object" ||
          detail.type !== "thread-stream-state-changed" ||
          !detail.conversationId ||
          !detail.hostId ||
          !detail.change
        ) {
          return null;
        }
        return {
          type: "ipc-broadcast",
          method: "thread-stream-state-changed",
          sourceClientId: clientId,
          version: Number.isFinite(detail.version) ? detail.version : 6,
          params: clone(detail),
        };
      };
      const onMessageFromView = (event) => {
        const broadcast = toIpcBroadcast(event?.detail);
        if (broadcast) {
          messages.push(broadcast);
        }
      };
      window.addEventListener("codex-message-from-view", onMessageFromView, true);
      try {
        const status = {
          type: "ipc-broadcast",
          method: "client-status-changed",
          sourceClientId: clientId,
          version: 0,
          params: {
            clientId,
            clientType: "web",
            status: "connected",
          },
          reason,
        };
        markBridgeForwarded(status);
        window.dispatchEvent(
          new MessageEvent("message", {
            data: status,
            origin: window.location.origin,
            source: window,
          }),
        );
        await new Promise((resolve) => window.setTimeout(resolve, 100));
      } finally {
        window.removeEventListener("codex-message-from-view", onMessageFromView, true);
      }
      return {
        messages,
        snapshotRequest: {
          reason,
          snapshotCount: messages.length,
        },
      };
    })()"#
        .replace("__CODEX_WEB_BRIDGE_SNAPSHOT_REASON__", &reason)
        .replace("__CODEX_WEB_BRIDGE_SNAPSHOT_CLIENT_ID__", &client_id)
}
