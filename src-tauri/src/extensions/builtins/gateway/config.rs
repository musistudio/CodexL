use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const NEXT_AI_GATEWAY_PROVIDER_NAME: &str = "next-ai-gateway";
pub const NEXT_AI_GATEWAY_API_KEY: &str = "codexl-next-ai-gateway";

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
    let config = serde_json::from_str::<Value>(&content).map_err(|err| err.to_string())?;
    if !config.is_object() {
        return Err("Gateway config must be a JSON object".to_string());
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

pub fn codex_provider_api_key() -> Result<String, String> {
    let file = read_gateway_config()?;
    Ok(codex_provider_api_key_from_config(&file.config))
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
    let auth = config.get("auth");
    let auth_enabled = auth
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if auth_enabled {
        for keys in [
            auth.and_then(|value| value.get("principals")),
            auth.and_then(|value| value.get("keys")),
            config.get("principals"),
            config.get("keys"),
        ] {
            if let Some(key) = first_gateway_key(keys) {
                return key;
            }
        }
    }

    NEXT_AI_GATEWAY_API_KEY.to_string()
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
    if !config.is_object() {
        return Err("Gateway config must be a JSON object".to_string());
    }

    let path = gateway_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let temp_path = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    std::fs::write(&temp_path, format!("{}\n", content)).map_err(|err| err.to_string())?;
    std::fs::rename(&temp_path, &path).map_err(|err| err.to_string())?;

    Ok(GatewayConfigFile {
        path: path.to_string_lossy().to_string(),
        config,
    })
}

fn ensure_gateway_config_file(path: &PathBuf) -> Result<(), String> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let content =
        serde_json::to_string_pretty(&default_gateway_config()).map_err(|err| err.to_string())?;
    std::fs::write(path, format!("{}\n", content)).map_err(|err| err.to_string())
}

fn gateway_config_path() -> PathBuf {
    env_path("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH")
        .or_else(|| env_path("GATEWAY_CONFIG_PATH"))
        .unwrap_or_else(|| gateway_home_dir().join("gateway.config.json"))
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
        "auth": {
            "enabled": false
        },
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
