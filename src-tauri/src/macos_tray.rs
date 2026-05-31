use crate::{
    config::{self, AppConfig, ProviderProfile},
    launcher, remote, server, AppState,
};
use std::collections::HashSet;
use std::process::Command;
use tauri::{
    menu::{Menu, MenuBuilder},
    tray::TrayIconBuilder,
    App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};

const TRAY_ID: &str = "codexl-workspaces";
const WORKSPACE_ITEM_PREFIX: &str = "workspace:";
const SHOW_MAIN_ITEM_ID: &str = "show-main";
const QUIT_ITEM_ID: &str = "quit";
const REMOTE_WINDOW_LABEL: &str = "workspace-remote-control";
const TRAY_TEMPLATE_PNG: &[u8] = include_bytes!("../icons/tray-template.png");

pub(crate) fn install(app: &mut App, state: AppState) -> Result<(), String> {
    let (config, active_workspaces) = tauri::async_runtime::block_on(async {
        let config = state.config.lock().await.clone();
        let active_workspaces = active_workspace_selectors(&state)
            .await
            .unwrap_or_else(|err| {
                eprintln!(
                    "Failed to read workspace launch status for tray menu: {}",
                    err
                );
                HashSet::new()
            });
        (config, active_workspaces)
    });
    let menu = build_menu(app.handle(), &config, &active_workspaces)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CodexL Workspaces")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event({
            let state = state.clone();
            move |app, event| {
                handle_menu_event(app.clone(), state.clone(), event.id().as_ref().to_string());
            }
        });

    match tauri::image::Image::from_bytes(TRAY_TEMPLATE_PNG) {
        Ok(icon) => {
            builder = builder.icon(icon).icon_as_template(true);
        }
        Err(err) => {
            eprintln!("Failed to load macOS tray template icon: {}", err);
            if let Some(icon) = app.default_window_icon().cloned() {
                builder = builder.icon(icon);
            }
        }
    }

    builder
        .build(app)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

pub(crate) async fn refresh_menu(
    app: &AppHandle,
    state: &AppState,
    config: &AppConfig,
) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let active_workspaces = active_workspace_selectors(state).await?;
    tray.set_menu(Some(build_menu(app, config, &active_workspaces)?))
        .map_err(|err| err.to_string())
}

fn build_menu(
    app: &AppHandle,
    config: &AppConfig,
    active_workspaces: &HashSet<String>,
) -> Result<Menu<tauri::Wry>, String> {
    let mut builder = MenuBuilder::new(app).text(SHOW_MAIN_ITEM_ID, "Show CodexL");
    builder = builder.separator();

    if config.provider_profiles.is_empty() {
        builder = builder.text("no-workspaces", "No workspaces");
    } else {
        for profile in &config.provider_profiles {
            let key = config::provider_profile_key(profile);
            let item_id = format!("{}{}", WORKSPACE_ITEM_PREFIX, key);
            builder = builder.text(
                item_id,
                workspace_menu_label(profile, workspace_is_active(profile, active_workspaces)),
            );
        }
    }

    builder = builder.separator().text(QUIT_ITEM_ID, "Quit");
    builder.build().map_err(|err| err.to_string())
}

async fn active_workspace_selectors(state: &AppState) -> Result<HashSet<String>, String> {
    let statuses = server::instance_statuses(state).await?;
    let mut active = HashSet::new();
    for status in statuses {
        let remote_running = status
            .remote_control
            .as_ref()
            .map(|remote| remote.running)
            .unwrap_or(false);
        if status.running || remote_running {
            active.insert(status.profile_name);
        }
    }
    Ok(active)
}

fn workspace_is_active(profile: &ProviderProfile, active_workspaces: &HashSet<String>) -> bool {
    active_workspaces.contains(&config::provider_profile_key(profile))
        || active_workspaces.contains(&profile.name)
        || active_workspaces.contains(&profile.id)
}

fn workspace_menu_label(profile: &ProviderProfile, active: bool) -> String {
    let mut label = profile.name.trim().to_string();
    if label.is_empty() {
        label = config::provider_profile_key(profile);
    }
    let mode = config::normalized_remote_frontend_mode(&profile.remote_frontend_mode);
    let mode_label = match mode.as_str() {
        config::REMOTE_FRONTEND_MODE_APP => "App",
        config::REMOTE_FRONTEND_MODE_CLI => "Remote",
        config::REMOTE_FRONTEND_MODE_CLAUDE_CODE => "Claude Code",
        _ => "Remote",
    };
    if active {
        menu_text(format!("{} [{}] - active", label, mode_label))
    } else {
        menu_text(format!("{} [{}]", label, mode_label))
    }
}

fn menu_text(value: String) -> String {
    value.replace('&', "&&")
}

fn handle_menu_event(app: AppHandle, state: AppState, menu_id: String) {
    if menu_id == SHOW_MAIN_ITEM_ID {
        if let Err(err) = show_main_window(&app) {
            eprintln!("Failed to show CodexL window from tray: {}", err);
        }
        return;
    }
    if menu_id == QUIT_ITEM_ID {
        app.exit(0);
        return;
    }
    let Some(profile_selector) = menu_id.strip_prefix(WORKSPACE_ITEM_PREFIX) else {
        return;
    };
    let profile_selector = profile_selector.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = open_workspace_from_tray(app, state, profile_selector).await {
            eprintln!("Failed to open workspace from tray: {}", err);
        }
    });
}

async fn open_workspace_from_tray(
    app: AppHandle,
    state: AppState,
    profile_selector: String,
) -> Result<(), String> {
    let profile = {
        let config = state.config.lock().await;
        config
            .provider_profile(&profile_selector)
            .ok_or_else(|| format!("Workspace not found: {}", profile_selector))?
    };
    let profile_key = config::provider_profile_key(&profile);
    let mode = config::normalized_remote_frontend_mode(&profile.remote_frontend_mode);

    if mode == config::REMOTE_FRONTEND_MODE_APP {
        if let Some(info) = running_app_info_for_profile(&state, &profile).await? {
            raise_codex_app_window(&info)?;
        } else {
            let info = server::launch_codex_instance(
                &state,
                server::LaunchRequest {
                    profile_name: Some(profile_key.clone()),
                    ..server::LaunchRequest::default()
                },
            )
            .await?;
            raise_codex_app_window(&info)?;
        }
    } else {
        let info = remote::start_remote_control(
            &state,
            profile_key.clone(),
            None,
            Some(false),
            Some(false),
        )
        .await?;
        open_remote_window(&app, &profile.name, &info.lan_url)?;
    }

    let config = {
        let mut config = state.config.lock().await;
        config.active_provider = profile_key;
        config.normalize();
        if let Err(err) = config.save() {
            eprintln!("Failed to save active workspace from tray: {}", err);
        }
        config.clone()
    };
    refresh_menu(&app, &state, &config).await?;
    Ok(())
}

async fn running_app_info_for_profile(
    state: &AppState,
    profile: &ProviderProfile,
) -> Result<Option<server::LaunchInfo>, String> {
    let profile_key = config::provider_profile_key(profile);
    let statuses = server::instance_statuses(state).await?;
    Ok(statuses
        .into_iter()
        .find(|status| {
            status.running
                && status.core_mode == config::REMOTE_FRONTEND_MODE_APP
                && (status.profile_name == profile_key
                    || status.profile_name == profile.name
                    || status.profile_name == profile.id)
        })
        .map(|status| server::LaunchInfo {
            running: status.running,
            pid: status.pid,
            cdp_host: status.cdp_host,
            cdp_port: status.cdp_port,
            http_host: status.http_host,
            http_port: status.http_port,
            codex_path: status.codex_path,
            codex_home: status.codex_home,
            proxy_url: status.proxy_url,
            profile_name: status.profile_name,
            cli_stdio_path: status.cli_stdio_path,
            core_mode: status.core_mode,
        }))
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("main window not found".to_string());
    };
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    window.set_focus().map_err(|err| err.to_string())
}

fn open_remote_window(app: &AppHandle, workspace_name: &str, url: &str) -> Result<(), String> {
    let parsed_url = url
        .parse()
        .map_err(|err| format!("invalid remote URL: {}", err))?;
    let title = if workspace_name.trim().is_empty() {
        "CodexL Remote Control".to_string()
    } else {
        format!("CodexL Remote - {}", workspace_name.trim())
    };
    if let Some(window) = app.get_webview_window(REMOTE_WINDOW_LABEL) {
        window
            .navigate(parsed_url)
            .map_err(|err| format!("failed to navigate remote control window: {}", err))?;
        window.set_title(&title).map_err(|err| err.to_string())?;
        window.show().map_err(|err| err.to_string())?;
        window.unminimize().map_err(|err| err.to_string())?;
        return window.set_focus().map_err(|err| err.to_string());
    }

    let window =
        WebviewWindowBuilder::new(app, REMOTE_WINDOW_LABEL, WebviewUrl::External(parsed_url))
            .title(title)
            .inner_size(1200.0, 800.0)
            .min_inner_size(600.0, 400.0)
            .build()
            .map_err(|err| err.to_string())?;
    window.set_focus().map_err(|err| err.to_string())
}

fn raise_codex_app_window(info: &server::LaunchInfo) -> Result<(), String> {
    let Some(pid) = info.pid else {
        return Err(format!("workspace {} is not running", info.profile_name));
    };
    match raise_process_window(pid) {
        Ok(()) => Ok(()),
        Err(err) => {
            if let Some(app_path) = codex_app_bundle_path(&info.codex_path) {
                let output = Command::new("/usr/bin/open")
                    .arg(app_path)
                    .output()
                    .map_err(|open_err| open_err.to_string())?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "{}; open fallback failed: {}",
                        err,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ))
                }
            } else {
                Err(err)
            }
        }
    }
}

fn raise_process_window(pid: u32) -> Result<(), String> {
    let script = format!(
        r#"tell application "System Events"
  set targetProcess to first application process whose unix id is {}
  set frontmost of targetProcess to true
  if (count of windows of targetProcess) > 0 then
    perform action "AXRaise" of window 1 of targetProcess
  end if
end tell"#,
        pid
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn codex_app_bundle_path(codex_path: &str) -> Option<String> {
    let path = std::path::Path::new(codex_path);
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.extension().and_then(|ext| ext.to_str()) == Some("app") {
            return Some(candidate.to_string_lossy().to_string());
        }
        current = candidate.parent();
    }
    launcher::find_codex_app().and_then(|value| {
        let path = std::path::Path::new(&value);
        let mut current = Some(path);
        while let Some(candidate) = current {
            if candidate.extension().and_then(|ext| ext.to_str()) == Some("app") {
                return Some(candidate.to_string_lossy().to_string());
            }
            current = candidate.parent();
        }
        None
    })
}
