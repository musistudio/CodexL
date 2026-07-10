use crate::{
    cli_middleware,
    config::{
        self, AppConfig, ProviderProfile, WorkspaceRequest, REMOTE_FRONTEND_MODE_APP,
        REMOTE_FRONTEND_MODE_CLAUDE_CODE, REMOTE_FRONTEND_MODE_CLI,
    },
    launcher, remote, server, AppState,
};
use serde::Serialize;
use std::{ffi::OsString, process::Command};

pub fn run_if_requested() -> bool {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return false;
    }
    if args.first().is_some_and(|arg| arg == "app" || arg == "gui") {
        return false;
    }

    let code = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run(args)),
        Err(err) => {
            eprintln!("failed to create async runtime: {}", err);
            1
        }
    };
    std::process::exit(code);
}

async fn run(args: Vec<String>) -> i32 {
    match run_result(args).await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err);
            1
        }
    }
}

async fn run_result(args: Vec<String>) -> Result<i32, String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(0);
    };
    let rest = &args[1..];
    match command {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(0)
        }
        "version" | "--version" | "-V" => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        "config" => run_config_command(rest),
        "workspace" => run_workspace_command(rest),
        "codex-app" => run_codex_app_command(rest),
        "codex-web" => run_codex_web_command(rest).await,
        "remote" => run_remote_command(rest).await,
        "codex" => run_codex_command(rest),
        _ => {
            print_help();
            Ok(2)
        }
    }
}

fn run_config_command(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("show") | None => {
            print_json(&AppConfig::load())?;
            Ok(0)
        }
        Some("path") => {
            println!("{}", config_path_display());
            Ok(0)
        }
        Some("help" | "--help" | "-h") => {
            print_config_help();
            Ok(0)
        }
        Some(command) => Err(format!("unknown config command: {}", command)),
    }
}

fn run_workspace_command(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("list") | None => workspace_list(&args.get(1..).unwrap_or_default()),
        Some("show") => workspace_show(&args[1..]),
        Some("use") => workspace_use(&args[1..]),
        Some("create") => workspace_create(&args[1..]),
        Some("set-remote-frontend") => workspace_set_remote_frontend(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print_workspace_help();
            Ok(0)
        }
        Some(command) => Err(format!("unknown workspace command: {}", command)),
    }
}

fn workspace_list(args: &[String]) -> Result<i32, String> {
    let json = has_flag(args, "--json");
    let config = AppConfig::load();
    if json {
        print_json(&config.provider_profiles)?;
        return Ok(0);
    }

    println!("NAME\tMODE\tPROVIDER\tMODEL\tREMOTE_FRONTEND");
    for profile in &config.provider_profiles {
        let provider = if profile.provider_name.trim().is_empty() {
            "-"
        } else {
            profile.provider_name.as_str()
        };
        let model = if profile.model.trim().is_empty() {
            "-"
        } else {
            profile.model.as_str()
        };
        println!(
            "{}\t{}\t{}\t{}\t{}",
            profile.name,
            workspace_kind(profile),
            provider,
            model,
            remote_frontend_label(profile)
        );
    }
    Ok(0)
}

fn workspace_show(args: &[String]) -> Result<i32, String> {
    let (name, json) = parse_name_and_json(args, "workspace show <name>")?;
    let config = AppConfig::load();
    let profile = config
        .provider_profile(&name)
        .ok_or_else(|| format!("workspace not found: {}", name))?;
    if json {
        print_json(&profile)?;
    } else {
        println!("name: {}", profile.name);
        println!("kind: {}", workspace_kind(&profile));
        println!("provider: {}", empty_dash(&profile.provider_name));
        println!("model: {}", empty_dash(&profile.model));
        println!("codex_home: {}", empty_dash(&profile.codex_home));
        println!("remote_frontend: {}", remote_frontend_label(&profile));
        if config::remote_frontend_mode_uses_cli(&profile.remote_frontend_mode) {
            println!(
                "registry_url: {}",
                empty_dash(&profile.remote_web_asset_registry_url)
            );
            println!(
                "registry_version: {}",
                empty_dash(&profile.remote_web_asset_version)
            );
        }
    }
    Ok(0)
}

fn workspace_use(args: &[String]) -> Result<i32, String> {
    let name = positional_name(args, "workspace use <name>")?;
    let mut config = AppConfig::load();
    let profile = config
        .provider_profile(&name)
        .ok_or_else(|| format!("workspace not found: {}", name))?;
    config.active_provider = config::provider_profile_key(&profile);
    config.normalize();
    config.save()?;
    println!("active workspace: {}", name);
    Ok(0)
}

fn workspace_create(args: &[String]) -> Result<i32, String> {
    let mut name = String::new();
    let mut proxy_url = String::new();
    let mut mode = REMOTE_FRONTEND_MODE_APP.to_string();
    let mut registry_url = String::new();
    let mut version = "latest".to_string();
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--proxy-url" => {
                index += 1;
                proxy_url = read_arg_value(args, index, "--proxy-url")?;
            }
            "--mode" => {
                index += 1;
                mode = normalized_remote_frontend_mode(&read_arg_value(args, index, "--mode")?);
            }
            "--registry-url" => {
                index += 1;
                registry_url =
                    normalize_registry_url(&read_arg_value(args, index, "--registry-url")?);
            }
            "--version" => {
                index += 1;
                version = normalized_version(&read_arg_value(args, index, "--version")?);
            }
            "--help" | "-h" => {
                print_workspace_help();
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown workspace create option: {}", value));
            }
            value if name.is_empty() => name = value.to_string(),
            value => return Err(format!("unexpected argument: {}", value)),
        }
        index += 1;
    }

    if name.trim().is_empty() {
        return Err(
            "usage: codexl workspace create <name> [--mode app|cli|claude-code]".to_string(),
        );
    }
    if config::remote_frontend_mode_uses_cli(&mode) && registry_url.is_empty() {
        registry_url = default_registry_url();
    }

    let mut config = AppConfig::load();
    let profile = config::create_workspace_profile(WorkspaceRequest {
        workspace_name: name,
        proxy_url,
        remote_frontend_mode: mode,
        remote_web_asset_registry_url: registry_url,
        remote_web_asset_version: version,
        bot: config::BotProfileConfig::default(),
    })?;
    let profile_name = config::provider_profile_key(&profile);
    config.add_provider_profile(profile);
    config.save()?;
    let saved_profile = config
        .provider_profile(&profile_name)
        .ok_or_else(|| format!("workspace not found after create: {}", profile_name))?;
    if json {
        print_json(&saved_profile)?;
    } else {
        println!("created workspace: {}", saved_profile.name);
    }
    Ok(0)
}

fn workspace_set_remote_frontend(args: &[String]) -> Result<i32, String> {
    let mut name = String::new();
    let mut mode = String::new();
    let mut registry_url: Option<String> = None;
    let mut version: Option<String> = None;
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--mode" => {
                index += 1;
                mode = normalized_remote_frontend_mode(&read_arg_value(args, index, "--mode")?);
            }
            "--registry-url" => {
                index += 1;
                registry_url = Some(normalize_registry_url(&read_arg_value(
                    args,
                    index,
                    "--registry-url",
                )?));
            }
            "--version" => {
                index += 1;
                version = Some(normalized_version(&read_arg_value(
                    args,
                    index,
                    "--version",
                )?));
            }
            "--help" | "-h" => {
                print_workspace_help();
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(format!(
                    "unknown workspace set-remote-frontend option: {}",
                    value
                ));
            }
            value if name.is_empty() => name = value.to_string(),
            value => return Err(format!("unexpected argument: {}", value)),
        }
        index += 1;
    }

    if name.trim().is_empty() || mode.is_empty() {
        return Err(
            "usage: codexl workspace set-remote-frontend <name> --mode app|cli|claude-code"
                .to_string(),
        );
    }

    let mut config = AppConfig::load();
    let mut profile = config
        .provider_profile(&name)
        .ok_or_else(|| format!("workspace not found: {}", name))?;
    profile.remote_frontend_mode = mode;
    if let Some(registry_url) = registry_url {
        profile.remote_web_asset_registry_url = registry_url;
    }
    if let Some(version) = version {
        profile.remote_web_asset_version = version;
    }
    if config::remote_frontend_mode_uses_cli(&profile.remote_frontend_mode) {
        if profile.remote_web_asset_registry_url.trim().is_empty() {
            profile.remote_web_asset_registry_url = default_registry_url();
        }
        if profile.remote_web_asset_version.trim().is_empty() {
            profile.remote_web_asset_version = "latest".to_string();
        }
    }
    let profile_name = config::provider_profile_key(&profile);
    config.update_provider_profile(&name, profile)?;
    config.save()?;
    let saved_profile = config
        .provider_profile(&profile_name)
        .ok_or_else(|| format!("workspace not found after update: {}", profile_name))?;
    if json {
        print_json(&saved_profile)?;
    } else {
        println!(
            "updated {} remote frontend: {}",
            saved_profile.name,
            remote_frontend_label(&saved_profile)
        );
    }
    Ok(0)
}

fn run_codex_app_command(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("find") | None => {
            let path =
                launcher::find_codex_app().ok_or_else(|| "ChatGPT app not found".to_string())?;
            println!("{}", path);
            Ok(0)
        }
        Some("help" | "--help" | "-h") => {
            print_codex_app_help();
            Ok(0)
        }
        Some(command) => Err(format!("unknown codex-app command: {}", command)),
    }
}

async fn run_codex_web_command(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("versions") | None => {
            let mut registry_url = default_registry_url();
            let mut json = false;
            let mut index = if args.first().map(String::as_str) == Some("versions") {
                1
            } else {
                0
            };
            while index < args.len() {
                match args[index].as_str() {
                    "--json" => json = true,
                    "--registry-url" => {
                        index += 1;
                        registry_url =
                            normalize_registry_url(&read_arg_value(args, index, "--registry-url")?);
                    }
                    "--help" | "-h" => {
                        print_codex_web_help();
                        return Ok(0);
                    }
                    value => return Err(format!("unknown codex-web versions option: {}", value)),
                }
                index += 1;
            }
            let result = crate::list_codex_web_asset_versions(registry_url).await?;
            if json {
                print_json(&result)?;
            } else {
                println!("latest: {}", result.latest);
                for version in result.versions {
                    println!("{}", version);
                }
            }
            Ok(0)
        }
        Some("help" | "--help" | "-h") => {
            print_codex_web_help();
            Ok(0)
        }
        Some(command) => Err(format!("unknown codex-web command: {}", command)),
    }
}

async fn run_remote_command(args: &[String]) -> Result<i32, String> {
    match args.first().map(String::as_str) {
        Some("start") => remote_start(&args[1..]).await,
        Some("help" | "--help" | "-h") | None => {
            print_remote_help();
            Ok(0)
        }
        Some(command) => Err(format!("unknown remote command: {}", command)),
    }
}

async fn remote_start(args: &[String]) -> Result<i32, String> {
    let mut profile_name = String::new();
    let mut use_cloud = false;
    let mut password: Option<String> = None;
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--cloud" => use_cloud = true,
            "--json" => json = true,
            "--password" => {
                index += 1;
                password = Some(read_arg_value(args, index, "--password")?);
            }
            "--help" | "-h" => {
                print_remote_help();
                return Ok(0);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown remote start option: {}", value));
            }
            value if profile_name.is_empty() => profile_name = value.to_string(),
            value => return Err(format!("unexpected argument: {}", value)),
        }
        index += 1;
    }

    let state = AppState::new(AppConfig::load());
    if profile_name.is_empty() {
        profile_name = state.config.lock().await.active_provider.clone();
    }
    if state
        .config
        .lock()
        .await
        .provider_profile(&profile_name)
        .is_none()
    {
        return Err(format!("workspace not found: {}", profile_name));
    }

    let info = remote::start_remote_control(
        &state,
        profile_name.clone(),
        password,
        Some(use_cloud),
        Some(use_cloud),
    )
    .await?;
    if json {
        print_json(&info)?;
    } else {
        println!("remote control started: {}", profile_name);
        println!("url: {}", info.url);
        println!("lan_url: {}", info.lan_url);
        println!("token: {}", info.token);
        println!("press Ctrl+C to stop");
    }

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("failed to listen for Ctrl+C: {}", e))?;
    remote::stop_remote_control(&state, &profile_name).await?;
    server::stop_codex_instance(&state, Some(profile_name)).await?;
    Ok(0)
}

fn run_codex_command(args: &[String]) -> Result<i32, String> {
    let mut profile_name: Option<String> = None;
    let mut codex_path: Option<String> = None;
    let mut forwarded = Vec::new();
    let mut forward_rest = false;

    let mut index = 0;
    while index < args.len() {
        if forward_rest {
            forwarded.push(args[index].clone());
            index += 1;
            continue;
        }
        match args[index].as_str() {
            "--" => forward_rest = true,
            "--profile" => {
                index += 1;
                profile_name = Some(read_arg_value(args, index, "--profile")?);
            }
            "--codex-path" => {
                index += 1;
                codex_path = Some(read_arg_value(args, index, "--codex-path")?);
            }
            "--help" | "-h" if args.len() == 1 => {
                print_codex_help();
                return Ok(0);
            }
            value => forwarded.push(value.to_string()),
        }
        index += 1;
    }

    let mut config = AppConfig::load();
    let profile_name = profile_name.unwrap_or_else(|| config.active_provider.clone());
    let profile = config
        .provider_profile(&profile_name)
        .ok_or_else(|| format!("workspace not found: {}", profile_name))?;
    config.active_provider = config::provider_profile_key(&profile);

    let executable = resolve_codex_cli_executable(codex_path.as_deref(), &config)?;
    let profile_config_format = config::codex_profile_config_format_for_cli(&executable);
    config.codex_home =
        config::ensure_provider_codex_home_with_format(&profile, profile_config_format)?;
    config.normalize();

    let active_cli_profile = config.active_cli_profile();
    let active_cli_model_provider = config.active_cli_model_provider();
    let real_args = cli_middleware::real_cli_args(
        active_cli_profile.as_deref(),
        active_cli_model_provider.as_deref(),
        profile_config_format,
        forwarded.into_iter().map(OsString::from).collect(),
    );
    let status = Command::new(&executable)
        .args(&real_args)
        .env("CODEX_HOME", config.codex_home.clone())
        .status()
        .map_err(|e| format!("failed to run Codex CLI {}: {}", executable, e))?;
    Ok(status.code().unwrap_or(1))
}

fn resolve_codex_cli_executable(
    explicit_path: Option<&str>,
    config: &AppConfig,
) -> Result<String, String> {
    Ok(launcher::resolve_codex_cli_executable(
        explicit_path,
        &config.codex_path,
    ))
}

fn parse_name_and_json(args: &[String], usage: &str) -> Result<(String, bool), String> {
    let mut name = String::new();
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            value if value.starts_with('-') => return Err(format!("unknown option: {}", value)),
            value if name.is_empty() => name = value.to_string(),
            value => return Err(format!("unexpected argument: {}", value)),
        }
    }
    if name.is_empty() {
        return Err(format!("usage: codexl {}", usage));
    }
    Ok((name, json))
}

fn positional_name(args: &[String], usage: &str) -> Result<String, String> {
    let mut name = String::new();
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => return Err(format!("usage: codexl {}", usage)),
            value if value.starts_with('-') => return Err(format!("unknown option: {}", value)),
            value if name.is_empty() => name = value.to_string(),
            value => return Err(format!("unexpected argument: {}", value)),
        }
    }
    if name.is_empty() {
        return Err(format!("usage: codexl {}", usage));
    }
    Ok(name)
}

fn read_arg_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("missing value for {}", flag))
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn normalized_remote_frontend_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        REMOTE_FRONTEND_MODE_CLI => REMOTE_FRONTEND_MODE_CLI.to_string(),
        REMOTE_FRONTEND_MODE_CLAUDE_CODE => REMOTE_FRONTEND_MODE_CLAUDE_CODE.to_string(),
        REMOTE_FRONTEND_MODE_APP => REMOTE_FRONTEND_MODE_APP.to_string(),
        _ => REMOTE_FRONTEND_MODE_APP.to_string(),
    }
}

fn normalize_registry_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn normalized_version(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "latest".to_string()
    } else {
        value.to_string()
    }
}

fn workspace_kind(profile: &ProviderProfile) -> &'static str {
    if profile.provider_name.trim().is_empty() && profile.model.trim().is_empty() {
        "workspace"
    } else {
        "provider"
    }
}

fn remote_frontend_label(profile: &ProviderProfile) -> String {
    match normalized_remote_frontend_mode(&profile.remote_frontend_mode).as_str() {
        REMOTE_FRONTEND_MODE_CLI => {
            format!(
                "cli@{}",
                normalized_version(&profile.remote_web_asset_version)
            )
        }
        REMOTE_FRONTEND_MODE_CLAUDE_CODE => {
            format!(
                "claude-code@{}",
                normalized_version(&profile.remote_web_asset_version)
            )
        }
        _ => "app".to_string(),
    }
}

fn empty_dash(value: &str) -> &str {
    if value.trim().is_empty() {
        "-"
    } else {
        value
    }
}

fn default_registry_url() -> String {
    std::env::var("CODEXL_REMOTE_WEB_ASSET_REGISTRY_URL")
        .ok()
        .map(|value| normalize_registry_url(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://web.codexl.io".to_string())
}

fn config_path_display() -> String {
    std::env::var("CODEXL_CONFIG_PATH").unwrap_or_else(|_| "~/.codexl/config.json".to_string())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    let output = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    println!("{}", output);
    Ok(())
}

fn print_help() {
    println!(
        r#"CodexL CLI

Usage:
  codexl app
  codexl version
  codexl config show
  codexl workspace list [--json]
  codexl workspace show <name> [--json]
  codexl workspace use <name>
  codexl workspace create <name> [--mode app|cli|claude-code] [--registry-url URL] [--version VERSION]
  codexl workspace set-remote-frontend <name> --mode app|cli|claude-code [--registry-url URL] [--version VERSION]
  codexl codex [--profile NAME] [--codex-path PATH] -- [ARGS...]
  codexl codex-app find
  codexl codex-web versions [--registry-url URL] [--json]
  codexl remote start [name] [--cloud] [--password PASSWORD] [--json]

No arguments starts the desktop app."#
    );
}

fn print_config_help() {
    println!(
        r#"Usage:
  codexl config show
  codexl config path"#
    );
}

fn print_workspace_help() {
    println!(
        r#"Usage:
  codexl workspace list [--json]
  codexl workspace show <name> [--json]
  codexl workspace use <name>
  codexl workspace create <name> [--mode app|cli|claude-code] [--registry-url URL] [--version VERSION]
  codexl workspace set-remote-frontend <name> --mode app|cli|claude-code [--registry-url URL] [--version VERSION]"#
    );
}

fn print_codex_app_help() {
    println!("Usage:\n  codexl codex-app find");
}

fn print_codex_web_help() {
    println!("Usage:\n  codexl codex-web versions [--registry-url URL] [--json]");
}

fn print_remote_help() {
    println!("Usage:\n  codexl remote start [name] [--cloud] [--password PASSWORD] [--json]");
}

fn print_codex_help() {
    println!("Usage:\n  codexl codex [--profile NAME] [--codex-path PATH] -- [ARGS...]");
}
