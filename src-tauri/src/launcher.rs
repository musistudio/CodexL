use crate::cli_middleware;
use crate::config::BotProfileConfig;
#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DEFAULT_MAC_APP_NAMES: &[&str] = &["Codex.app", "OpenAI Codex.app"];

#[derive(Debug)]
pub struct CodexLaunch {
    pub child: Child,
    pub cli_stdio_path: Option<String>,
}

pub fn find_codex_app() -> Option<String> {
    if cfg!(target_os = "macos") {
        find_mac_app()
    } else {
        None
    }
}

pub fn launch_codex(
    executable: &str,
    cdp_port: u16,
    codex_home: Option<&str>,
    stdio_name: Option<&str>,
    codex_profile: Option<&str>,
    codex_model_provider: Option<&str>,
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

    let mut cli_stdio_path = None;
    if !cli_middleware::is_disabled() {
        let middleware = cli_middleware::prepare(
            executable,
            codex_home,
            stdio_name,
            codex_profile,
            codex_model_provider,
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
    }

    configure_bot_gateway_bridge_env(&mut command, stdio_name, bot_config, language);
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
            .filter(|entry| is_codex_app_server_for_profile(&entry.command, profile_name))
        {
            pids.insert(entry.pid);
            pgids.insert(entry.pgid);

            if let Some(parent) = entries.iter().find(|parent| {
                parent.pid == entry.ppid && is_codexl_middleware_command(&parent.command)
            }) {
                pids.insert(parent.pid);
                collect_descendant_pids(&entries, parent.pid, &mut pids);
            } else {
                collect_descendant_pids(&entries, entry.pid, &mut pids);
            }
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
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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

    Ok(())
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

fn find_mac_app() -> Option<String> {
    let home = std::env::var("HOME").ok();
    let candidates: Vec<PathBuf> = DEFAULT_MAC_APP_NAMES
        .iter()
        .flat_map(|name| {
            let mut paths = vec![PathBuf::from("/Applications").join(name)];
            if let Some(ref h) = home {
                paths.push(PathBuf::from(h).join("Applications").join(name));
            }
            paths
        })
        .collect();

    for app_path in &candidates {
        if app_path.is_dir() {
            if let Some(exe) = executable_from_app_bundle(app_path) {
                return Some(exe);
            }
        }
    }
    None
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

#[cfg(unix)]
fn is_codex_app_server_for_profile(command: &str, profile_name: &str) -> bool {
    command.contains(" app-server")
        && command_matches_profile(command, profile_name)
        && command.contains(".app/Contents/Resources/codex")
}

#[cfg(unix)]
fn command_matches_profile(command: &str, profile_name: &str) -> bool {
    command.contains(&format!("profile=\"{}\"", profile_name))
        || command.contains(&format!("profile='{}'", profile_name))
        || command
            .split_whitespace()
            .any(|token| token == format!("profile={}", profile_name))
}

#[cfg(unix)]
fn is_codexl_middleware_command(command: &str) -> bool {
    command.contains("--codexl-cli-middleware") && command.contains("app-server")
}

#[cfg(unix)]
fn is_bot_gateway_stdio_command(command: &str) -> bool {
    command.contains("/bot-gateway/") && command.contains("/stdio/stdio.js")
}

#[cfg(unix)]
fn is_next_ai_gateway_command(command: &str) -> bool {
    command.contains("/next-ai-gateway/") && command.contains("/gateway/start.js")
}

#[cfg(unix)]
fn is_bot_media_mcp_command(command: &str) -> bool {
    command.contains("--codexl-bot-media-mcp")
}

#[cfg(unix)]
fn is_codexl_extension_process(command: &str) -> bool {
    command.contains("/.codexl/extensions/")
        || is_bot_gateway_stdio_command(command)
        || is_next_ai_gateway_command(command)
        || is_bot_media_mcp_command(command)
}

#[cfg(unix)]
fn is_orphaned_codexl_extension_process(entry: &ProcessEntry) -> bool {
    entry.ppid == 1 && is_codexl_extension_process(&entry.command)
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

    #[test]
    fn detects_codex_app_server_for_profile() {
        let command = r#"/Applications/Codex.app/Contents/Resources/codex -c profile="nextai" app-server --analytics-default-enabled"#;

        assert!(is_codex_app_server_for_profile(command, "nextai"));
        assert!(!is_codex_app_server_for_profile(command, "other"));
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

    fn command_env_value(command: &Command, key: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().to_string()))
    }

    fn process_entry(pid: u32, ppid: u32, pgid: u32) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            pgid,
            command: String::new(),
        }
    }
}
