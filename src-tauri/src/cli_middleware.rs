use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::extensions::builtins::bot_bridge;
use serde_json::{json, Value};

const DISABLE_ENV: &str = "CODEXL_DISABLE_CLI_MIDDLEWARE";
const REAL_CLI_ENV: &str = "CODEXL_REAL_CODEX_CLI_PATH";
const MIDDLEWARE_LOG_ENV: &str = "CODEXL_CLI_MIDDLEWARE_LOG";
pub const CODEX_PROFILE_ENV: &str = "CODEXL_CODEX_PROFILE";
pub const CODEX_MODEL_PROVIDER_ENV: &str = "CODEXL_CODEX_MODEL_PROVIDER";
pub const CODEX_WORKSPACE_NAME_ENV: &str = "CODEXL_CODEX_WORKSPACE_NAME";
const LEGACY_CODEX_INSTANCE_NAME_ENV: &str = "CODEXL_CODEX_INSTANCE_NAME";
const CODEX_CLI_PATH_ENV: &str = "CODEX_CLI_PATH";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const RUN_MODE_ARG: &str = "--codexl-cli-middleware";
const STDIO_RUN_MODE_ARG: &str = "--codexl-cli-stdio";
const BOT_MEDIA_MCP_RUN_MODE_ARG: &str = "--codexl-bot-media-mcp";

type RequestMap = Arc<Mutex<std::collections::HashMap<String, RequestInfo>>>;
type SharedChildStdin = Arc<Mutex<ChildStdin>>;

#[cfg(windows)]
const MIDDLEWARE_FILE_NAME: &str = "codexl-codex-cli-middleware.cmd";
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
}

#[derive(Debug, Clone)]
struct RequestInfo {
    method: String,
    include_token: bool,
}

#[derive(Debug, Clone, Default)]
struct ChatGptAuth {
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
) -> Result<MiddlewareEnv, String> {
    let executable_path = middleware_path();
    let export_stdio_path = stdio_path(stdio_name);
    let default_stdio_path = stdio_path(None);
    let real_cli_path = resolve_real_cli_path(codex_app_executable, &executable_path)?;
    let host_executable = std::env::current_exe().map_err(|e| e.to_string())?;
    write_middleware(&executable_path, &host_executable)?;
    let log_path = default_log_path();
    let codex_home = normalize_profile(codex_home);
    let profile = normalize_profile(codex_profile);
    let workspace_name = normalize_profile(stdio_name).or_else(|| profile.clone());
    let model_provider = normalize_profile(codex_model_provider);
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
    })
}

pub fn run_if_requested() -> bool {
    let mut args = std::env::args_os();
    let _program = args.next();
    let Some(mode) = args.next() else {
        return false;
    };

    let forwarded_args: Vec<OsString> = args.collect();
    let exit_code = match mode.as_os_str() {
        value if value == OsStr::new(RUN_MODE_ARG) => run_stdio_middleware(forwarded_args),
        value if value == OsStr::new(STDIO_RUN_MODE_ARG) => {
            run_stdio_middleware(external_stdio_args(forwarded_args))
        }
        value if value == OsStr::new(BOT_MEDIA_MCP_RUN_MODE_ARG) => {
            bot_bridge::run_bot_media_mcp_stdio()
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

fn resolve_real_cli_path(
    codex_app_executable: &str,
    middleware_path: &Path,
) -> Result<PathBuf, String> {
    let explicit_real_cli = std::env::var(REAL_CLI_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(path) = explicit_real_cli {
        return validate_cli_path(expand_home_path(&path));
    }

    let inherited_cli = std::env::var(CODEX_CLI_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_home_path(&value))
        .filter(|path| !same_path(path, middleware_path));
    if let Some(path) = inherited_cli {
        return validate_cli_path(path);
    }

    bundled_cli_path(codex_app_executable)
        .ok_or_else(|| {
            format!(
                "Could not resolve bundled Codex CLI from Codex app executable: {}",
                codex_app_executable
            )
        })
        .and_then(validate_cli_path)
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
    let contents_dir = executable.parent()?.parent()?;
    let file_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let candidate = contents_dir.join("Resources").join(file_name);
    candidate.is_file().then_some(candidate)
}

fn middleware_path() -> PathBuf {
    codexl_home_dir().join("bin").join(MIDDLEWARE_FILE_NAME)
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
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
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

fn codexl_home_dir() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codexl")
}

fn write_middleware(path: &Path, host_executable: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let content = middleware_script(host_executable);
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

fn run_stdio_middleware_with_io<R, W>(
    args: Vec<OsString>,
    input: R,
    output: W,
) -> Result<i32, String>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let real_cli = std::env::var(REAL_CLI_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} is not set", REAL_CLI_ENV))
        .map(|value| expand_home_path(&value))?;
    validate_cli_path(real_cli.clone())?;

    let profile = std::env::var(CODEX_PROFILE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let model_provider = std::env::var(CODEX_MODEL_PROVIDER_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let real_args = real_cli_args(profile.as_deref(), model_provider.as_deref(), args);
    log_invocation(
        &real_cli,
        profile.as_deref(),
        model_provider.as_deref(),
        &real_args,
    );

    let mut child = Command::new(&real_cli)
        .args(&real_args)
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
    let shared_child_stdin = Arc::new(Mutex::new(child_stdin));
    let bridge_stdout_tx = bot_bridge::spawn_app_stdio_bot_bridge(Arc::clone(&shared_child_stdin));
    let stdin_request_map = Arc::clone(&request_map);
    let stdout_request_map = Arc::clone(&request_map);
    let _stdin_handle =
        thread::spawn(move || copy_stdin_and_track(input, shared_child_stdin, stdin_request_map));
    let stdout_handle = thread::spawn(move || {
        copy_stdout_and_rewrite(
            child_stdout,
            output,
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

fn real_cli_args(
    profile: Option<&str>,
    model_provider: Option<&str>,
    args: Vec<OsString>,
) -> Vec<OsString> {
    let mut real_args = Vec::new();
    if let Some(profile) = profile {
        real_args.push(OsString::from("-c"));
        real_args.push(OsString::from(cli_config_string("profile", profile)));
    }
    if let Some(model_provider) = model_provider {
        real_args.push(OsString::from("-c"));
        real_args.push(OsString::from(cli_config_string(
            "model_provider",
            model_provider,
        )));
    }
    real_args.extend(args);
    real_args
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

fn copy_stdin_and_track<R>(
    reader: R,
    writer: SharedChildStdin,
    request_map: RequestMap,
) -> std::io::Result<u64>
where
    R: Read,
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

        track_request_line(&line, &request_map);
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
    mut writer: W,
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
            writer.write_all(&rewritten)?;
            writer.flush()?;
        }
        copied += size as u64;
    }

    Ok(copied)
}

fn track_request_line(line: &[u8], request_map: &RequestMap) {
    let Ok(value) = serde_json::from_slice::<Value>(trim_json_line(line)) else {
        return;
    };
    let Some(id) = value.get("id").and_then(Value::as_str) else {
        return;
    };
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };

    if !matches!(method, "account/read" | "getAuthStatus") {
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
            },
        );
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
        return line.to_vec();
    }

    match request.method.as_str() {
        "account/read" => value["result"] = chatgpt_auth.account_read_result(),
        "getAuthStatus" => value["result"] = chatgpt_auth.auth_status_result(request.include_token),
        _ => return line.to_vec(),
    }

    let Ok(mut rewritten) = serde_json::to_vec(&value) else {
        return line.to_vec();
    };
    rewritten.extend_from_slice(line_ending(line));
    rewritten
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

    fn account_read_result(&self) -> Value {
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

    fn auth_status_result(&self, include_token: bool) -> Value {
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
fn middleware_script(host_executable: &Path) -> String {
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
) -> String {
    let mut script = String::from("@echo off\r\n");
    push_cmd_env(
        &mut script,
        CODEX_CLI_PATH_ENV,
        &middleware_path.to_string_lossy(),
    );
    push_cmd_env(&mut script, REAL_CLI_ENV, &real_cli_path.to_string_lossy());
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
    if let Some(model_provider) = model_provider {
        push_cmd_env(&mut script, CODEX_MODEL_PROVIDER_ENV, model_provider);
    }
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
        );

        assert!(script.contains("export CODEX_CLI_PATH='/tmp/codexl-codex-cli-middleware'\n"));
        assert!(script.contains("export CODEXL_REAL_CODEX_CLI_PATH='/tmp/Real Codex'\n"));
        assert!(script.contains("export CODEXL_CLI_MIDDLEWARE_LOG='/tmp/middleware.log'\n"));
        assert!(!script.contains("CODEXL_CLI_MIDDLEWARE_STDIN_LOG"));
        assert!(!script.contains("CODEXL_CLI_MIDDLEWARE_STDOUT_LOG"));
        assert!(script.contains("export CODEX_HOME='/tmp/codex home'\n"));
        assert!(script.contains("export CODEXL_CODEX_WORKSPACE_NAME='custom-instance'\n"));
        assert!(script.contains("export CODEXL_CODEX_PROFILE='custom-profile'\n"));
        assert!(script.contains("export CODEXL_CODEX_MODEL_PROVIDER='custom-provider'\n"));
        assert!(script.contains("exec '/tmp/CodexL Host' --codexl-cli-stdio \"$@\"\n"));
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

        track_request_line(
            br#"{"id":"account-id","method":"account/read","params":{"refreshToken":false}}
"#,
            &request_map,
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

        track_request_line(
            br#"{"id":"auth-id","method":"getAuthStatus","params":{"includeToken":true,"refreshToken":false}}
"#,
            &request_map,
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
}

#[cfg(not(windows))]
fn middleware_script(host_executable: &Path) -> String {
    format!(
        "#!/bin/sh\nexec {} {} \"$@\"\n",
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
) -> String {
    let mut script = String::from("#!/bin/sh\n");
    push_shell_export(
        &mut script,
        CODEX_CLI_PATH_ENV,
        &middleware_path.to_string_lossy(),
    );
    push_shell_export(&mut script, REAL_CLI_ENV, &real_cli_path.to_string_lossy());
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
    if let Some(model_provider) = model_provider {
        push_shell_export(&mut script, CODEX_MODEL_PROVIDER_ENV, model_provider);
    }
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
