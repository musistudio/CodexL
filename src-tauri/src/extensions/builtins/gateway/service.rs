use super::config as gateway_config;
use crate::config::AppConfig;
use crate::extensions::{self, BuiltinNodeExtension};
use crate::launcher;
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_millis(800);
const STALE_GATEWAY_STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct GatewayServiceHandle {
    child: Child,
    health_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayServiceStatus {
    pub running: bool,
    pub managed: bool,
    pub pid: Option<u32>,
    pub health_url: String,
    pub message: String,
}

impl Drop for GatewayServiceHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub async fn sync_with_config(
    state: &AppState,
    config: &AppConfig,
) -> Result<GatewayServiceStatus, String> {
    if config.extensions.enabled && config.extensions.next_ai_gateway_enabled {
        ensure_running(state).await
    } else {
        stop(state).await?;
        Ok(GatewayServiceStatus {
            running: false,
            managed: false,
            pid: None,
            health_url: gateway_config::gateway_health_url().unwrap_or_default(),
            message: "disabled".to_string(),
        })
    }
}

pub async fn ensure_running(state: &AppState) -> Result<GatewayServiceStatus, String> {
    let app_config = state.config.lock().await.clone();
    let health_url = gateway_config::gateway_health_url()?;
    let auth_probe_url = gateway_config::gateway_agent_tools_url()?;
    if let Some(status) = managed_running_status(state, &health_url, &auth_probe_url).await? {
        return Ok(status);
    }
    if gateway_health_ok(&health_url).await {
        if gateway_rejects_unauthenticated(&auth_probe_url).await {
            return Ok(unmanaged_running_status(&health_url));
        }
    }

    let extension =
        tokio::task::spawn_blocking(extensions::resolve_builtin_next_ai_gateway_extension)
            .await
            .map_err(|err| err.to_string())??;
    let config_file = gateway_config::read_gateway_config()?;
    let auth_introspection_url = codexl_gateway_auth_introspection_url(&app_config);
    let auth_secret = gateway_config::codex_provider_api_key()?;
    let usage_webhook_url = if gateway_usage_capture_enabled(&config_file.config) {
        Some(codexl_gateway_usage_webhook_url(&app_config))
    } else {
        None
    };
    let request_logging_enabled = gateway_request_logging_enabled(&config_file.config);

    let mut guard = state.gateway_service.lock().await;
    if let Some(status) =
        managed_status_from_guard(&mut guard, &health_url, &auth_probe_url).await?
    {
        return Ok(status);
    }
    if let Some(status) =
        unmanaged_running_status_or_clear_stale(&health_url, &auth_probe_url).await?
    {
        return Ok(status);
    }

    let mut handle = start_process(
        &extension,
        &config_file.path,
        health_url.clone(),
        &auth_introspection_url,
        &auth_secret,
        usage_webhook_url.as_deref(),
        request_logging_enabled,
    )?;
    let pid = handle.child.id();
    wait_until_ready(&mut handle).await?;
    if !gateway_rejects_unauthenticated(&auth_probe_url).await {
        return Err(format!(
            "NeXT AI Gateway started at {}, but authentication was not enforced.",
            health_url
        ));
    }
    *guard = Some(handle);

    Ok(GatewayServiceStatus {
        running: true,
        managed: true,
        pid: Some(pid),
        health_url,
        message: "started".to_string(),
    })
}

pub async fn restart(state: &AppState) -> Result<GatewayServiceStatus, String> {
    stop(state).await?;
    ensure_running(state).await
}

pub async fn stop(state: &AppState) -> Result<(), String> {
    let handle = state.gateway_service.lock().await.take();
    drop(handle);
    Ok(())
}

fn unmanaged_running_status(health_url: &str) -> GatewayServiceStatus {
    GatewayServiceStatus {
        running: true,
        managed: false,
        pid: None,
        health_url: health_url.to_string(),
        message: "running".to_string(),
    }
}

async fn unmanaged_running_status_or_clear_stale(
    health_url: &str,
    auth_probe_url: &str,
) -> Result<Option<GatewayServiceStatus>, String> {
    if !gateway_health_ok(health_url).await {
        return Ok(None);
    }

    let Some(status) = gateway_unauthenticated_probe_status(auth_probe_url).await else {
        return Err(format!(
            "NeXT AI Gateway is healthy at {}, but CodexL could not verify whether it enforces authentication. Stop that process or change the Gateway port.",
            health_url
        ));
    };
    if gateway_status_rejects_unauthenticated(status) {
        return Ok(Some(unmanaged_running_status(health_url)));
    }

    clear_stale_unauthenticated_gateway(health_url, auth_probe_url).await?;
    Ok(None)
}

async fn clear_stale_unauthenticated_gateway(
    health_url: &str,
    auth_probe_url: &str,
) -> Result<(), String> {
    if let Some(port) = gateway_port_from_url(health_url) {
        let stopped = tokio::task::spawn_blocking(move || {
            launcher::stop_next_ai_gateway_processes_for_port(port)
        })
        .await
        .map_err(|err| err.to_string())?
        .unwrap_or(false);
        if stopped && wait_until_gateway_cleared_or_authenticated(health_url, auth_probe_url).await
        {
            return Ok(());
        }
    }

    if gateway_is_next_ai_gateway(health_url).await {
        tokio::task::spawn_blocking(launcher::stop_next_ai_gateway_processes)
            .await
            .map_err(|err| err.to_string())??;
        if wait_until_gateway_cleared_or_authenticated(health_url, auth_probe_url).await {
            return Ok(());
        }
    }

    Err(format!(
        "{} CodexL tried to stop stale NeXT AI Gateway processes, but the Gateway at this port is still unauthenticated.",
        unauthenticated_gateway_error(health_url)
    ))
}

async fn wait_until_gateway_cleared_or_authenticated(
    health_url: &str,
    auth_probe_url: &str,
) -> bool {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < STALE_GATEWAY_STOP_TIMEOUT {
        if !gateway_health_ok(health_url).await
            || gateway_rejects_unauthenticated(auth_probe_url).await
        {
            return true;
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL).await;
    }
    false
}

fn unauthenticated_gateway_error(health_url: &str) -> String {
    format!(
        "NeXT AI Gateway is already running at {}, but it does not enforce CodexL authentication. Stop that process or change the Gateway port.",
        health_url
    )
}

async fn managed_running_status(
    state: &AppState,
    health_url: &str,
    auth_probe_url: &str,
) -> Result<Option<GatewayServiceStatus>, String> {
    let mut guard = state.gateway_service.lock().await;
    managed_status_from_guard(&mut guard, health_url, auth_probe_url).await
}

async fn managed_status_from_guard(
    guard: &mut Option<GatewayServiceHandle>,
    health_url: &str,
    auth_probe_url: &str,
) -> Result<Option<GatewayServiceStatus>, String> {
    let Some(handle) = guard.as_mut() else {
        return Ok(None);
    };

    if handle
        .child
        .try_wait()
        .map_err(|err| err.to_string())?
        .is_some()
    {
        *guard = None;
        return Ok(None);
    }

    if handle.health_url == health_url
        && gateway_health_ok(health_url).await
        && gateway_rejects_unauthenticated(auth_probe_url).await
    {
        return Ok(Some(GatewayServiceStatus {
            running: true,
            managed: true,
            pid: Some(handle.child.id()),
            health_url: health_url.to_string(),
            message: "running".to_string(),
        }));
    }

    *guard = None;
    Ok(None)
}

fn start_process(
    extension: &BuiltinNodeExtension,
    config_path: &str,
    health_url: String,
    auth_introspection_url: &str,
    auth_secret: &str,
    usage_webhook_url: Option<&str>,
    request_logging_enabled: bool,
) -> Result<GatewayServiceHandle, String> {
    let mut command = Command::new(&extension.node.executable);
    command
        .arg(&extension.entry_path)
        .current_dir(&extension.root_dir)
        .env("CODEXL_HOME", super::super::codexl_home_dir())
        .env("GATEWAY_CONFIG_PATH", config_path)
        .env("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH", config_path)
        .env("AUTH_ENABLED", "true")
        .env("AUTH_MODE", "http_introspection")
        .env("AUTH_REQUIRED", "true")
        .env("AUTH_INTROSPECTION_ENDPOINT", auth_introspection_url)
        .env("AUTH_INTROSPECTION_TOKEN_HEADER", "authorization")
        .env("AUTH_INTROSPECTION_TOKEN_BEARER_ONLY", "true")
        .env(
            "AUTH_INTROSPECTION_CREDENTIAL_HEADER",
            gateway_config::GATEWAY_AUTH_CREDENTIAL_HEADER,
        )
        .env(
            "AUTH_INTROSPECTION_CREDENTIAL_ENV",
            "CODEXL_GATEWAY_AUTH_SECRET",
        )
        .env("CODEXL_GATEWAY_AUTH_SECRET", auth_secret)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(usage_webhook_url) = usage_webhook_url {
        command
            .env("BILLING_ENABLED", "true")
            .env("BILLING_WEBHOOK_ENABLED", "true")
            .env("BILLING_WEBHOOK_ENDPOINT", usage_webhook_url)
            .env("BILLING_WEBHOOK_TIMEOUT_MS", "2000");
    }

    if request_logging_enabled {
        command
            .env("RAW_TRACE_MODE", "wire_raw")
            .env("RAW_TRACE_MAX_PART_BYTES", "2147483647")
            .env("RAW_TRACE_DELETE_LOCAL_AFTER_UPLOAD", "false");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let child = command
        .spawn()
        .map_err(|err| format!("failed to start NeXT AI Gateway: {}", err))?;

    Ok(GatewayServiceHandle { child, health_url })
}

fn gateway_usage_capture_enabled(config: &Value) -> bool {
    match config
        .get("codexlUsageCapture")
        .and_then(|value| value.get("enabled"))
    {
        Some(Value::Bool(enabled)) => *enabled,
        Some(Value::String(enabled)) => enabled.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn gateway_request_logging_enabled(config: &Value) -> bool {
    let Some(raw_trace) = config.get("rawTrace") else {
        return false;
    };
    let enabled = match raw_trace.get("enabled") {
        Some(Value::Bool(enabled)) => *enabled,
        Some(Value::String(enabled)) => enabled.trim().eq_ignore_ascii_case("true"),
        _ => false,
    };
    let mode = raw_trace
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    enabled && !mode.eq_ignore_ascii_case("disabled")
}

fn codexl_gateway_usage_webhook_url(config: &AppConfig) -> String {
    format!("{}/gateway/usage", codexl_http_origin(config))
}

fn codexl_gateway_auth_introspection_url(config: &AppConfig) -> String {
    format!("{}/gateway/auth/introspect", codexl_http_origin(config))
}

fn codexl_http_origin(config: &AppConfig) -> String {
    let host = match config.http_host.trim() {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        value => value,
    };
    let host_part = if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    };
    format!("http://{}:{}", host_part, config.http_port)
}

async fn wait_until_ready(handle: &mut GatewayServiceHandle) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < READINESS_TIMEOUT {
        if gateway_health_ok(&handle.health_url).await {
            return Ok(());
        }
        if let Some(status) = handle.child.try_wait().map_err(|err| err.to_string())? {
            return Err(format!(
                "NeXT AI Gateway exited before it became healthy (status {})",
                status
            ));
        }
        tokio::time::sleep(READINESS_POLL_INTERVAL).await;
    }

    Err(format!(
        "NeXT AI Gateway did not become healthy at {} within {} seconds",
        handle.health_url,
        READINESS_TIMEOUT.as_secs()
    ))
}

async fn gateway_is_next_ai_gateway(health_url: &str) -> bool {
    let Some(root_url) = gateway_root_url_from_health_url(health_url) else {
        return false;
    };
    let client = match reqwest::Client::builder()
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let response = match client.get(root_url).send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return false,
    };
    let payload = match response.json::<Value>().await {
        Ok(payload) => payload,
        Err(_) => return false,
    };

    payload
        .get("name")
        .and_then(Value::as_str)
        .map(|name| name == "next-ai-gateway")
        .unwrap_or(false)
}

fn gateway_root_url_from_health_url(health_url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(health_url).ok()?;
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn gateway_port_from_url(url: &str) -> Option<u16> {
    let url = reqwest::Url::parse(url).ok()?;
    url.port_or_known_default()
}

async fn gateway_health_ok(url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    client
        .get(url)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

async fn gateway_rejects_unauthenticated(url: &str) -> bool {
    gateway_unauthenticated_probe_status(url)
        .await
        .map(gateway_status_rejects_unauthenticated)
        .unwrap_or(false)
}

async fn gateway_unauthenticated_probe_status(url: &str) -> Option<reqwest::StatusCode> {
    let client = match reqwest::Client::builder()
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => return None,
    };

    client
        .get(url)
        .send()
        .await
        .map(|response| response.status())
        .ok()
}

fn gateway_status_rejects_unauthenticated(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_root_url_from_health_url_normalizes_to_origin_root() {
        assert_eq!(
            gateway_root_url_from_health_url("http://127.0.0.1:14589/health").as_deref(),
            Some("http://127.0.0.1:14589/")
        );
        assert_eq!(
            gateway_root_url_from_health_url("http://[::1]:14589/health?probe=1").as_deref(),
            Some("http://[::1]:14589/")
        );
    }

    #[test]
    fn gateway_port_from_url_reads_explicit_and_known_default_ports() {
        assert_eq!(
            gateway_port_from_url("http://127.0.0.1:14589/health"),
            Some(14589)
        );
        assert_eq!(
            gateway_port_from_url("https://example.test/health"),
            Some(443)
        );
    }
}
