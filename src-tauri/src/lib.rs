mod cli_middleware;
mod config;
mod extensions;
mod launcher;
mod platforms;
mod ports;
pub(crate) mod remote;
mod server;

use config::{
    AppConfig, BotProfileConfig, DefaultProviderProfile, ExistingProviderRequest,
    NewProviderRequest, NextAiGatewayProviderRequest, UpdateNextAiGatewayProviderRequest,
    UpdateProviderRequest, UpdateWorkspaceRequest, WorkspaceRequest, DEFAULT_PROVIDER_PROFILE_NAME,
};
use extensions::builtins::bot_bridge;
use extensions::builtins::gateway::{config as gateway_config, service as gateway_service};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;

pub fn run_cli_middleware_if_requested() -> bool {
    cli_middleware::run_if_requested()
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

#[tauri::command]
fn find_codex() -> Result<String, String> {
    launcher::find_codex_app().ok_or_else(|| "Codex app not found".to_string())
}

#[tauri::command]
async fn launch_codex(
    state: tauri::State<'_, AppState>,
    cdp_port: Option<u16>,
    codex_path: Option<String>,
    codex_home: Option<String>,
    profile_name: Option<String>,
) -> Result<server::LaunchInfo, String> {
    server::launch_codex_instance(
        state.inner(),
        server::LaunchRequest {
            cdp_port,
            codex_path,
            codex_home,
            profile_name,
        },
    )
    .await
}

#[tauri::command]
async fn stop_codex(
    state: tauri::State<'_, AppState>,
    profile_name: Option<String>,
) -> Result<(), String> {
    server::stop_codex_instance(state.inner(), profile_name).await
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().await;
    Ok(config.clone())
}

#[tauri::command]
async fn update_config(
    mut new_config: AppConfig,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    new_config.normalize();
    ensure_extensions_runtime_for_config(&new_config).await?;
    new_config.save()?;
    let gateway_config = new_config.clone();
    let mut config = state.config.lock().await;
    *config = new_config;
    drop(config);
    gateway_service::sync_with_config(state.inner(), &gateway_config)
        .await
        .map(|_| ())?;
    Ok(())
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
    state: tauri::State<'_, AppState>,
) -> Result<remote::RemoteControlInfo, String> {
    remote::start_remote_control(
        state.inner(),
        profile_name,
        remote_password,
        use_cloud_relay,
        require_e2ee,
    )
    .await
}

#[tauri::command]
async fn stop_remote_control(
    profile_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    remote::stop_remote_control(state.inner(), &profile_name).await
}

#[tauri::command]
async fn set_start_remote_on_launch(
    profile_name: String,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let mut config = state.config.lock().await;
    config.set_start_remote_on_launch(&profile_name, enabled)?;
    Ok(config.clone())
}

#[tauri::command]
async fn set_remote_launch_options(
    profile_name: String,
    start_remote: bool,
    start_cloud: bool,
    remote_e2ee_password: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let mut config = state.config.lock().await;
    config.set_remote_launch_options(
        &profile_name,
        start_remote,
        start_cloud,
        remote_e2ee_password,
    )?;
    Ok(config.clone())
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
fn get_default_providers() -> Result<Vec<DefaultProviderProfile>, String> {
    config::read_default_provider_profiles()
}

#[tauri::command]
async fn add_existing_provider(
    provider: ExistingProviderRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile = config::add_existing_provider_profile(provider)?;
    let mut config = state.config.lock().await;
    config.add_provider_profile(profile);
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn create_workspace(
    provider: WorkspaceRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile = config::create_workspace_profile(provider)?;
    let mut config = state.config.lock().await;
    config.add_provider_profile(profile);
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn create_provider(
    provider: NewProviderRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let profile = config::create_default_provider(provider)?;
    let mut config = state.config.lock().await;
    config.add_provider_profile(profile);
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn create_next_ai_gateway_provider(
    provider: NextAiGatewayProviderRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    ensure_next_ai_gateway_enabled(state.inner()).await?;
    gateway_service::ensure_running(state.inner()).await?;
    let profile = config::create_next_ai_gateway_provider(provider)?;
    let mut config = state.config.lock().await;
    config.add_provider_profile(profile);
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn delete_provider(
    name: String,
    remove_codex_home: bool,
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

    let config = state.config.lock().await;
    Ok(config.clone())
}

#[tauri::command]
async fn update_provider(
    provider: UpdateProviderRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    if provider.original_name == DEFAULT_PROVIDER_PROFILE_NAME {
        let bot = provider.bot.clone();
        let proxy_url = provider.proxy_url.trim().to_string();
        config::update_default_provider_selection(ExistingProviderRequest {
            workspace_name: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
            profile_name: provider.profile_name,
            base_url: provider.base_url,
            api_key: provider.api_key,
            model: provider.model,
            proxy_url: proxy_url.clone(),
            bot: BotProfileConfig::default(),
        })?;
        let mut config = state.config.lock().await;
        if let Some(profile) = config
            .provider_profiles
            .iter_mut()
            .find(|profile| profile.name == DEFAULT_PROVIDER_PROFILE_NAME)
        {
            profile.bot = bot;
            profile.proxy_url = proxy_url;
            let profile_id = profile.id.clone();
            profile
                .bot
                .normalize_for_profile_instance(DEFAULT_PROVIDER_PROFILE_NAME, &profile_id);
        }
        config.normalize();
        config.save()?;
        return Ok(config.clone());
    }

    let original_name = provider.original_name.clone();
    let profile = config::update_existing_provider_profile(provider)?;
    let mut config = state.config.lock().await;
    config.update_provider_profile(&original_name, profile)?;
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn update_workspace(
    provider: UpdateWorkspaceRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let original_name = provider.original_name.clone();
    let profile = config::update_workspace_profile(provider)?;
    let mut config = state.config.lock().await;
    config.update_provider_profile(&original_name, profile)?;
    config.save()?;
    Ok(config.clone())
}

#[tauri::command]
async fn update_next_ai_gateway_provider(
    provider: UpdateNextAiGatewayProviderRequest,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    ensure_next_ai_gateway_enabled(state.inner()).await?;
    gateway_service::ensure_running(state.inner()).await?;
    let original_name = provider.original_name.clone();
    let profile = config::update_next_ai_gateway_provider_profile(provider)?;
    let mut config = state.config.lock().await;
    config.update_provider_profile(&original_name, profile)?;
    config.save()?;
    Ok(config.clone())
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
        .find(|profile| profile.name == profile_name)
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
        .find(|profile| profile.name == profile_name)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new(AppConfig::load());
    let server_state = state.clone();
    let gateway_state = state.clone();
    let auto_launch_state = state.clone();
    let shutdown_state = state.clone();
    let shutdown_started_for_run = Arc::new(AtomicBool::new(false));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(move |_app| {
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
                async move {
                    let (should_launch, profile_name, start_remote) = {
                        let config = auto_launch_state.config.lock().await;
                        let profile_name = config.active_provider.clone();
                        let start_remote = config
                            .provider_profile(&profile_name)
                            .map(|profile| profile.start_remote_on_launch)
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

                        if let Err(err) = result {
                            eprintln!("Auto launch failed: {}", err);
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            find_codex,
            launch_codex,
            stop_codex,
            get_status,
            get_instance_statuses,
            get_config,
            update_config,
            start_remote_control,
            stop_remote_control,
            set_start_remote_on_launch,
            set_remote_launch_options,
            get_gateway_config,
            update_gateway_config,
            get_default_providers,
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
