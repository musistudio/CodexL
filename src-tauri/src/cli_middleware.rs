use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::claude_code_app_server;
use crate::extensions::builtins::bot_bridge;
use crate::{
    config::{AppConfig, CodexProfileConfigFormat},
    remote,
};
use serde_json::{json, Map, Value};

const DISABLE_ENV: &str = "CODEXL_DISABLE_CLI_MIDDLEWARE";
const REAL_CLI_ENV: &str = "CODEXL_REAL_CODEX_CLI_PATH";
const BUNDLED_REAL_CLI_ENV: &str = "CODEXL_BUNDLED_CODEX_CLI_PATH";
const MIDDLEWARE_LOG_ENV: &str = "CODEXL_CLI_MIDDLEWARE_LOG";
pub const CODEX_PROFILE_ENV: &str = "CODEXL_CODEX_PROFILE";
pub const CODEX_MODEL_PROVIDER_ENV: &str = "CODEXL_CODEX_MODEL_PROVIDER";
pub const CODEX_WORKSPACE_NAME_ENV: &str = "CODEXL_CODEX_WORKSPACE_NAME";
pub const CODEX_CORE_MODE_ENV: &str = "CODEXL_CODEX_CORE_MODE";
const CODEX_PROFILE_CONFIG_FORMAT_ENV: &str = "CODEXL_CODEX_PROFILE_CONFIG_FORMAT";
const LEGACY_CODEX_INSTANCE_NAME_ENV: &str = "CODEXL_CODEX_INSTANCE_NAME";
const CODEX_CLI_PATH_ENV: &str = "CODEX_CLI_PATH";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
#[cfg(target_os = "macos")]
const SIGNED_SUPERVISOR_NODE_ENV: &str = "CODEXL_SIGNED_CODEX_SUPERVISOR_NODE_PATH";
#[cfg(target_os = "macos")]
const SIGNED_SUPERVISOR_SOURCE: &str = r#"const { spawn } = require("node:child_process");
const executable = process.argv[1];
const args = process.argv.slice(2);
const child = spawn(executable, args, { env: process.env, stdio: "inherit" });
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => child.kill(signal));
}
child.once("error", (error) => {
  console.error(`Failed to launch supervised Codex CLI: ${error.message}`);
  process.exit(1);
});
child.once("exit", (code) => process.exit(code ?? 1));
"#;
const RUN_MODE_ARG: &str = "--codexl-cli-middleware";
const STDIO_RUN_MODE_ARG: &str = "--codexl-cli-stdio";
const BOT_MEDIA_MCP_RUN_MODE_ARG: &str = "--codexl-bot-media-mcp";
pub const CLAUDE_CODE_APP_SERVER_RUN_MODE_ARG: &str = "--codexl-claude-code-app-server";
pub const CLAUDE_CODE_MCP_METADATA_RELAY_RUN_MODE_ARG: &str =
    "--codexl-claude-code-mcp-metadata-relay";
const CODEXL_WORKSPACE_CWD_FILTER_KEY: &str = "codexlWorkspaceCwd";

type RequestMap = Arc<Mutex<std::collections::HashMap<String, RequestInfo>>>;
type SharedChildStdin = bot_bridge::SharedAppStdin;
type SharedCurrentCwd = Arc<Mutex<Option<String>>>;
type SharedOutput<W> = Arc<Mutex<W>>;

#[cfg(windows)]
const MIDDLEWARE_FILE_NAME: &str = "codexl-codex-cli-middleware.exe";
#[cfg(windows)]
const MIDDLEWARE_FILE_STEM: &str = "codexl-codex-cli-middleware";
#[cfg(windows)]
const STDIO_FILE_NAME: &str = "codexl-codex-cli-stdio.cmd";

#[cfg(not(windows))]
const MIDDLEWARE_FILE_NAME: &str = "codexl-codex-cli-middleware";
#[cfg(not(windows))]
const STDIO_FILE_NAME: &str = "codexl-codex-cli-stdio";

#[derive(Debug, Clone)]
pub struct MiddlewareEnv {
    pub executable_path: PathBuf,
    pub stdio_path: PathBuf,
    pub real_cli_path: PathBuf,
    pub log_path: PathBuf,
    pub workspace_name: Option<String>,
    pub profile: Option<String>,
    pub model_provider: Option<String>,
    pub core_mode: Option<String>,
}

#[derive(Debug, Clone)]
struct RequestInfo {
    method: String,
    include_token: bool,
    params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionHome {
    profile_name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct SessionFile {
    profile_name: String,
    path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatGptAuth {
    email: Option<String>,
    workspace_name: Option<String>,
    plan_type: Option<String>,
    auth_token: Option<String>,
}

pub fn is_disabled() -> bool {
    std::env::var(DISABLE_ENV)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn prepare(
    codex_app_executable: &str,
    codex_home: Option<&str>,
    stdio_name: Option<&str>,
    codex_profile: Option<&str>,
    codex_model_provider: Option<&str>,
    core_mode: Option<&str>,
) -> Result<MiddlewareEnv, String> {
    let executable_path = middleware_path();
    let export_stdio_path = stdio_path(stdio_name);
    let default_stdio_path = stdio_path(None);
    let real_cli_path = resolve_real_cli_path(codex_app_executable, &executable_path)?;
    let profile_config_format =
        crate::config::codex_profile_config_format_for_cli(&real_cli_path.to_string_lossy());
    let host_executable = std::env::current_exe().map_err(|e| e.to_string())?;
    write_middleware(&executable_path, &host_executable, &real_cli_path)?;
    write_real_cli_cache(&real_cli_path)?;
    let log_path = default_log_path();
    let codex_home = normalize_profile(codex_home);
    let profile = normalize_profile(codex_profile);
    let workspace_name = normalize_profile(stdio_name).or_else(|| profile.clone());
    let model_provider = normalize_model_provider(codex_model_provider, profile.as_deref());
    let core_mode = normalize_profile(core_mode);
    write_stdio_export(
        &export_stdio_path,
        &host_executable,
        &executable_path,
        &real_cli_path,
        &log_path,
        codex_home.as_deref(),
        workspace_name.as_deref(),
        profile.as_deref(),
        model_provider.as_deref(),
        core_mode.as_deref(),
        profile_config_format,
    )?;
    if default_stdio_path != export_stdio_path {
        write_stdio_export(
            &default_stdio_path,
            &host_executable,
            &executable_path,
            &real_cli_path,
            &log_path,
            codex_home.as_deref(),
            workspace_name.as_deref(),
            profile.as_deref(),
            model_provider.as_deref(),
            core_mode.as_deref(),
            profile_config_format,
        )?;
    }
    Ok(MiddlewareEnv {
        executable_path,
        stdio_path: export_stdio_path,
        real_cli_path,
        log_path,
        workspace_name,
        profile,
        model_provider,
        core_mode,
    })
}

pub fn run_if_requested() -> bool {
    let mut args = std::env::args_os();
    let program = args.next();
    #[cfg(not(windows))]
    let _ = &program;
    let Some(mode) = args.next() else {
        #[cfg(windows)]
        {
            if program_is_windows_middleware_shim(program.as_deref()) {
                let exit_code = run_stdio_middleware(Vec::new());
                let exit_code = match exit_code {
                    Ok(code) => code,
                    Err(err) => {
                        eprintln!("{}", err);
                        1
                    }
                };
                std::process::exit(exit_code);
            }
        }
        return false;
    };

    let forwarded_args: Vec<OsString> = args.collect();
    #[cfg(windows)]
    if program_is_windows_middleware_shim(program.as_deref()) {
        let mut all_args = Vec::with_capacity(forwarded_args.len() + 1);
        all_args.push(mode);
        all_args.extend(forwarded_args);
        let exit_code = match run_stdio_middleware(all_args) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("{}", err);
                1
            }
        };
        std::process::exit(exit_code);
    }

    let exit_code = match mode.as_os_str() {
        value if value == OsStr::new(RUN_MODE_ARG) => run_stdio_middleware(forwarded_args),
        value if value == OsStr::new(STDIO_RUN_MODE_ARG) => {
            run_stdio_middleware(external_stdio_args(forwarded_args))
        }
        value if value == OsStr::new(BOT_MEDIA_MCP_RUN_MODE_ARG) => {
            bot_bridge::run_bot_media_mcp_stdio()
        }
        value if value == OsStr::new(CLAUDE_CODE_APP_SERVER_RUN_MODE_ARG) => {
            claude_code_app_server::run_stdio_app_server(forwarded_args)
        }
        value if value == OsStr::new(CLAUDE_CODE_MCP_METADATA_RELAY_RUN_MODE_ARG) => {
            claude_code_app_server::run_mcp_metadata_relay(forwarded_args)
        }
        _ => return false,
    };

    let exit_code = match exit_code {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err);
            1
        }
    };
    std::process::exit(exit_code);
}

#[cfg(windows)]
fn program_is_windows_middleware_shim(program: Option<&OsStr>) -> bool {
    program
        .and_then(|program| Path::new(program).file_name())
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name == MIDDLEWARE_FILE_NAME
                || (name.starts_with(MIDDLEWARE_FILE_STEM) && name.ends_with(".exe"))
        })
}

fn resolve_real_cli_path(
    codex_app_executable: &str,
    middleware_path: &Path,
) -> Result<PathBuf, String> {
    for key in [REAL_CLI_ENV, BUNDLED_REAL_CLI_ENV] {
        let explicit_real_cli = std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(path) = explicit_real_cli {
            return prepare_real_cli_path(expand_home_path(&path));
        }
    }

    let inherited_cli = std::env::var(CODEX_CLI_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))
        .filter(|path| !same_path(path, middleware_path));
    if let Some(path) = inherited_cli {
        return prepare_real_cli_path(path);
    }

    bundled_cli_path(codex_app_executable)
        .ok_or_else(|| {
            format!(
                "Could not resolve bundled Codex CLI from ChatGPT app executable: {}",
                codex_app_executable
            )
        })
        .and_then(prepare_real_cli_path)
}

fn resolve_runtime_real_cli_path() -> Result<PathBuf, String> {
    let mut invalid_candidates = Vec::new();
    for (source, value) in [
        (REAL_CLI_ENV, std::env::var(REAL_CLI_ENV).ok()),
        (
            BUNDLED_REAL_CLI_ENV,
            std::env::var(BUNDLED_REAL_CLI_ENV).ok(),
        ),
        (CODEX_CLI_PATH_ENV, std::env::var(CODEX_CLI_PATH_ENV).ok()),
        (
            "cached real CLI path",
            std::fs::read_to_string(real_cli_cache_path()).ok(),
        ),
    ] {
        let Some(value) = value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let path = expand_home_path(&value);
        if cli_path_is_middleware(&path) {
            continue;
        }
        match prepare_real_cli_path(path) {
            Ok(path) => return Ok(path),
            Err(error) => invalid_candidates.push(format!("{}: {}", source, error)),
        }
    }

    if invalid_candidates.is_empty() {
        Err(format!(
            "Could not resolve the real Codex CLI; {} and {} are not set and the cached path is missing",
            REAL_CLI_ENV, BUNDLED_REAL_CLI_ENV
        ))
    } else {
        Err(format!(
            "Could not resolve the real Codex CLI ({})",
            invalid_candidates.join("; ")
        ))
    }
}

fn cli_path_is_middleware(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(|name| {
            name.to_ascii_lowercase()
                .starts_with("codexl-codex-cli-middleware")
        })
        .unwrap_or(false)
}

fn prepare_real_cli_path(path: PathBuf) -> Result<PathBuf, String> {
    let path = validate_cli_path(path)?;
    materialize_windows_appx_cli(path)
}

fn validate_cli_path(path: PathBuf) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!(
            "Resolved Codex CLI path does not exist: {}",
            path.to_string_lossy()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "Resolved Codex CLI path is not executable: {}",
                path.to_string_lossy()
            ));
        }
    }

    Ok(path)
}

#[cfg(windows)]
fn materialize_windows_appx_cli(path: PathBuf) -> Result<PathBuf, String> {
    if !is_windows_appx_path(&path) {
        return Ok(path);
    }

    let package_slug = windows_appx_package_slug(&path).unwrap_or_else(|| "appx".to_string());
    let target = codexl_home_dir()
        .join("bin")
        .join(format!("codexl-real-codex-{}.exe", package_slug));
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if should_copy_middleware_exe(&target, &path) {
        std::fs::copy(&path, &target).map_err(|e| {
            format!(
                "Failed to copy Windows Store Codex CLI from {} to {}: {}",
                path.to_string_lossy(),
                target.to_string_lossy(),
                e
            )
        })?;
    }
    validate_cli_path(target)
}

#[cfg(not(windows))]
fn materialize_windows_appx_cli(path: PathBuf) -> Result<PathBuf, String> {
    Ok(path)
}

#[cfg(windows)]
fn is_windows_appx_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("WindowsApps"))
    })
}

#[cfg(windows)]
fn windows_appx_package_slug(path: &Path) -> Option<String> {
    let mut components = path.components();
    while let Some(component) = components.next() {
        if component
            .as_os_str()
            .to_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("WindowsApps"))
        {
            return components
                .next()
                .and_then(|package| package.as_os_str().to_str())
                .map(slugify_file_segment);
        }
    }
    None
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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

fn middleware_path() -> PathBuf {
    codexl_home_dir()
        .join("bin")
        .join(windows_versioned_middleware_file_name())
}

fn real_cli_cache_path() -> PathBuf {
    codexl_home_dir().join("bin").join("real-codex-cli-path")
}

fn write_real_cli_cache(real_cli_path: &Path) -> Result<(), String> {
    let path = real_cli_cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = real_cli_path.to_string_lossy();
    let should_write = std::fs::read_to_string(&path)
        .map(|existing| existing.trim() != content)
        .unwrap_or(true);
    if should_write {
        std::fs::write(path, content.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn windows_versioned_middleware_file_name() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return MIDDLEWARE_FILE_NAME.to_string();
    };
    let Ok(metadata) = std::fs::metadata(&exe) else {
        return MIDDLEWARE_FILE_NAME.to_string();
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();

    format!(
        "{}-{}-{}.exe",
        MIDDLEWARE_FILE_STEM,
        metadata.len(),
        modified
    )
}

#[cfg(not(windows))]
fn windows_versioned_middleware_file_name() -> String {
    MIDDLEWARE_FILE_NAME.to_string()
}

fn stdio_path(name: Option<&str>) -> PathBuf {
    let file_name = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let slug = slugify_file_segment(value);
            if cfg!(windows) {
                format!("codexl-codex-cli-stdio-{}.cmd", slug)
            } else {
                format!("codexl-codex-cli-stdio-{}", slug)
            }
        })
        .unwrap_or_else(|| STDIO_FILE_NAME.to_string());
    codexl_home_dir().join("bin").join(file_name)
}

fn default_log_path() -> PathBuf {
    std::env::var(MIDDLEWARE_LOG_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))
        .unwrap_or_else(|| codexl_home_dir().join("codex-cli-middleware.log"))
}

fn expand_home_path(path: &str) -> PathBuf {
    let trimmed = path.trim();
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

fn normalize_profile(profile: Option<&str>) -> Option<String> {
    profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_model_provider(model_provider: Option<&str>, profile: Option<&str>) -> Option<String> {
    let model_provider = normalize_profile(model_provider)?;
    let profile = profile.map(str::trim).filter(|value| !value.is_empty());

    if model_provider.eq_ignore_ascii_case("default")
        && profile
            .map(|value| value.eq_ignore_ascii_case(crate::config::DEFAULT_PROVIDER_PROFILE_NAME))
            .unwrap_or(true)
    {
        return None;
    }

    Some(model_provider)
}

fn codexl_home_dir() -> PathBuf {
    std::env::var("CODEXL_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                if let Some(local_app_data) = env_path_without_home_expansion("LOCALAPPDATA") {
                    return local_app_data.join("CodexL");
                }
                if let Some(app_data) = env_path_without_home_expansion("APPDATA") {
                    return app_data.join("CodexL");
                }
            }
            user_home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codexl")
        })
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

fn write_middleware(
    path: &Path,
    host_executable: &Path,
    real_cli_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    #[cfg(windows)]
    {
        if same_path(path, host_executable) {
            return Ok(());
        }
        if should_copy_middleware_exe(path, host_executable) {
            std::fs::copy(host_executable, path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let content = middleware_script(host_executable, real_cli_path);
        let should_write = std::fs::read_to_string(path)
            .map(|existing| existing != content)
            .unwrap_or(true);
        if should_write {
            std::fs::write(path, content).map_err(|e| e.to_string())?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path)
                .map_err(|e| e.to_string())?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

#[cfg(windows)]
fn should_copy_middleware_exe(path: &Path, host_executable: &Path) -> bool {
    let Ok(source) = std::fs::metadata(host_executable) else {
        return false;
    };
    let Ok(target) = std::fs::metadata(path) else {
        return true;
    };
    if source.len() != target.len() {
        return true;
    }
    match (source.modified(), target.modified()) {
        (Ok(source_modified), Ok(target_modified)) => source_modified > target_modified,
        _ => false,
    }
}

fn write_stdio_export(
    path: &Path,
    host_executable: &Path,
    middleware_path: &Path,
    real_cli_path: &Path,
    log_path: &Path,
    codex_home: Option<&str>,
    workspace_name: Option<&str>,
    profile: Option<&str>,
    model_provider: Option<&str>,
    core_mode: Option<&str>,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let content = stdio_export_script(
        host_executable,
        middleware_path,
        real_cli_path,
        log_path,
        codex_home,
        workspace_name,
        profile,
        model_provider,
        core_mode,
        profile_config_format,
    );
    let should_write = std::fs::read_to_string(path)
        .map(|existing| existing != content)
        .unwrap_or(true);
    if should_write {
        std::fs::write(path, content).map_err(|e| e.to_string())?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn run_stdio_middleware(args: Vec<OsString>) -> Result<i32, String> {
    run_stdio_middleware_with_io(args, std::io::stdin(), std::io::stdout())
}

fn external_stdio_args(args: Vec<OsString>) -> Vec<OsString> {
    if args.is_empty() {
        vec![
            OsString::from("app-server"),
            OsString::from("--analytics-default-enabled"),
        ]
    } else {
        args
    }
}

fn real_cli_command(real_cli: &Path, real_args: &[OsString]) -> Command {
    #[cfg(target_os = "macos")]
    if let Some(node) = signed_supervisor_node(real_cli) {
        // The packaged Browser authorizer checks the connecting process plus its parent and
        // grandparent. Keep the protocol middleware while putting a signed OpenAI runtime between
        // CodexL and the main app-server, so node_repl's grandparent is signed `node`, not CodexL.
        let mut command = Command::new(node);
        command
            .arg("-e")
            .arg(SIGNED_SUPERVISOR_SOURCE)
            .arg(real_cli)
            .args(real_args);
        return command;
    }

    let mut command = Command::new(real_cli);
    command.args(real_args);
    command
}

#[cfg(target_os = "macos")]
fn signed_supervisor_node(real_cli: &Path) -> Option<PathBuf> {
    if let Some(path) =
        env_path_without_home_expansion(SIGNED_SUPERVISOR_NODE_ENV).filter(|path| path.is_file())
    {
        return Some(path);
    }

    real_cli
        .parent()
        .map(|resources| resources.join("cua_node").join("bin").join("node"))
        .filter(|path| path.is_file())
}

fn run_stdio_middleware_with_io<R, W>(
    args: Vec<OsString>,
    input: R,
    output: W,
) -> Result<i32, String>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    if should_run_claude_code_app_server(&args) {
        return claude_code_app_server::run_stdio_app_server_with_io(
            claude_code_app_server_args(),
            input,
            output,
        );
    }

    let real_cli = resolve_runtime_real_cli_path()?;

    let profile = std::env::var(CODEX_PROFILE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let raw_model_provider = std::env::var(CODEX_MODEL_PROVIDER_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let model_provider =
        normalize_model_provider(raw_model_provider.as_deref(), profile.as_deref());
    let profile_config_format = crate::config::codex_profile_config_format_from_env()
        .unwrap_or_else(|| {
            crate::config::codex_profile_config_format_for_cli(&real_cli.to_string_lossy())
        });
    let real_args = real_cli_args(
        profile.as_deref(),
        model_provider.as_deref(),
        profile_config_format,
        args,
    );
    log_invocation(
        &real_cli,
        profile.as_deref(),
        model_provider.as_deref(),
        &real_args,
    );

    let mut command = real_cli_command(&real_cli, &real_args);
    let mut child = command
        .env_remove(CODEX_CLI_PATH_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Failed to launch real Codex CLI: {}", e))?;

    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open real Codex CLI stdin".to_string())?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to open real Codex CLI stdout".to_string())?;

    let request_map = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let chatgpt_auth = ChatGptAuth::load();
    let shared_child_stdin = bot_bridge::shared_app_stdin(child_stdin);
    let shared_output = Arc::new(Mutex::new(output));
    let current_cwd = Arc::new(Mutex::new(None));
    let bridge_stdout_tx = bot_bridge::spawn_app_stdio_bot_bridge(Arc::clone(&shared_child_stdin));
    let stdin_request_map = Arc::clone(&request_map);
    let stdout_request_map = Arc::clone(&request_map);
    let stdin_output = Arc::clone(&shared_output);
    let stdout_output = Arc::clone(&shared_output);
    let stdin_current_cwd = Arc::clone(&current_cwd);
    let _stdin_handle = thread::spawn(move || {
        copy_stdin_and_track(
            input,
            shared_child_stdin,
            stdin_request_map,
            stdin_output,
            stdin_current_cwd,
        )
    });
    let stdout_handle = thread::spawn(move || {
        copy_stdout_and_rewrite(
            child_stdout,
            stdout_output,
            stdout_request_map,
            chatgpt_auth,
            bridge_stdout_tx,
        )
    });

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for real Codex CLI: {}", e))?;

    let _ = stdout_handle
        .join()
        .map_err(|_| "stdout forwarding thread panicked".to_string())?
        .map_err(|e| e.to_string())?;

    Ok(status.code().unwrap_or(1))
}

fn should_run_claude_code_app_server(args: &[OsString]) -> bool {
    let core_mode = std::env::var(CODEX_CORE_MODE_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(
        core_mode.as_str(),
        crate::config::REMOTE_FRONTEND_MODE_CLAUDE_CODE | "claude_code" | "claude code"
    ) {
        return false;
    }
    args.first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| arg == "app-server")
}

fn claude_code_app_server_args() -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(workspace_name) = std::env::var(CODEX_WORKSPACE_NAME_ENV)
        .ok()
        .or_else(|| std::env::var(LEGACY_CODEX_INSTANCE_NAME_ENV).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--workspace-name"));
        args.push(OsString::from(workspace_name));
    }
    args
}

pub(crate) fn real_cli_args(
    profile: Option<&str>,
    model_provider: Option<&str>,
    profile_config_format: CodexProfileConfigFormat,
    args: Vec<OsString>,
) -> Vec<OsString> {
    let mut real_args = Vec::new();
    if let Some(profile) = profile {
        match profile_config_format {
            CodexProfileConfigFormat::SeparateProfileFiles
                if codex_args_accept_profile_flag(&args) =>
            {
                real_args.push(OsString::from("--profile"));
                real_args.push(OsString::from(profile));
            }
            CodexProfileConfigFormat::SeparateProfileFiles => {}
            CodexProfileConfigFormat::LegacyProfilesTable => {
                real_args.push(OsString::from("-c"));
                real_args.push(OsString::from(cli_config_string("profile", profile)));
            }
        }
    }
    if let Some(model_provider) = model_provider {
        if let Some(model_provider) = normalize_model_provider(Some(model_provider), profile) {
            real_args.push(OsString::from("-c"));
            real_args.push(OsString::from(cli_config_string(
                "model_provider",
                &model_provider,
            )));
        }
    }
    real_args.extend(args);
    real_args
}

pub(crate) fn codex_args_accept_profile_flag(args: &[OsString]) -> bool {
    let positionals = codex_positional_args(args);
    let Some(command) = positionals.first().map(String::as_str) else {
        return true;
    };

    match command {
        "exec" | "e" | "review" | "resume" | "fork" | "mcp" | "sandbox" => true,
        "debug" => positionals
            .get(1)
            .is_some_and(|subcommand| subcommand == "prompt-input"),
        "login" | "logout" | "plugin" | "mcp-server" | "app-server" | "remote-control" | "app"
        | "completion" | "update" | "doctor" | "apply" | "a" | "cloud" | "exec-server"
        | "features" | "help" => false,
        _ => true,
    }
}

fn codex_positional_args(args: &[OsString]) -> Vec<String> {
    let mut positionals = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        let Some(arg) = arg.to_str() else {
            continue;
        };
        if arg == "--" {
            break;
        }
        if codex_option_takes_value(arg) {
            if !arg.contains('=') {
                skip_next = true;
            }
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }

        positionals.push(arg.to_string());
        if positionals.len() >= 2 {
            break;
        }
    }

    positionals
}

fn codex_option_takes_value(arg: &str) -> bool {
    let option = arg.split_once('=').map(|(name, _)| name).unwrap_or(arg);
    matches!(
        option,
        "-c" | "--config"
            | "--enable"
            | "--disable"
            | "--remote"
            | "--remote-auth-token-env"
            | "-i"
            | "--image"
            | "-m"
            | "--model"
            | "--local-provider"
            | "-p"
            | "--profile"
            | "-s"
            | "--sandbox"
            | "-C"
            | "--cd"
            | "--add-dir"
            | "-a"
            | "--ask-for-approval"
    )
}

fn cli_config_string(key: &str, value: &str) -> String {
    format!("{}=\"{}\"", key, toml_string_escape(value))
}

fn toml_string_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn open_log_file_from_env(env_name: &str) -> Option<File> {
    let path = std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))?;

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

fn copy_stdin_and_track<R, W>(
    reader: R,
    writer: SharedChildStdin,
    request_map: RequestMap,
    output: SharedOutput<W>,
    current_cwd: SharedCurrentCwd,
) -> std::io::Result<u64>
where
    R: Read,
    W: Write,
{
    let mut copied = 0;
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();
        let size = reader.read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }

        if let Some(response) = custom_session_response_for_app_server_line(&line) {
            write_json_line_to_output(&output, &response, line_ending(&line))?;
            copied += size as u64;
            continue;
        }

        if let Some(response) = custom_transcribe_response_for_app_server_line(&line) {
            write_json_line_to_output(&output, &response, line_ending(&line))?;
            copied += size as u64;
            continue;
        }

        track_request_line_with_workspace(&line, &request_map, Some(&current_cwd));
        let mut writer = writer
            .lock()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "stdin mutex poisoned"))?;
        writer.write_all(&line)?;
        writer.flush()?;
        copied += size as u64;
    }

    Ok(copied)
}

fn copy_stdout_and_rewrite<R, W>(
    reader: R,
    writer: SharedOutput<W>,
    request_map: RequestMap,
    chatgpt_auth: ChatGptAuth,
    bridge_stdout_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
) -> std::io::Result<u64>
where
    R: Read,
    W: Write,
{
    let mut copied = 0;
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();
        let size = reader.read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }

        let rewritten = rewrite_stdout_line(&line, &request_map, &chatgpt_auth);
        let suppress_for_app = bot_bridge::should_intercept_app_server_line(&rewritten);
        if let Some(tx) = bridge_stdout_tx.as_ref() {
            let _ = tx.send(rewritten.clone());
        }
        if !suppress_for_app {
            let mut writer = writer.lock().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::Other, "stdout mutex poisoned")
            })?;
            writer.write_all(&rewritten)?;
            writer.flush()?;
        }
        copied += size as u64;
    }

    Ok(copied)
}

fn custom_transcribe_response_for_app_server_line(line: &[u8]) -> Option<Value> {
    let message = app_server_fetch_message_from_line(line)?;
    let request_id = message
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let config = AppConfig::load();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let response = runtime.block_on(remote::custom_transcribe_fetch_response_for_config(
        &message,
        &config,
        "desktop-app-server",
    ))?;
    eprintln!(
        "[codexl-cli] intercepted desktop transcribe requestId={}",
        request_id
    );
    Some(response)
}

fn custom_session_response_for_app_server_line(line: &[u8]) -> Option<Value> {
    if !show_all_sessions_enabled() {
        return None;
    }

    let request = serde_json::from_slice::<Value>(trim_json_line(line)).ok()?;
    let method = request.get("method").and_then(Value::as_str)?;
    if !matches!(
        method,
        "thread/read" | "thread/resume" | "thread/turns/list"
    ) {
        return None;
    }

    let session = foreign_session_file_for_request(&request)?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let response = match method {
        "thread/read" => thread_read_response(id, &session, &params)?,
        "thread/resume" => thread_resume_response(id, &session)?,
        "thread/turns/list" => thread_turns_list_response(id, &session)?,
        _ => return None,
    };
    Some(response)
}

fn app_server_fetch_message_from_line(line: &[u8]) -> Option<Value> {
    let value = serde_json::from_slice::<Value>(trim_json_line(line)).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("fetch") {
        return None;
    }
    if !value
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method.eq_ignore_ascii_case("POST"))
    {
        return None;
    }
    if !value
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(app_server_fetch_url_is_transcribe)
    {
        return None;
    }
    Some(value)
}

fn app_server_fetch_url_is_transcribe(url: &str) -> bool {
    let url = url.trim();
    if url == "/transcribe" {
        return true;
    }
    reqwest::Url::parse(url)
        .map(|url| url.path() == "/transcribe" && matches!(url.scheme(), "app" | "http" | "https"))
        .unwrap_or(false)
}

fn write_json_line_to_output<W: Write>(
    output: &SharedOutput<W>,
    value: &Value,
    ending: &[u8],
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.extend_from_slice(ending);
    let mut writer = output
        .lock()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "stdout mutex poisoned"))?;
    writer.write_all(&line)?;
    writer.flush()
}

fn track_request_line_with_workspace(
    line: &[u8],
    request_map: &RequestMap,
    current_cwd: Option<&SharedCurrentCwd>,
) {
    let Ok(value) = serde_json::from_slice::<Value>(trim_json_line(line)) else {
        return;
    };
    let Some(id) = value.get("id").and_then(Value::as_str) else {
        return;
    };
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };

    update_current_cwd_from_request(&value, method, current_cwd);

    if !matches!(
        method,
        "account/read" | "getAuthStatus" | "thread/list" | "config/read" | "model/list"
    ) {
        return;
    }

    let include_token = value
        .get("params")
        .and_then(|params| params.get("includeToken"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Ok(mut request_map) = request_map.lock() {
        request_map.insert(
            id.to_string(),
            RequestInfo {
                method: method.to_string(),
                include_token,
                params: request_params_with_workspace_fallback(&value, method, current_cwd),
            },
        );
    }
}

fn update_current_cwd_from_request(
    value: &Value,
    method: &str,
    current_cwd: Option<&SharedCurrentCwd>,
) {
    let Some(current_cwd) = current_cwd else {
        return;
    };
    let Some(cwd) = request_workspace_cwd(value, method) else {
        return;
    };
    if let Ok(mut current) = current_cwd.lock() {
        *current = Some(cwd);
    }
}

fn request_params_with_workspace_fallback(
    value: &Value,
    method: &str,
    current_cwd: Option<&SharedCurrentCwd>,
) -> Value {
    let mut params = value.get("params").cloned().unwrap_or(Value::Null);
    if method != "thread/list" || list_cwd_filter(&params).is_some() {
        return params;
    }
    let Some(cwd) = current_cwd
        .and_then(|current| current.lock().ok().and_then(|value| value.clone()))
        .filter(|value| !value.trim().is_empty())
    else {
        return params;
    };

    if let Some(object) = params.as_object_mut() {
        object.insert(CODEXL_WORKSPACE_CWD_FILTER_KEY.to_string(), json!(cwd));
        params
    } else {
        json!({ CODEXL_WORKSPACE_CWD_FILTER_KEY: cwd })
    }
}

fn request_workspace_cwd(value: &Value, method: &str) -> Option<String> {
    let params = value.get("params")?;
    match method {
        "config/read" | "thread/resume" | "turn/start" => params
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        "hooks/list" => {
            let cwds = params.get("cwds").and_then(Value::as_array)?;
            let values = cwds
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if values.len() == 1 {
                Some(values[0].to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn rewrite_stdout_line(
    line: &[u8],
    request_map: &RequestMap,
    chatgpt_auth: &ChatGptAuth,
) -> Vec<u8> {
    let trimmed = trim_json_line(line);
    let Ok(mut value) = serde_json::from_slice::<Value>(trimmed) else {
        return line.to_vec();
    };
    let Some(id) = value.get("id").and_then(Value::as_str) else {
        return line.to_vec();
    };
    let Some(request) = request_map
        .lock()
        .ok()
        .and_then(|mut request_map| request_map.remove(id))
    else {
        return line.to_vec();
    };

    if value.get("error").is_some() {
        log_protocol_response_summary(&request, &value);
        return line.to_vec();
    }

    log_protocol_response_summary(&request, &value);

    match request.method.as_str() {
        "account/read" => value["result"] = chatgpt_auth.account_read_result(),
        "getAuthStatus" => value["result"] = chatgpt_auth.auth_status_result(request.include_token),
        "thread/list" => rewrite_thread_list_response(&mut value, &request.params),
        _ => return line.to_vec(),
    }

    let Ok(mut rewritten) = serde_json::to_vec(&value) else {
        return line.to_vec();
    };
    rewritten.extend_from_slice(line_ending(line));
    rewritten
}

fn log_protocol_response_summary(request: &RequestInfo, response: &Value) {
    let Some(summary) = protocol_response_summary(request, response) else {
        return;
    };
    let Some(mut log) = open_log_file_from_env(MIDDLEWARE_LOG_ENV) else {
        return;
    };
    let _ = writeln!(log, "[{}] {}", timestamp_seconds(), summary);
}

fn protocol_response_summary(request: &RequestInfo, response: &Value) -> Option<String> {
    if let Some(error) = response.get("error") {
        let code = error
            .get("code")
            .map(json_log_scalar)
            .unwrap_or_else(|| "unknown".to_string());
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(log_preview)
            .unwrap_or_default();
        return Some(format!(
            "response method={} error_code={} error_message={}",
            request.method, code, message
        ));
    }

    match request.method.as_str() {
        "config/read" => Some(config_read_response_summary(request, response)),
        "model/list" => Some(model_list_response_summary(response)),
        _ => None,
    }
}

fn config_read_response_summary(request: &RequestInfo, response: &Value) -> String {
    let result = response.get("result").unwrap_or(&Value::Null);
    let config = result.get("config").unwrap_or(result);
    format!(
        "response method=config/read model={} model_provider={} profile={} model_catalog_json={} cwd={}",
        json_path_string(config, &["model"]).unwrap_or_default(),
        json_path_string(config, &["model_provider"]).unwrap_or_default(),
        json_path_string(config, &["profile"]).unwrap_or_default(),
        json_path_string(config, &["model_catalog_json"]).unwrap_or_default(),
        json_path_string(&request.params, &["cwd"]).unwrap_or_default()
    )
}

fn model_list_response_summary(response: &Value) -> String {
    let result = response.get("result").unwrap_or(&Value::Null);
    let models = model_list_array(result);
    let count = models.map(|models| models.len()).unwrap_or(0);
    let hidden_count = models
        .map(|models| {
            models
                .iter()
                .filter(|model| model.get("hidden").and_then(Value::as_bool) == Some(true))
                .count()
        })
        .unwrap_or(0);
    let first = models
        .map(|models| {
            models
                .iter()
                .take(5)
                .filter_map(model_display_id)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let default_model = [
        "default_model",
        "defaultModel",
        "default",
        "selected_model",
        "selectedModel",
    ]
    .iter()
    .find_map(|key| json_path_string(result, &[*key]))
    .unwrap_or_default();
    format!(
        "response method=model/list count={} hidden_count={} default_model={} first_models={}",
        count, hidden_count, default_model, first
    )
}

fn model_list_array(result: &Value) -> Option<&Vec<Value>> {
    result
        .get("models")
        .and_then(Value::as_array)
        .or_else(|| result.get("data").and_then(Value::as_array))
        .or_else(|| result.get("items").and_then(Value::as_array))
        .or_else(|| result.as_array())
}

fn model_display_id(model: &Value) -> Option<String> {
    ["model", "id", "slug", "name", "label"]
        .iter()
        .find_map(|key| model.get(*key).and_then(Value::as_str))
        .map(log_preview)
}

fn json_path_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::String(value) => Some(log_preview(value)),
        Value::Null => None,
        other => Some(json_log_scalar(other)),
    }
}

fn json_log_scalar(value: &Value) -> String {
    match value {
        Value::String(value) => log_preview(value),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        Value::Array(values) => format!("array:{}", values.len()),
        Value::Object(values) => format!("object:{}", values.len()),
    }
}

fn log_preview(value: &str) -> String {
    let mut chars = value.chars();
    let preview = chars.by_ref().take(120).collect::<String>();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

fn rewrite_thread_list_response(value: &mut Value, params: &Value) {
    if !show_all_sessions_enabled() {
        return;
    }
    let homes = session_homes_for_all_providers();
    if homes.is_empty() {
        return;
    }
    merge_session_files_into_thread_list(value, &homes, params);
}

fn show_all_sessions_enabled() -> bool {
    remote::cdp_resources::renderer_core_plugin_bool_setting(
        remote::cdp_resources::SHOW_ALL_SESSIONS_KEY,
    )
}

fn session_homes_for_all_providers() -> Vec<SessionHome> {
    let config = AppConfig::load();
    let mut homes = Vec::new();
    let mut seen = HashSet::new();
    let current_home_key = current_codex_home_path().map(|path| session_home_key(&path));

    if !config.codex_home.trim().is_empty() {
        push_session_home(
            &mut homes,
            &mut seen,
            &current_home_key,
            "Active".to_string(),
            expand_home_path(&config.codex_home),
        );
    }

    for profile in &config.codex_home_profiles {
        let path = profile.path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        push_session_home(
            &mut homes,
            &mut seen,
            &current_home_key,
            profile.name.clone(),
            expand_home_path(&path),
        );
    }

    for profile in config.provider_profiles {
        let path = profile.codex_home.trim().to_string();
        if path.is_empty() {
            continue;
        }
        push_session_home(
            &mut homes,
            &mut seen,
            &current_home_key,
            profile.name.clone(),
            expand_home_path(&path),
        );
    }

    for path in discovered_codex_home_dirs() {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Codex Home")
            .to_string();
        push_session_home(&mut homes, &mut seen, &current_home_key, name, path);
    }

    homes
}

fn current_codex_home_path() -> Option<PathBuf> {
    std::env::var(CODEX_HOME_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))
}

fn push_session_home(
    homes: &mut Vec<SessionHome>,
    seen: &mut HashSet<String>,
    current_home_key: &Option<String>,
    profile_name: String,
    path: PathBuf,
) {
    let key = session_home_key(&path);
    if current_home_key
        .as_ref()
        .is_some_and(|current| current == &key)
    {
        return;
    }
    if !seen.insert(key) {
        return;
    }
    homes.push(SessionHome { profile_name, path });
}

fn discovered_codex_home_dirs() -> Vec<PathBuf> {
    let root = codexl_home_dir().join("codex-homes");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("sessions").is_dir())
        .collect()
}

fn session_home_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn merge_session_files_into_thread_list(value: &mut Value, homes: &[SessionHome], params: &Value) {
    if let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) {
        result
            .entry("nextCursor".to_string())
            .or_insert(Value::Null);
        result
            .entry("backwardsCursor".to_string())
            .or_insert(Value::Null);
    }

    let Some(threads) = value
        .get_mut("result")
        .and_then(|result| result.get_mut("data"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };

    let mut seen_ids = threads
        .iter()
        .filter_map(|thread| thread.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<HashSet<_>>();

    for session in session_files_from_homes(homes, params) {
        let Some(thread) = thread_summary_from_session_file(&session.path, &session.profile_name)
        else {
            continue;
        };
        if !thread_matches_list_params(&thread, params) {
            continue;
        }
        let Some(id) = thread.get("id").and_then(Value::as_str) else {
            continue;
        };
        if seen_ids.insert(id.to_string()) {
            threads.push(thread);
        }
    }

    sort_thread_list(threads, params);
}

fn thread_list_per_home_session_limit(params: &Value) -> usize {
    let requested = params
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(50);
    requested.min(100)
}

fn foreign_session_file_for_request(request: &Value) -> Option<SessionFile> {
    let params = request.get("params").unwrap_or(&Value::Null);
    let requested_path = params
        .get("path")
        .or_else(|| params.get("rolloutPath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home_path);
    let thread_id = params
        .get("threadId")
        .or_else(|| params.get("conversationId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let current_home_key = current_codex_home_path().map(|path| session_home_key(&path));

    for home in session_homes_for_all_providers() {
        if current_home_key
            .as_ref()
            .is_some_and(|key| *key == session_home_key(&home.path))
        {
            continue;
        }

        if let Some(path) = requested_path.as_ref() {
            if path_is_under(path, &home.path) && path.is_file() {
                return Some(SessionFile {
                    profile_name: home.profile_name,
                    path: path.clone(),
                });
            }
        }

        let Some(thread_id) = thread_id.as_deref() else {
            continue;
        };
        if let Some(path) = session_file_for_thread_id(&home.path, thread_id) {
            return Some(SessionFile {
                profile_name: home.profile_name,
                path,
            });
        }
    }

    None
}

fn path_is_under(path: &Path, root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(root).is_ok()
}

fn session_file_for_thread_id(codex_home: &Path, thread_id: &str) -> Option<PathBuf> {
    let sessions_dir = codex_home.join("sessions");
    let mut files = Vec::new();
    collect_session_jsonl_files(&sessions_dir, &mut files);
    files.into_iter().find(|path| {
        session_meta_payload(path)
            .and_then(|payload| {
                payload
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .is_some_and(|id| id == thread_id)
    })
}

fn thread_read_response(id: Value, session: &SessionFile, params: &Value) -> Option<Value> {
    let include_turns = params
        .get("includeTurns")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut thread = thread_summary_from_session_file(&session.path, &session.profile_name)?;
    if include_turns {
        add_session_turns_to_thread(&mut thread, &session.path);
    }
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "thread": thread,
        },
    }))
}

fn thread_resume_response(id: Value, session: &SessionFile) -> Option<Value> {
    let mut thread = thread_summary_from_session_file(&session.path, &session.profile_name)?;
    add_session_turns_to_thread(&mut thread, &session.path);
    let payload = session_meta_payload(&session.path)?;
    let model = session_model_from_payload(&payload);
    let model_provider = session_model_provider_from_payload(&payload);
    let cwd = thread.get("cwd").cloned().unwrap_or_else(|| json!("/"));
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "thread": thread,
            "model": model,
            "modelProvider": model_provider,
            "serviceTier": Value::Null,
            "cwd": cwd,
            "instructionSources": [],
            "reasoningEffort": Value::Null,
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "sandbox": {
                "type": "workspaceWrite",
                "writableRoots": [],
                "networkAccess": false,
                "excludeTmpdirEnvVar": false,
                "excludeSlashTmp": false,
            },
        },
    }))
}

fn add_session_turns_to_thread(thread: &mut Value, path: &Path) {
    if let Value::Object(map) = thread {
        map.insert(
            "turns".to_string(),
            Value::Array(thread_turns_from_session_file(path)),
        );
    }
}

fn thread_turns_list_response(id: Value, session: &SessionFile) -> Option<Value> {
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "data": thread_turns_from_session_file(&session.path),
            "nextCursor": Value::Null,
            "backwardsCursor": Value::Null,
        },
    }))
}

fn session_files_from_homes(homes: &[SessionHome], params: &Value) -> Vec<SessionFile> {
    let mut files = Vec::new();
    let per_home_limit = thread_list_per_home_session_limit(params);
    let take_limit = if has_thread_workspace_filter(params) {
        usize::MAX
    } else {
        per_home_limit
    };
    for home in homes {
        let mut paths = Vec::new();
        collect_session_jsonl_files(&home.path.join("sessions"), &mut paths);
        paths.sort_by(|first, second| {
            let first_updated = session_file_updated_at(first);
            let second_updated = session_file_updated_at(second);
            second_updated.cmp(&first_updated)
        });
        files.extend(paths.into_iter().take(take_limit).map(|path| SessionFile {
            profile_name: home.profile_name.clone(),
            path,
        }));
    }
    files.sort_by(|first, second| {
        let first_updated = session_file_updated_at(&first.path);
        let second_updated = session_file_updated_at(&second.path);
        second_updated.cmp(&first_updated)
    });
    files
}

fn thread_matches_list_params(thread: &Value, params: &Value) -> bool {
    if params
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }

    if let Some(cwd_filter) = list_cwd_filter(params) {
        let cwd = thread.get("cwd").and_then(Value::as_str).unwrap_or("");
        if !cwd_filter.iter().any(|candidate| candidate == cwd) {
            return false;
        }
    } else if let Some(cwd_filter) = list_inherited_workspace_cwd_filter(params) {
        let cwd = thread.get("cwd").and_then(Value::as_str).unwrap_or("");
        if !cwd_filter.iter().any(|candidate| candidate == cwd) && !is_projectless_cwd(cwd) {
            return false;
        }
    }

    if let Some(providers) = non_empty_string_array(params.get("modelProviders")) {
        let provider = thread
            .get("modelProvider")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !providers.iter().any(|candidate| candidate == provider) {
            return false;
        }
    }

    if let Some(sources) = non_empty_string_array(params.get("sourceKinds")) {
        let source = thread.get("source").and_then(Value::as_str).unwrap_or("");
        if !sources.iter().any(|candidate| candidate == source) {
            return false;
        }
    }

    if let Some(search_term) = params
        .get("searchTerm")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
    {
        let haystack = [
            thread.get("id").and_then(Value::as_str).unwrap_or(""),
            thread.get("name").and_then(Value::as_str).unwrap_or(""),
            thread.get("preview").and_then(Value::as_str).unwrap_or(""),
        ]
        .join("\n")
        .to_ascii_lowercase();
        if !haystack.contains(&search_term) {
            return false;
        }
    }

    true
}

fn has_thread_workspace_filter(params: &Value) -> bool {
    list_cwd_filter(params).is_some() || list_inherited_workspace_cwd_filter(params).is_some()
}

fn list_cwd_filter(params: &Value) -> Option<Vec<String>> {
    match params.get("cwd")? {
        Value::String(cwd) if !cwd.trim().is_empty() => Some(vec![cwd.to_string()]),
        Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(values)
        }
        _ => None,
    }
}

fn list_inherited_workspace_cwd_filter(params: &Value) -> Option<Vec<String>> {
    let cwd = params
        .get(CODEXL_WORKSPACE_CWD_FILTER_KEY)?
        .as_str()?
        .trim();
    (!cwd.is_empty()).then(|| vec![cwd.to_string()])
}

fn is_projectless_cwd(cwd: &str) -> bool {
    let Some(workspace_root) = projectless_workspace_root() else {
        return false;
    };
    is_projectless_cwd_under_root(Path::new(cwd.trim()), &workspace_root)
}

fn projectless_workspace_root() -> Option<PathBuf> {
    user_home_dir().map(|home| home.join("Documents").join("Codex"))
}

fn is_projectless_cwd_under_root(cwd: &Path, workspace_root: &Path) -> bool {
    let Ok(relative) = cwd.strip_prefix(workspace_root) else {
        return false;
    };
    let segments = relative
        .iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [date_slug] => is_projectless_date_slug_segment(date_slug),
        [date, slug] => is_projectless_date_segment(date) && !slug.trim().is_empty(),
        _ => false,
    }
}

fn is_projectless_date_slug_segment(segment: &str) -> bool {
    let Some(date) = segment.get(..10) else {
        return false;
    };
    segment.as_bytes().get(10).is_some_and(|byte| *byte == b'-')
        && is_projectless_date_segment(date)
        && segment.len() > 11
}

fn is_projectless_date_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn non_empty_string_array(value: Option<&Value>) -> Option<Vec<String>> {
    let values = value?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn sort_thread_list(threads: &mut [Value], params: &Value) {
    let sort_key = match params.get("sortKey").and_then(Value::as_str) {
        Some("created_at") => "createdAt",
        _ => "updatedAt",
    };
    let ascending = params
        .get("sortDirection")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "asc");
    threads.sort_by(|first, second| {
        let ordering = json_number(first.get(sort_key))
            .partial_cmp(&json_number(second.get(sort_key)))
            .unwrap_or(std::cmp::Ordering::Equal);
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

fn collect_session_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn thread_summary_from_session_file(path: &Path, _profile_name: &str) -> Option<Value> {
    let payload = session_meta_payload(path)?;
    let id = payload.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }

    let (created_at, updated_at) = session_file_times(path);
    let preview = session_file_preview(path);
    let mut thread = Map::new();
    thread.insert("id".to_string(), json!(id));
    thread.insert(
        "sessionId".to_string(),
        json!(payload
            .get("sessionId")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(id)),
    );
    thread.insert(
        "forkedFromId".to_string(),
        payload
            .get("forkedFromId")
            .or_else(|| payload.get("forked_from_id"))
            .and_then(Value::as_str)
            .map(|value| json!(value))
            .unwrap_or(Value::Null),
    );
    thread.insert(
        "preview".to_string(),
        json!(preview.clone().unwrap_or_default()),
    );
    thread.insert("ephemeral".to_string(), json!(false));
    thread.insert(
        "modelProvider".to_string(),
        json!(session_model_provider_from_payload(&payload)),
    );
    thread.insert("createdAt".to_string(), json!(created_at));
    thread.insert("updatedAt".to_string(), json!(updated_at));
    thread.insert("status".to_string(), json!({ "type": "notLoaded" }));
    thread.insert(
        "path".to_string(),
        json!(path.to_string_lossy().to_string()),
    );
    thread.insert(
        "cwd".to_string(),
        json!(session_cwd_from_payload(&payload, path)),
    );
    thread.insert(
        "cliVersion".to_string(),
        json!(payload
            .get("cliVersion")
            .or_else(|| payload.get("cli_version"))
            .and_then(Value::as_str)
            .unwrap_or("")),
    );
    thread.insert("source".to_string(), normalized_session_source(&payload));
    thread.insert(
        "threadSource".to_string(),
        normalized_thread_source(&payload),
    );
    thread.insert("agentNickname".to_string(), Value::Null);
    thread.insert("agentRole".to_string(), Value::Null);
    thread.insert("gitInfo".to_string(), Value::Null);
    thread.insert(
        "name".to_string(),
        payload
            .get("name")
            .and_then(Value::as_str)
            .map(|value| json!(value))
            .or_else(|| preview.map(|value| json!(value)))
            .unwrap_or(Value::Null),
    );
    thread.insert("turns".to_string(), Value::Array(Vec::new()));

    Some(Value::Object(thread))
}

fn session_model_from_payload(payload: &Value) -> String {
    payload
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn session_model_provider_from_payload(payload: &Value) -> String {
    payload
        .get("modelProvider")
        .or_else(|| payload.get("model_provider"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn session_cwd_from_payload(payload: &Value, path: &Path) -> String {
    payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            path.parent()
                .map(|parent| parent.to_string_lossy().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "/".to_string())
}

fn normalized_session_source(payload: &Value) -> Value {
    match payload.get("source").and_then(Value::as_str) {
        Some("cli" | "vscode" | "exec" | "appServer" | "unknown") => {
            json!(payload.get("source").and_then(Value::as_str).unwrap())
        }
        Some("codex_cli_rs") => json!("cli"),
        Some(value) if value.to_ascii_lowercase().contains("vscode") => json!("vscode"),
        Some(value) if value.to_ascii_lowercase().contains("app") => json!("appServer"),
        Some(value) if !value.trim().is_empty() => json!("cli"),
        _ => json!("unknown"),
    }
}

fn normalized_thread_source(payload: &Value) -> Value {
    match payload
        .get("threadSource")
        .or_else(|| payload.get("thread_source"))
        .and_then(Value::as_str)
    {
        Some("user" | "subagent" | "memory_consolidation") => {
            json!(payload
                .get("threadSource")
                .or_else(|| payload.get("thread_source"))
                .and_then(Value::as_str)
                .unwrap())
        }
        _ => Value::Null,
    }
}

fn session_meta_payload(path: &Path) -> Option<Value> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let meta: Value = serde_json::from_str(line.trim_end()).ok()?;
    if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    meta.get("payload").cloned()
}

fn session_file_preview(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().map_while(Result::ok).skip(1).take(400) {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if let Some(text) = session_line_preview(&value) {
            return Some(text);
        }
    }
    None
}

fn thread_turns_from_session_file(path: &Path) -> Vec<Value> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut turns = Vec::new();
    let mut current_items = Vec::new();
    let mut current_turn_id = String::new();
    let mut current_started_at = Value::Null;
    let mut current_completed_at = Value::Null;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if let Some((turn_id, started_at)) = task_started_turn(&value) {
            push_session_turn(
                &mut turns,
                &mut current_items,
                &current_turn_id,
                &current_started_at,
                &current_completed_at,
            );
            current_turn_id = turn_id;
            current_started_at = started_at;
            current_completed_at = Value::Null;
            continue;
        }
        if let Some(completed_at) = task_completed_at(&value) {
            current_completed_at = completed_at;
            continue;
        }
        let Some(item) = session_turn_item(&value) else {
            continue;
        };
        if item.get("type").and_then(Value::as_str) == Some("userMessage")
            && !current_items.is_empty()
        {
            push_session_turn(
                &mut turns,
                &mut current_items,
                &current_turn_id,
                &current_started_at,
                &current_completed_at,
            );
            current_turn_id.clear();
            current_started_at = Value::Null;
            current_completed_at = Value::Null;
        }
        current_items.push(item);
    }

    push_session_turn(
        &mut turns,
        &mut current_items,
        &current_turn_id,
        &current_started_at,
        &current_completed_at,
    );
    turns
}

fn push_session_turn(
    turns: &mut Vec<Value>,
    current_items: &mut Vec<Value>,
    current_turn_id: &str,
    current_started_at: &Value,
    current_completed_at: &Value,
) {
    if current_items.is_empty() {
        return;
    }
    let index = turns.len() + 1;
    let turn_id = if current_turn_id.trim().is_empty() {
        format!("codexl-session-turn-{index}")
    } else {
        current_turn_id.to_string()
    };
    turns.push(json!({
        "id": turn_id,
        "items": std::mem::take(current_items),
        "itemsView": "full",
        "status": "completed",
        "error": Value::Null,
        "startedAt": current_started_at.clone(),
        "completedAt": current_completed_at.clone(),
        "durationMs": Value::Null,
    }));
}

fn task_started_turn(value: &Value) -> Option<(String, Value)> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("task_started") {
        return None;
    }
    let turn_id = payload
        .get("turn_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("codexl-session-turn")
        .to_string();
    let started_at = payload
        .get("started_at")
        .or_else(|| payload.get("startedAt"))
        .and_then(timestamp_seconds_value)
        .unwrap_or(Value::Null);
    Some((turn_id, started_at))
}

fn task_completed_at(value: &Value) -> Option<Value> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("task_complete") {
        return None;
    }
    payload
        .get("completed_at")
        .or_else(|| payload.get("completedAt"))
        .and_then(timestamp_seconds_value)
}

fn timestamp_seconds_value(value: &Value) -> Option<Value> {
    match value {
        Value::Number(number) => number.as_i64().map(|value| json!(value)),
        Value::String(text) => text.trim().parse::<i64>().ok().map(|value| json!(value)),
        _ => None,
    }
}

fn session_turn_item(value: &Value) -> Option<Value> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    match payload.get("role").and_then(Value::as_str) {
        Some("user") => {
            let text = session_message_text(payload.get("content"))?;
            if is_synthetic_user_message(&text) {
                return None;
            }
            let content = normalize_session_message_content(payload.get("content"));
            if content.as_array().is_some_and(|items| items.is_empty()) {
                return None;
            }
            Some(json!({
                "type": "userMessage",
                "id": session_message_id(payload, "user"),
                "content": content,
            }))
        }
        Some("assistant") => Some(json!({
            "type": "agentMessage",
            "id": session_message_id(payload, "assistant"),
            "text": session_message_text(payload.get("content")).unwrap_or_default(),
            "phase": "final_answer",
            "memoryCitation": Value::Null,
        })),
        _ => None,
    }
}

fn session_message_id(payload: &Value, prefix: &str) -> String {
    payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("codexl-session-{prefix}-item"))
}

fn normalize_session_message_content(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return json!([]);
    };
    match value {
        Value::String(text) => json!([{ "type": "text", "text": text, "text_elements": [] }]),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter_map(normalized_session_content_part)
                .collect(),
        ),
        _ => json!([]),
    }
}

fn normalized_session_content_part(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let text = object
        .get("text")
        .or_else(|| object.get("input_text"))
        .and_then(Value::as_str)?;
    if is_synthetic_user_message(text) {
        return None;
    }
    Some(json!({
        "type": "text",
        "text": text,
        "text_elements": [],
    }))
}

fn session_message_text(value: Option<&Value>) -> Option<String> {
    text_from_json_value(value?)
}

fn session_file_times(path: &Path) -> (u64, u64) {
    let metadata = std::fs::metadata(path).ok();
    let updated_at = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_seconds)
        .unwrap_or(0);
    let created_at = metadata
        .as_ref()
        .and_then(|metadata| metadata.created().ok())
        .and_then(system_time_seconds)
        .unwrap_or(updated_at);
    (created_at, updated_at)
}

fn session_file_updated_at(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_seconds)
        .unwrap_or(0)
}

fn system_time_seconds(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn session_line_preview(value: &Value) -> Option<String> {
    if let Some(message) = value
        .get("payload")
        .filter(|_| value.get("type").and_then(Value::as_str) == Some("event_msg"))
        .filter(|payload| payload.get("type").and_then(Value::as_str) == Some("user_message"))
        .and_then(|payload| payload.get("message"))
        .and_then(text_from_json_value)
    {
        return real_user_message_preview(&message);
    }

    let payload = value.get("payload").unwrap_or(value);
    if payload.get("role").and_then(Value::as_str) == Some("user") {
        return payload
            .get("content")
            .and_then(text_from_json_value)
            .or_else(|| payload.get("text").and_then(text_from_json_value))
            .and_then(|text| real_user_message_preview(&text));
    }
    let item = payload.get("item").unwrap_or(payload);
    if item.get("role").and_then(Value::as_str) == Some("user") {
        return item
            .get("content")
            .and_then(text_from_json_value)
            .or_else(|| item.get("text").and_then(text_from_json_value))
            .and_then(|text| real_user_message_preview(&text));
    }
    None
}

fn text_from_json_value(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_text_from_json_value(value, &mut parts);
    let text = parts.join("\n");
    non_empty_text(&text)
}

fn collect_text_from_json_value(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_text_from_json_value(item, parts);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map
                .get("text")
                .or_else(|| map.get("input_text"))
                .and_then(Value::as_str)
            {
                parts.push(text.to_string());
            } else if let Some(content) = map.get("content") {
                collect_text_from_json_value(content, parts);
            }
        }
        _ => {}
    }
}

fn real_user_message_preview(text: &str) -> Option<String> {
    if is_synthetic_user_message(text) {
        None
    } else {
        non_empty_preview(text)
    }
}

fn is_synthetic_user_message(text: &str) -> bool {
    let trimmed = text.trim_start();
    [
        "<environment_context>",
        "<system",
        "<developer",
        "<permissions instructions>",
        "<app-context>",
        "<collaboration_mode>",
        "<skills_instructions>",
        "<plugins_instructions>",
        "<apps_instructions>",
        "You are ChatGPT",
        "You are Codex",
        "You are a coding agent",
        "You are a helpful assistant",
        "Knowledge cutoff:",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn non_empty_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn non_empty_preview(text: &str) -> Option<String> {
    let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.is_empty() {
        None
    } else {
        let mut chars = preview.chars();
        let truncated = chars.by_ref().take(120).collect::<String>();
        if chars.next().is_some() {
            Some(format!("{}...", truncated))
        } else {
            Some(preview)
        }
    }
}

fn json_number(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

fn trim_json_line(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .unwrap_or(line)
}

fn line_ending(line: &[u8]) -> &'static [u8] {
    if line.ends_with(b"\r\n") {
        b"\r\n"
    } else if line.ends_with(b"\n") {
        b"\n"
    } else {
        b""
    }
}

impl ChatGptAuth {
    fn load() -> Self {
        let workspace_name = current_workspace_name();
        let mut auth = auth_json_candidates()
            .into_iter()
            .find_map(|path| Self::from_auth_json_path(&path))
            .unwrap_or_default();
        auth.workspace_name = workspace_name;
        auth
    }

    pub(crate) fn load_for_codex_home(codex_home: &str, workspace_name: &str) -> Self {
        let auth_path = expand_home_path(codex_home.trim()).join("auth.json");
        let mut auth = Self::from_auth_json_path(&auth_path).unwrap_or_default();
        auth.workspace_name = normalize_profile(Some(workspace_name));
        auth
    }

    fn from_auth_json_path(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        if value
            .get("auth_mode")
            .and_then(Value::as_str)
            .is_some_and(|auth_mode| auth_mode != "chatgpt")
        {
            return None;
        }

        let tokens = value.get("tokens")?;
        let auth_token = tokens
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .map(ToString::to_string);
        let id_token = tokens
            .get("id_token")
            .and_then(Value::as_str)
            .filter(|token| !token.trim().is_empty());

        let claims = auth_token
            .as_deref()
            .and_then(jwt_payload_claims)
            .or_else(|| id_token.and_then(jwt_payload_claims));

        let email = claims
            .as_ref()
            .and_then(jwt_email)
            .or_else(|| value.get("email").and_then(Value::as_str))
            .map(ToString::to_string);
        let plan_type = claims
            .as_ref()
            .and_then(jwt_plan_type)
            .map(ToString::to_string);

        Some(Self {
            email,
            workspace_name: None,
            plan_type,
            auth_token,
        })
    }

    pub(crate) fn account_read_result(&self) -> Value {
        json!({
            "account": {
                "type": "chatgpt",
                "email": self.account_email(),
                "planType": self.plan_type.as_deref().unwrap_or("unknown"),
            },
            "requiresOpenaiAuth": true,
        })
    }

    fn account_email(&self) -> &str {
        self.email
            .as_deref()
            .or(self.workspace_name.as_deref())
            .unwrap_or("codex")
    }

    pub(crate) fn auth_status_result(&self, include_token: bool) -> Value {
        let mut result = serde_json::Map::new();
        result.insert("authMethod".to_string(), json!("chatgpt"));
        if include_token {
            result.insert(
                "authToken".to_string(),
                self.auth_token
                    .as_ref()
                    .map(|token| json!(token))
                    .unwrap_or(Value::Null),
            );
        }
        result.insert("requiresOpenaiAuth".to_string(), json!(true));
        Value::Object(result)
    }
}

fn current_workspace_name() -> Option<String> {
    std::env::var(CODEX_WORKSPACE_NAME_ENV)
        .ok()
        .and_then(|value| normalize_profile(Some(&value)))
        .or_else(|| {
            std::env::var(LEGACY_CODEX_INSTANCE_NAME_ENV)
                .ok()
                .and_then(|value| normalize_profile(Some(&value)))
        })
        .or_else(|| {
            std::env::var(CODEX_PROFILE_ENV)
                .ok()
                .and_then(|value| normalize_profile(Some(&value)))
        })
}

fn auth_json_candidates() -> Vec<PathBuf> {
    std::env::var(CODEX_HOME_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|codex_home| vec![expand_home_path(&codex_home).join("auth.json")])
        .unwrap_or_default()
}

fn jwt_payload_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64_url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_email(claims: &Value) -> Option<&str> {
    claims
        .get("https://api.openai.com/profile")
        .and_then(|profile| profile.get("email"))
        .and_then(Value::as_str)
        .or_else(|| claims.get("email").and_then(Value::as_str))
}

fn jwt_plan_type(claims: &Value) -> Option<&str> {
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .or_else(|| claims.get("chatgpt_plan_type").and_then(Value::as_str))
}

fn base64_url_decode(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;

        buffer = (buffer << 6) | value;
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    Some(output)
}

fn log_invocation(
    real_cli: &Path,
    profile: Option<&str>,
    model_provider: Option<&str>,
    args: &[OsString],
) {
    let Some(mut log) = open_log_file_from_env(MIDDLEWARE_LOG_ENV) else {
        return;
    };

    let _ = write!(
        log,
        "[{}] real_cli={} profile={} model_provider={} args=",
        timestamp_seconds(),
        real_cli.to_string_lossy(),
        profile.unwrap_or(""),
        model_provider.unwrap_or("")
    );
    for arg in args {
        let _ = write!(log, " {}", arg.to_string_lossy());
    }
    let _ = writeln!(log);
}

fn timestamp_seconds() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{}", seconds)
}

fn slugify_file_segment(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "default".to_string()
    } else {
        slug
    }
}

#[cfg(windows)]
fn middleware_script(host_executable: &Path, _real_cli_path: &Path) -> String {
    format!(
        "@echo off\r\n\"{}\" {} %*\r\nexit /b %ERRORLEVEL%\r\n",
        host_executable.to_string_lossy(),
        RUN_MODE_ARG
    )
}

#[cfg(windows)]
fn stdio_export_script(
    host_executable: &Path,
    middleware_path: &Path,
    real_cli_path: &Path,
    log_path: &Path,
    codex_home: Option<&str>,
    workspace_name: Option<&str>,
    profile: Option<&str>,
    model_provider: Option<&str>,
    core_mode: Option<&str>,
    profile_config_format: CodexProfileConfigFormat,
) -> String {
    let mut script = String::from("@echo off\r\n");
    let model_provider = normalize_model_provider(model_provider, profile);
    push_cmd_env(
        &mut script,
        CODEX_CLI_PATH_ENV,
        &middleware_path.to_string_lossy(),
    );
    push_cmd_env(&mut script, REAL_CLI_ENV, &real_cli_path.to_string_lossy());
    push_cmd_env(
        &mut script,
        BUNDLED_REAL_CLI_ENV,
        &real_cli_path.to_string_lossy(),
    );
    push_cmd_env(&mut script, MIDDLEWARE_LOG_ENV, &log_path.to_string_lossy());
    if let Some(codex_home) = codex_home {
        push_cmd_env(&mut script, CODEX_HOME_ENV, codex_home);
    }
    if let Some(workspace_name) = workspace_name {
        push_cmd_env(&mut script, CODEX_WORKSPACE_NAME_ENV, workspace_name);
    }
    if let Some(profile) = profile {
        push_cmd_env(&mut script, CODEX_PROFILE_ENV, profile);
    }
    if let Some(model_provider) = model_provider.as_deref() {
        push_cmd_env(&mut script, CODEX_MODEL_PROVIDER_ENV, model_provider);
    }
    if let Some(core_mode) = core_mode {
        push_cmd_env(&mut script, CODEX_CORE_MODE_ENV, core_mode);
    }
    push_cmd_env(
        &mut script,
        CODEX_PROFILE_CONFIG_FORMAT_ENV,
        profile_config_format.env_value(),
    );
    script.push_str(&format!(
        "\"{}\" {} %*\r\nexit /b %ERRORLEVEL%\r\n",
        host_executable.to_string_lossy(),
        STDIO_RUN_MODE_ARG
    ));
    script
}

#[cfg(windows)]
fn push_cmd_env(script: &mut String, name: &str, value: &str) {
    script.push_str(&format!(
        "set \"{}={}\"\r\n",
        name,
        value.replace('"', "\\\"")
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("codexl-{}-{}-{}", name, std::process::id(), nanos))
    }

    fn restore_env(name: &str, value: Option<OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }

    #[test]
    fn resolves_bundled_cli_from_macos_app_executable() {
        let root = test_dir("bundle-path");
        let macos_dir = root.join("Codex.app").join("Contents").join("MacOS");
        let resources_dir = root.join("Codex.app").join("Contents").join("Resources");
        std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
        std::fs::create_dir_all(&resources_dir).expect("create Resources dir");

        let app_executable = macos_dir.join("Codex");
        let cli_executable = resources_dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        std::fs::write(&app_executable, "").expect("write app executable");
        std::fs::write(&cli_executable, "").expect("write CLI executable");

        assert_eq!(
            bundled_cli_path(&app_executable.to_string_lossy()),
            Some(cli_executable)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn main_app_server_uses_signed_node_supervisor_when_bundled_node_exists() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_supervisor = std::env::var_os(SIGNED_SUPERVISOR_NODE_ENV);
        std::env::remove_var(SIGNED_SUPERVISOR_NODE_ENV);

        let root = test_dir("signed-node-supervisor");
        let resources = root.join("ChatGPT.app").join("Contents").join("Resources");
        let real_cli = resources.join("codex");
        let node = resources.join("cua_node").join("bin").join("node");
        std::fs::create_dir_all(node.parent().expect("node parent")).expect("create node dir");
        std::fs::write(&real_cli, "").expect("write fake CLI");
        std::fs::write(&node, "").expect("write fake node");

        let args = vec![
            OsString::from("app-server"),
            OsString::from("--analytics-default-enabled"),
        ];
        let command = real_cli_command(&real_cli, &args);
        assert_eq!(command.get_program(), node.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![
                OsStr::new("-e"),
                OsStr::new(SIGNED_SUPERVISOR_SOURCE),
                real_cli.as_os_str(),
                OsStr::new("app-server"),
                OsStr::new("--analytics-default-enabled"),
            ]
        );

        restore_env(SIGNED_SUPERVISOR_NODE_ENV, old_supervisor);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn main_app_server_falls_back_to_real_cli_without_signed_node() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_supervisor = std::env::var_os(SIGNED_SUPERVISOR_NODE_ENV);
        std::env::remove_var(SIGNED_SUPERVISOR_NODE_ENV);

        let real_cli = PathBuf::from("/tmp/codex-without-bundled-node");
        let args = vec![OsString::from("app-server")];
        let command = real_cli_command(&real_cli, &args);
        assert_eq!(command.get_program(), real_cli.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("app-server")]
        );

        restore_env(SIGNED_SUPERVISOR_NODE_ENV, old_supervisor);
    }

    #[cfg(unix)]
    #[test]
    fn generated_middleware_forwards_to_real_cli() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("forward");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let real_cli = root.join("codex");
        let middleware = root.join("codexl-codex-cli-middleware");
        let log_path = root.join("middleware.log");

        std::fs::write(
            &real_cli,
            r#"#!/bin/sh
if [ -n "${CODEX_CLI_PATH:-}" ]; then
  echo "CODEX_CLI_PATH leaked" >&2
  exit 42
fi
IFS= read -r first_line || first_line=
printf 'real'
for arg in "$@"; do
  printf ':%s' "$arg"
done
printf ':stdin=%s\n' "$first_line"
"#,
        )
        .expect("write fake CLI");
        let mut permissions = std::fs::metadata(&real_cli)
            .expect("fake CLI metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&real_cli, permissions).expect("chmod fake CLI");

        std::env::set_var(REAL_CLI_ENV, &real_cli);
        std::env::set_var(MIDDLEWARE_LOG_ENV, &log_path);
        std::env::set_var(CODEX_PROFILE_ENV, "test-profile");
        std::env::set_var(CODEX_MODEL_PROVIDER_ENV, "test-provider");
        std::env::set_var(CODEX_CLI_PATH_ENV, &middleware);

        let protocol_stdout = root.join("protocol-stdout.log");
        let status = run_stdio_middleware_with_io(
            vec![
                OsString::from("app-server"),
                OsString::from("--analytics-default-enabled"),
            ],
            std::io::Cursor::new(b"ping\n".to_vec()),
            File::create(&protocol_stdout).expect("create protocol stdout"),
        )
        .expect("run middleware");

        std::env::remove_var(REAL_CLI_ENV);
        std::env::remove_var(MIDDLEWARE_LOG_ENV);
        std::env::remove_var(CODEX_PROFILE_ENV);
        std::env::remove_var(CODEX_MODEL_PROVIDER_ENV);
        std::env::remove_var(CODEX_CLI_PATH_ENV);

        assert_eq!(status, 0);
        assert_eq!(
            std::fs::read_to_string(protocol_stdout).expect("read protocol stdout"),
            "real:-c:profile=\"test-profile\":-c:model_provider=\"test-provider\":app-server:--analytics-default-enabled:stdin=ping\n"
        );
        assert!(std::fs::read_to_string(log_path)
            .expect("read middleware log")
            .contains("profile=test-profile model_provider=test-provider args="));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn generated_middleware_embeds_real_cli_path() {
        let script = middleware_script(Path::new("/tmp/CodexL Host"), Path::new("/tmp/Real Codex"));

        assert!(script.contains("export CODEXL_REAL_CODEX_CLI_PATH='/tmp/Real Codex'\n"));
        assert!(script.contains("export CODEXL_BUNDLED_CODEX_CLI_PATH='/tmp/Real Codex'\n"));
        assert!(script.contains(
            "if [ \"${1:-}\" = 'sandbox' ] || { [ \"${1:-}\" = 'app-server' ] && [ \"${2:-}\" = '--listen' ] && [ \"${3:-}\" = 'stdio://' ]; }; then\n"
        ));
        assert!(script.contains("  unset CODEX_CLI_PATH\n"));
        assert!(script.contains("  exec '/tmp/Real Codex' \"$@\"\n"));
        assert!(script.contains("exec '/tmp/CodexL Host' --codexl-cli-middleware \"$@\"\n"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_middleware_execs_sandbox_with_real_cli() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_dir("sandbox-passthrough");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let real_cli = root.join("Real Codex");
        let host = root.join("CodexL Host");
        let middleware = root.join("codexl-codex-cli-middleware");
        std::fs::write(
            &real_cli,
            "#!/bin/sh\nprintf 'real:%s:cli_path=%s\\n' \"$1\" \"${CODEX_CLI_PATH:-}\"\n",
        )
        .expect("write fake real CLI");
        std::fs::write(&host, "#!/bin/sh\nprintf 'middleware-host\\n'\n")
            .expect("write fake middleware host");
        std::fs::write(&middleware, middleware_script(&host, &real_cli)).expect("write middleware");
        for path in [&real_cli, &host, &middleware] {
            let mut permissions = std::fs::metadata(path)
                .expect("read executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("chmod executable");
        }

        let output = Command::new(&middleware)
            .arg("sandbox")
            .env(CODEX_CLI_PATH_ENV, &middleware)
            .output()
            .expect("run middleware sandbox passthrough");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8 stdout"),
            "real:sandbox:cli_path=\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn generated_middleware_execs_browser_app_server_with_real_cli() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_dir("browser-app-server-passthrough");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let real_cli = root.join("Real Codex");
        let host = root.join("CodexL Host");
        let middleware = root.join("codexl-codex-cli-middleware");
        std::fs::write(
            &real_cli,
            "#!/bin/sh\nprintf 'real:%s:%s:%s:cli_path=%s\\n' \"$1\" \"$2\" \"$3\" \"${CODEX_CLI_PATH:-}\"\n",
        )
        .expect("write fake real CLI");
        std::fs::write(&host, "#!/bin/sh\nprintf 'middleware-host\\n'\n")
            .expect("write fake middleware host");
        std::fs::write(&middleware, middleware_script(&host, &real_cli)).expect("write middleware");
        for path in [&real_cli, &host, &middleware] {
            let mut permissions = std::fs::metadata(path)
                .expect("read executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("chmod executable");
        }

        let output = Command::new(&middleware)
            .args(["app-server", "--listen", "stdio://"])
            .env(CODEX_CLI_PATH_ENV, &middleware)
            .output()
            .expect("run middleware browser app-server passthrough");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8 stdout"),
            "real:app-server:--listen:stdio://:cli_path=\n"
        );

        let main_output = Command::new(&middleware)
            .args(["app-server", "--analytics-default-enabled"])
            .env(CODEX_CLI_PATH_ENV, &middleware)
            .output()
            .expect("run middleware main app-server path");
        assert!(main_output.status.success());
        assert_eq!(
            String::from_utf8(main_output.stdout).expect("utf8 stdout"),
            "middleware-host\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_cli_resolver_uses_cached_path_without_environment() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("runtime-cli-cache");
        let real_cli = root.join("codex");
        std::fs::create_dir_all(&root).expect("create temp dir");
        std::fs::write(&real_cli, "#!/bin/sh\nexit 0\n").expect("write fake CLI");
        let mut permissions = std::fs::metadata(&real_cli)
            .expect("fake CLI metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&real_cli, permissions).expect("chmod fake CLI");

        let old_home = std::env::var_os("CODEXL_HOME");
        let old_real = std::env::var_os(REAL_CLI_ENV);
        let old_bundled = std::env::var_os(BUNDLED_REAL_CLI_ENV);
        let old_cli_path = std::env::var_os(CODEX_CLI_PATH_ENV);
        std::env::set_var("CODEXL_HOME", &root);
        std::env::remove_var(REAL_CLI_ENV);
        std::env::remove_var(BUNDLED_REAL_CLI_ENV);
        std::env::remove_var(CODEX_CLI_PATH_ENV);
        write_real_cli_cache(&real_cli).expect("write real CLI cache");

        let resolved = resolve_runtime_real_cli_path().expect("resolve cached real CLI");

        restore_env("CODEXL_HOME", old_home);
        restore_env(REAL_CLI_ENV, old_real);
        restore_env(BUNDLED_REAL_CLI_ENV, old_bundled);
        restore_env(CODEX_CLI_PATH_ENV, old_cli_path);
        assert_eq!(resolved, real_cli);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_stdio_defaults_to_app_server_args() {
        assert_eq!(
            external_stdio_args(Vec::new()),
            vec![
                OsString::from("app-server"),
                OsString::from("--analytics-default-enabled")
            ]
        );

        assert_eq!(
            external_stdio_args(vec![OsString::from("exec")]),
            vec![OsString::from("exec")]
        );
    }

    #[test]
    fn separate_profile_files_use_profile_flag_for_runtime_real_cli() {
        let args = real_cli_args(
            Some("test-profile"),
            Some("test-provider"),
            CodexProfileConfigFormat::SeparateProfileFiles,
            vec![OsString::from("exec")],
        );

        assert_eq!(
            args,
            vec![
                OsString::from("--profile"),
                OsString::from("test-profile"),
                OsString::from("-c"),
                OsString::from("model_provider=\"test-provider\""),
                OsString::from("exec"),
            ]
        );
    }

    #[test]
    fn app_server_skips_profile_flag_for_real_cli() {
        let args = real_cli_args(
            Some("test-profile"),
            Some("test-provider"),
            CodexProfileConfigFormat::SeparateProfileFiles,
            vec![OsString::from("app-server")],
        );

        assert_eq!(
            args,
            vec![
                OsString::from("-c"),
                OsString::from("model_provider=\"test-provider\""),
                OsString::from("app-server"),
            ]
        );
    }

    #[test]
    fn default_model_provider_placeholder_is_not_forwarded_to_real_cli() {
        let args = real_cli_args(
            None,
            Some("default"),
            CodexProfileConfigFormat::SeparateProfileFiles,
            vec![OsString::from("app-server")],
        );

        assert_eq!(args, vec![OsString::from("app-server")]);
    }

    #[test]
    fn default_model_provider_name_is_preserved_for_explicit_profile() {
        let args = real_cli_args(
            Some("custom-profile"),
            Some("default"),
            CodexProfileConfigFormat::SeparateProfileFiles,
            vec![OsString::from("exec")],
        );

        assert_eq!(
            args,
            vec![
                OsString::from("--profile"),
                OsString::from("custom-profile"),
                OsString::from("-c"),
                OsString::from("model_provider=\"default\""),
                OsString::from("exec"),
            ]
        );
    }

    #[test]
    fn profile_flag_detection_handles_global_options() {
        assert!(!codex_args_accept_profile_flag(&[
            OsString::from("-c"),
            OsString::from("model=\"gpt-5\""),
            OsString::from("app-server"),
        ]));
        assert!(codex_args_accept_profile_flag(&[
            OsString::from("--config=model=\"gpt-5\""),
            OsString::from("exec"),
        ]));
        assert!(codex_args_accept_profile_flag(&[
            OsString::from("debug"),
            OsString::from("prompt-input"),
        ]));
        assert!(!codex_args_accept_profile_flag(&[
            OsString::from("debug"),
            OsString::from("models"),
        ]));
    }

    #[test]
    fn detects_app_server_fetch_messages_for_middleware_intercept() {
        let message = app_server_fetch_message_from_line(
            br#"{"type":"fetch","requestId":"voice-1","method":"POST","url":"/transcribe"}"#,
        )
        .expect("fetch message");

        assert_eq!(
            message.get("requestId").and_then(Value::as_str),
            Some("voice-1")
        );
        assert_eq!(
            app_server_fetch_message_from_line(br#"{"method":"initialize"}"#),
            None
        );
        assert_eq!(
            app_server_fetch_message_from_line(
                br#"{"type":"fetch","requestId":"ipc-1","method":"POST","url":"vscode://codex/ipc-request"}"#
            ),
            None
        );
    }

    #[test]
    fn stdio_path_uses_profile_slug() {
        let path = stdio_path(Some("My Provider/Profile"));
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("stdio file name");

        if cfg!(windows) {
            assert_eq!(file_name, "codexl-codex-cli-stdio-my-provider-profile.cmd");
        } else {
            assert_eq!(file_name, "codexl-codex-cli-stdio-my-provider-profile");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_appx_cli_is_copied_before_use() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("windows-appx-cli");
        let source = root
            .join("WindowsApps")
            .join("OpenAI.Codex_test")
            .join("app")
            .join("resources")
            .join("codex.exe");
        std::fs::create_dir_all(source.parent().expect("source parent"))
            .expect("create source dir");
        std::fs::write(&source, b"fake codex cli").expect("write source cli");

        let codexl_home = root.join("codexl-home");
        std::env::set_var("CODEXL_HOME", &codexl_home);

        let prepared = materialize_windows_appx_cli(source.clone()).expect("materialize cli");

        std::env::remove_var("CODEXL_HOME");
        assert_eq!(
            prepared,
            codexl_home
                .join("bin")
                .join("codexl-real-codex-openai-codex-test.exe")
        );
        assert_eq!(
            std::fs::read(&prepared).expect("read copied cli"),
            b"fake codex cli"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_middleware_shim_accepts_versioned_file_name() {
        assert!(program_is_windows_middleware_shim(Some(OsStr::new(
            "codexl-codex-cli-middleware.exe"
        ))));
        assert!(program_is_windows_middleware_shim(Some(OsStr::new(
            "codexl-codex-cli-middleware-123-456.exe"
        ))));
        assert!(!program_is_windows_middleware_shim(Some(OsStr::new(
            "codexl-codex-cli-stdio.cmd"
        ))));
    }

    #[cfg(unix)]
    #[test]
    fn generated_stdio_export_embeds_external_environment() {
        let script = stdio_export_script(
            Path::new("/tmp/CodexL Host"),
            Path::new("/tmp/codexl-codex-cli-middleware"),
            Path::new("/tmp/Real Codex"),
            Path::new("/tmp/middleware.log"),
            Some("/tmp/codex home"),
            Some("custom-instance"),
            Some("custom-profile"),
            Some("custom-provider"),
            Some("claude-code"),
            CodexProfileConfigFormat::SeparateProfileFiles,
        );

        assert!(script.contains("export CODEX_CLI_PATH='/tmp/codexl-codex-cli-middleware'\n"));
        assert!(script.contains("export CODEXL_REAL_CODEX_CLI_PATH='/tmp/Real Codex'\n"));
        assert!(script.contains("export CODEXL_BUNDLED_CODEX_CLI_PATH='/tmp/Real Codex'\n"));
        assert!(script.contains("export CODEXL_CLI_MIDDLEWARE_LOG='/tmp/middleware.log'\n"));
        assert!(!script.contains("CODEXL_CLI_MIDDLEWARE_STDIN_LOG"));
        assert!(!script.contains("CODEXL_CLI_MIDDLEWARE_STDOUT_LOG"));
        assert!(script.contains("export CODEX_HOME='/tmp/codex home'\n"));
        assert!(script.contains("export CODEXL_CODEX_WORKSPACE_NAME='custom-instance'\n"));
        assert!(script.contains("export CODEXL_CODEX_PROFILE='custom-profile'\n"));
        assert!(script.contains("export CODEXL_CODEX_MODEL_PROVIDER='custom-provider'\n"));
        assert!(script.contains("export CODEXL_CODEX_CORE_MODE='claude-code'\n"));
        assert!(
            script.contains("export CODEXL_CODEX_PROFILE_CONFIG_FORMAT='separate_profile_files'\n")
        );
        assert!(script.contains("exec '/tmp/CodexL Host' --codexl-cli-stdio \"$@\"\n"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_stdio_export_skips_default_model_provider_placeholder() {
        let script = stdio_export_script(
            Path::new("/tmp/CodexL Host"),
            Path::new("/tmp/codexl-codex-cli-middleware"),
            Path::new("/tmp/Real Codex"),
            Path::new("/tmp/middleware.log"),
            Some("/tmp/codex home"),
            Some("default-instance"),
            Some(crate::config::DEFAULT_PROVIDER_PROFILE_NAME),
            Some("default"),
            Some("app"),
            CodexProfileConfigFormat::SeparateProfileFiles,
        );

        assert!(!script.contains("CODEXL_CODEX_MODEL_PROVIDER"));
        assert!(script.contains("export CODEXL_CODEX_PROFILE='Default'\n"));
    }

    #[test]
    #[ignore = "requires real ccr code service and Claude Code auth"]
    fn real_claude_code_core_mode_routes_middleware_to_ccr() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("middleware-real-ccr");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let output_path = root.join("out.jsonl");
        let thread_id = "33333333-3333-4333-8333-333333333333";
        let token = "CODEXL_MIDDLEWARE_CLAUDE_OK";
        let input = format!(
            "{{\"id\":\"1\",\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\"}}}}\n{{\"method\":\"initialized\",\"params\":{{}}}}\n{{\"id\":\"2\",\"method\":\"thread/resume\",\"params\":{{\"threadId\":\"{}\",\"cwd\":\"{}\",\"model\":\"sonnet\"}}}}\n{{\"id\":\"3\",\"method\":\"turn/start\",\"params\":{{\"threadId\":\"{}\",\"input\":[{{\"type\":\"text\",\"text\":\"Reply exactly with this token and nothing else: {}\"}}]}}}}\n",
            thread_id,
            root.to_string_lossy(),
            thread_id,
            token
        );

        std::env::set_var(
            CODEX_CORE_MODE_ENV,
            crate::config::REMOTE_FRONTEND_MODE_CLAUDE_CODE,
        );
        std::env::set_var(CODEX_WORKSPACE_NAME_ENV, "middleware-real-ccr");
        std::env::remove_var(REAL_CLI_ENV);

        run_stdio_middleware_with_io(
            vec![
                OsString::from("app-server"),
                OsString::from("--analytics-default-enabled"),
            ],
            std::io::Cursor::new(input.into_bytes()),
            std::fs::File::create(&output_path).expect("create output"),
        )
        .expect("run middleware");

        std::env::remove_var(CODEX_CORE_MODE_ENV);
        std::env::remove_var(CODEX_WORKSPACE_NAME_ENV);

        let output = std::fs::read_to_string(&output_path).expect("read output");
        assert!(output.contains(r#""method":"item/started""#));
        assert!(output.contains(r#""method":"item/commandExecution/outputDelta""#));
        assert!(output.contains(r#""method":"item/agentMessage/delta""#));
        assert!(output.contains(token), "output was:\n{}", output);
        assert!(output.contains(r#""method":"turn/completed""#));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rewrites_auth_responses_as_chatgpt() {
        let root = test_dir("auth-rewrite");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let auth_path = root.join("auth.json");
        let token = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL3Byb2ZpbGUiOnsiZW1haWwiOiJ1c2VyQGV4YW1wbGUuY29tIn0sImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3BsYW5fdHlwZSI6InBsdXMifX0.signature";
        std::fs::write(
            &auth_path,
            format!(
                r#"{{
  "auth_mode": "chatgpt",
  "OPENAI_API_KEY": null,
  "tokens": {{
    "access_token": "{}",
    "id_token": "{}",
    "refresh_token": "refresh",
    "account_id": "account"
  }}
}}"#,
                token, token
            ),
        )
        .expect("write auth json");

        let auth = ChatGptAuth::from_auth_json_path(&auth_path).expect("load auth");
        let request_map = Arc::new(Mutex::new(std::collections::HashMap::new()));

        track_request_line_with_workspace(
            br#"{"id":"account-id","method":"account/read","params":{"refreshToken":false}}
"#,
            &request_map,
            None,
        );
        let account_line = rewrite_stdout_line(
            br#"{"id":"account-id","result":{"account":null,"requiresOpenaiAuth":false}}
"#,
            &request_map,
            &auth,
        );
        let account: Value = serde_json::from_slice(trim_json_line(&account_line)).expect("json");
        assert_eq!(account["result"]["account"]["type"], "chatgpt");
        assert_eq!(account["result"]["account"]["email"], "user@example.com");
        assert_eq!(account["result"]["account"]["planType"], "plus");
        assert_eq!(account["result"]["requiresOpenaiAuth"], true);

        track_request_line_with_workspace(
            br#"{"id":"auth-id","method":"getAuthStatus","params":{"includeToken":true,"refreshToken":false}}
"#,
            &request_map,
            None,
        );
        let auth_line = rewrite_stdout_line(
            br#"{"id":"auth-id","result":{"authMethod":null,"authToken":null,"requiresOpenaiAuth":false}}
"#,
            &request_map,
            &auth,
        );
        let status: Value = serde_json::from_slice(trim_json_line(&auth_line)).expect("json");
        assert_eq!(status["result"]["authMethod"], "chatgpt");
        assert_eq!(status["result"]["authToken"], token);
        assert_eq!(status["result"]["requiresOpenaiAuth"], true);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_request_inherits_recent_workspace_cwd() {
        let request_map = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let current_cwd = Arc::new(Mutex::new(None));

        track_request_line_with_workspace(
            br#"{"id":"config-id","method":"config/read","params":{"includeLayers":false,"cwd":"/tmp/current-project"}}
"#,
            &request_map,
            Some(&current_cwd),
        );
        track_request_line_with_workspace(
            br#"{"id":"plugin-id","method":"plugin/list","params":{"cwds":["/tmp/marketplace-cache"]}}
"#,
            &request_map,
            Some(&current_cwd),
        );
        track_request_line_with_workspace(
            br#"{"id":"list-id","method":"thread/list","params":{"limit":50,"cursor":null,"sortKey":"updated_at","modelProviders":null,"archived":false,"sourceKinds":[]}}
"#,
            &request_map,
            Some(&current_cwd),
        );

        let requests = request_map.lock().expect("request map");
        let request = requests.get("list-id").expect("tracked thread/list");
        assert_eq!(
            request
                .params
                .get(CODEXL_WORKSPACE_CWD_FILTER_KEY)
                .and_then(Value::as_str),
            Some("/tmp/current-project")
        );
        assert!(request.params.get("cwd").is_none());
    }

    #[test]
    fn mock_account_email_falls_back_to_workspace_name() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("auth-workspace-name");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let old_codex_home = std::env::var(CODEX_HOME_ENV).ok();
        let old_workspace_name = std::env::var(CODEX_WORKSPACE_NAME_ENV).ok();
        let old_instance_name = std::env::var(LEGACY_CODEX_INSTANCE_NAME_ENV).ok();
        let old_profile = std::env::var(CODEX_PROFILE_ENV).ok();

        std::env::set_var(CODEX_HOME_ENV, &root);
        std::env::set_var(CODEX_WORKSPACE_NAME_ENV, "workspace-a");
        std::env::remove_var(LEGACY_CODEX_INSTANCE_NAME_ENV);
        std::env::remove_var(CODEX_PROFILE_ENV);

        let auth = ChatGptAuth::load();
        let account = auth.account_read_result();
        assert_eq!(account["account"]["email"], "workspace-a");

        if let Some(value) = old_codex_home {
            std::env::set_var(CODEX_HOME_ENV, value);
        } else {
            std::env::remove_var(CODEX_HOME_ENV);
        }
        if let Some(value) = old_workspace_name {
            std::env::set_var(CODEX_WORKSPACE_NAME_ENV, value);
        } else {
            std::env::remove_var(CODEX_WORKSPACE_NAME_ENV);
        }
        if let Some(value) = old_instance_name {
            std::env::set_var(LEGACY_CODEX_INSTANCE_NAME_ENV, value);
        } else {
            std::env::remove_var(LEGACY_CODEX_INSTANCE_NAME_ENV);
        }
        if let Some(value) = old_profile {
            std::env::set_var(CODEX_PROFILE_ENV, value);
        } else {
            std::env::remove_var(CODEX_PROFILE_ENV);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn auth_candidates_only_use_current_codex_home() {
        let root = test_dir("auth-current-home-only");
        let current_home = root.join("current");
        let default_home = root.join(".codex");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var(CODEX_HOME_ENV).ok();

        std::fs::create_dir_all(&current_home).expect("create current home");
        std::fs::create_dir_all(&default_home).expect("create default home");

        std::env::set_var("HOME", &root);
        std::env::set_var(CODEX_HOME_ENV, &current_home);
        assert_eq!(auth_json_candidates(), vec![current_home.join("auth.json")]);

        std::env::remove_var(CODEX_HOME_ENV);
        assert!(auth_json_candidates().is_empty());

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var(CODEX_HOME_ENV, value);
        } else {
            std::env::remove_var(CODEX_HOME_ENV);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_merge_reads_sessions_from_other_provider_homes() {
        let root = test_dir("all-sessions");
        let other_home = root.join("other-provider");
        let session_dir = other_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let session_path = session_dir.join("thread-other.jsonl");
        std::fs::write(
            &session_path,
            r#"{"type":"session_meta","payload":{"id":"thread-other","cwd":"/tmp/other","source":"codex_cli_rs","model_provider":"openai"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello from another provider"}]}}
"#,
        )
        .expect("write session");
        std::fs::write(
            session_dir.join("thread-different-project.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"thread-different-project","cwd":"/tmp/different","source":"vscode","model_provider":"openai"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"not this project"}]}}
"#,
        )
        .expect("write different project session");

        let mut response = json!({
            "id": "list-1",
            "result": {
                "data": [{
                    "id": "thread-current",
                    "cwd": "/tmp/current",
                    "updatedAt": 1
                }],
                "nextCursor": Value::Null
            }
        });
        let homes = vec![SessionHome {
            profile_name: "Other Provider".to_string(),
            path: other_home,
        }];

        merge_session_files_into_thread_list(
            &mut response,
            &homes,
            &json!({ "cwd": "/tmp/other" }),
        );
        let threads = response
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("threads");
        let other = threads
            .iter()
            .find(|thread| thread.get("id").and_then(Value::as_str) == Some("thread-other"))
            .expect("other provider thread");

        assert_eq!(threads.len(), 2);
        assert!(threads.iter().all(|thread| {
            thread.get("id").and_then(Value::as_str) != Some("thread-different-project")
        }));
        assert_eq!(other.get("cwd").and_then(Value::as_str), Some("/tmp/other"));
        assert_eq!(
            other.get("sessionId").and_then(Value::as_str),
            Some("thread-other")
        );
        assert_eq!(other.get("ephemeral").and_then(Value::as_bool), Some(false));
        assert_eq!(
            other.pointer("/status/type").and_then(Value::as_str),
            Some("notLoaded")
        );
        assert_eq!(other.get("cliVersion").and_then(Value::as_str), Some(""));
        assert_eq!(other.get("source").and_then(Value::as_str), Some("cli"));
        assert!(other.get("codexlProviderName").is_none());
        assert!(other.get("workspaceName").is_none());
        assert_eq!(
            other.get("path").and_then(Value::as_str),
            Some(session_path.to_string_lossy().as_ref())
        );
        assert_eq!(
            other.get("preview").and_then(Value::as_str),
            Some("hello from another provider")
        );
        assert_eq!(
            other.get("modelProvider").and_then(Value::as_str),
            Some("openai")
        );

        let session = SessionFile {
            profile_name: "Other Provider".to_string(),
            path: session_path.clone(),
        };
        let read =
            thread_read_response(json!("read-1"), &session, &json!({ "includeTurns": true }))
                .expect("thread read response");
        assert_eq!(read.get("jsonrpc").and_then(Value::as_str), Some("2.0"));
        assert_eq!(
            read.pointer("/result/thread/turns/0/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("hello from another provider")
        );
        assert_eq!(
            read.pointer("/result/thread/turns/0/itemsView")
                .and_then(Value::as_str),
            Some("full")
        );

        let turns =
            thread_turns_list_response(json!("turns-1"), &session).expect("turns list response");
        assert_eq!(turns.get("jsonrpc").and_then(Value::as_str), Some("2.0"));
        assert!(turns.pointer("/result/backwardsCursor").is_some());
        assert_eq!(
            turns
                .pointer("/result/data/0/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("hello from another provider")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_collects_sessions_per_provider_home_fairly() {
        let root = test_dir("all-sessions-fairness");
        let busy_home = root.join("busy-provider");
        let quiet_home = root.join("quiet-provider");
        let busy_dir = busy_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        let quiet_dir = quiet_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        std::fs::create_dir_all(&busy_dir).expect("create busy session dir");
        std::fs::create_dir_all(&quiet_dir).expect("create quiet session dir");

        for index in 0..5 {
            std::fs::write(
                busy_dir.join(format!("busy-{index}.jsonl")),
                format!(
                    r#"{{"type":"session_meta","payload":{{"id":"busy-{index}","cwd":"/tmp/busy","model_provider":"openai"}}}}
"#
                ),
            )
            .expect("write busy session");
        }
        std::fs::write(
            quiet_dir.join("quiet-0.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"quiet-0","cwd":"/tmp/quiet","model_provider":"openai"}}
"#,
        )
        .expect("write quiet session");

        let homes = vec![
            SessionHome {
                profile_name: "Busy".to_string(),
                path: busy_home,
            },
            SessionHome {
                profile_name: "Quiet".to_string(),
                path: quiet_home,
            },
        ];

        let files = session_files_from_homes(&homes, &json!({ "limit": 1 }));
        let busy_count = files
            .iter()
            .filter(|file| file.profile_name == "Busy")
            .count();
        let quiet_count = files
            .iter()
            .filter(|file| file.profile_name == "Quiet")
            .count();

        assert_eq!(busy_count, 1);
        assert_eq!(quiet_count, 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inherited_workspace_filter_keeps_projectless_sessions() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("all-sessions-projectless");
        let home_dir = root.join("home");
        let other_home = root.join("other-provider");
        let session_dir = other_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        let projectless_cwd = home_dir
            .join("Documents")
            .join("Codex")
            .join("2026-05-23")
            .join("chat-0");
        let old_home = std::env::var("HOME").ok();

        std::fs::create_dir_all(&session_dir).expect("create session dir");
        std::env::set_var("HOME", &home_dir);
        std::fs::write(
            session_dir.join("thread-current.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"thread-current","cwd":"/tmp/current","model_provider":"openai"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"current project"}]}}
"#,
        )
        .expect("write current session");
        std::fs::write(
            session_dir.join("thread-projectless.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"thread-projectless","cwd":"{}","model_provider":"openai"}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"projectless chat"}}]}}}}
"#,
                projectless_cwd.to_string_lossy()
            ),
        )
        .expect("write projectless session");
        std::fs::write(
            session_dir.join("thread-different.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"thread-different","cwd":"/tmp/different","model_provider":"openai"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"different project"}]}}
"#,
        )
        .expect("write different session");

        let mut response = json!({ "result": { "data": [] } });
        let homes = vec![SessionHome {
            profile_name: "Other Provider".to_string(),
            path: other_home,
        }];

        merge_session_files_into_thread_list(
            &mut response,
            &homes,
            &json!({ CODEXL_WORKSPACE_CWD_FILTER_KEY: "/tmp/current" }),
        );
        let ids = response
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("threads")
            .iter()
            .filter_map(|thread| thread.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"thread-current"));
        assert!(ids.contains(&"thread-projectless"));
        assert!(!ids.contains(&"thread-different"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_summary_skips_synthetic_context_and_resume_includes_turns() {
        let root = test_dir("all-sessions-context-skip");
        let session_dir = root.join("sessions").join("2026").join("05").join("23");
        std::fs::create_dir_all(&session_dir).expect("create session dir");
        let session_path = session_dir.join("thread-context.jsonl");
        std::fs::write(
            &session_path,
            r#"{"type":"session_meta","payload":{"id":"thread-context","cwd":"/tmp/current","model_provider":"openai"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context><cwd>/tmp/current</cwd></environment_context>"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"Fix session restore"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix session restore"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}}
"#,
        )
        .expect("write session");

        let thread = thread_summary_from_session_file(&session_path, "Other Provider")
            .expect("thread summary");
        assert_eq!(
            thread.get("preview").and_then(Value::as_str),
            Some("Fix session restore")
        );
        assert_eq!(
            thread.get("name").and_then(Value::as_str),
            Some("Fix session restore")
        );

        let session = SessionFile {
            profile_name: "Other Provider".to_string(),
            path: session_path.clone(),
        };
        let resume = thread_resume_response(json!("resume-1"), &session).expect("resume response");
        let turns = resume
            .pointer("/result/thread/turns")
            .and_then(Value::as_array)
            .expect("turns");
        assert_eq!(turns.len(), 1);
        assert_eq!(
            resume
                .pointer("/result/thread/turns/0/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("Fix session restore")
        );
        assert_eq!(
            resume
                .pointer("/result/thread/turns/0/items/1/text")
                .and_then(Value::as_str),
            Some("Done")
        );

        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(not(windows))]
fn middleware_script(host_executable: &Path, real_cli_path: &Path) -> String {
    // The macOS Browser native-pipe authorizer validates the connecting Node process and its
    // parent/grandparent signing identities. Keep CodexL out of that chain when node_repl asks the
    // CLI to launch its sandbox or the Browser's dedicated stdio app-server. The main desktop
    // app-server does not use `--listen stdio://`, so it still passes through the middleware.
    format!(
        "#!/bin/sh\nexport {}={}\nexport {}={}\nif [ \"${{1:-}}\" = 'sandbox' ] || {{ [ \"${{1:-}}\" = 'app-server' ] && [ \"${{2:-}}\" = '--listen' ] && [ \"${{3:-}}\" = 'stdio://' ]; }}; then\n  unset {}\n  exec {} \"$@\"\nfi\nexec {} {} \"$@\"\n",
        REAL_CLI_ENV,
        shell_quote(real_cli_path),
        BUNDLED_REAL_CLI_ENV,
        shell_quote(real_cli_path),
        CODEX_CLI_PATH_ENV,
        shell_quote(real_cli_path),
        shell_quote(host_executable),
        RUN_MODE_ARG
    )
}

#[cfg(not(windows))]
fn stdio_export_script(
    host_executable: &Path,
    middleware_path: &Path,
    real_cli_path: &Path,
    log_path: &Path,
    codex_home: Option<&str>,
    workspace_name: Option<&str>,
    profile: Option<&str>,
    model_provider: Option<&str>,
    core_mode: Option<&str>,
    profile_config_format: CodexProfileConfigFormat,
) -> String {
    let mut script = String::from("#!/bin/sh\n");
    let model_provider = normalize_model_provider(model_provider, profile);
    push_shell_export(
        &mut script,
        CODEX_CLI_PATH_ENV,
        &middleware_path.to_string_lossy(),
    );
    push_shell_export(&mut script, REAL_CLI_ENV, &real_cli_path.to_string_lossy());
    push_shell_export(
        &mut script,
        BUNDLED_REAL_CLI_ENV,
        &real_cli_path.to_string_lossy(),
    );
    push_shell_export(&mut script, MIDDLEWARE_LOG_ENV, &log_path.to_string_lossy());
    if let Some(codex_home) = codex_home {
        push_shell_export(&mut script, CODEX_HOME_ENV, codex_home);
    }
    if let Some(workspace_name) = workspace_name {
        push_shell_export(&mut script, CODEX_WORKSPACE_NAME_ENV, workspace_name);
    }
    if let Some(profile) = profile {
        push_shell_export(&mut script, CODEX_PROFILE_ENV, profile);
    }
    if let Some(model_provider) = model_provider.as_deref() {
        push_shell_export(&mut script, CODEX_MODEL_PROVIDER_ENV, model_provider);
    }
    if let Some(core_mode) = core_mode {
        push_shell_export(&mut script, CODEX_CORE_MODE_ENV, core_mode);
    }
    push_shell_export(
        &mut script,
        CODEX_PROFILE_CONFIG_FORMAT_ENV,
        profile_config_format.env_value(),
    );
    script.push_str(&format!(
        "exec {} {} \"$@\"\n",
        shell_quote(host_executable),
        STDIO_RUN_MODE_ARG
    ));
    script
}

#[cfg(not(windows))]
fn push_shell_export(script: &mut String, name: &str, value: &str) {
    script.push_str(&format!("export {}={}\n", name, shell_quote_str(value)));
}

#[cfg(not(windows))]
fn shell_quote(path: &Path) -> String {
    shell_quote_str(&path.to_string_lossy())
}

#[cfg(not(windows))]
fn shell_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
