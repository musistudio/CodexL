use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};
#[cfg(windows)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

pub const NEXT_AI_GATEWAY_PROVIDER_NAME: &str = "next-ai-gateway";
pub const NEXT_AI_GATEWAY_API_KEY: &str = "codexl-next-ai-gateway";
pub const CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY: &str = "codexl-codex-model-rewrite";
const CODEXL_CODEX_MODEL_REWRITE_NOOP_HEADER: &str = "x-codexl-codex-model-rewrite-noop";
const CODEXL_CODEX_MODEL_REWRITE_TARGET_HEADER: &str = "x-codexl-codex-model-rewrite-target";
pub const GATEWAY_AUTH_CREDENTIAL_HEADER: &str = "x-codexl-gateway-auth";
pub const GATEWAY_AUTH_USER_ID: &str = "codexl";
pub const GATEWAY_AUTH_TENANT_ID: &str = "local";
pub const GATEWAY_AUTH_SUBJECT: &str = "codexl-local";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfigFile {
    pub path: String,
    pub config: Value,
}

pub fn read_gateway_config() -> Result<GatewayConfigFile, String> {
    let path = gateway_config_path();
    ensure_gateway_config_file(&path)?;
    let content = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let mut config = serde_json::from_str::<Value>(&content).map_err(|err| err.to_string())?;
    if !config.is_object() {
        return Err("Gateway config must be a JSON object".to_string());
    }
    let mut changed = ensure_gateway_auth_config(&mut config, None);
    changed |= ensure_codex_model_rewrite_plugin_config(&mut config);
    if changed {
        write_gateway_config_value(&path, &config)?;
    }

    Ok(GatewayConfigFile {
        path: path.to_string_lossy().to_string(),
        config,
    })
}

pub fn codex_provider_base_url() -> Result<String, String> {
    let file = read_gateway_config()?;
    Ok(format!("{}/v1", gateway_origin_from_config(&file.config)))
}

pub fn gateway_health_url() -> Result<String, String> {
    let file = read_gateway_config()?;
    Ok(format!(
        "{}/health",
        gateway_origin_from_config(&file.config)
    ))
}

pub fn gateway_agent_tools_url() -> Result<String, String> {
    let file = read_gateway_config()?;
    Ok(format!(
        "{}/agent/tools",
        gateway_origin_from_config(&file.config)
    ))
}

pub fn codex_provider_api_key() -> Result<String, String> {
    let file = read_gateway_config()?;
    Ok(codex_provider_api_key_from_config(&file.config))
}

pub fn write_codex_model_catalog(selected_model: &str) -> Result<String, String> {
    let mut file = read_gateway_config()?;
    let selected_model = selected_model.trim();
    if !selected_model.is_empty()
        && ensure_codex_model_rewrite_plugin_config_with_target(
            &mut file.config,
            Some(selected_model),
        )
    {
        write_gateway_config_value(&gateway_config_path(), &file.config)?;
    }

    let mut models = Vec::new();
    push_unique_model(&mut models, selected_model);
    for model in gateway_model_options_from_config(&file.config) {
        push_unique_model(&mut models, &model);
    }
    if models.is_empty() {
        return Err("Gateway model catalog requires at least one model".to_string());
    }

    let catalog = json!({
        "models": models
            .iter()
            .enumerate()
            .map(|(index, model)| codex_model_catalog_item(model, index))
            .collect::<Vec<_>>(),
    });
    let path = codex_model_catalog_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let temp_path = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(&catalog).map_err(|err| err.to_string())?;
    std::fs::write(&temp_path, format!("{}\n", content)).map_err(|err| err.to_string())?;
    replace_file(&temp_path, &path)?;

    Ok(path.to_string_lossy().to_string())
}

fn codex_model_catalog_item(model: &str, priority: usize) -> Value {
    json!({
        "slug": model,
        "display_name": model,
        "description": format!("NextAI Gateway model {}", model),
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            { "effort": "low", "description": "Low reasoning" },
            { "effort": "medium", "description": "Medium reasoning" },
            { "effort": "high", "description": "High reasoning" },
            { "effort": "xhigh", "description": "Extra high reasoning" }
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": priority,
        "additional_speed_tiers": [],
        "service_tiers": [],
        "availability_nux": Value::Null,
        "upgrade": Value::Null,
        "base_instructions": "You are Codex, a coding agent.",
        "supports_reasoning_summaries": true,
        "default_reasoning_summary": "none",
        "support_verbosity": true,
        "default_verbosity": "low",
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": "text_and_image",
        "truncation_policy": { "mode": "tokens", "limit": 10000 },
        "supports_parallel_tool_calls": true,
        "supports_image_detail_original": true,
        "context_window": 128000,
        "max_context_window": 128000,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": true
    })
}

pub fn gateway_model_options_from_config(config: &Value) -> Vec<String> {
    let mut models = Vec::new();
    let mut providers = Vec::new();
    if let Some(items) = config.get("Providers").and_then(Value::as_array) {
        providers.extend(items.iter());
    }
    if let Some(items) = config.get("providers").and_then(Value::as_array) {
        providers.extend(items.iter());
    }

    for provider in providers {
        let provider_name = provider
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if provider_name.is_empty() {
            continue;
        }
        for model in gateway_provider_models(provider) {
            let option = gateway_model_option(provider_name, &model);
            push_unique_model(&mut models, &option);
        }
    }

    for model in gateway_virtual_model_options_from_config(config, &models) {
        push_unique_model(&mut models, &model);
    }

    models
}

fn gateway_virtual_model_options_from_config(
    config: &Value,
    base_models: &[String],
) -> Vec<String> {
    let mut models = Vec::new();
    let Some(profiles) = config.get("virtualModelProfiles").and_then(Value::as_array) else {
        return models;
    };

    for profile in profiles {
        if !json_bool(profile.get("enabled"), true) {
            continue;
        }
        let materialization = profile.get("materialization");
        if !json_bool(materialization.and_then(|value| value.get("enabled")), true)
            || !json_bool(
                materialization.and_then(|value| value.get("includeInGatewayModels")),
                true,
            )
        {
            continue;
        }

        let match_config = profile.get("match");
        let prefixes = string_list(match_config.and_then(|value| value.get("prefixes")));
        let suffixes = string_list(match_config.and_then(|value| value.get("suffixes")));
        for base_model in base_models {
            let Some((provider, model)) = base_model.split_once('/') else {
                continue;
            };
            for prefix in &prefixes {
                push_unique_model(&mut models, &format!("{}/{}{}", provider, prefix, model));
            }
            for suffix in &suffixes {
                push_unique_model(&mut models, &format!("{}/{}{}", provider, model, suffix));
            }
        }

        let fixed_model = profile
            .get("baseModel")
            .and_then(|value| value.get("fixedModel"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if fixed_model.is_empty() {
            continue;
        }
        let fixed_provider = fixed_model
            .split_once('/')
            .map(|(provider, _)| provider)
            .unwrap_or_default();
        for alias in string_list(match_config.and_then(|value| value.get("exactAliases"))) {
            let option = if alias.contains('/') {
                alias
            } else {
                gateway_model_option(fixed_provider, &alias)
            };
            push_unique_model(&mut models, &option);
        }
    }

    models
}

fn gateway_provider_models(provider: &Value) -> Vec<String> {
    match provider.get("models") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(gateway_model_name)
            .collect::<Vec<_>>(),
        Some(Value::String(models)) => comma_list(models),
        _ => Vec::new(),
    }
}

fn gateway_model_name(item: &Value) -> Option<String> {
    if let Some(model) = item
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(model.to_string());
    }
    let object = item.as_object()?;
    for field in ["name", "id", "model"] {
        if let Some(model) = object
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(model.to_string());
        }
    }
    None
}

fn gateway_model_option(provider_name: &str, model_name: &str) -> String {
    let provider = provider_name.trim();
    let model = model_name.trim().trim_start_matches('/');
    if provider.is_empty() || model.is_empty() {
        return String::new();
    }
    if model.starts_with(&format!("{}/", provider)) {
        model.to_string()
    } else {
        format!("{}/{}", provider, model)
    }
}

fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(items)) => comma_list(items),
        _ => Vec::new(),
    }
}

fn json_bool(value: Option<&Value>, fallback: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) if value.eq_ignore_ascii_case("true") => true,
        Some(Value::String(value)) if value.eq_ignore_ascii_case("false") => false,
        _ => fallback,
    }
}

fn push_unique_model(models: &mut Vec<String>, model: &str) {
    let model = model.trim();
    if !model.is_empty() && !models.iter().any(|item| item == model) {
        models.push(model.to_string());
    }
}

fn gateway_origin_from_config(config: &Value) -> String {
    let host = config
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1");
    let connect_host = match host {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        value => value,
    };
    let host_part = if connect_host.contains(':') && !connect_host.starts_with('[') {
        format!("[{}]", connect_host)
    } else {
        connect_host.to_string()
    };
    let port = config
        .get("port")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0 && *value <= u16::MAX as u64)
        .unwrap_or(14589);

    format!("http://{}:{}", host_part, port)
}

fn codex_provider_api_key_from_config(config: &Value) -> String {
    first_gateway_key_from_config(config).unwrap_or_else(|| NEXT_AI_GATEWAY_API_KEY.to_string())
}

fn first_gateway_key_from_config(config: &Value) -> Option<String> {
    let auth = config.get("auth");
    for keys in [
        auth.and_then(|value| value.get("principals")),
        auth.and_then(|value| value.get("keys")),
        config.get("principals"),
        config.get("keys"),
    ] {
        if let Some(key) = first_gateway_key(keys) {
            return Some(key);
        }
    }
    None
}

fn first_gateway_key(value: Option<&Value>) -> Option<String> {
    let items = value?.as_array()?;
    for item in items {
        if let Some(key) = item.as_str().map(str::trim).filter(|key| !key.is_empty()) {
            return Some(key.to_string());
        }
        let Some(object) = item.as_object() else {
            continue;
        };
        for field in ["key", "apiKey", "api_key", "token"] {
            if let Some(key) = object
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|key| !key.is_empty())
            {
                return Some(key.to_string());
            }
        }
    }
    None
}

pub fn write_gateway_config(config: Value) -> Result<GatewayConfigFile, String> {
    let mut config = config;
    if !config.is_object() {
        return Err("Gateway config must be a JSON object".to_string());
    }

    let path = gateway_config_path();
    let previous = read_gateway_config_value(&path).ok();
    ensure_gateway_auth_config(&mut config, previous.as_ref());
    ensure_codex_model_rewrite_plugin_config(&mut config);
    write_gateway_config_value(&path, &config)?;

    Ok(GatewayConfigFile {
        path: path.to_string_lossy().to_string(),
        config,
    })
}

fn ensure_gateway_config_file(path: &Path) -> Result<(), String> {
    if path.is_file() {
        #[cfg(windows)]
        tighten_gateway_config_permissions_once(path)?;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content =
        serde_json::to_string_pretty(&default_gateway_config()).map_err(|err| err.to_string())?;
    std::fs::write(path, format!("{}\n", content)).map_err(|err| err.to_string())?;
    tighten_gateway_config_permissions(path)?;
    Ok(())
}

fn read_gateway_config_value(path: &Path) -> Result<Value, String> {
    let content = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let config = serde_json::from_str::<Value>(&content).map_err(|err| err.to_string())?;
    if !config.is_object() {
        return Err("Gateway config must be a JSON object".to_string());
    }
    Ok(config)
}

fn write_gateway_config_value(path: &Path, config: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let temp_path = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(config).map_err(|err| err.to_string())?;
    std::fs::write(&temp_path, format!("{}\n", content)).map_err(|err| err.to_string())?;
    tighten_gateway_config_permissions(&temp_path)?;
    replace_file(&temp_path, path)?;
    tighten_gateway_config_permissions(path)?;
    Ok(())
}

fn gateway_config_path() -> PathBuf {
    env_path("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH")
        .or_else(|| env_path("GATEWAY_CONFIG_PATH"))
        .unwrap_or_else(|| gateway_home_dir().join("gateway.config.json"))
}

fn codex_model_catalog_path() -> PathBuf {
    gateway_home_dir().join("codex-model-catalog.json")
}

pub fn gateway_raw_trace_spool_dirs() -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    if let Some(path) = env_path("RAW_TRACE_SPOOL_DIR") {
        push_unique_path(&mut paths, path);
    }

    // start.js sets RAW_TRACE_SPOOL_DIR to this path before the Gateway bundle
    // reads rawTrace.spoolDir, so it is the runtime default even when the config
    // contains another value.
    push_unique_path(&mut paths, gateway_home_dir().join("raw-trace-spool"));

    let file = read_gateway_config()?;
    if let Some(value) = file
        .config
        .get("rawTrace")
        .and_then(Value::as_object)
        .and_then(|raw_trace| raw_trace.get("spoolDir"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = expand_home_path(value.to_string());
        let path = if path.is_absolute() {
            path
        } else {
            gateway_working_dir().join(path)
        };
        push_unique_path(&mut paths, path);
    }

    push_unique_path(&mut paths, gateway_working_dir().join(".raw-trace-spool"));
    Ok(paths)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn gateway_working_dir() -> PathBuf {
    match super::super::resolve_builtin_next_ai_gateway_extension() {
        Ok(extension) => extension.root_dir,
        Err(_) => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn gateway_home_dir() -> PathBuf {
    env_path("CODEXL_NEXT_AI_GATEWAY_HOME")
        .unwrap_or_else(|| codexl_home_dir().join("next-ai-gateway"))
}

fn codexl_home_dir() -> PathBuf {
    super::super::codexl_home_dir()
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(expand_home_path)
}

fn expand_home_path(value: String) -> PathBuf {
    super::super::expand_home_path(value)
}

fn default_gateway_config() -> Value {
    json!({
        "host": "127.0.0.1",
        "port": 14589,
        "bodyLimitBytes": 52428800,
        "Providers": [],
        "providerPlugins": [
            codex_model_rewrite_plugin_config(None)
        ],
        "auth": default_gateway_auth_config(),
        "billing": {
            "enabled": false
        },
        "billingQueue": {
            "enabled": false
        },
        "billingWebhook": {
            "enabled": false
        },
        "rawTrace": {
            "enabled": false,
            "mode": "disabled"
        },
        "agent": {
            "storage": {
                "type": "filesystem"
            },
            "mcpServers": []
        },
        "mcpGateway": {
            "enabled": false
        }
    })
}

fn codex_model_rewrite_plugin_config(target_model: Option<&str>) -> Value {
    let target_model = target_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut codex_model_rewrite = json!({
        "enabled": true
    });
    if let Some(target_model) = target_model {
        codex_model_rewrite["targetModel"] = json!(target_model);
    }

    json!({
        "key": CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY,
        "enabled": true,
        "codexModelRewrite": codex_model_rewrite,
        "request": codex_model_rewrite_plugin_request_config(target_model)
    })
}

fn codex_model_rewrite_plugin_request_config(target_model: Option<&str>) -> Value {
    let target_model = target_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut headers = serde_json::Map::new();
    headers.insert(
        CODEXL_CODEX_MODEL_REWRITE_NOOP_HEADER.to_string(),
        json!("1"),
    );
    if let Some(target_model) = target_model {
        headers.insert(
            CODEXL_CODEX_MODEL_REWRITE_TARGET_HEADER.to_string(),
            json!(target_model),
        );
    }

    json!({
        "headers": Value::Object(headers),
        "removeHeaders": [
            CODEXL_CODEX_MODEL_REWRITE_NOOP_HEADER,
            CODEXL_CODEX_MODEL_REWRITE_TARGET_HEADER
        ]
    })
}

fn default_gateway_auth_config() -> Value {
    gateway_auth_config_with_key(&random_gateway_api_key())
}

fn gateway_auth_config_with_key(key: &str) -> Value {
    json!({
        "enabled": true,
        "mode": "http_introspection",
        "required": true,
        "keys": [gateway_auth_key_entry(key)],
        "introspection": {
            "tokenHeader": "authorization",
            "tokenBearerOnly": true
        }
    })
}

fn ensure_gateway_auth_config(config: &mut Value, previous: Option<&Value>) -> bool {
    if !config.is_object() {
        return false;
    }

    let key = first_non_default_gateway_key(config)
        .or_else(|| previous.and_then(first_non_default_gateway_key))
        .unwrap_or_else(random_gateway_api_key);

    let object = config.as_object_mut().expect("checked object");
    let auth = object.entry("auth").or_insert_with(|| json!({}));
    let mut changed = false;
    if !auth.is_object() {
        *auth = json!({});
        changed = true;
    }
    let auth_object = auth.as_object_mut().expect("checked object");

    changed |= set_json_bool(auth_object, "enabled", true);
    changed |= set_json_string(auth_object, "mode", "http_introspection");
    changed |= set_json_bool(auth_object, "required", true);

    let keys_need_update = auth_object
        .get("keys")
        .and_then(|value| first_gateway_key(Some(value)))
        .map(|value| value == NEXT_AI_GATEWAY_API_KEY)
        .unwrap_or(true);
    if keys_need_update {
        auth_object.insert("keys".to_string(), json!([gateway_auth_key_entry(&key)]));
        changed = true;
    }

    let introspection = auth_object
        .entry("introspection")
        .or_insert_with(|| json!({}));
    if !introspection.is_object() {
        *introspection = json!({});
        changed = true;
    }
    let introspection_object = introspection.as_object_mut().expect("checked object");
    changed |= set_json_string(introspection_object, "tokenHeader", "authorization");
    changed |= set_json_bool(introspection_object, "tokenBearerOnly", true);

    changed
}

fn ensure_codex_model_rewrite_plugin_config(config: &mut Value) -> bool {
    ensure_codex_model_rewrite_plugin_config_with_target(config, None)
}

fn ensure_codex_model_rewrite_plugin_config_with_target(
    config: &mut Value,
    target_model: Option<&str>,
) -> bool {
    if !config.is_object() {
        return false;
    }
    let target_model = target_model
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let object = config.as_object_mut().expect("checked object");
    let provider_plugins = object
        .entry("providerPlugins")
        .or_insert_with(|| Value::Array(Vec::new()));
    let mut changed = false;
    if !provider_plugins.is_array() {
        *provider_plugins = Value::Array(Vec::new());
        changed = true;
    }

    let plugins = provider_plugins.as_array_mut().expect("checked array");
    for plugin in plugins.iter_mut() {
        if plugin
            .get("key")
            .and_then(Value::as_str)
            .map(|key| key == CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY)
            .unwrap_or(false)
        {
            if !plugin.is_object() {
                *plugin = codex_model_rewrite_plugin_config(target_model);
                return true;
            }

            let existing_target_model =
                target_model.map(str::to_string).or_else(|| {
                    codex_model_rewrite_target_model_from_plugin(plugin)
                });
            let plugin_object = plugin.as_object_mut().expect("checked object");
            changed |= set_json_string(
                plugin_object,
                "key",
                CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY,
            );
            changed |= set_json_bool(plugin_object, "enabled", true);
            if plugin_object.remove("provider").is_some() {
                changed = true;
            }
            if plugin_object.remove("providerName").is_some() {
                changed = true;
            }

            let rewrite = plugin_object
                .entry("codexModelRewrite")
                .or_insert_with(|| json!({}));
            if !rewrite.is_object() {
                *rewrite = json!({});
                changed = true;
            }
            let rewrite_object = rewrite.as_object_mut().expect("checked object");
            changed |= set_json_bool(rewrite_object, "enabled", true);
            if let Some(target_model) = existing_target_model.as_deref() {
                changed |= set_json_string(rewrite_object, "targetModel", target_model);
            }

            let request = codex_model_rewrite_plugin_request_config(existing_target_model.as_deref());
            if plugin_object.get("request") != Some(&request) {
                plugin_object.insert("request".to_string(), request);
                changed = true;
            }
            return changed;
        }
    }

    plugins.push(codex_model_rewrite_plugin_config(target_model));
    true
}

fn codex_model_rewrite_target_model_from_plugin(plugin: &Value) -> Option<String> {
    plugin
        .get("request")
        .and_then(Value::as_object)
        .and_then(|request| request.get("headers"))
        .and_then(Value::as_object)
        .and_then(|headers| headers.get(CODEXL_CODEX_MODEL_REWRITE_TARGET_HEADER))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            plugin
                .get("codexModelRewrite")
                .and_then(Value::as_object)
                .and_then(|rewrite| rewrite.get("targetModel"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn first_non_default_gateway_key(config: &Value) -> Option<String> {
    first_gateway_key_from_config(config).filter(|key| key != NEXT_AI_GATEWAY_API_KEY)
}

fn gateway_auth_key_entry(key: &str) -> Value {
    json!({
        "key": key,
        "userId": GATEWAY_AUTH_USER_ID,
        "tenantId": GATEWAY_AUTH_TENANT_ID,
        "subject": GATEWAY_AUTH_SUBJECT
    })
}

fn set_json_bool(object: &mut serde_json::Map<String, Value>, key: &str, value: bool) -> bool {
    if object.get(key).and_then(Value::as_bool) == Some(value) {
        return false;
    }
    object.insert(key.to_string(), Value::Bool(value));
    true
}

fn set_json_string(object: &mut serde_json::Map<String, Value>, key: &str, value: &str) -> bool {
    if object.get(key).and_then(Value::as_str) == Some(value) {
        return false;
    }
    object.insert(key.to_string(), Value::String(value.to_string()));
    true
}

fn random_gateway_api_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut key = String::from("cxl_gw_");
    for byte in bytes {
        key.push_str(&format!("{:02x}", byte));
    }
    key
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    std::fs::rename(temp_path, path).map_err(|err| err.to_string())
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return std::fs::rename(temp_path, path).map_err(|err| err.to_string());
    }

    let backup_path = path.with_extension("json.bak");
    let _ = std::fs::remove_file(&backup_path);
    std::fs::rename(path, &backup_path).map_err(|err| {
        format!(
            "failed to prepare Windows file replacement for {}: {}",
            path.display(),
            err
        )
    })?;

    match std::fs::rename(temp_path, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup_path);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::rename(&backup_path, path);
            Err(format!(
                "failed to replace {} on Windows: {}",
                path.display(),
                err
            ))
        }
    }
}

fn tighten_gateway_config_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|err| err.to_string())?
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|err| err.to_string())?;
        Ok(())
    }

    #[cfg(windows)]
    {
        tighten_gateway_config_permissions_windows(path)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
fn tighten_gateway_config_permissions_once(path: &Path) -> Result<(), String> {
    static SECURED_GATEWAY_CONFIG_PATHS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

    let cache_key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let secured_paths = SECURED_GATEWAY_CONFIG_PATHS.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let guard = secured_paths
            .lock()
            .map_err(|_| "Gateway config permission cache is poisoned".to_string())?;
        if guard.contains(&cache_key) {
            return Ok(());
        }
    }

    tighten_gateway_config_permissions(path)?;

    let mut guard = secured_paths
        .lock()
        .map_err(|_| "Gateway config permission cache is poisoned".to_string())?;
    guard.insert(cache_key);
    Ok(())
}

#[cfg(windows)]
fn tighten_gateway_config_permissions_windows(path: &Path) -> Result<(), String> {
    let script = r#"
$ErrorActionPreference = 'Stop'
$target = $args[0]
if ([string]::IsNullOrWhiteSpace($target)) {
    throw 'missing Gateway config path'
}

$item = Get-Item -LiteralPath $target -Force
$current = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$system = [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$admins = [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$acl = [System.Security.AccessControl.FileSecurity]::new()
$acl.SetOwner($current)
$acl.SetAccessRuleProtection($true, $false)
$rights = [System.Security.AccessControl.FileSystemRights]::FullControl
$inheritance = [System.Security.AccessControl.InheritanceFlags]::None
$propagation = [System.Security.AccessControl.PropagationFlags]::None
$type = [System.Security.AccessControl.AccessControlType]::Allow

foreach ($sid in @($current, $system, $admins)) {
    $rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
        $sid,
        $rights,
        $inheritance,
        $propagation,
        $type
    )
    $acl.AddAccessRule($rule)
}

Set-Acl -LiteralPath $item.FullName -AclObject $acl
"#;

    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let output = command.output().map_err(|err| {
        format!(
            "failed to secure Gateway config permissions on Windows: {}",
            err
        )
    })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err("failed to secure Gateway config permissions on Windows".to_string())
    } else {
        Err(format!(
            "failed to secure Gateway config permissions on Windows: {}",
            stderr
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_gateway_model_catalog_advertises_image_and_search_capabilities() {
        let model = codex_model_catalog_item("Provider/model", 0);

        assert_eq!(model["input_modalities"], json!(["text", "image"]));
        assert_eq!(model["supports_image_detail_original"], json!(true));
        assert_eq!(model["supports_search_tool"], json!(true));
        assert_eq!(model["web_search_tool_type"], json!("text_and_image"));
    }

    #[test]
    fn gateway_model_options_materialize_virtual_model_profiles() {
        let config = json!({
            "Providers": [
                {
                    "name": "openai",
                    "models": ["gpt-4.1"]
                }
            ],
            "virtualModelProfiles": [
                {
                    "id": "vision-search",
                    "key": "vision-search",
                    "displayName": "Vision + Search",
                    "enabled": true,
                    "match": {
                        "suffixes": [":vision-search"],
                        "prefixes": ["vision:"],
                        "exactAliases": ["openai/search-fixed", "search-fixed-short"]
                    },
                    "baseModel": {
                        "fixedModel": "openai/gpt-4.1"
                    },
                    "materialization": {
                        "enabled": true,
                        "includeInGatewayModels": true
                    }
                }
            ]
        });

        assert_eq!(
            gateway_model_options_from_config(&config),
            vec![
                "openai/gpt-4.1",
                "openai/vision:gpt-4.1",
                "openai/gpt-4.1:vision-search",
                "openai/search-fixed",
                "openai/search-fixed-short",
            ]
        );
    }

    #[test]
    fn gateway_config_adds_codex_model_rewrite_plugin() {
        let mut config = json!({
            "Providers": []
        });

        assert!(ensure_codex_model_rewrite_plugin_config(&mut config));
        assert_eq!(
            config["providerPlugins"][0]["key"],
            json!(CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY)
        );
        assert_eq!(config["providerPlugins"][0]["enabled"], json!(true));
        assert_eq!(
            config["providerPlugins"][0]["codexModelRewrite"]["enabled"],
            json!(true)
        );
        assert_eq!(
            config["providerPlugins"][0]["request"],
            codex_model_rewrite_plugin_request_config(None)
        );
    }

    #[test]
    fn gateway_config_refreshes_codex_model_rewrite_plugin_without_removing_user_plugins() {
        let mut config = json!({
            "providerPlugins": [
                {
                    "key": "custom-plugin",
                    "enabled": true,
                    "request": {
                        "headers": {
                            "x-custom": "1"
                        }
                    }
                },
                {
                    "key": CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY,
                    "enabled": false,
                    "providerName": "old-provider",
                    "codexModelRewrite": {
                        "enabled": false,
                        "targetModel": "zhipu/glm-4.6"
                    }
                }
            ]
        });

        assert!(ensure_codex_model_rewrite_plugin_config(&mut config));

        let plugins = config["providerPlugins"].as_array().expect("plugins array");
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0]["key"], json!("custom-plugin"));
        assert_eq!(plugins[1]["key"], json!(CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY));
        assert_eq!(plugins[1]["enabled"], json!(true));
        assert_eq!(plugins[1].get("providerName"), None);
        assert_eq!(plugins[1]["codexModelRewrite"]["enabled"], json!(true));
        assert_eq!(
            plugins[1]["codexModelRewrite"]["targetModel"],
            json!("zhipu/glm-4.6")
        );
        assert_eq!(
            plugins[1]["request"],
            codex_model_rewrite_plugin_request_config(Some("zhipu/glm-4.6"))
        );
    }

    #[test]
    fn gateway_config_sets_codex_model_rewrite_target_model() {
        let mut config = json!({
            "providerPlugins": [
                {
                    "key": CODEXL_CODEX_MODEL_REWRITE_PLUGIN_KEY,
                    "enabled": true,
                    "request": {
                        "headers": {
                            "x-codexl-codex-model-rewrite-noop": "1"
                        },
                        "removeHeaders": ["x-codexl-codex-model-rewrite-noop"]
                    }
                }
            ]
        });

        assert!(ensure_codex_model_rewrite_plugin_config_with_target(
            &mut config,
            Some("moonshot/kimi-k2")
        ));

        let plugin = &config["providerPlugins"][0];
        assert_eq!(
            plugin["codexModelRewrite"]["targetModel"],
            json!("moonshot/kimi-k2")
        );
        assert_eq!(
            plugin["request"],
            codex_model_rewrite_plugin_request_config(Some("moonshot/kimi-k2"))
        );
    }

    #[test]
    fn gateway_auth_config_generates_local_api_key() {
        let mut config = json!({
            "auth": {
                "enabled": false
            }
        });

        assert!(ensure_gateway_auth_config(&mut config, None));
        let key = codex_provider_api_key_from_config(&config);

        assert_eq!(config["auth"]["enabled"], json!(true));
        assert_eq!(config["auth"]["mode"], json!("http_introspection"));
        assert_eq!(config["auth"]["required"], json!(true));
        assert_eq!(
            config["auth"]["introspection"]["tokenHeader"],
            json!("authorization")
        );
        assert_eq!(
            config["auth"]["introspection"]["tokenBearerOnly"],
            json!(true)
        );
        assert_ne!(key, NEXT_AI_GATEWAY_API_KEY);
        assert!(key.starts_with("cxl_gw_"));
        assert_eq!(config["auth"]["keys"][0]["key"], json!(key));
    }

    #[test]
    fn gateway_auth_config_preserves_previous_key_when_rewritten() {
        let previous = json!({
            "auth": {
                "enabled": true,
                "keys": [
                    {
                        "key": "cxl_gw_existing"
                    }
                ]
            }
        });
        let mut config = json!({
            "Providers": []
        });

        assert!(ensure_gateway_auth_config(&mut config, Some(&previous)));

        assert_eq!(
            codex_provider_api_key_from_config(&config),
            "cxl_gw_existing"
        );
    }

    #[test]
    fn replace_file_overwrites_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "codexl-gateway-replace-file-{}-{}",
            std::process::id(),
            random_gateway_api_key()
        ));
        std::fs::create_dir_all(&root).expect("create test dir");
        let target = root.join("gateway.config.json");
        let temp = root.join("gateway.config.json.tmp");
        std::fs::write(&target, "old").expect("write target");
        std::fs::write(&temp, "new").expect("write temp");

        replace_file(&temp, &target).expect("replace file");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read target"),
            "new"
        );
        assert!(!temp.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
