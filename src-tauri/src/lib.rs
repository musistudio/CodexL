mod claude_code_app_server;
mod cli;
mod cli_middleware;
mod config;
mod extensions;
mod gateway_usage;
mod launcher;
#[cfg(target_os = "macos")]
mod macos_tray;
mod platforms;
mod ports;
pub(crate) mod remote;
mod server;

use config::{
    AppConfig, BotProfileConfig, CodexProfileConfigFormat, DefaultProviderProfile,
    ExistingProviderRequest, NewProviderRequest, NextAiGatewayProviderRequest,
    RemoteCloudAuthConfig, UpdateNextAiGatewayProviderRequest, UpdateProviderRequest,
    UpdateWorkspaceRequest, WorkspaceRequest, DEFAULT_PROVIDER_PROFILE_NAME,
};
use extensions::builtins::bot_bridge;
use extensions::builtins::gateway::{config as gateway_config, service as gateway_service};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;

pub fn run_cli_middleware_if_requested() -> bool {
    cli_middleware::run_if_requested()
}

pub fn run_cli_if_requested() -> bool {
    cli::run_if_requested()
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) instances: Arc<Mutex<HashMap<String, server::ManagedInstance>>>,
    pub(crate) remote_controls: Arc<Mutex<HashMap<String, remote::RemoteControlHandle>>>,
    pub(crate) bot_login_sessions: Arc<Mutex<HashMap<String, Arc<bot_bridge::BotQrLoginSession>>>>,
    pub(crate) gateway_service: Arc<Mutex<Option<gateway_service::GatewayServiceHandle>>>,
    pub(crate) config: Arc<Mutex<AppConfig>>,
}

impl AppState {
    fn new(config: AppConfig) -> Self {
        Self {
            instances: Arc::new(Mutex::new(HashMap::new())),
            remote_controls: Arc::new(Mutex::new(HashMap::new())),
            bot_login_sessions: Arc::new(Mutex::new(HashMap::new())),
            gateway_service: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(config)),
        }
    }
}

async fn profile_config_format_for_state(state: &AppState) -> CodexProfileConfigFormat {
    let codex_path = {
        let config = state.config.lock().await;
        config.codex_path.clone()
    };
    let executable = launcher::resolve_codex_cli_executable(None, &codex_path);
    config::codex_profile_config_format_for_cli(&executable)
}

#[tauri::command]
async fn find_codex(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let configured_path = {
        let config = state.config.lock().await;
        config.codex_path.trim().to_string()
    };
    if !configured_path.is_empty() && std::path::Path::new(&configured_path).is_file() {
        return Ok(configured_path);
    }
    launcher::find_codex_app().ok_or_else(|| "Codex app not found".to_string())
}

#[derive(Debug, Clone, Serialize)]
struct CodexWebAssetVersions {
    latest: String,
    versions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderModelsProbeRequest {
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    provider_hint: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderModelsProbeResponse {
    models: Vec<String>,
}

#[tauri::command]
async fn list_codex_web_asset_versions(
    registry_url: String,
) -> Result<CodexWebAssetVersions, String> {
    let registry_url = registry_url.trim().trim_end_matches('/');
    if registry_url.is_empty() {
        return Err("REGISTRY_URL is required".to_string());
    }
    let versions_url = reqwest::Url::parse(&format!("{}/versions.json", registry_url))
        .map_err(|e| format!("Invalid REGISTRY_URL: {}", e))?;
    let response = reqwest::get(versions_url)
        .await
        .map_err(|e| format!("Failed to fetch versions.json: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Failed to fetch versions.json: {}", e))?;
    let manifest = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Invalid versions.json: {}", e))?;
    let latest = manifest
        .get("latest")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let mut versions = Vec::new();
    if let Some(items) = manifest.get("versions").and_then(|value| value.as_array()) {
        for item in items {
            let version = item
                .as_str()
                .or_else(|| item.get("version").and_then(|value| value.as_str()))
                .map(str::trim)
                .unwrap_or_default();
            if !version.is_empty() && !versions.iter().any(|existing| existing == version) {
                versions.push(version.to_string());
            }
        }
    }
    if !latest.is_empty() && !versions.iter().any(|version| version == &latest) {
        versions.insert(0, latest.clone());
    }
    if versions.is_empty() {
        return Err("versions.json does not contain any versions".to_string());
    }
    Ok(CodexWebAssetVersions { latest, versions })
}

#[tauri::command]
async fn probe_provider_models(
    request: ProviderModelsProbeRequest,
) -> Result<ProviderModelsProbeResponse, String> {
    let provider_hint = request.provider_hint.trim().to_ascii_lowercase();
    let urls = provider_models_probe_urls(&request.base_url, &provider_hint)?;
    let api_key = request.api_key.trim().to_string();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|err| err.to_string())?;

    let mut last_error = String::new();
    for url in urls {
        let mut url = url;
        let uses_gemini_key_query = provider_models_uses_gemini_key_query(&url, &provider_hint);
        if uses_gemini_key_query && !api_key.is_empty() {
            let has_key = url.query_pairs().any(|(key, _)| key == "key");
            if !has_key {
                url.query_pairs_mut().append_pair("key", &api_key);
            }
        }

        let mut probe = client.get(url.clone());
        if provider_models_uses_anthropic_headers(&url, &provider_hint) {
            probe = probe.header("anthropic-version", "2023-06-01");
        }
        if !api_key.is_empty() && !uses_gemini_key_query {
            probe = probe
                .bearer_auth(&api_key)
                .header("x-api-key", &api_key)
                .header("api-key", &api_key);
        }

        match probe.send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    last_error = format!("models request failed: {}", status);
                    continue;
                }
                match response.json::<serde_json::Value>().await {
                    Ok(value) => {
                        return Ok(ProviderModelsProbeResponse {
                            models: models_from_list_response(&value),
                        });
                    }
                    Err(err) => {
                        last_error = format!("invalid models response: {}", err);
                    }
                }
            }
            Err(err) => {
                last_error = format!("failed to fetch models: {}", err);
            }
        }
    }

    Err(if last_error.is_empty() {
        "failed to fetch models".to_string()
    } else {
        last_error
    })
}

fn provider_models_probe_urls(
    base_url: &str,
    provider_hint: &str,
) -> Result<Vec<reqwest::Url>, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("base_url is required".to_string());
    }
    let base = reqwest::Url::parse(trimmed).map_err(|err| format!("invalid base_url: {}", err))?;
    if base.scheme() != "http" && base.scheme() != "https" {
        return Err("base_url must use http or https".to_string());
    }
    if base.path().trim_end_matches('/').ends_with("/models") {
        return Ok(vec![base]);
    }

    let looks_like_gemini = provider_models_looks_like_gemini(&base, provider_hint);
    let mut result: Vec<reqwest::Url> = Vec::new();
    let mut push_url = |value: String| {
        if let Ok(url) = reqwest::Url::parse(&value) {
            if !result.iter().any(|item| item.as_str() == url.as_str()) {
                result.push(url);
            }
        }
    };
    let normalized = format!("{}/", trimmed);
    if base.path() == "/" || base.path().is_empty() {
        if looks_like_gemini {
            push_url(format!("{}v1beta/models", normalized));
        }
        push_url(format!("{}v1/models", normalized));
    }
    push_url(format!("{}models", normalized));
    Ok(result)
}

fn provider_models_looks_like_gemini(url: &reqwest::Url, provider_hint: &str) -> bool {
    provider_hint.contains("gemini")
        || provider_hint.contains("google")
        || url
            .host_str()
            .map(|host| host.contains("generativelanguage.googleapis.com"))
            .unwrap_or(false)
}

fn provider_models_uses_gemini_key_query(url: &reqwest::Url, provider_hint: &str) -> bool {
    provider_models_looks_like_gemini(url, provider_hint)
        && url
            .host_str()
            .map(|host| host.ends_with("googleapis.com"))
            .unwrap_or(false)
}

fn provider_models_uses_anthropic_headers(url: &reqwest::Url, provider_hint: &str) -> bool {
    provider_hint.contains("anthropic")
        || url
            .host_str()
            .map(|host| host.contains("anthropic.com"))
            .unwrap_or(false)
}

fn models_from_list_response(value: &serde_json::Value) -> Vec<String> {
    let mut models = Vec::new();
    collect_models_from_value(value, &mut models);
    models
}

fn collect_models_from_value(value: &serde_json::Value, models: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_model_item(item, models);
            }
        }
        serde_json::Value::Object(map) => {
            let mut found_list = false;
            for key in ["data", "models", "items", "results", "model_list"] {
                if let Some(list) = map.get(key) {
                    found_list = true;
                    collect_models_from_value(list, models);
                }
            }
            if !found_list {
                collect_model_item(value, models);
            }
        }
        serde_json::Value::String(value) => push_model_list(models, value),
        _ => {}
    }
}

fn collect_model_item(value: &serde_json::Value, models: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => push_model_list(models, value),
        serde_json::Value::Object(map) => {
            if map
                .get("hidden")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                return;
            }
            let mut found_list = false;
            for key in ["data", "models", "items", "results", "model_list"] {
                if let Some(list) = map.get(key) {
                    found_list = true;
                    collect_models_from_value(list, models);
                }
            }
            if found_list {
                return;
            }
            for key in ["id", "model", "name", "slug", "display_name", "displayName"] {
                if let Some(model) = model_string_value(map.get(key)) {
                    push_unique_model(models, &model);
                    return;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_model_item(item, models);
            }
        }
        _ => {}
    }
}

fn model_string_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Object(map) => ["id", "model", "name", "slug"]
            .iter()
            .find_map(|key| model_string_value(map.get(*key))),
        _ => None,
    }
}

fn push_model_list(models: &mut Vec<String>, value: &str) {
    for model in value.split(',') {
        push_unique_model(models, model);
    }
}

fn push_unique_model(models: &mut Vec<String>, value: &str) {
    let model = value.trim().trim_start_matches('/').to_string();
    if !model.is_empty() && !models.iter().any(|item| item == &model) {
        models.push(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn models_list_response_supports_common_shapes() {
        let value = json!({
            "data": [
                { "id": "gpt-4.1" },
                { "id": "hidden-model", "hidden": true },
                { "model": { "id": "o3" } },
                "claude-3-7-sonnet, /gemini-2.5-pro",
                { "name": "gpt-4.1" }
            ],
            "models": [
                "deepseek-chat",
                { "slug": "qwen-max" }
            ],
            "items": [
                { "displayName": "llama-3.3" }
            ],
            "results": [
                { "model": "mistral-large" }
            ],
            "model_list": [
                { "name": "command-r-plus" }
            ]
        });

        assert_eq!(
            models_from_list_response(&value),
            vec![
                "gpt-4.1",
                "o3",
                "claude-3-7-sonnet",
                "gemini-2.5-pro",
                "deepseek-chat",
                "qwen-max",
                "llama-3.3",
                "mistral-large",
                "command-r-plus"
            ]
        );
    }

    #[test]
    fn models_list_response_prefers_nested_lists_over_provider_names() {
        let value = json!({
            "data": [
                {
                    "name": "openai",
                    "models": [
                        { "id": "gpt-4.1" },
                        { "id": "gpt-4.1-mini" }
                    ]
                },
                {
                    "provider": "anthropic",
                    "items": [
                        { "name": "claude-3-7-sonnet" }
                    ]
                }
            ]
        });

        assert_eq!(
            models_from_list_response(&value),
            vec!["gpt-4.1", "gpt-4.1-mini", "claude-3-7-sonnet"]
        );
    }

    #[test]
    fn provider_models_probe_urls_include_gemini_v1beta_for_google_root() {
        let urls = provider_models_probe_urls(
            "https://generativelanguage.googleapis.com",
            "gemini_generate_content",
        )
        .expect("probe urls");

        assert_eq!(
            urls.iter().map(reqwest::Url::as_str).collect::<Vec<_>>(),
            vec![
                "https://generativelanguage.googleapis.com/v1beta/models",
                "https://generativelanguage.googleapis.com/v1/models",
                "https://generativelanguage.googleapis.com/models"
            ]
        );
        assert!(provider_models_uses_gemini_key_query(
            &urls[0],
            "gemini_generate_content"
        ));
    }

    #[test]
    fn provider_models_probe_urls_add_anthropic_header_hint() {
        let urls = provider_models_probe_urls("https://api.anthropic.com/v1", "anthropic_messages")
            .expect("probe urls");

        assert_eq!(
            urls.iter().map(reqwest::Url::as_str).collect::<Vec<_>>(),
            vec!["https://api.anthropic.com/v1/models"]
        );
        assert!(provider_models_uses_anthropic_headers(
            &urls[0],
            "anthropic_messages"
        ));
    }
}

#[tauri::command]
async fn launch_codex(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    cdp_port: Option<u16>,
    codex_path: Option<String>,
    codex_home: Option<String>,
    profile_name: Option<String>,
) -> Result<server::LaunchInfo, String> {
    let requested_profile_name = {
        let config = state.config.lock().await;
        profile_name
            .clone()
            .unwrap_or_else(|| config.active_provider.clone())
    };
    let uses_cli_mode = {
        let config = state.config.lock().await;
        config
            .provider_profile(&requested_profile_name)
            .map(|profile| config::remote_frontend_mode_uses_cli(&profile.remote_frontend_mode))
            .unwrap_or(false)
    };
    if uses_cli_mode {
        return Err(
            "CLI mode workspaces do not launch Codex App. Start remote control instead."
                .to_string(),
        );
    }

    let info = server::launch_codex_instance(
        state.inner(),
        server::LaunchRequest {
            cdp_port,
            codex_path,
            codex_home,
            profile_name,
        },
    )
    .await?;
    let config = state.config.lock().await.clone();
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(info)
}

#[tauri::command]
async fn stop_codex(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    profile_name: Option<String>,
) -> Result<(), String> {
    server::stop_codex_instance(state.inner(), profile_name).await?;
    let config = state.config.lock().await.clone();
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(())
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().await;
    Ok(config.clone())
}

#[tauri::command]
async fn update_config(
    mut new_config: AppConfig,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    {
        let current_config = state.config.lock().await;
        for (key, token) in &current_config.remote_control_tokens {
            new_config
                .remote_control_tokens
                .entry(key.clone())
                .or_insert_with(|| token.clone());
        }
    }
    new_config.normalize();
    ensure_extensions_runtime_for_config(&new_config).await?;
    new_config.save()?;
    if let Some(codex_home) = new_config.active_codex_home() {
        config::remove_retired_builtin_mcp_configs_for_launch(codex_home)?;
    }
    let gateway_config = new_config.clone();
    let mut config = state.config.lock().await;
    *config = new_config;
    drop(config);
    remote::update_remote_transcribe_api_config(state.inner(), &gateway_config).await;
    gateway_service::sync_with_config(state.inner(), &gateway_config)
        .await
        .map(|_| ())?;
    refresh_macos_tray_menu(&app, state.inner(), &gateway_config).await;
    Ok(())
}

#[tauri::command]
async fn update_remote_cloud_auth(
    mut remote_cloud_auth: RemoteCloudAuthConfig,
    remote_relay_url: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    remote_cloud_auth.normalize();

    let next_config = {
        let mut config = state.config.lock().await;
        config.remote_cloud_auth = remote_cloud_auth;
        config.remote_relay_url = remote_relay_url;
        config.normalize();
        config.save()?;
        config.clone()
    };

    refresh_macos_tray_menu(&app, state.inner(), &next_config).await;
    Ok(next_config)
}

async fn ensure_extensions_runtime_for_config(config: &AppConfig) -> Result<(), String> {
    if !config.extensions.enabled {
        return Ok(());
    }
    tokio::task::spawn_blocking(extensions::prepare_builtin_extensions_runtime)
        .await
        .map_err(|err| err.to_string())?
        .map(|_| ())
        .map_err(|err| {
            format!(
                "Extensions require Node.js 20+; automatic Node.js setup failed: {}",
                err
            )
        })
}

#[tauri::command]
async fn get_status(state: tauri::State<'_, AppState>) -> Result<server::LaunchInfo, String> {
    server::current_launch_info(state.inner()).await
}

#[tauri::command]
async fn get_instance_statuses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<server::InstanceStatus>, String> {
    server::instance_statuses(state.inner()).await
}

#[tauri::command]
async fn start_remote_control(
    profile_name: String,
    remote_password: Option<String>,
    use_cloud_relay: Option<bool>,
    require_e2ee: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<remote::RemoteControlInfo, String> {
    let info = remote::start_remote_control(
        state.inner(),
        profile_name,
        remote_password,
        use_cloud_relay,
        require_e2ee,
    )
    .await?;
    let config = state.config.lock().await.clone();
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(info)
}

#[tauri::command]
async fn stop_remote_control(
    profile_name: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    remote::stop_remote_control(state.inner(), &profile_name).await?;
    let config = state.config.lock().await.clone();
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(())
}

#[tauri::command]
async fn set_start_remote_on_launch(
    profile_name: String,
    enabled: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let config = {
        let mut config = state.config.lock().await;
        config.set_start_remote_on_launch(&profile_name, enabled)?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn set_remote_launch_options(
    profile_name: String,
    start_remote: bool,
    start_cloud: bool,
    remote_e2ee_password: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let config = {
        let mut config = state.config.lock().await;
        config.set_remote_launch_options(
            &profile_name,
            start_remote,
            start_cloud,
            remote_e2ee_password,
        )?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
fn get_gateway_config() -> Result<gateway_config::GatewayConfigFile, String> {
    gateway_config::read_gateway_config()
}

#[tauri::command]
async fn update_gateway_config(
    config: serde_json::Value,
    state: tauri::State<'_, AppState>,
) -> Result<gateway_config::GatewayConfigFile, String> {
    let file = gateway_config::write_gateway_config(config)?;
    let should_restart = {
        let config = state.config.lock().await;
        config.extensions.enabled && config.extensions.next_ai_gateway_enabled
    };
    if should_restart {
        gateway_service::restart(state.inner()).await?;
    }
    Ok(file)
}

#[tauri::command]
async fn get_gateway_tools(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    ensure_next_ai_gateway_enabled(state.inner()).await?;
    gateway_service::ensure_running(state.inner()).await?;

    let url = gateway_config::gateway_agent_tools_url()?;
    let api_key = gateway_config::codex_provider_api_key()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|err| err.to_string())?;
    let mut request = client.get(&url);
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|err| format!("failed to fetch Gateway MCP tools: {}", err))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Gateway MCP tools request failed: {}{}",
            status,
            if body.trim().is_empty() {
                String::new()
            } else {
                format!(" {}", body.trim())
            }
        ));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| format!("failed to parse Gateway MCP tools: {}", err))
}

#[tauri::command]
async fn get_gateway_usage_summary(
    state: tauri::State<'_, AppState>,
    days: Option<u32>,
    start_date: Option<String>,
    end_date: Option<String>,
    hours: Option<u32>,
) -> Result<gateway_usage::GatewayUsageSummary, String> {
    let codex_home = {
        let config = state.config.lock().await;
        config
            .provider_profile(&config.active_provider)
            .map(|profile| config::generated_codex_home(&profile))
            .or_else(|| config.active_codex_home().map(std::path::PathBuf::from))
    };
    gateway_usage::load_usage_summary(days, start_date, end_date, hours, codex_home).await
}

#[tauri::command]
fn get_default_providers() -> Result<Vec<DefaultProviderProfile>, String> {
    config::read_default_provider_profiles()
}

#[tauri::command]
async fn save_default_provider_profile(
    provider: DefaultProviderProfile,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    let provider =
        config::save_default_provider_profile_with_format(provider, profile_config_format)?;
    let config = {
        let mut config = state.config.lock().await;
        config::sync_workspace_profiles_for_default_provider(
            &mut config,
            &provider,
            profile_config_format,
        )?;
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn delete_default_provider_profile(
    name: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile_name = name.trim().to_string();
    let config = {
        let mut config = state.config.lock().await;
        if let Some(profile) = config
            .provider_profiles
            .iter()
            .find(|profile| profile.codex_profile_name == profile_name)
        {
            return Err(format!(
                "Provider profile is used by workspace {}",
                profile.name
            ));
        }
        config::delete_default_provider_profile(&profile_name)?;
        config.normalize();
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn add_existing_provider(
    provider: ExistingProviderRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    let profile =
        config::add_existing_provider_profile_with_format(provider, profile_config_format)?;
    let config = {
        let mut config = state.config.lock().await;
        config.add_provider_profile(profile);
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn create_workspace(
    provider: WorkspaceRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile = config::create_workspace_profile(provider)?;
    let config = {
        let mut config = state.config.lock().await;
        config.add_provider_profile(profile);
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn create_provider(
    provider: NewProviderRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    let profile = config::create_default_provider_with_format(provider, profile_config_format)?;
    let config = {
        let mut config = state.config.lock().await;
        if profile.name == DEFAULT_PROVIDER_PROFILE_NAME {
            config.update_provider_profile(DEFAULT_PROVIDER_PROFILE_NAME, profile)?;
        } else {
            config.add_provider_profile(profile);
        }
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn create_next_ai_gateway_provider(
    provider: NextAiGatewayProviderRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    ensure_next_ai_gateway_enabled(state.inner()).await?;
    gateway_service::ensure_running(state.inner()).await?;
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    let profile =
        config::create_next_ai_gateway_provider_with_format(provider, profile_config_format)?;
    let config = {
        let mut config = state.config.lock().await;
        config.add_provider_profile(profile);
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn delete_provider(
    name: String,
    remove_codex_home: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    remote::stop_remote_control(state.inner(), &name).await?;
    server::stop_codex_instance(state.inner(), Some(name.clone())).await?;

    let removed_profile = {
        let mut config = state.config.lock().await;
        config.remove_provider_profile(&name)?
    };

    if remove_codex_home {
        let codex_home = removed_profile.codex_home.trim().to_string();
        if !codex_home.is_empty() {
            let path = std::path::PathBuf::from(&codex_home);
            if path.exists() {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
            }
        }
    }

    let config = {
        let config = state.config.lock().await;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn update_provider(
    provider: UpdateProviderRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    if provider.original_name == DEFAULT_PROVIDER_PROFILE_NAME {
        let bot = provider.bot.clone();
        let proxy_url = provider.proxy_url.trim().to_string();
        let remote_frontend_mode = provider.remote_frontend_mode.clone();
        let remote_web_asset_registry_url = provider.remote_web_asset_registry_url.clone();
        let remote_web_asset_version = provider.remote_web_asset_version.clone();
        config::update_default_provider_selection_with_format(
            ExistingProviderRequest {
                workspace_name: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
                profile_name: provider.profile_name,
                base_url: provider.base_url,
                api_key: provider.api_key,
                model: provider.model,
                proxy_url: proxy_url.clone(),
                remote_frontend_mode: String::new(),
                remote_web_asset_registry_url: String::new(),
                remote_web_asset_version: String::new(),
                bot: BotProfileConfig::default(),
            },
            profile_config_format,
        )?;
        let config = {
            let mut config = state.config.lock().await;
            if let Some(profile) = config
                .provider_profiles
                .iter_mut()
                .find(|profile| profile.name == DEFAULT_PROVIDER_PROFILE_NAME)
            {
                profile.bot = bot;
                profile.proxy_url = proxy_url;
                profile.remote_frontend_mode = remote_frontend_mode;
                profile.remote_web_asset_registry_url = remote_web_asset_registry_url;
                profile.remote_web_asset_version = remote_web_asset_version;
                let profile_id = profile.id.clone();
                profile
                    .bot
                    .normalize_for_profile_instance(DEFAULT_PROVIDER_PROFILE_NAME, &profile_id);
            }
            config.normalize();
            config.save()?;
            config.clone()
        };
        refresh_macos_tray_menu(&app, state.inner(), &config).await;
        return Ok(config);
    }

    let original_name = provider.original_name.clone();
    let profile =
        config::update_existing_provider_profile_with_format(provider, profile_config_format)?;
    let config = {
        let mut config = state.config.lock().await;
        config.update_provider_profile(&original_name, profile)?;
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn update_workspace(
    provider: UpdateWorkspaceRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let original_name = provider.original_name.clone();
    let profile = config::update_workspace_profile(provider)?;
    let config = {
        let mut config = state.config.lock().await;
        config.update_provider_profile(&original_name, profile)?;
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn update_next_ai_gateway_provider(
    provider: UpdateNextAiGatewayProviderRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    ensure_next_ai_gateway_enabled(state.inner()).await?;
    gateway_service::ensure_running(state.inner()).await?;
    let original_name = provider.original_name.clone();
    let profile_config_format = profile_config_format_for_state(state.inner()).await;
    let profile = config::update_next_ai_gateway_provider_profile_with_format(
        provider,
        profile_config_format,
    )?;
    let config = {
        let mut config = state.config.lock().await;
        config.update_provider_profile(&original_name, profile)?;
        config.save()?;
        config.clone()
    };
    refresh_macos_tray_menu(&app, state.inner(), &config).await;
    Ok(config)
}

#[tauri::command]
async fn start_weixin_bot_login(
    profile_name: String,
    force: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<bot_bridge::BotQrLoginStartInfo, String> {
    let bot_config = bot_config_for_profile(state.inner(), &profile_name).await?;
    let task_profile_name = profile_name.clone();
    let result = tokio::task::spawn_blocking(move || {
        bot_bridge::start_weixin_qr_login_session(
            &task_profile_name,
            &bot_config,
            force.unwrap_or(true),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let (result, session) = result;

    state
        .bot_login_sessions
        .lock()
        .await
        .insert(result.session_id.clone(), Arc::new(session));

    update_profile_bot_status(
        state.inner(),
        &result.profile_name,
        &result.tenant_id,
        &result.integration_id,
        "qr_pending",
        false,
    )
    .await?;
    Ok(result)
}

#[tauri::command]
async fn wait_weixin_bot_login(
    profile_name: String,
    session_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<bot_bridge::BotQrLoginWaitInfo, String> {
    let session = {
        let sessions = state.bot_login_sessions.lock().await;
        sessions.get(&session_id).cloned().ok_or_else(|| {
            "Weixin QR login session not found; regenerate the QR code".to_string()
        })?
    };
    let task_session_id = session_id.clone();
    let result = tokio::task::spawn_blocking(move || session.wait(&task_session_id))
        .await
        .map_err(|e| e.to_string())??;
    if result.profile_name != profile_name {
        return Err("Weixin QR login session belongs to a different workspace".to_string());
    }

    let status = if result.confirmed {
        "active"
    } else {
        result.status.as_str()
    };
    update_profile_bot_status(
        state.inner(),
        &result.profile_name,
        &result.tenant_id,
        &result.integration_id,
        status,
        result.confirmed,
    )
    .await?;
    if is_terminal_bot_login_status(&result.status) {
        state.bot_login_sessions.lock().await.remove(&session_id);
    }
    Ok(result)
}

#[tauri::command]
async fn cancel_weixin_bot_login(
    session_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.bot_login_sessions.lock().await.remove(&session_id);
    Ok(())
}

#[tauri::command]
async fn configure_bot_integration(
    profile_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let bot_config = bot_config_for_profile(state.inner(), &profile_name).await?;
    let task_profile_name = profile_name.clone();
    let result = tokio::task::spawn_blocking(move || {
        bot_bridge::configure_bot_integration(&task_profile_name, &bot_config)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut config = state.config.lock().await;
    let Some(profile) = config
        .provider_profiles
        .iter_mut()
        .find(|profile| profile.id == profile_name || profile.name == profile_name)
    else {
        return Err(format!("Provider profile not found: {}", profile_name));
    };

    profile.bot.enabled = true;
    profile.bot.platform = result.platform;
    profile.bot.auth_type = result.auth_type;
    profile.bot.tenant_id = result.tenant_id;
    profile.bot.integration_id = result.integration_id;
    profile.bot.status = result.status;
    let profile_name = profile.name.clone();
    let profile_id = profile.id.clone();
    profile
        .bot
        .normalize_for_profile_instance(&profile_name, &profile_id);
    config.upsert_saved_bot_config_from_profile(&profile_name)?;
    config.normalize();
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn scan_bot_handoff_wifi_targets() -> Result<Vec<bot_bridge::BotHandoffScanTarget>, String> {
    tokio::task::spawn_blocking(bot_bridge::scan_handoff_wifi_targets)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn scan_bot_handoff_bluetooth_targets(
) -> Result<Vec<bot_bridge::BotHandoffScanTarget>, String> {
    tokio::task::spawn_blocking(bot_bridge::scan_handoff_bluetooth_targets)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_builtin_extensions() -> Result<Vec<extensions::BuiltinExtensionStatus>, String> {
    Ok(vec![
        extensions::builtin_bot_gateway_status(),
        extensions::builtin_next_ai_gateway_status(),
    ])
}

#[tauri::command]
async fn prepare_builtin_extension(
    extension_id: String,
) -> Result<extensions::BuiltinExtensionStatus, String> {
    let task = match extension_id.as_str() {
        "bot-gateway" => extensions::prepare_builtin_bot_gateway,
        "next-ai-gateway" => extensions::prepare_builtin_next_ai_gateway,
        _ => return Err(format!("Unknown built-in extension: {}", extension_id)),
    };
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_extensions_runtime() -> Result<extensions::RuntimeStatus, String> {
    tokio::task::spawn_blocking(extensions::prepare_builtin_extensions_runtime)
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| {
            format!(
                "Extensions require Node.js 20+; automatic Node.js setup failed: {}",
                err
            )
        })
}

async fn ensure_next_ai_gateway_enabled(state: &AppState) -> Result<(), String> {
    let config = state.config.lock().await;
    if !config.extensions.enabled {
        return Err("Extensions are disabled. Enable extensions in Settings first.".to_string());
    }
    if !config.extensions.next_ai_gateway_enabled {
        return Err(
            "NeXT AI Gateway extension is disabled. Enable it in Settings first.".to_string(),
        );
    }
    Ok(())
}

async fn bot_config_for_profile(
    state: &AppState,
    profile_name: &str,
) -> Result<BotProfileConfig, String> {
    let config = state.config.lock().await;
    if !config.extensions.enabled {
        return Err("Extensions are disabled. Enable extensions in Settings first.".to_string());
    }
    if !config.extensions.bot_gateway_enabled {
        return Err("Bot extension is disabled. Enable it in Settings first.".to_string());
    }
    config
        .provider_profile(profile_name)
        .map(|profile| profile.bot)
        .ok_or_else(|| format!("Provider profile not found: {}", profile_name))
}

async fn update_profile_bot_status(
    state: &AppState,
    profile_name: &str,
    tenant_id: &str,
    integration_id: &str,
    status: &str,
    confirmed: bool,
) -> Result<(), String> {
    let mut config = state.config.lock().await;
    let Some(profile) = config
        .provider_profiles
        .iter_mut()
        .find(|profile| profile.id == profile_name || profile.name == profile_name)
    else {
        return Err(format!("Provider profile not found: {}", profile_name));
    };

    profile.bot.enabled = true;
    profile.bot.platform = config::BOT_PLATFORM_WEIXIN_ILINK.to_string();
    profile.bot.tenant_id = tenant_id.to_string();
    profile.bot.integration_id = integration_id.to_string();
    profile.bot.status = status.to_string();
    if confirmed {
        profile.bot.last_login_at = timestamp_seconds();
    }
    let profile_name = profile.name.clone();
    let profile_id = profile.id.clone();
    profile
        .bot
        .normalize_for_profile_instance(&profile_name, &profile_id);
    if confirmed {
        config.upsert_saved_bot_config_from_profile(&profile_name)?;
    }
    config.normalize();
    config.save()
}

fn timestamp_seconds() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{}", seconds)
}

fn is_terminal_bot_login_status(status: &str) -> bool {
    matches!(status, "confirmed" | "expired" | "already_bound" | "failed")
}

async fn refresh_macos_tray_menu(app: &tauri::AppHandle, state: &AppState, config: &AppConfig) {
    #[cfg(target_os = "macos")]
    if let Err(err) = macos_tray::refresh_menu(app, state, config).await {
        eprintln!("Failed to refresh macOS tray menu: {}", err);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (app, state, config);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new(AppConfig::load());
    let server_state = state.clone();
    let gateway_state = state.clone();
    let auto_launch_state = state.clone();
    let shutdown_state = state.clone();
    let tray_state = state.clone();
    let shutdown_started_for_run = Arc::new(AtomicBool::new(false));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            if let Err(err) = macos_tray::install(app, tray_state.clone()) {
                eprintln!("Failed to install macOS tray menu: {}", err);
            }

            tauri::async_runtime::spawn({
                let server_state = server_state.clone();
                async move {
                    if let Err(err) = server::serve(server_state).await {
                        eprintln!("{}", err);
                    }
                }
            });

            tauri::async_runtime::spawn({
                let gateway_state = gateway_state.clone();
                async move {
                    let config = gateway_state.config.lock().await.clone();
                    if let Err(err) =
                        gateway_service::sync_with_config(&gateway_state, &config).await
                    {
                        eprintln!("NeXT AI Gateway auto-start failed: {}", err);
                    }
                }
            });

            tauri::async_runtime::spawn({
                let auto_launch_state = auto_launch_state.clone();
                let auto_launch_app = app.handle().clone();
                async move {
                    let (should_launch, profile_name, start_remote) = {
                        let config = auto_launch_state.config.lock().await;
                        let profile_name = config.active_provider.clone();
                        let start_remote = config
                            .provider_profile(&profile_name)
                            .map(|profile| {
                                profile.start_remote_on_launch
                                    || config::remote_frontend_mode_uses_cli(
                                        &profile.remote_frontend_mode,
                                    )
                            })
                            .unwrap_or(false);
                        (config.auto_launch, profile_name, start_remote)
                    };
                    if should_launch {
                        let result = if start_remote {
                            let use_cloud_relay = {
                                let config = auto_launch_state.config.lock().await;
                                config
                                    .provider_profile(&profile_name)
                                    .map(|profile| profile.start_remote_cloud_on_launch)
                            };
                            let use_cloud_relay = use_cloud_relay.unwrap_or(false);
                            remote::start_remote_control(
                                &auto_launch_state,
                                profile_name,
                                None,
                                Some(use_cloud_relay),
                                Some(use_cloud_relay),
                            )
                            .await
                            .map(|_| ())
                        } else {
                            server::launch_codex_instance(
                                &auto_launch_state,
                                server::LaunchRequest {
                                    profile_name: Some(profile_name),
                                    ..server::LaunchRequest::default()
                                },
                            )
                            .await
                            .map(|_| ())
                        };

                        match result {
                            Ok(()) => {
                                let config = auto_launch_state.config.lock().await.clone();
                                refresh_macos_tray_menu(
                                    &auto_launch_app,
                                    &auto_launch_state,
                                    &config,
                                )
                                .await;
                            }
                            Err(err) => {
                                eprintln!("Auto launch failed: {}", err);
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            find_codex,
            list_codex_web_asset_versions,
            probe_provider_models,
            launch_codex,
            stop_codex,
            get_status,
            get_instance_statuses,
            get_config,
            update_config,
            update_remote_cloud_auth,
            start_remote_control,
            stop_remote_control,
            set_start_remote_on_launch,
            set_remote_launch_options,
            get_gateway_config,
            update_gateway_config,
            get_gateway_tools,
            get_gateway_usage_summary,
            get_default_providers,
            save_default_provider_profile,
            delete_default_provider_profile,
            add_existing_provider,
            create_workspace,
            create_provider,
            create_next_ai_gateway_provider,
            update_workspace,
            update_provider,
            update_next_ai_gateway_provider,
            delete_provider,
            start_weixin_bot_login,
            wait_weixin_bot_login,
            cancel_weixin_bot_login,
            configure_bot_integration,
            scan_bot_handoff_wifi_targets,
            scan_bot_handoff_bluetooth_targets,
            get_builtin_extensions,
            prepare_extensions_runtime,
            prepare_builtin_extension,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(move |_app_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) && !shutdown_started_for_run.swap(true, Ordering::SeqCst)
        {
            cleanup_on_app_shutdown(&shutdown_state);
        }
    });
}

fn cleanup_on_app_shutdown(state: &AppState) {
    tauri::async_runtime::block_on(async {
        if let Err(err) = server::stop_codex_instance(state, None).await {
            eprintln!("Failed to stop Codex instances during shutdown: {}", err);
        }
        state.bot_login_sessions.lock().await.clear();
        if let Err(err) = gateway_service::stop(state).await {
            eprintln!("Failed to stop NeXT AI Gateway during shutdown: {}", err);
        }
    });

    if let Err(err) = launcher::stop_all_extension_processes() {
        eprintln!(
            "Failed to stop extension processes during shutdown: {}",
            err
        );
    }
}
