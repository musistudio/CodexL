use crate::cli_middleware;
use crate::config::BotProfileConfig;
#[cfg(any(unix, windows))]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DEFAULT_MAC_APP_NAMES: &[&str] = &[
    "ChatGPT.app",
    "OpenAI ChatGPT.app",
    "Codex.app",
    "OpenAI Codex.app",
];
const DEFAULT_WINDOWS_APP_DIRS: &[&str] = &[
    "ChatGPT",
    "OpenAI ChatGPT",
    "OpenAIChatGPT",
    "Codex",
    "OpenAI Codex",
    "OpenAICodex",
];
const DEFAULT_WINDOWS_EXE_NAMES: &[&str] = &[
    "ChatGPT.exe",
    "chatgpt.exe",
    "OpenAI ChatGPT.exe",
    "OpenAIChatGPT.exe",
    "Codex.exe",
    "codex.exe",
    "OpenAI Codex.exe",
    "OpenAICodex.exe",
];
#[cfg(windows)]
const DEFAULT_WINDOWS_PACKAGE_PREFIXES: &[&str] = &[
    "OpenAI.ChatGPT_",
    "OpenAI.ChatGPTApp_",
    "OpenAI.OpenAIChatGPT_",
    "OpenAIChatGPT_",
    "ChatGPT_",
    "OpenAI.Codex_",
    "OpenAI.CodexApp_",
    "OpenAICodex_",
    "OpenAI.OpenAICodex_",
];

#[derive(Debug)]
pub struct CodexLaunch {
    pub child: Child,
    pub cli_stdio_path: Option<String>,
}

pub fn find_codex_app() -> Option<String> {
    if cfg!(target_os = "macos") {
        find_mac_app()
    } else if cfg!(windows) {
        find_windows_app()
    } else {
        None
    }
}

pub fn resolve_codex_cli_executable(
    explicit_path: Option<&str>,
    configured_codex_path: &str,
) -> String {
    if let Some(path) = explicit_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return path.to_string();
    }

    for key in ["CODEXL_REAL_CODEX_CLI_PATH", "CODEX_CLI_PATH"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }

    if !configured_codex_path.trim().is_empty() {
        if let Some(path) = bundled_cli_path(configured_codex_path) {
            return path.to_string_lossy().to_string();
        }
        if configured_codex_path.contains(".app/Contents/MacOS/") {
            return "codex".to_string();
        }
        if Path::new(configured_codex_path).is_file() {
            return configured_codex_path.to_string();
        }
    }

    "codex".to_string()
}

pub fn launch_codex(
    executable: &str,
    cdp_port: u16,
    codex_home: Option<&str>,
    stdio_name: Option<&str>,
    codex_profile: Option<&str>,
    codex_model_provider: Option<&str>,
    core_mode: Option<&str>,
    proxy_url: Option<&str>,
    bot_config: Option<&BotProfileConfig>,
    language: Option<&str>,
) -> std::io::Result<CodexLaunch> {
    let mut command = Command::new(executable);
    command
        .args([
            &format!("--remote-debugging-port={}", cdp_port),
            "--remote-allow-origins=*",
            "--disable-renderer-backgrounding",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
        ])
        .env("ELECTRON_ENABLE_LOGGING", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    configure_electron_user_data_dir(&mut command, codex_home, stdio_name)?;

    let mut cli_stdio_path = None;
    if !cli_middleware::is_disabled() {
        let middleware = cli_middleware::prepare(
            executable,
            codex_home,
            stdio_name,
            codex_profile,
            codex_model_provider,
            core_mode,
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        cli_stdio_path = Some(middleware.stdio_path.to_string_lossy().to_string());
        command
            .env("CODEX_CLI_PATH", middleware.executable_path)
            .env("CODEXL_REAL_CODEX_CLI_PATH", middleware.real_cli_path)
            .env("CODEXL_CLI_MIDDLEWARE_LOG", middleware.log_path);
        if let Some(workspace_name) = middleware.workspace_name {
            command.env(cli_middleware::CODEX_WORKSPACE_NAME_ENV, workspace_name);
        }
        if let Some(profile) = middleware.profile {
            command.env(cli_middleware::CODEX_PROFILE_ENV, profile);
        }
        if let Some(model_provider) = middleware.model_provider {
            command.env(cli_middleware::CODEX_MODEL_PROVIDER_ENV, model_provider);
        }
        if let Some(core_mode) = middleware.core_mode {
            command.env(cli_middleware::CODEX_CORE_MODE_ENV, core_mode);
        }
    }

    configure_bot_gateway_bridge_env(&mut command, stdio_name, bot_config, language);
    configure_global_computer_use_env(&mut command);
    configure_proxy_env(&mut command, proxy_url);

    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }

    #[cfg(unix)]
    {
        command.process_group(0);
    }

    command.spawn().map(|child| CodexLaunch {
        child,
        cli_stdio_path,
    })
}

#[cfg(target_os = "macos")]
fn configure_global_computer_use_env(command: &mut Command) {
    if let Some(path) = global_computer_use_app_path() {
        command.env("SKY_CUA_SERVICE_PATH", path);
    }
}

#[cfg(not(target_os = "macos"))]
fn configure_global_computer_use_env(_command: &mut Command) {}

#[cfg(target_os = "macos")]
fn global_computer_use_app_path() -> Option<PathBuf> {
    let path = global_codex_home_candidate()?
        .join("computer-use")
        .join("Codex Computer Use.app");
    path.is_dir().then_some(path)
}

#[cfg(target_os = "macos")]
fn global_codex_home_candidate() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("CODEXL_CODEX_HOME") {
        let value = value.trim();
        if !value.is_empty() {
            return Some(PathBuf::from(crate::config::normalize_home_path(value)));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".codex"))
}

pub fn stop_codex(child: &mut Child) -> std::io::Result<()> {
    let pid = child.id();
    #[cfg(unix)]
    {
        let process_group = format!("-{}", pid);
        let _ = send_signal("-TERM", &process_group);

        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let _ = send_signal("-KILL", &process_group);
    }

    #[cfg(windows)]
    {
        let _ = terminate_process_tree(pid);
    }

    child.kill().ok();
    child.wait().ok();
    Ok(())
}

pub fn stop_stale_profile_processes(profile_name: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        let entries = process_entries()?;
        let mut pids = BTreeSet::new();
        let mut pgids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codex_process_for_profile(&entry.command, profile_name))
        {
            pids.insert(entry.pid);
            pgids.insert(entry.pgid);

            if is_codex_app_server_for_profile(&entry.command, profile_name) {
                if let Some(parent) = entries.iter().find(|parent| {
                    parent.pid == entry.ppid && is_codexl_middleware_command(&parent.command)
                }) {
                    pids.insert(parent.pid);
                    collect_descendant_pids(&entries, parent.pid, &mut pids);
                    continue;
                }
            }

            collect_descendant_pids(&entries, entry.pid, &mut pids);
        }

        for entry in entries.iter().filter(|entry| pgids.contains(&entry.pgid)) {
            if is_codexl_middleware_command(&entry.command)
                || is_codexl_extension_process(&entry.command)
            {
                pids.insert(entry.pid);
            }
        }

        for entry in entries
            .iter()
            .filter(|entry| is_orphaned_codexl_extension_process(entry))
        {
            pids.insert(entry.pid);
        }

        terminate_pids(pids);
    }
    #[cfg(windows)]
    {
        let entries = windows_process_entries()?;
        let mut pids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codex_process_for_profile(&entry.command, profile_name))
        {
            pids.insert(entry.pid);
            if is_codex_app_server_for_profile(&entry.command, profile_name) {
                if let Some(parent) = entries.iter().find(|parent| {
                    parent.pid == entry.ppid && is_codexl_middleware_command(&parent.command)
                }) {
                    pids.insert(parent.pid);
                    collect_windows_descendant_pids(&entries, parent.pid, &mut pids);
                    continue;
                }
            }

            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        for entry in entries
            .iter()
            .filter(|entry| is_orphaned_codexl_extension_process_windows(entry))
        {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids_windows(pids);
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = profile_name;
    }

    Ok(())
}

pub fn stop_profile_extension_processes(profile_name: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        let entries = process_entries()?;
        let mut pids = BTreeSet::new();
        let mut pgids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codex_app_server_for_profile(&entry.command, profile_name))
        {
            pgids.insert(entry.pgid);
        }

        for entry in entries.iter().filter(|entry| {
            pgids.contains(&entry.pgid) && is_codexl_extension_process(&entry.command)
        }) {
            pids.insert(entry.pid);
            collect_descendant_pids(&entries, entry.pid, &mut pids);
        }

        for entry in entries
            .iter()
            .filter(|entry| is_orphaned_codexl_extension_process(entry))
        {
            pids.insert(entry.pid);
        }

        terminate_pids(pids);
    }
    #[cfg(windows)]
    {
        let entries = windows_process_entries()?;
        let mut pids = BTreeSet::new();
        let mut candidate_descendants = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codex_app_server_for_profile(&entry.command, profile_name))
        {
            collect_windows_descendant_pids(&entries, entry.pid, &mut candidate_descendants);
        }

        for entry in entries.iter().filter(|entry| {
            candidate_descendants.contains(&entry.pid)
                && is_codexl_extension_process(&entry.command)
        }) {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        for entry in entries
            .iter()
            .filter(|entry| is_orphaned_codexl_extension_process_windows(entry))
        {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids_windows(pids);
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = profile_name;
    }

    Ok(())
}

pub fn stop_all_extension_processes() -> Result<(), String> {
    #[cfg(unix)]
    {
        let entries = process_entries()?;
        let mut pids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codexl_extension_process(&entry.command))
        {
            pids.insert(entry.pid);
            collect_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids(pids);
    }
    #[cfg(windows)]
    {
        let entries = windows_process_entries()?;
        let mut pids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_codexl_extension_process(&entry.command))
        {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids_windows(pids);
    }

    Ok(())
}

pub fn stop_next_ai_gateway_processes() -> Result<(), String> {
    #[cfg(unix)]
    {
        let entries = process_entries()?;
        let mut pids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_next_ai_gateway_command(&entry.command))
        {
            pids.insert(entry.pid);
            collect_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids(pids);
    }
    #[cfg(windows)]
    {
        let entries = windows_process_entries()?;
        let mut pids = BTreeSet::new();

        for entry in entries
            .iter()
            .filter(|entry| is_next_ai_gateway_command(&entry.command))
        {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        terminate_pids_windows(pids);
    }

    Ok(())
}

pub fn stop_next_ai_gateway_processes_for_port(port: u16) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let entries = process_entries()?;
        let listener_pids = tcp_listener_pids_for_port(port)?;
        let mut pids = BTreeSet::new();

        for entry in entries.iter().filter(|entry| {
            listener_pids.contains(&entry.pid) && is_next_ai_gateway_command(&entry.command)
        }) {
            pids.insert(entry.pid);
            collect_descendant_pids(&entries, entry.pid, &mut pids);
        }

        let stopped = !pids.is_empty();
        terminate_pids(pids);
        return Ok(stopped);
    }
    #[cfg(windows)]
    {
        let entries = windows_process_entries()?;
        let listener_pids = windows_tcp_listener_pids_for_port(port)?;
        let mut pids = BTreeSet::new();

        for entry in entries.iter().filter(|entry| {
            listener_pids.contains(&entry.pid) && is_next_ai_gateway_command(&entry.command)
        }) {
            pids.insert(entry.pid);
            collect_windows_descendant_pids(&entries, entry.pid, &mut pids);
        }

        let stopped = !pids.is_empty();
        terminate_pids_windows(pids);
        return Ok(stopped);
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = port;
        Ok(false)
    }
}

#[cfg(unix)]
fn send_signal(signal: &str, target: &str) -> std::io::Result<std::process::ExitStatus> {
    Command::new("kill")
        .args([signal, target])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(unix)]
fn terminate_pids(mut pids: BTreeSet<u32>) {
    pids.remove(&std::process::id());
    if pids.is_empty() {
        return;
    }

    for pid in &pids {
        let _ = send_signal("-TERM", &pid.to_string());
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    for pid in &pids {
        let _ = send_signal("-KILL", &pid.to_string());
    }
}

#[cfg(unix)]
fn tcp_listener_pids_for_port(port: u16) -> Result<BTreeSet<u32>, String> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{}", port), "-sTCP:LISTEN", "-Fp"])
        .output()
        .map_err(|err| format!("failed to inspect TCP listeners: {}", err))?;
    if !output.status.success() {
        return Ok(BTreeSet::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_lsof_pid_lines(&stdout))
}

#[cfg(unix)]
fn parse_lsof_pid_lines(stdout: &str) -> BTreeSet<u32> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix('p'))
        .filter_map(|pid| pid.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) -> std::io::Result<std::process::ExitStatus> {
    Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(windows)]
fn terminate_pids_windows(mut pids: BTreeSet<u32>) {
    pids.remove(&std::process::id());
    if pids.is_empty() {
        return;
    }

    for pid in &pids {
        let _ = terminate_process_tree(*pid);
    }
}

#[cfg(windows)]
fn windows_tcp_listener_pids_for_port(port: u16) -> Result<BTreeSet<u32>, String> {
    let script = format!(
        "Get-NetTCPConnection -LocalPort {} -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess | ConvertTo-Json -Compress",
        port
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|err| format!("failed to inspect TCP listeners: {}", err))?;
    if !output.status.success() {
        return Ok(BTreeSet::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_windows_listener_pids_json(stdout.trim())
}

#[cfg(windows)]
fn parse_windows_listener_pids_json(stdout: &str) -> Result<BTreeSet<u32>, String> {
    if stdout.is_empty() {
        return Ok(BTreeSet::new());
    }
    let value: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|err| format!("failed to parse TCP listener pids: {}", err))?;
    let mut pids = BTreeSet::new();
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                push_json_pid(&item, &mut pids);
            }
        }
        item => push_json_pid(&item, &mut pids),
    }
    Ok(pids)
}

#[cfg(windows)]
fn push_json_pid(value: &serde_json::Value, pids: &mut BTreeSet<u32>) {
    if let Some(pid) = value.as_u64().and_then(|value| u32::try_from(value).ok()) {
        pids.insert(pid);
    } else if let Some(pid) = value.as_str().and_then(|value| value.parse::<u32>().ok()) {
        pids.insert(pid);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn configure_electron_user_data_dir(
    command: &mut Command,
    codex_home: Option<&str>,
    stdio_name: Option<&str>,
) -> std::io::Result<()> {
    let user_data_dir = electron_user_data_dir(codex_home, stdio_name);
    std::fs::create_dir_all(&user_data_dir)?;
    command.arg(format!(
        "--user-data-dir={}",
        user_data_dir.to_string_lossy()
    ));
    command.env("CODEX_ELECTRON_USER_DATA_PATH", &user_data_dir);
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
fn electron_user_data_dir(codex_home: Option<&str>, stdio_name: Option<&str>) -> PathBuf {
    let base = codex_home
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(value.to_string()))
        .unwrap_or_else(|| {
            user_home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".codex")
                .join("codexl")
        });

    base.join(".codexl")
        .join("codex-app-user-data")
        .join(safe_path_segment(stdio_name.unwrap_or("default")))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn configure_electron_user_data_dir(
    _command: &mut Command,
    _codex_home: Option<&str>,
    _stdio_name: Option<&str>,
) -> std::io::Result<()> {
    Ok(())
}

fn configure_bot_gateway_bridge_env(
    command: &mut Command,
    stdio_name: Option<&str>,
    bot_config: Option<&BotProfileConfig>,
    language: Option<&str>,
) {
    let profile_name = stdio_name.unwrap_or_default();
    let mut bot_config = bot_config.cloned().unwrap_or_default();
    bot_config.normalize_for_profile(profile_name);

    if !bot_config.bridge_enabled() {
        command.env("CODEXL_BOT_GATEWAY_ENABLED", "false");
        return;
    }

    let state_dir = if bot_config.state_dir.trim().is_empty() {
        crate::config::generated_bot_gateway_state_dir(profile_name)
    } else {
        std::path::PathBuf::from(crate::config::normalize_home_path(&bot_config.state_dir))
    };

    command
        .env("CODEXL_BOT_GATEWAY_ENABLED", "true")
        .env("CODEXL_BOT_GATEWAY_PLATFORM", &bot_config.platform)
        .env("CODEXL_BOT_GATEWAY_TENANT_ID", &bot_config.tenant_id)
        .env(
            "CODEXL_BOT_GATEWAY_FORWARD_ALL_CODEX_MESSAGES",
            if bot_config.forward_all_codex_messages {
                "true"
            } else {
                "false"
            },
        )
        .env(
            "CODEXL_BOT_HANDOFF_ENABLED",
            if bot_config.handoff.enabled {
                "true"
            } else {
                "false"
            },
        )
        .env(
            "CODEXL_BOT_HANDOFF_IDLE_SECONDS",
            bot_config.handoff.idle_seconds.to_string(),
        )
        .env(
            "CODEXL_BOT_HANDOFF_SCREEN_LOCK",
            if bot_config.handoff.screen_lock {
                "true"
            } else {
                "false"
            },
        )
        .env(
            "CODEXL_BOT_HANDOFF_USER_IDLE",
            if bot_config.handoff.user_idle {
                "true"
            } else {
                "false"
            },
        )
        .env(
            "CODEXL_BOT_HANDOFF_PHONE_WIFI_TARGETS",
            bot_config.handoff.phone_wifi_targets.join("\n"),
        )
        .env(
            "CODEXL_BOT_HANDOFF_PHONE_BLUETOOTH_TARGETS",
            bot_config.handoff.phone_bluetooth_targets.join("\n"),
        )
        .env("CODEXL_BOT_GATEWAY_STATE_DIR", state_dir)
        .env(
            "CODEXL_LANGUAGE",
            match language
                .unwrap_or("en")
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "zh" | "zh-cn" | "chinese" => "zh",
                _ => "en",
            },
        )
        .env(
            "CODEXL_BOT_GATEWAY_INTEGRATION_ID",
            &bot_config.integration_id,
        );
}

fn configure_proxy_env(command: &mut Command, proxy_url: Option<&str>) {
    let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    for key in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        command.env(key, proxy_url);
    }
}

fn safe_path_segment(value: &str) -> String {
    let mut segment = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            segment.push(ch);
        } else {
            segment.push('-');
        }
    }
    let segment = segment.trim_matches('-');
    if segment.is_empty() {
        "default".to_string()
    } else {
        segment.to_string()
    }
}

fn find_mac_app() -> Option<String> {
    for key in [
        "CODEXL_CHATGPT_PATH",
        "CHATGPT_APP_PATH",
        "CODEXL_CODEX_PATH",
        "CODEX_APP_PATH",
    ] {
        if let Some(path) = env_path(key) {
            if let Some(exe) = normalize_mac_app_candidate(&path) {
                return Some(exe);
            }
        }
    }

    let home = user_home_dir();
    let candidates: Vec<PathBuf> = DEFAULT_MAC_APP_NAMES
        .iter()
        .flat_map(|name| {
            let mut paths = vec![PathBuf::from("/Applications").join(name)];
            if let Some(ref h) = home {
                paths.push(h.join("Applications").join(name));
            }
            paths
        })
        .collect();

    for app_path in &candidates {
        if let Some(exe) = normalize_mac_app_candidate(app_path) {
            return Some(exe);
        }
    }
    None
}

fn normalize_mac_app_candidate(path: &Path) -> Option<String> {
    if path.is_file() {
        return Some(path.to_string_lossy().to_string());
    }
    if path.is_dir() {
        return executable_from_app_bundle(path);
    }
    None
}

fn find_windows_app() -> Option<String> {
    for key in [
        "CODEXL_CHATGPT_PATH",
        "CHATGPT_APP_PATH",
        "CODEXL_CODEX_PATH",
        "CODEX_APP_PATH",
    ] {
        if let Some(path) = env_path(key) {
            if let Some(path) = normalize_windows_codex_app_candidate(path) {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }

    for candidate in windows_appx_package_candidates_from_powershell() {
        if let Some(path) = normalize_windows_codex_app_candidate(candidate) {
            return Some(path.to_string_lossy().to_string());
        }
    }

    for candidate in windows_appx_package_candidates() {
        if let Some(path) = normalize_windows_codex_app_candidate(candidate) {
            return Some(path.to_string_lossy().to_string());
        }
    }

    for candidate in windows_registry_app_candidates() {
        if let Some(path) = normalize_windows_codex_app_candidate(candidate) {
            return Some(path.to_string_lossy().to_string());
        }
    }

    for candidate in windows_app_candidates() {
        if let Some(path) = normalize_windows_codex_app_candidate(candidate) {
            return Some(path.to_string_lossy().to_string());
        }
    }

    for candidate in windows_where_codex_candidates() {
        if let Some(path) = normalize_windows_codex_app_candidate(candidate) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

fn normalize_windows_codex_app_candidate(path: PathBuf) -> Option<PathBuf> {
    if path.is_dir() {
        return windows_codex_executable_in_app_dir(&path);
    }

    if let Some(parent) = path.parent() {
        let parent_name = parent.file_name()?.to_string_lossy().to_ascii_lowercase();
        if parent_name == "resources" {
            if let Some(app_dir) = parent.parent() {
                for exe_name in DEFAULT_WINDOWS_EXE_NAMES {
                    let app_exe = app_dir.join(exe_name);
                    if app_exe.is_file() {
                        return Some(app_exe);
                    }
                }
            }
        }
    }

    let file_name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if !windows_codex_exe_name_matches(&file_name) {
        return None;
    }

    path.is_file().then_some(path)
}

fn windows_codex_executable_in_app_dir(dir: &Path) -> Option<PathBuf> {
    if let Some(path) = windows_codex_executable_direct_child(dir) {
        return Some(path);
    }

    let nested_roots = ["app", "current", "Current"];
    for nested in nested_roots {
        if let Some(path) = windows_codex_executable_direct_child(&dir.join(nested)) {
            return Some(path);
        }
    }

    let mut versioned_dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("app-") {
                versioned_dirs.push(path);
            }
        }
    }
    versioned_dirs.sort();
    versioned_dirs.reverse();

    for versioned_dir in versioned_dirs {
        if let Some(path) = windows_codex_executable_direct_child(&versioned_dir) {
            return Some(path);
        }
    }

    None
}

fn windows_codex_executable_direct_child(dir: &Path) -> Option<PathBuf> {
    for exe_name in DEFAULT_WINDOWS_EXE_NAMES {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn windows_codex_exe_name_matches(file_name: &str) -> bool {
    DEFAULT_WINDOWS_EXE_NAMES
        .iter()
        .any(|name| file_name.eq_ignore_ascii_case(name))
}

#[cfg(windows)]
fn windows_where_codex_candidates() -> Vec<PathBuf> {
    [
        "ChatGPT",
        "chatgpt",
        "OpenAI ChatGPT",
        "Codex",
        "codex",
        "OpenAI Codex",
    ]
    .iter()
    .flat_map(|name| where_command_candidates(name))
    .collect()
}

#[cfg(not(windows))]
fn windows_where_codex_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn where_command_candidates(name: &str) -> Vec<PathBuf> {
    let Ok(output) = Command::new("where.exe")
        .arg(name)
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn windows_app_candidates() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in [
        "LOCALAPPDATA",
        "APPDATA",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
    ] {
        if let Some(path) = env_path(key) {
            roots.push(path);
        }
    }
    if let Some(home) = user_home_dir() {
        roots.push(home.join("AppData").join("Local"));
        roots.push(home.join("AppData").join("Roaming"));
    }

    windows_app_candidates_from_roots(roots)
}

fn windows_app_candidates_from_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in roots {
        let install_roots = [
            root.clone(),
            root.join("Programs"),
            root.join("Programs").join("OpenAI"),
            root.join("OpenAI"),
            root.join("Microsoft").join("WindowsApps"),
        ];
        for install_root in install_roots {
            for exe_name in DEFAULT_WINDOWS_EXE_NAMES {
                push_unique_path(&mut candidates, install_root.join(exe_name));
            }
            for dir_name in DEFAULT_WINDOWS_APP_DIRS {
                push_unique_path(&mut candidates, install_root.join(dir_name));
                for exe_name in DEFAULT_WINDOWS_EXE_NAMES {
                    push_unique_path(&mut candidates, install_root.join(dir_name).join(exe_name));
                }
            }
        }
    }
    candidates
}

#[cfg(windows)]
fn windows_appx_package_candidates_from_powershell() -> Vec<PathBuf> {
    let script = r#"
$packages = @(
  Get-AppxPackage -Name '*ChatGPT*' -ErrorAction SilentlyContinue
  Get-AppxPackage -Name '*Codex*' -ErrorAction SilentlyContinue
)
foreach ($package in $packages) {
  if ($package.Name -match 'ChatGPT|Codex|OpenAI' -or $package.PackageFullName -match 'ChatGPT|Codex|OpenAI') {
    $package.InstallLocation
  }
}
"#;
    powershell_lines(script)
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(not(windows))]
fn windows_appx_package_candidates_from_powershell() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn windows_appx_package_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(program_files) = env_path("ProgramFiles") else {
        return candidates;
    };
    let windows_apps = program_files.join("WindowsApps");
    let Ok(entries) = std::fs::read_dir(windows_apps) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !DEFAULT_WINDOWS_PACKAGE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        candidates.push(path.clone());
        candidates.push(path.join("app"));
        for exe_name in DEFAULT_WINDOWS_EXE_NAMES {
            candidates.push(path.join("app").join(exe_name));
            candidates.push(path.join(exe_name));
        }
    }

    candidates.sort();
    candidates.reverse();
    candidates
}

#[cfg(not(windows))]
fn windows_appx_package_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn windows_registry_app_candidates() -> Vec<PathBuf> {
    let script = r#"
$roots = @(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
)
foreach ($item in Get-ItemProperty $roots -ErrorAction SilentlyContinue) {
  if ($item.DisplayName -match '(^|\s)(OpenAI\s+)?(ChatGPT|Codex)(\s|$)') {
    $item.DisplayIcon
    $item.InstallLocation
    $item.UninstallString
  }
}
"#;
    powershell_lines(script)
        .into_iter()
        .filter_map(|line| windows_path_from_registry_value(&line))
        .collect()
}

#[cfg(not(windows))]
fn windows_registry_app_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn powershell_lines(script: &str) -> Vec<String> {
    let Ok(output) = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(windows)]
fn windows_path_from_registry_value(value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }

    let lower = value.to_ascii_lowercase();
    if let Some(index) = lower.find(".exe") {
        let end = index + ".exe".len();
        let executable = value[..end].trim().trim_matches('"').trim();
        if executable.is_empty() {
            return None;
        }
        return Some(PathBuf::from(executable));
    }

    Some(PathBuf::from(value))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(expand_home_path)
}

fn expand_home_path(value: String) -> PathBuf {
    let trimmed = value.trim();
    if trimmed == "~" {
        return user_home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = user_home_dir() {
            return home.join(rest);
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~\\") {
        if let Some(home) = user_home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let key = path.to_string_lossy().to_ascii_lowercase();
    if paths
        .iter()
        .any(|existing| existing.to_string_lossy().to_ascii_lowercase() == key)
    {
        return;
    }
    paths.push(path);
}

fn user_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path_without_home_expansion("USERPROFILE")
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                let combined = format!("{}{}", drive.trim(), path.trim());
                if combined.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(combined))
                }
            })
            .or_else(|| env_path_without_home_expansion("HOME"))
    } else {
        env_path_without_home_expansion("HOME")
    }
}

fn env_path_without_home_expansion(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(unix)]
#[derive(Debug, Clone)]
struct ProcessEntry {
    pid: u32,
    ppid: u32,
    pgid: u32,
    command: String,
}

#[cfg(unix)]
fn process_entries() -> Result<Vec<ProcessEntry>, String> {
    let output = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,pgid=,command="])
        .output()
        .map_err(|err| format!("failed to inspect running processes: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect running processes".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter_map(parse_process_entry).collect())
}

#[cfg(unix)]
fn parse_process_entry(line: &str) -> Option<ProcessEntry> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let ppid = parts.next()?.parse().ok()?;
    let pgid = parts.next()?.parse().ok()?;
    let command = parts.collect::<Vec<_>>().join(" ");
    Some(ProcessEntry {
        pid,
        ppid,
        pgid,
        command,
    })
}

#[cfg(unix)]
fn collect_descendant_pids(entries: &[ProcessEntry], root_pid: u32, pids: &mut BTreeSet<u32>) {
    let mut frontier = vec![root_pid];
    while let Some(parent_pid) = frontier.pop() {
        for entry in entries.iter().filter(|entry| entry.ppid == parent_pid) {
            if pids.insert(entry.pid) {
                frontier.push(entry.pid);
            }
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct WindowsProcessEntry {
    pid: u32,
    ppid: u32,
    command: String,
}

#[cfg(windows)]
fn windows_process_entries() -> Result<Vec<WindowsProcessEntry>, String> {
    let script = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,CommandLine | ConvertTo-Json -Compress";
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|err| format!("failed to inspect running processes: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect running processes".to_string());
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("failed to parse running processes: {}", err))?;
    let mut entries = Vec::new();
    match &value {
        serde_json::Value::Array(items) => {
            entries.extend(items.iter().filter_map(parse_windows_process_entry));
        }
        serde_json::Value::Object(_) => {
            if let Some(entry) = parse_windows_process_entry(&value) {
                entries.push(entry);
            }
        }
        _ => {}
    }
    Ok(entries)
}

#[cfg(windows)]
fn parse_windows_process_entry(value: &serde_json::Value) -> Option<WindowsProcessEntry> {
    let pid = value.get("ProcessId")?.as_u64()? as u32;
    let ppid = value
        .get("ParentProcessId")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as u32;
    let command = value
        .get("CommandLine")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(WindowsProcessEntry { pid, ppid, command })
}

#[cfg(windows)]
fn collect_windows_descendant_pids(
    entries: &[WindowsProcessEntry],
    root_pid: u32,
    pids: &mut BTreeSet<u32>,
) {
    let mut frontier = vec![root_pid];
    while let Some(parent_pid) = frontier.pop() {
        for entry in entries.iter().filter(|entry| entry.ppid == parent_pid) {
            if pids.insert(entry.pid) {
                frontier.push(entry.pid);
            }
        }
    }
}

#[cfg(any(unix, windows))]
fn is_codex_app_server_for_profile(command: &str, profile_name: &str) -> bool {
    (command.contains(" app-server")
        && command_matches_profile(command, profile_name)
        && command_looks_like_codex_app_server(command))
        || is_claude_code_app_server_for_profile(command, profile_name)
}

#[cfg(any(unix, windows))]
fn is_codex_process_for_profile(command: &str, profile_name: &str) -> bool {
    is_codex_app_server_for_profile(command, profile_name)
        || is_codex_electron_app_for_profile(command, profile_name)
}

#[cfg(any(unix, windows))]
fn is_codex_electron_app_for_profile(command: &str, profile_name: &str) -> bool {
    if !command_looks_like_codex_electron_app(command) {
        return false;
    }
    let segment = safe_path_segment(profile_name);
    let normalized = command.replace('\\', "/");
    normalized.contains(&format!("/codex-app-user-data/{}", segment))
        || normalized.contains(&format!("--user-data-dir={}", segment))
}

#[cfg(any(unix, windows))]
fn command_looks_like_codex_electron_app(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("--remote-debugging-port=")
        && (normalized.contains(".app/contents/macos/")
            || DEFAULT_WINDOWS_EXE_NAMES.iter().any(|name| {
                let name = name.to_ascii_lowercase();
                normalized.contains(&format!("/{name}")) || normalized.ends_with(&name)
            }))
}

#[cfg(any(unix, windows))]
fn command_looks_like_codex_app_server(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains(".app/contents/resources/codex")
        || normalized.contains("codex.exe")
        || normalized.split_whitespace().any(|token| token == "codex")
}

#[cfg(any(unix, windows))]
fn command_matches_profile(command: &str, profile_name: &str) -> bool {
    command.contains(&format!("profile=\"{}\"", profile_name))
        || command.contains(&format!("profile='{}'", profile_name))
        || command.contains(&format!("--profile {}", profile_name))
        || command.contains(&format!("--profile \"{}\"", profile_name))
        || command.contains(&format!("--profile '{}'", profile_name))
        || command
            .split_whitespace()
            .any(|token| token == format!("profile={}", profile_name))
}

#[cfg(any(unix, windows))]
fn is_codexl_middleware_command(command: &str) -> bool {
    (command.contains("--codexl-cli-middleware") && command.contains("app-server"))
        || command.contains("--codexl-claude-code-app-server")
}

#[cfg(any(unix, windows))]
fn is_claude_code_app_server_for_profile(command: &str, profile_name: &str) -> bool {
    command.contains("--codexl-claude-code-app-server")
        && (command.contains(&format!("--workspace-name {}", profile_name))
            || command.contains(&format!("--workspace-name \"{}\"", profile_name))
            || command.contains(&format!("--workspace-name '{}'", profile_name)))
}

#[cfg(any(unix, windows))]
fn is_bot_gateway_stdio_command(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/bot-gateway/") && normalized.contains("/stdio/stdio.js")
}

#[cfg(any(unix, windows))]
fn is_next_ai_gateway_command(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/next-ai-gateway/") && normalized.contains("/gateway/start.js")
}

#[cfg(any(unix, windows))]
fn is_bot_media_mcp_command(command: &str) -> bool {
    command.contains("--codexl-bot-media-mcp")
}

#[cfg(any(unix, windows))]
fn is_codexl_extension_process(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("/.codexl/extensions/")
        || normalized.contains("/codexl/extensions/")
        || is_bot_gateway_stdio_command(command)
        || is_next_ai_gateway_command(command)
        || is_bot_media_mcp_command(command)
}

#[cfg(unix)]
fn is_orphaned_codexl_extension_process(entry: &ProcessEntry) -> bool {
    entry.ppid == 1 && is_codexl_extension_process(&entry.command)
}

#[cfg(windows)]
fn is_orphaned_codexl_extension_process_windows(entry: &WindowsProcessEntry) -> bool {
    entry.ppid <= 4 && is_codexl_extension_process(&entry.command)
}

fn executable_from_app_bundle(app_path: &Path) -> Option<String> {
    let info_path = app_path.join("Contents").join("Info.plist");
    let macos_dir = app_path.join("Contents").join("MacOS");

    if let Some(name) = read_bundle_executable(&info_path) {
        let exe_path = macos_dir.join(&name);
        if exe_path.is_file() {
            return Some(exe_path.to_string_lossy().to_string());
        }
    }

    // Fallback: use the app name
    let fallback_name = app_path.file_stem()?.to_string_lossy().to_string();
    let fallback_path = macos_dir.join(&fallback_name);
    if fallback_path.is_file() {
        return Some(fallback_path.to_string_lossy().to_string());
    }

    // Last resort: first file in MacOS dir
    if let Ok(entries) = std::fs::read_dir(&macos_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn bundled_cli_path(codex_app_executable: &str) -> Option<PathBuf> {
    let executable = PathBuf::from(codex_app_executable);
    let file_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let mut candidates = Vec::new();

    if cfg!(windows) {
        if let Some(app_dir) = executable.parent() {
            candidates.push(app_dir.join("resources").join(file_name));
            candidates.push(app_dir.join("Resources").join(file_name));
        }
    }

    if let Some(contents_dir) = executable.parent().and_then(|parent| parent.parent()) {
        candidates.push(contents_dir.join("Resources").join(file_name));
        candidates.push(contents_dir.join("resources").join(file_name));
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn read_bundle_executable(info_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(info_path).ok()?;
    // Simple plist parsing: find CFBundleExecutable value
    if let Some(idx) = content.find("<key>CFBundleExecutable</key>") {
        let rest = &content[idx + "<key>CFBundleExecutable</key>".len()..];
        let rest = rest.trim_start();
        if rest.starts_with('<') {
            return None;
        }
        if let Some(open) = rest.find("<string>") {
            let after_open = &rest[open + "<string>".len()..];
            if let Some(close) = after_open.find("</string>") {
                return Some(after_open[..close].to_string());
            }
        }
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[cfg(target_os = "macos")]
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolves_cli_from_codex_app_resources_without_launching_app_binary() {
        let root = unique_test_dir("codex-cli-resources");
        let macos_dir = root.join("Codex.app").join("Contents").join("MacOS");
        let resources_dir = root.join("Codex.app").join("Contents").join("Resources");
        std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
        std::fs::create_dir_all(&resources_dir).expect("create Resources dir");
        let app_executable = macos_dir.join("Codex");
        let cli_executable = resources_dir.join("codex");
        std::fs::write(&app_executable, "").expect("write app executable");
        std::fs::write(&cli_executable, "").expect("write cli executable");

        let resolved = resolve_codex_cli_executable(None, &app_executable.to_string_lossy());
        assert_ne!(resolved, app_executable.to_string_lossy().to_string());
        assert!(
            resolved == cli_executable.to_string_lossy()
                || !resolved.contains(".app/Contents/MacOS/")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_cli_from_chatgpt_app_resources_without_launching_app_binary() {
        let root = unique_test_dir("chatgpt-cli-resources");
        let macos_dir = root.join("ChatGPT.app").join("Contents").join("MacOS");
        let resources_dir = root.join("ChatGPT.app").join("Contents").join("Resources");
        std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
        std::fs::create_dir_all(&resources_dir).expect("create Resources dir");
        let app_executable = macos_dir.join("ChatGPT");
        let cli_executable = resources_dir.join("codex");
        std::fs::write(&app_executable, "").expect("write app executable");
        std::fs::write(&cli_executable, "").expect("write cli executable");

        let resolved = resolve_codex_cli_executable(None, &app_executable.to_string_lossy());
        assert_eq!(resolved, cli_executable.to_string_lossy().to_string());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cli_resolver_never_falls_back_to_macos_app_executable() {
        let app_executable = "/tmp/MissingCodex.app/Contents/MacOS/Codex";

        let resolved = resolve_codex_cli_executable(None, app_executable);
        assert_ne!(resolved, app_executable);
        assert!(!resolved.contains(".app/Contents/MacOS/"));
    }

    #[test]
    fn detects_codex_app_server_for_profile() {
        let command = r#"/Applications/Codex.app/Contents/Resources/codex -c profile="nextai" app-server --analytics-default-enabled"#;

        assert!(is_codex_app_server_for_profile(command, "nextai"));
        assert!(!is_codex_app_server_for_profile(command, "other"));
    }

    #[test]
    fn detects_codex_electron_app_for_profile() {
        let command = "/Applications/Codex.app/Contents/MacOS/Codex --remote-debugging-port=9227 --user-data-dir=/Users/me/.codex/.codexl/codex-app-user-data/17fd99a1";

        assert!(is_codex_electron_app_for_profile(command, "17fd99a1"));
        assert!(!is_codex_electron_app_for_profile(command, "other"));
        assert!(!is_codex_electron_app_for_profile(
            "/Applications/Codex.app/Contents/Resources/codex app-server",
            "17fd99a1"
        ));
    }

    #[test]
    fn detects_chatgpt_electron_app_for_profile() {
        let command = "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT --remote-debugging-port=9227 --user-data-dir=/Users/me/.codex/.codexl/codex-app-user-data/17fd99a1";

        assert!(is_codex_electron_app_for_profile(command, "17fd99a1"));
        assert!(!is_codex_electron_app_for_profile(command, "other"));
    }

    #[test]
    fn collects_process_descendants() {
        let entries = vec![
            process_entry(10, 1, 10),
            process_entry(11, 10, 10),
            process_entry(12, 11, 10),
            process_entry(20, 1, 20),
        ];
        let mut pids = BTreeSet::new();

        collect_descendant_pids(&entries, 10, &mut pids);

        assert!(pids.contains(&11));
        assert!(pids.contains(&12));
        assert!(!pids.contains(&20));
    }

    #[test]
    fn detects_codexl_extension_processes() {
        assert!(is_codexl_extension_process(
            "/usr/local/bin/node /Users/me/.codexl/extensions/bot-gateway/1.0.0/stdio/stdio.js"
        ));
        assert!(is_codexl_extension_process(
            "/usr/local/bin/node /Users/me/.codexl/extensions/next-ai-gateway/1.0.0/gateway/start.js"
        ));
        assert!(is_codexl_extension_process(
            "/Applications/Codex Launcher.app/Contents/MacOS/codex-launcher --codexl-bot-media-mcp"
        ));
        assert!(!is_codexl_extension_process(
            "/Applications/Codex.app/Contents/Resources/codex app-server"
        ));
    }

    #[test]
    fn detects_next_ai_gateway_processes() {
        assert!(is_next_ai_gateway_command(
            "/usr/local/bin/node /Users/me/.codexl/extensions/next-ai-gateway/1.0.0/gateway/start.js"
        ));
        assert!(is_next_ai_gateway_command(
            r"C:\Users\me\CodexL\extensions\next-ai-gateway\1.0.0\gateway\start.js"
        ));
        assert!(!is_next_ai_gateway_command(
            "/usr/local/bin/node /Users/me/.codexl/extensions/bot-gateway/1.0.0/stdio/stdio.js"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn parses_lsof_pid_lines() {
        let pids = parse_lsof_pid_lines("p123\nn*:14589\np456\n");

        assert!(pids.contains(&123));
        assert!(pids.contains(&456));
        assert!(!pids.contains(&14589));
    }

    #[test]
    fn detects_orphaned_codexl_extension_processes() {
        let mut entry = process_entry(10, 1, 10);
        entry.command =
            "/usr/local/bin/node /Users/me/.codexl/extensions/bot-gateway/1.0.0/stdio/stdio.js"
                .to_string();

        assert!(is_orphaned_codexl_extension_process(&entry));

        entry.ppid = 20;
        assert!(!is_orphaned_codexl_extension_process(&entry));

        entry.ppid = 1;
        entry.command = "/usr/local/bin/node /tmp/other/stdio.js".to_string();
        assert!(!is_orphaned_codexl_extension_process(&entry));
    }

    #[test]
    fn normalizes_windows_squirrel_app_directory_to_versioned_executable() {
        let root = unique_test_dir("windows-squirrel-codex-app");
        let app_dir = root.join("Codex");
        let versioned_dir = app_dir.join("app-1.2.3");
        let executable = versioned_dir.join("Codex.exe");
        std::fs::create_dir_all(&versioned_dir).expect("create app dir");
        std::fs::write(&executable, "").expect("write executable");

        assert_eq!(
            normalize_windows_codex_app_candidate(app_dir),
            Some(executable)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn windows_app_candidates_include_alias_and_openai_install_roots() {
        let root = unique_test_dir("windows-app-candidates");
        let candidates = windows_app_candidates_from_roots(vec![root.clone()]);

        let windows_apps = root.join("Microsoft").join("WindowsApps");
        assert!(candidates.iter().any(|candidate| {
            candidate.parent() == Some(windows_apps.as_path())
                && candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
        }));
        assert!(candidates.contains(&root.join("Programs").join("OpenAI").join("Codex")));
        assert!(candidates.contains(&root.join("OpenAI").join("OpenAI Codex")));
    }

    #[test]
    fn configure_proxy_env_sets_common_proxy_variables() {
        let mut command = Command::new("codex");

        configure_proxy_env(&mut command, Some(" http://127.0.0.1:7890 "));

        for key in [
            "http_proxy",
            "HTTP_PROXY",
            "https_proxy",
            "HTTPS_PROXY",
            "all_proxy",
            "ALL_PROXY",
        ] {
            assert_eq!(
                command_env_value(&command, key),
                Some("http://127.0.0.1:7890".to_string())
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn global_computer_use_env_uses_global_home_not_active_codex_home() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env lock");
        let root = unique_test_dir("global-computer-use-env");
        let profile_home = root.join(".codexl").join("codex-homes").join("Workspace");
        let global_app = root
            .join(".codex")
            .join("computer-use")
            .join("Codex Computer Use.app");
        std::fs::create_dir_all(&profile_home).expect("create profile home");
        std::fs::create_dir_all(&global_app).expect("create global computer use app");

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_codexl_home = std::env::var_os("CODEXL_CODEX_HOME");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &profile_home);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let mut command = Command::new("codex");
        configure_global_computer_use_env(&mut command);

        assert_eq!(
            command_env_value(&command, "SKY_CUA_SERVICE_PATH"),
            Some(global_app.to_string_lossy().to_string())
        );

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env("CODEXL_CODEX_HOME", old_codexl_home);
        let _ = std::fs::remove_dir_all(root);
    }

    fn command_env_value(command: &Command, key: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().to_string()))
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), nanos))
    }

    fn process_entry(pid: u32, ppid: u32, pgid: u32) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            pgid,
            command: String::new(),
        }
    }

    #[cfg(target_os = "macos")]
    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod electron_user_data_tests {
    use super::*;

    #[test]
    fn electron_user_data_dir_is_scoped_by_profile() {
        let home = if cfg!(windows) {
            r"C:\Users\me\.codex"
        } else {
            "/Users/me/.codex"
        };

        let first = electron_user_data_dir(Some(home), Some("Default"));
        let second = electron_user_data_dir(Some(home), Some("Other Provider"));

        assert_ne!(first, second);
        assert!(first.ends_with(Path::new("codex-app-user-data").join("Default")));
        assert!(second.ends_with(Path::new("codex-app-user-data").join("Other-Provider")));
    }

    #[test]
    fn configure_electron_user_data_dir_sets_codex_electron_env() {
        let home =
            std::env::temp_dir().join(format!("codexl-user-data-env-{}", std::process::id()));
        let mut command = Command::new(if cfg!(windows) { "Codex.exe" } else { "Codex" });

        configure_electron_user_data_dir(
            &mut command,
            Some(&home.to_string_lossy()),
            Some("Default"),
        )
        .expect("configure user data dir");

        let value = command
            .get_envs()
            .find(|(name, _)| *name == std::ffi::OsStr::new("CODEX_ELECTRON_USER_DATA_PATH"))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().to_string()))
            .expect("CODEX_ELECTRON_USER_DATA_PATH env");

        assert_eq!(
            PathBuf::from(value),
            home.join(".codexl")
                .join("codex-app-user-data")
                .join("Default")
        );
        let _ = std::fs::remove_dir_all(home);
    }
}
