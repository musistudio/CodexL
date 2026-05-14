use super::*;

pub(super) async fn list_targets(cdp_host: &str, cdp_port: u16) -> Result<Vec<CdpTarget>, String> {
    let url = format!("http://{}:{}/json/list", cdp_host, cdp_port);
    reqwest::get(url)
        .await
        .map_err(|e| e.to_string())?
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|e| e.to_string())
}

pub(super) fn select_target(targets: &[CdpTarget]) -> Option<CdpTarget> {
    let page_targets: Vec<&CdpTarget> = targets
        .iter()
        .filter(|target| !target.web_socket_debugger_url.is_empty() && target.target_type == "page")
        .collect();
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
        .or_else(|| {
            targets
                .iter()
                .find(|target| !target.web_socket_debugger_url.is_empty())
                .cloned()
        })
}

pub(super) async fn connect_target(
    target: &CdpTarget,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    let (socket, _) = tokio_tungstenite::connect_async(&target.web_socket_debugger_url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(socket)
}
pub(super) async fn cdp_send(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    socket
        .send(Message::Text(
            json!({ "id": id, "method": method, "params": params }).to_string(),
        ))
        .await
        .map_err(|e| e.to_string())?;

    loop {
        let message =
            tokio::time::timeout(Duration::from_millis(CDP_COMMAND_TIMEOUT_MS), socket.next())
                .await
                .map_err(|_| format!("CDP command timed out: {}", method))?
                .ok_or_else(|| "CDP socket closed".to_string())?
                .map_err(|e| e.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let value = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
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
        return Ok(value.get("result").cloned().unwrap_or_else(|| json!({})));
    }
}
