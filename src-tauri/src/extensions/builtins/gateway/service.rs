use super::config as gateway_config;
use crate::config::AppConfig;
use crate::extensions::{self, BuiltinNodeExtension};
use crate::AppState;
use serde::Serialize;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_millis(800);

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
    let health_url = gateway_config::gateway_health_url()?;
    if let Some(status) = managed_running_status(state, &health_url).await? {
        return Ok(status);
    }
    if gateway_health_ok(&health_url).await {
        return Ok(GatewayServiceStatus {
            running: true,
            managed: false,
            pid: None,
            health_url,
            message: "running".to_string(),
        });
    }

    let extension =
        tokio::task::spawn_blocking(extensions::resolve_builtin_next_ai_gateway_extension)
            .await
            .map_err(|err| err.to_string())??;
    let config_file = gateway_config::read_gateway_config()?;

    let mut guard = state.gateway_service.lock().await;
    if let Some(status) = managed_status_from_guard(&mut guard, &health_url).await? {
        return Ok(status);
    }
    if gateway_health_ok(&health_url).await {
        return Ok(GatewayServiceStatus {
            running: true,
            managed: false,
            pid: None,
            health_url,
            message: "running".to_string(),
        });
    }

    let mut handle = start_process(&extension, &config_file.path, health_url.clone())?;
    let pid = handle.child.id();
    wait_until_ready(&mut handle).await?;
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

async fn managed_running_status(
    state: &AppState,
    health_url: &str,
) -> Result<Option<GatewayServiceStatus>, String> {
    let mut guard = state.gateway_service.lock().await;
    managed_status_from_guard(&mut guard, health_url).await
}

async fn managed_status_from_guard(
    guard: &mut Option<GatewayServiceHandle>,
    health_url: &str,
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

    if handle.health_url == health_url && gateway_health_ok(health_url).await {
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
) -> Result<GatewayServiceHandle, String> {
    let mut command = Command::new(&extension.node.executable);
    command
        .arg(&extension.entry_path)
        .current_dir(&extension.root_dir)
        .env("GATEWAY_CONFIG_PATH", config_path)
        .env("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH", config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command
        .spawn()
        .map_err(|err| format!("failed to start NeXT AI Gateway: {}", err))?;

    Ok(GatewayServiceHandle { child, health_url })
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
