use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::extensions::{self, builtins::gateway::config as gateway_config};

pub const DEFAULT_PROVIDER_PROFILE_NAME: &str = "Default";
pub const REMOTE_FRONTEND_MODE_APP: &str = "app";
pub const REMOTE_FRONTEND_MODE_CLI: &str = "cli";
pub const REMOTE_FRONTEND_MODE_CLAUDE_CODE: &str = "claude-code";
pub const BOT_PLATFORM_NONE: &str = "none";
pub const BOT_PLATFORM_SLACK: &str = "slack";
pub const BOT_PLATFORM_DISCORD: &str = "discord";
pub const BOT_PLATFORM_TELEGRAM: &str = "telegram";
pub const BOT_PLATFORM_LINE: &str = "line";
pub const BOT_PLATFORM_FEISHU: &str = "feishu";
pub const BOT_PLATFORM_DINGTALK: &str = "dingtalk";
pub const BOT_PLATFORM_WEIXIN_ILINK: &str = "weixin-ilink";
pub const BOT_PLATFORM_WECOM: &str = "wecom";
pub const BOT_AUTH_APP_SECRET: &str = "app_secret";
pub const BOT_AUTH_BOT_TOKEN: &str = "bot_token";
pub const BOT_AUTH_OAUTH2: &str = "oauth2";
pub const BOT_AUTH_QR_LOGIN: &str = "qr_login";
pub const BOT_AUTH_WEBHOOK_SECRET: &str = "webhook_secret";
pub const DEFAULT_BOT_TENANT_ID: &str = "demo";
const BOT_MEDIA_MCP_RUN_MODE_ARG: &str = "--codexl-bot-media-mcp";
const BOT_MEDIA_MCP_SERVER_NAME: &str = "codexl_bot";
const LEGACY_BOT_MEDIA_MCP_SERVER_NAME: &str = "codexl_bot_media";
const BOT_MEDIA_MCP_MANAGED_BEGIN: &str = "# BEGIN CODEXL BOT MEDIA MCP";
const BOT_MEDIA_MCP_MANAGED_END: &str = "# END CODEXL BOT MEDIA MCP";
const RETIRED_DOUBAO_IME_MCP_SERVER_NAME: &str = "codexl_doubao_ime";
const RETIRED_DOUBAO_IME_MCP_MANAGED_BEGIN: &str = "# BEGIN CODEXL DOUBAO IME MCP";
const RETIRED_DOUBAO_IME_MCP_MANAGED_END: &str = "# END CODEXL DOUBAO IME MCP";
const BOT_MEDIA_MCP_OPTIONAL_ENV_NAMES: &[&str] = &[
    "CODEXL_BOT_GATEWAY_LOG",
    "CODEXL_BUILTIN_BOT_GATEWAY_ENTRY",
    "CODEXL_BUILTIN_BOT_GATEWAY_PACKAGE",
    "CODEXL_BUILTIN_BOT_GATEWAY_PACKAGE_URL",
    "CODEXL_BUILTIN_BOT_GATEWAY_UPDATE_MANIFEST_URL",
    "CODEXL_NODE_PATH",
    "CODEXL_NODE_DIST_BASE_URL",
];
const DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE: &str = "profile";
const DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL: &str = "top_level";
const CODEX_PROFILE_CONFIG_FORMAT_ENV: &str = "CODEXL_CODEX_PROFILE_CONFIG_FORMAT";
const CODEX_PROFILE_CONFIG_FORMAT_LEGACY: &str = "legacy";
const CODEX_PROFILE_CONFIG_FORMAT_SEPARATE: &str = "separate_profile_files";
const CODEX_PROFILE_CONFIG_FILES_MIN_VERSION: (u64, u64, u64) = (0, 134, 0);
static UUID_FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexProfileConfigFormat {
    LegacyProfilesTable,
    SeparateProfileFiles,
}

impl CodexProfileConfigFormat {
    pub fn env_value(self) -> &'static str {
        match self {
            Self::LegacyProfilesTable => CODEX_PROFILE_CONFIG_FORMAT_LEGACY,
            Self::SeparateProfileFiles => CODEX_PROFILE_CONFIG_FORMAT_SEPARATE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub codex_profile_name: String,
    pub provider_name: String,
    pub provider_config_format: String,
    pub base_url: String,
    pub model: String,
    pub proxy_url: String,
    pub remote_frontend_mode: String,
    pub remote_web_asset_registry_url: String,
    pub remote_web_asset_version: String,
    pub codex_home: String,
    pub start_remote_on_launch: bool,
    pub start_remote_cloud_on_launch: bool,
    pub start_remote_e2ee_on_launch: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote_e2ee_password: String,
    pub bot: BotProfileConfig,
}

impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            codex_profile_name: String::new(),
            provider_name: String::new(),
            provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
            base_url: String::new(),
            model: String::new(),
            proxy_url: String::new(),
            remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: "latest".to_string(),
            codex_home: String::new(),
            start_remote_on_launch: false,
            start_remote_cloud_on_launch: false,
            start_remote_e2ee_on_launch: false,
            remote_e2ee_password: String::new(),
            bot: BotProfileConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProviderRequest {
    pub workspace_name: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NextAiGatewayProviderRequest {
    pub workspace_name: String,
    pub name: String,
    pub model: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNextAiGatewayProviderRequest {
    pub original_name: String,
    pub workspace_name: String,
    pub name: String,
    pub model: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExistingProviderRequest {
    pub workspace_name: String,
    pub profile_name: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProviderRequest {
    pub original_name: String,
    pub workspace_name: String,
    pub profile_name: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceRequest {
    pub workspace_name: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWorkspaceRequest {
    pub original_name: String,
    pub workspace_name: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub remote_frontend_mode: String,
    #[serde(default)]
    pub remote_web_asset_registry_url: String,
    #[serde(default)]
    pub remote_web_asset_version: String,
    #[serde(default)]
    pub bot: BotProfileConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BotProfileConfig {
    pub enabled: bool,
    pub platform: String,
    pub auth_type: String,
    pub auth_fields: BTreeMap<String, String>,
    pub forward_all_codex_messages: bool,
    pub handoff: BotHandoffConfig,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub saved_config_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tenant_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub integration_id: String,
    #[serde(skip_serializing)]
    pub project_dir: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state_dir: String,
    #[serde(skip_serializing)]
    pub codex_cwd: String,
    pub status: String,
    pub last_login_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BotHandoffConfig {
    pub enabled: bool,
    pub idle_seconds: u64,
    pub screen_lock: bool,
    pub user_idle: bool,
    pub phone_wifi_targets: Vec<String>,
    pub phone_bluetooth_targets: Vec<String>,
}

impl Default for BotHandoffConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_seconds: 30,
            screen_lock: true,
            user_idle: true,
            phone_wifi_targets: Vec::new(),
            phone_bluetooth_targets: Vec::new(),
        }
    }
}

impl BotHandoffConfig {
    pub fn normalize(&mut self) {
        self.idle_seconds = self.idle_seconds.clamp(30, 86_400);
        self.phone_wifi_targets =
            normalize_handoff_targets(std::mem::take(&mut self.phone_wifi_targets));
        self.phone_bluetooth_targets =
            normalize_handoff_targets(std::mem::take(&mut self.phone_bluetooth_targets));
        self.phone_wifi_targets.truncate(1);
        self.phone_bluetooth_targets.truncate(1);
    }
}

impl Default for BotProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            platform: BOT_PLATFORM_NONE.to_string(),
            auth_type: String::new(),
            auth_fields: BTreeMap::new(),
            forward_all_codex_messages: false,
            handoff: BotHandoffConfig::default(),
            saved_config_id: String::new(),
            tenant_id: String::new(),
            integration_id: String::new(),
            project_dir: String::new(),
            state_dir: String::new(),
            codex_cwd: String::new(),
            status: String::new(),
            last_login_at: String::new(),
        }
    }
}

impl BotProfileConfig {
    pub fn normalize_for_profile(&mut self, profile_name: &str) {
        self.normalize_for_profile_instance(profile_name, "");
    }

    pub fn normalize_for_profile_instance(&mut self, profile_name: &str, instance_id: &str) {
        self.saved_config_id = self.saved_config_id.trim().to_string();
        self.platform = normalize_bot_platform(&self.platform);
        self.auth_type = normalize_bot_auth_type(&self.platform, &self.auth_type);
        let auth_fields = std::mem::take(&mut self.auth_fields)
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .filter(|(key, value)| !key.is_empty() && !value.is_empty())
            .collect();
        self.auth_fields = normalize_bot_auth_fields(&self.platform, auth_fields);
        let profile_name = profile_name.trim();
        let instance_id = instance_id.trim();
        self.status = self.status.trim().to_string();
        self.last_login_at = self.last_login_at.trim().to_string();
        self.handoff.normalize();
        self.project_dir.clear();
        self.codex_cwd.clear();
        let saved_config_selected = !self.saved_config_id.is_empty();
        let explicit_tenant_id = self.tenant_id.trim().to_string();
        let explicit_integration_id = self.integration_id.trim().to_string();
        let explicit_state_dir = normalize_home_path(&self.state_dir);

        if !self.enabled || self.platform == BOT_PLATFORM_NONE {
            self.enabled = false;
            self.platform = BOT_PLATFORM_NONE.to_string();
            self.auth_type.clear();
            self.forward_all_codex_messages = false;
            self.handoff.enabled = false;
            self.saved_config_id.clear();
            self.tenant_id.clear();
            self.integration_id.clear();
            self.state_dir.clear();
            return;
        }

        let fallback_tenant_id = if profile_name.is_empty() {
            DEFAULT_BOT_TENANT_ID.to_string()
        } else {
            profile_name.to_string()
        };

        if saved_config_selected {
            self.tenant_id = if explicit_tenant_id.is_empty() {
                fallback_tenant_id
            } else {
                explicit_tenant_id
            };
            self.integration_id = if !explicit_integration_id.is_empty() {
                explicit_integration_id
            } else if is_uuid_like(instance_id) {
                instance_id.to_string()
            } else {
                new_uuid_v4()
            };
            self.state_dir = explicit_state_dir;
        } else {
            self.tenant_id = fallback_tenant_id;
            self.integration_id = if is_uuid_like(instance_id) {
                instance_id.to_string()
            } else if is_uuid_like(&self.integration_id) {
                self.integration_id.trim().to_string()
            } else {
                new_uuid_v4()
            };
            self.state_dir.clear();
        }
    }

    pub fn normalize_for_saved_config(&mut self, fallback_name: &str) {
        if self.saved_config_id.trim().is_empty() {
            self.saved_config_id = self.integration_id.trim().to_string();
        }
        if self.saved_config_id.trim().is_empty() {
            self.saved_config_id = new_uuid_v4();
        }
        self.normalize_for_profile_instance(fallback_name, "");
        self.forward_all_codex_messages = false;
        self.handoff = BotHandoffConfig::default();
        if self.bridge_enabled() && self.saved_config_id.trim().is_empty() {
            self.saved_config_id = self.integration_id.trim().to_string();
        }
    }

    pub fn bridge_enabled(&self) -> bool {
        self.enabled && self.platform != BOT_PLATFORM_NONE
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedBotConfig {
    pub id: String,
    pub name: String,
    pub bot: BotProfileConfig,
    pub updated_at: String,
}

impl Default for SavedBotConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            bot: BotProfileConfig::default(),
            updated_at: String::new(),
        }
    }
}

fn normalize_handoff_targets(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        for part in value.split('\n') {
            let part = part.trim();
            if part.is_empty() || normalized.iter().any(|existing| existing == part) {
                continue;
            }
            normalized.push(part.to_string());
        }
    }
    normalized
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultProviderProfile {
    pub name: String,
    pub provider_name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub config_format: String,
}

impl Default for DefaultProviderProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            provider_name: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
        }
    }
}

impl DefaultProviderProfile {
    fn to_provider_profile(&self) -> ProviderProfile {
        ProviderProfile {
            id: String::new(),
            name: self.name.clone(),
            codex_profile_name: if uses_top_level_provider_config(self) {
                DEFAULT_PROVIDER_PROFILE_NAME.to_string()
            } else {
                self.name.clone()
            },
            provider_name: self.provider_name.clone(),
            provider_config_format: self.config_format.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            proxy_url: String::new(),
            remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: "latest".to_string(),
            codex_home: String::new(),
            start_remote_on_launch: false,
            start_remote_cloud_on_launch: false,
            start_remote_e2ee_on_launch: false,
            remote_e2ee_password: String::new(),
            bot: BotProfileConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexHomeProfile {
    pub name: String,
    pub path: String,
}

impl Default for CodexHomeProfile {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            path: default_codex_home(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionSettings {
    pub enabled: bool,
    pub bot_gateway_enabled: bool,
    pub next_ai_gateway_enabled: bool,
}

impl Default for ExtensionSettings {
    fn default() -> Self {
        Self {
            enabled: env_bool("CODEXL_EXTENSIONS_ENABLED", false),
            bot_gateway_enabled: false,
            next_ai_gateway_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub cdp_host: String,
    pub cdp_port: u16,
    pub http_host: String,
    pub http_port: u16,
    pub remote_control_host: String,
    pub remote_control_port: u16,
    pub remote_relay_url: String,
    pub remote_web_asset_registry_url: String,
    pub remote_web_asset_version: String,
    pub remote_transcribe_base_url: String,
    pub remote_transcribe_api_url: String,
    pub remote_transcribe_api_key: String,
    pub remote_transcribe_model: String,
    pub device_uuid: String,
    pub remote_cloud_auth: RemoteCloudAuthConfig,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub remote_control_tokens: BTreeMap<String, String>,
    pub language: String,
    pub appearance: String,
    pub codex_path: String,
    pub codex_home: String,
    pub codex_home_profiles: Vec<CodexHomeProfile>,
    pub active_provider: String,
    pub provider_profiles: Vec<ProviderProfile>,
    pub bot_configs: Vec<SavedBotConfig>,
    pub auto_launch: bool,
    pub extensions: ExtensionSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        let codex_home = default_codex_home();
        let legacy_transcribe_api_url = env_string("CODEXL_REMOTE_TRANSCRIBE_API_URL", "");
        Self {
            cdp_host: env_string("CODEXL_CDP_HOST", "127.0.0.1"),
            cdp_port: env_u16("CODEXL_CDP_PORT", 9222),
            http_host: env_string("CODEXL_HTTP_HOST", "0.0.0.0"),
            http_port: env_u16("CODEXL_HTTP_PORT", 14588),
            remote_control_host: env_string("CODEXL_REMOTE_CONTROL_HOST", "0.0.0.0"),
            remote_control_port: env_u16("CODEXL_REMOTE_CONTROL_PORT", 3147),
            remote_relay_url: env_string("CODEXL_REMOTE_RELAY_URL", ""),
            remote_web_asset_registry_url: env_string("CODEXL_REMOTE_WEB_ASSET_REGISTRY_URL", ""),
            remote_web_asset_version: env_string("CODEXL_REMOTE_WEB_ASSET_VERSION", "latest"),
            remote_transcribe_base_url: env_string(
                "CODEXL_REMOTE_TRANSCRIBE_BASE_URL",
                &legacy_transcribe_api_url,
            ),
            remote_transcribe_api_url: legacy_transcribe_api_url,
            remote_transcribe_api_key: env_string("CODEXL_REMOTE_TRANSCRIBE_API_KEY", ""),
            remote_transcribe_model: env_string(
                "CODEXL_REMOTE_TRANSCRIBE_MODEL",
                "gpt-4o-mini-transcribe",
            ),
            device_uuid: env_string("CODEXL_DEVICE_UUID", ""),
            remote_cloud_auth: RemoteCloudAuthConfig::from_env(),
            remote_control_tokens: BTreeMap::new(),
            language: env_string("CODEXL_LANGUAGE", "en"),
            appearance: env_string("CODEXL_APPEARANCE", "system"),
            codex_path: std::env::var("CODEXL_CODEX_PATH").unwrap_or_default(),
            codex_home: codex_home.clone(),
            codex_home_profiles: vec![CodexHomeProfile {
                name: "Default".to_string(),
                path: codex_home,
            }],
            active_provider: String::new(),
            provider_profiles: vec![default_provider_profile()],
            bot_configs: Vec::new(),
            auto_launch: env_bool("CODEXL_AUTO_LAUNCH", false),
            extensions: ExtensionSettings::default(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        let first_launch = !path.exists();
        let mut config = std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<AppConfig>(&content).ok())
            .unwrap_or_default();
        if first_launch {
            config.apply_initial_extension_defaults();
        }
        config.normalize();
        let _ = config.save();
        config
    }

    fn apply_initial_extension_defaults(&mut self) {
        if !extensions::user_node_runtime_available() {
            return;
        }
        self.extensions.enabled = true;
        self.extensions.bot_gateway_enabled = true;
        self.extensions.next_ai_gateway_enabled = true;
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, content).map_err(|e| e.to_string())
    }

    pub fn normalize(&mut self) {
        self.remote_relay_url = self
            .remote_relay_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        self.remote_web_asset_registry_url = self
            .remote_web_asset_registry_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        self.remote_web_asset_version =
            normalized_remote_web_asset_version(&self.remote_web_asset_version);
        self.remote_transcribe_base_url =
            normalize_transcribe_base_url(&self.remote_transcribe_base_url);
        self.remote_transcribe_api_url =
            normalize_transcribe_base_url(&self.remote_transcribe_api_url);
        if self.remote_transcribe_base_url.is_empty() && !self.remote_transcribe_api_url.is_empty()
        {
            self.remote_transcribe_base_url = self.remote_transcribe_api_url.clone();
        }
        self.remote_transcribe_api_url = self.remote_transcribe_base_url.clone();
        self.remote_transcribe_api_key = self.remote_transcribe_api_key.trim().to_string();
        self.remote_transcribe_model = self.remote_transcribe_model.trim().to_string();
        if self.remote_transcribe_model.is_empty() {
            self.remote_transcribe_model = "gpt-4o-mini-transcribe".to_string();
        }
        self.device_uuid = self.device_uuid.trim().to_ascii_lowercase();
        if !is_uuid_like(&self.device_uuid) {
            self.device_uuid = new_uuid_v4();
        }
        self.remote_cloud_auth.normalize();
        self.language = match self.language.trim().to_lowercase().as_str() {
            "zh" | "zh-cn" | "chinese" => "zh".to_string(),
            _ => "en".to_string(),
        };
        self.appearance = match self.appearance.trim().to_lowercase().as_str() {
            "light" => "light".to_string(),
            "dark" => "dark".to_string(),
            _ => "system".to_string(),
        };
        let env_codex_path = std::env::var("CODEXL_CODEX_PATH")
            .ok()
            .map(|path| normalize_home_path(&path))
            .filter(|path| !path.trim().is_empty());
        if let Some(path) = env_codex_path {
            self.codex_path = path;
        } else {
            self.codex_path = normalize_home_path(&self.codex_path);
        }

        self.codex_home = normalize_home_path(&self.codex_home);
        if self.codex_home.is_empty() {
            self.codex_home = default_codex_home();
        }

        for profile in &mut self.codex_home_profiles {
            profile.name = profile.name.trim().to_string();
            profile.path = normalize_home_path(&profile.path);
            if profile.name.is_empty() {
                profile.name = profile.path.clone();
            }
        }

        self.codex_home_profiles
            .retain(|profile| !profile.path.is_empty());

        if !self
            .codex_home_profiles
            .iter()
            .any(|profile| profile.path == self.codex_home)
        {
            self.codex_home_profiles.push(CodexHomeProfile {
                name: "Current".to_string(),
                path: self.codex_home.clone(),
            });
        }

        let mut deduped = Vec::new();
        for profile in self.codex_home_profiles.drain(..) {
            if !deduped
                .iter()
                .any(|existing: &CodexHomeProfile| existing.path == profile.path)
            {
                deduped.push(profile);
            }
        }
        self.codex_home_profiles = deduped;

        self.provider_profiles =
            dedupe_provider_profiles(std::mem::take(&mut self.provider_profiles));
        if !self
            .provider_profiles
            .iter()
            .any(|profile| is_default_provider(profile))
        {
            self.provider_profiles.insert(0, default_provider_profile());
        }
        if self.active_provider.is_empty() || self.provider_profile(&self.active_provider).is_none()
        {
            if let Some(profile) = self.provider_profiles.first() {
                self.active_provider = provider_profile_key(profile);
            }
        }
        self.normalize_remote_control_tokens();

        let mut bot_configs = normalize_saved_bot_configs(std::mem::take(&mut self.bot_configs));
        for profile in &self.provider_profiles {
            if let Some(saved) = saved_bot_config_from_profile(profile, None) {
                upsert_saved_bot_config(&mut bot_configs, saved, true);
            }
        }
        self.bot_configs = bot_configs;
    }

    pub fn active_codex_home(&self) -> Option<&str> {
        let codex_home = self.codex_home.trim();
        if codex_home.is_empty() {
            None
        } else {
            Some(codex_home)
        }
    }

    pub fn active_cli_profile(&self) -> Option<String> {
        if self.active_provider.trim() == DEFAULT_PROVIDER_PROFILE_NAME {
            return None;
        }

        let profile = self.provider_profile(&self.active_provider)?;
        if is_providerless_workspace(&profile) {
            return None;
        }
        if provider_profile_uses_next_ai_gateway(&profile) {
            return None;
        }
        if provider_profile_uses_top_level_config(&profile) {
            return None;
        }
        let codex_profile_name = profile.codex_profile_name.trim();
        if codex_profile_name.is_empty() || codex_profile_name == DEFAULT_PROVIDER_PROFILE_NAME {
            None
        } else {
            Some(codex_profile_name.to_string())
        }
    }

    pub fn active_cli_model_provider(&self) -> Option<String> {
        if self.active_provider.trim() == DEFAULT_PROVIDER_PROFILE_NAME {
            return None;
        }

        let profile = self.provider_profile(&self.active_provider)?;
        if provider_profile_uses_next_ai_gateway(&profile) {
            return None;
        }
        if provider_profile_uses_top_level_config(&profile) {
            return None;
        }
        let provider_name = profile.provider_name.trim();
        if provider_name.is_empty() {
            None
        } else {
            Some(provider_name.to_string())
        }
    }

    pub fn add_provider_profile(&mut self, mut profile: ProviderProfile) {
        ensure_provider_profile_identity(&mut profile);
        let active_provider = provider_profile_key(&profile);
        self.provider_profiles.push(profile.clone());
        self.provider_profiles =
            dedupe_provider_profiles(std::mem::take(&mut self.provider_profiles));
        self.active_provider = active_provider;
    }

    pub fn remove_provider_profile(&mut self, selector: &str) -> Result<ProviderProfile, String> {
        let index = self
            .provider_profile_index(selector)
            .ok_or_else(|| format!("Provider profile not found: {}", selector))?;
        if is_default_provider(&self.provider_profiles[index]) {
            return Err("Cannot delete the Default provider".to_string());
        }

        let removed = self.provider_profiles.remove(index);
        let removed_key = provider_profile_key(&removed);
        if let Some(saved) =
            saved_bot_config_from_profile(&removed, Some(now_unix_seconds().to_string()))
        {
            upsert_saved_bot_config(&mut self.bot_configs, saved, true);
        }
        if self.active_provider == selector
            || self.active_provider == removed_key
            || self.active_provider == removed.name
        {
            self.active_provider = self
                .provider_profiles
                .first()
                .map(provider_profile_key)
                .unwrap_or_default();
        }
        self.normalize();
        self.save()?;
        Ok(removed)
    }

    pub fn update_provider_profile(
        &mut self,
        original_selector: &str,
        mut profile: ProviderProfile,
    ) -> Result<(), String> {
        if original_selector == DEFAULT_PROVIDER_PROFILE_NAME {
            if let Some(existing) = self
                .provider_profiles
                .iter_mut()
                .find(|profile| profile.name == DEFAULT_PROVIDER_PROFILE_NAME)
            {
                existing.bot = profile.bot;
                existing.proxy_url = profile.proxy_url.trim().to_string();
                existing.remote_frontend_mode =
                    normalized_remote_frontend_mode(&profile.remote_frontend_mode);
                existing.remote_web_asset_registry_url = normalized_remote_web_asset_registry_url(
                    &profile.remote_web_asset_registry_url,
                );
                existing.remote_web_asset_version =
                    normalized_remote_web_asset_version(&profile.remote_web_asset_version);
                let profile_id = existing.id.clone();
                existing
                    .bot
                    .normalize_for_profile_instance(DEFAULT_PROVIDER_PROFILE_NAME, &profile_id);
            }
            self.normalize();
            return Ok(());
        }

        let Some(index) = self.provider_profile_index(original_selector) else {
            return Err(format!("Provider profile not found: {}", original_selector));
        };

        let original = self.provider_profiles[index].clone();
        profile.start_remote_on_launch = self.provider_profiles[index].start_remote_on_launch;
        profile.start_remote_cloud_on_launch =
            self.provider_profiles[index].start_remote_cloud_on_launch;
        profile.start_remote_e2ee_on_launch =
            self.provider_profiles[index].start_remote_e2ee_on_launch;
        profile.remote_e2ee_password = self.provider_profiles[index].remote_e2ee_password.clone();
        if profile.id.trim().is_empty() {
            profile.id = self.provider_profiles[index].id.clone();
        }
        if profile.codex_home.trim().is_empty() {
            profile.codex_home = self.provider_profiles[index].codex_home.clone();
        }
        let was_active = self.active_provider == original_selector
            || self.active_provider == provider_profile_key(&original)
            || self.active_provider == original.name;
        self.provider_profiles[index] = profile.clone();
        if was_active {
            self.active_provider = provider_profile_key(&profile);
        }
        self.normalize();
        Ok(())
    }

    pub fn set_start_remote_on_launch(
        &mut self,
        profile_selector: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let Some(index) = self.provider_profile_index(profile_selector) else {
            return Err(format!("Provider profile not found: {}", profile_selector));
        };
        let profile = &mut self.provider_profiles[index];

        profile.start_remote_on_launch = enabled;
        if !enabled {
            profile.start_remote_cloud_on_launch = false;
            profile.start_remote_e2ee_on_launch = false;
        }
        self.normalize();
        self.save()
    }

    pub fn set_remote_launch_options(
        &mut self,
        profile_selector: &str,
        start_remote: bool,
        start_cloud: bool,
        remote_e2ee_password: Option<String>,
    ) -> Result<(), String> {
        let Some(index) = self.provider_profile_index(profile_selector) else {
            return Err(format!("Provider profile not found: {}", profile_selector));
        };
        let profile = &mut self.provider_profiles[index];

        let next_start_remote = start_remote;
        let next_start_cloud = next_start_remote && start_cloud;
        let next_start_e2ee = next_start_remote && next_start_cloud;
        let next_password = if next_start_e2ee {
            let password =
                remote_e2ee_password.unwrap_or_else(|| profile.remote_e2ee_password.clone());
            if password.is_empty() {
                return Err("End-to-end encrypted remote control requires a password.".to_string());
            }
            password
        } else {
            String::new()
        };

        profile.start_remote_on_launch = next_start_remote;
        profile.start_remote_cloud_on_launch = next_start_cloud;
        profile.start_remote_e2ee_on_launch = next_start_e2ee;
        profile.remote_e2ee_password = next_password;
        self.normalize();
        self.save()
    }

    pub fn remote_control_token_for_profile<F>(
        &mut self,
        profile_selector: &str,
        generate_token: F,
    ) -> Result<(String, bool), String>
    where
        F: FnOnce() -> String,
    {
        let token_key = self.remote_control_token_key_for_profile(profile_selector)?;

        if let Some(existing) = self
            .remote_control_tokens
            .get(&token_key)
            .map(|token| token.trim())
            .filter(|token| is_remote_control_token(token))
        {
            return Ok((existing.to_string(), false));
        }

        let token = generate_token().trim().to_string();
        if !is_remote_control_token(&token) {
            return Err("Generated remote control token is invalid.".to_string());
        }
        self.remote_control_tokens.insert(token_key, token.clone());
        Ok((token, true))
    }

    fn remote_control_token_key_for_profile(
        &self,
        profile_selector: &str,
    ) -> Result<String, String> {
        let token_key = self
            .provider_profile(profile_selector)
            .as_ref()
            .map(provider_profile_key)
            .unwrap_or_else(|| profile_selector.trim().to_string());
        if token_key.is_empty() {
            return Err("Remote control profile is missing.".to_string());
        }
        Ok(token_key)
    }

    fn normalize_remote_control_tokens(&mut self) {
        let previous = std::mem::take(&mut self.remote_control_tokens);
        if previous.is_empty() {
            return;
        }

        let mut next = BTreeMap::new();
        let mut live_keys = BTreeSet::new();
        for profile in &self.provider_profiles {
            let key = provider_profile_key(profile);
            live_keys.insert(key.clone());
            for candidate in [key.as_str(), profile.id.trim(), profile.name.trim()] {
                if candidate.is_empty() {
                    continue;
                }
                if let Some(token) = previous
                    .get(candidate)
                    .map(|token| token.trim())
                    .filter(|token| is_remote_control_token(token))
                {
                    next.insert(key.clone(), token.to_string());
                    break;
                }
            }
        }
        next.retain(|key, _| live_keys.contains(key));
        self.remote_control_tokens = next;
    }

    fn provider_profile_index(&self, selector: &str) -> Option<usize> {
        let selector = selector.trim();
        if selector.is_empty() {
            return None;
        }
        self.provider_profiles
            .iter()
            .position(|profile| profile.id.trim() == selector)
            .or_else(|| {
                self.provider_profiles
                    .iter()
                    .position(|profile| profile.name == selector)
            })
    }

    pub fn provider_profile(&self, selector: &str) -> Option<ProviderProfile> {
        let index = self.provider_profile_index(selector)?;
        self.provider_profiles.get(index).cloned()
    }

    pub fn upsert_saved_bot_config_from_profile(
        &mut self,
        profile_selector: &str,
    ) -> Result<(), String> {
        let Some(profile) = self.provider_profile(profile_selector) else {
            return Err(format!("Provider profile not found: {}", profile_selector));
        };
        let Some(saved) =
            saved_bot_config_from_profile(&profile, Some(now_unix_seconds().to_string()))
        else {
            return Ok(());
        };
        upsert_saved_bot_config(&mut self.bot_configs, saved, false);
        self.bot_configs = normalize_saved_bot_configs(std::mem::take(&mut self.bot_configs));
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteCloudAuthConfig {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub avatar_url: String,
    pub is_pro: bool,
    pub subscription_expires_at: u64,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

impl Default for RemoteCloudAuthConfig {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            display_name: String::new(),
            email: String::new(),
            avatar_url: String::new(),
            is_pro: false,
            subscription_expires_at: 0,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
        }
    }
}

impl RemoteCloudAuthConfig {
    fn from_env() -> Self {
        Self {
            user_id: env_string("CODEXL_REMOTE_CLOUD_USER_ID", ""),
            display_name: env_string("CODEXL_REMOTE_CLOUD_DISPLAY_NAME", ""),
            email: env_string("CODEXL_REMOTE_CLOUD_EMAIL", ""),
            avatar_url: env_string("CODEXL_REMOTE_CLOUD_AVATAR_URL", ""),
            is_pro: env_bool("CODEXL_REMOTE_CLOUD_IS_PRO", false),
            subscription_expires_at: env_u64("CODEXL_REMOTE_CLOUD_SUBSCRIPTION_EXPIRES_AT", 0),
            access_token: env_string("CODEXL_REMOTE_CLOUD_ACCESS_TOKEN", ""),
            refresh_token: env_string("CODEXL_REMOTE_CLOUD_REFRESH_TOKEN", ""),
            expires_at: env_u64("CODEXL_REMOTE_CLOUD_EXPIRES_AT", 0),
        }
    }

    pub fn normalize(&mut self) {
        self.user_id = self.user_id.trim().to_string();
        self.display_name = self.display_name.trim().to_string();
        self.email = self.email.trim().to_string();
        self.avatar_url = self.avatar_url.trim().to_string();
        self.access_token = self.access_token.trim().to_string();
        self.refresh_token = self.refresh_token.trim().to_string();
    }

    pub fn is_logged_in(&self) -> bool {
        !self.user_id.is_empty()
            && !self.access_token.is_empty()
            && (self.expires_at == 0 || self.expires_at > now_unix_seconds().saturating_add(60))
    }

    pub fn display_label(&self) -> String {
        if !self.email.is_empty() {
            self.email.clone()
        } else if !self.display_name.is_empty() {
            self.display_name.clone()
        } else {
            self.user_id.clone()
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("CODEXL_CONFIG_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(normalize_home_path(trimmed));
        }
    }

    codexl_home_dir().join("config.json")
}

pub fn default_codex_home() -> String {
    std::env::var("CODEXL_CODEX_HOME")
        .or_else(|_| std::env::var("CODEX_HOME"))
        .ok()
        .map(|value| normalize_home_path(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            user_home_dir()
                .map(|home| home.join(".codex").to_string_lossy().to_string())
                .unwrap_or_else(|| ".codex".to_string())
        })
}

pub fn default_codex_config_path() -> PathBuf {
    PathBuf::from(default_codex_home()).join("config.toml")
}

pub fn codex_profile_config_format_from_env() -> Option<CodexProfileConfigFormat> {
    std::env::var(CODEX_PROFILE_CONFIG_FORMAT_ENV)
        .ok()
        .and_then(|value| parse_codex_profile_config_format(&value))
}

pub fn codex_profile_config_format_for_cli(executable: &str) -> CodexProfileConfigFormat {
    if let Some(format) = codex_profile_config_format_from_env() {
        return format;
    }

    codex_cli_version(executable)
        .map(|version| codex_profile_config_format_for_version(version))
        .unwrap_or_else(default_codex_profile_config_format)
}

pub fn generated_codex_home(profile: &ProviderProfile) -> PathBuf {
    let configured = profile.codex_home.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    let slug = slugify(&profile.name);
    let id = profile.id.trim();
    let directory = if is_uuid_like(id) {
        format!("{}-{}", slug, short_profile_id(id))
    } else {
        slug
    };
    codexl_home_dir().join("codex-homes").join(directory)
}

pub fn generated_bot_gateway_state_dir(profile_name: &str) -> PathBuf {
    let name = profile_name.trim();
    let slug = slugify(if name.is_empty() {
        DEFAULT_PROVIDER_PROFILE_NAME
    } else {
        name
    });
    codexl_home_dir().join("bot-gateway").join(slug)
}

pub fn ensure_provider_codex_home(profile: &ProviderProfile) -> Result<String, String> {
    ensure_provider_codex_home_with_format(profile, requested_codex_profile_config_format())
}

pub fn ensure_provider_codex_home_with_format(
    profile: &ProviderProfile,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<String, String> {
    let codex_home = if is_default_provider(profile) {
        PathBuf::from(default_codex_home())
    } else {
        generated_codex_home(profile)
    };
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;

    let target_config_path = codex_home.join("config.toml");
    if !is_default_provider(profile) {
        if is_providerless_workspace(profile) {
            if !target_config_path.exists() {
                write_providerless_codex_home_config(profile)?;
            }
        } else if !target_config_path.exists() || provider_profile_uses_next_ai_gateway(profile) {
            let detail = provider_detail_from_default_config(profile);
            write_codex_home_config(&detail, &codex_home, false, profile_config_format)?;
        } else if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles {
            migrate_codex_home_profile_config(&codex_home, profile, true)?;
            sync_codex_home_app_server_defaults(&codex_home, profile)?;
        }
    }
    sync_provider_bot_media_mcp_config(profile, &codex_home, profile.bot.bridge_enabled())?;

    Ok(codex_home.to_string_lossy().to_string())
}

pub fn read_default_provider_profiles() -> Result<Vec<DefaultProviderProfile>, String> {
    read_provider_profiles_from_codex_home(&PathBuf::from(default_codex_home()))
}

fn default_provider_profile_by_name(profile_name: &str) -> Result<DefaultProviderProfile, String> {
    let profile_name = profile_name.trim();
    let profile = read_default_provider_profiles()?
        .into_iter()
        .find(|profile| profile.name == profile_name)
        .ok_or_else(|| format!("Provider profile not found: {}", profile_name))?;
    validate_default_provider_profile(&profile)?;
    Ok(profile)
}

fn validate_default_provider_profile(profile: &DefaultProviderProfile) -> Result<(), String> {
    if profile.provider_name.trim().is_empty() {
        return Err(format!("Provider is required for profile {}", profile.name));
    }
    if profile.model.trim().is_empty() {
        return Err(format!("model is required for profile {}", profile.name));
    }
    Ok(())
}

pub fn save_default_provider_profile_with_format(
    mut profile: DefaultProviderProfile,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<DefaultProviderProfile, String> {
    profile.name = profile.name.trim().to_string();
    profile.provider_name = profile.provider_name.trim().to_string();
    profile.base_url = profile.base_url.trim().to_string();
    profile.api_key = profile.api_key.trim().to_string();
    profile.model = profile.model.trim().to_string();
    profile.config_format = normalized_provider_config_format(&profile.config_format);

    validate_provider_name(&profile.name)?;
    if profile.provider_name.is_empty() {
        profile.provider_name = profile.name.clone();
    }
    if profile.model.is_empty() {
        return Err("model is required".to_string());
    }
    if profile.api_key.is_empty() {
        if let Ok(existing) = default_provider_profile_by_name(&profile.name) {
            profile.api_key = existing.api_key;
        }
    }
    if profile.api_key.is_empty() {
        return Err("apikey is required".to_string());
    }

    write_default_provider_profile(&profile, true, profile_config_format)?;
    Ok(profile)
}

pub fn delete_default_provider_profile(name: &str) -> Result<(), String> {
    let name = name.trim();
    validate_provider_name(name)?;

    let path = default_codex_config_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = remove_legacy_profile_config(&content, name);
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;

    if let Some(parent) = path.parent() {
        let separate_path = parent.join(format!("{}.config.toml", name));
        if separate_path.exists() {
            std::fs::remove_file(separate_path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

pub fn sync_workspace_profiles_for_default_provider(
    config: &mut AppConfig,
    provider: &DefaultProviderProfile,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    let provider_profile_name = provider_codex_profile_name(provider);
    for profile in config.provider_profiles.iter_mut() {
        if profile.codex_profile_name != provider_profile_name {
            continue;
        }
        if is_default_provider(profile) || is_providerless_workspace(profile) {
            continue;
        }
        profile.provider_name = provider.provider_name.clone();
        profile.provider_config_format = provider.config_format.clone();
        profile.base_url = provider.base_url.clone();
        profile.model = provider.model.clone();
        write_codex_home_config(
            provider,
            &generated_codex_home(profile),
            false,
            profile_config_format,
        )?;
    }
    config.normalize();
    Ok(())
}

pub fn add_existing_provider_profile(
    input: ExistingProviderRequest,
) -> Result<ProviderProfile, String> {
    add_existing_provider_profile_with_format(input, requested_codex_profile_config_format())
}

pub fn add_existing_provider_profile_with_format(
    input: ExistingProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<ProviderProfile, String> {
    let workspace_name = workspace_name_or_default(&input.workspace_name, &input.profile_name)?;
    let bot = input.bot.clone();
    let proxy_url = input.proxy_url.trim().to_string();
    let remote_frontend_mode = input.remote_frontend_mode.clone();
    let remote_web_asset_registry_url = input.remote_web_asset_registry_url.clone();
    let remote_web_asset_version = input.remote_web_asset_version.clone();
    let provider = default_provider_profile_by_name(&input.profile_name)?;
    if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles {
        write_default_provider_profile(&provider, false, profile_config_format)?;
    }
    let mut profile = provider.to_provider_profile();
    profile.name = workspace_name;
    profile.codex_profile_name = provider_codex_profile_name(&provider);
    profile.proxy_url = proxy_url;
    apply_remote_frontend_options(
        &mut profile,
        &remote_frontend_mode,
        &remote_web_asset_registry_url,
        &remote_web_asset_version,
    );
    profile.bot = bot;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_codex_home_config(
        &provider,
        &generated_codex_home(&profile),
        false,
        profile_config_format,
    )?;
    Ok(profile)
}

pub fn create_workspace_profile(input: WorkspaceRequest) -> Result<ProviderProfile, String> {
    let mut profile = workspace_profile(
        input.workspace_name,
        input.proxy_url,
        input.remote_frontend_mode,
        input.remote_web_asset_registry_url,
        input.remote_web_asset_version,
        input.bot,
    )?;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_providerless_codex_home_config(&profile)?;
    Ok(profile)
}

pub fn update_workspace_profile(input: UpdateWorkspaceRequest) -> Result<ProviderProfile, String> {
    if input.original_name == DEFAULT_PROVIDER_PROFILE_NAME {
        let workspace_name = input.workspace_name.trim();
        if !workspace_name.is_empty() && workspace_name != DEFAULT_PROVIDER_PROFILE_NAME {
            return Err("Default workspace cannot be renamed".to_string());
        }
        clear_default_codex_home_top_level_model_config()?;
        let mut profile = default_provider_profile();
        profile.proxy_url = input.proxy_url.trim().to_string();
        apply_remote_frontend_options(
            &mut profile,
            &input.remote_frontend_mode,
            &input.remote_web_asset_registry_url,
            &input.remote_web_asset_version,
        );
        profile.bot = input.bot;
        let profile_id = profile.id.clone();
        profile
            .bot
            .normalize_for_profile_instance(DEFAULT_PROVIDER_PROFILE_NAME, &profile_id);
        return Ok(profile);
    }

    let mut profile = workspace_profile(
        input.workspace_name,
        input.proxy_url,
        input.remote_frontend_mode,
        input.remote_web_asset_registry_url,
        input.remote_web_asset_version,
        input.bot,
    )?;
    if is_uuid_like(&input.original_name) {
        profile.id = input.original_name.clone();
    }
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_providerless_codex_home_config(&profile)?;
    Ok(profile)
}

pub fn update_existing_provider_profile(
    input: UpdateProviderRequest,
) -> Result<ProviderProfile, String> {
    update_existing_provider_profile_with_format(input, requested_codex_profile_config_format())
}

pub fn update_existing_provider_profile_with_format(
    input: UpdateProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<ProviderProfile, String> {
    let original_selector = input.original_name.clone();
    let workspace_name = workspace_name_or_default(&input.workspace_name, &input.profile_name)?;
    let bot = input.bot.clone();
    let proxy_url = input.proxy_url.trim().to_string();
    let remote_frontend_mode = input.remote_frontend_mode.clone();
    let remote_web_asset_registry_url = input.remote_web_asset_registry_url.clone();
    let remote_web_asset_version = input.remote_web_asset_version.clone();
    let provider = default_provider_profile_by_name(&input.profile_name)?;
    if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles {
        write_default_provider_profile(&provider, false, profile_config_format)?;
    }
    let mut profile = provider.to_provider_profile();
    if is_uuid_like(&original_selector) {
        profile.id = original_selector;
    }
    profile.name = workspace_name;
    profile.codex_profile_name = provider_codex_profile_name(&provider);
    profile.proxy_url = proxy_url;
    apply_remote_frontend_options(
        &mut profile,
        &remote_frontend_mode,
        &remote_web_asset_registry_url,
        &remote_web_asset_version,
    );
    profile.bot = bot;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_codex_home_config(
        &provider,
        &generated_codex_home(&profile),
        false,
        profile_config_format,
    )?;
    Ok(profile)
}

pub fn update_default_provider_selection(input: ExistingProviderRequest) -> Result<(), String> {
    update_default_provider_selection_with_format(input, requested_codex_profile_config_format())
}

pub fn update_default_provider_selection_with_format(
    input: ExistingProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    let profile = update_existing_default_provider(input, profile_config_format)?;
    let path = default_codex_config_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = if uses_top_level_provider_config(&profile)
        || default_provider_uses_next_ai_gateway(&profile)
    {
        set_top_level_model_provider_config(&content, &profile.provider_name, &profile.model)
    } else {
        set_top_level_model_config(&content, &profile.model)
    };
    let updated = set_next_ai_gateway_model_catalog_config(&updated, &profile)?;
    std::fs::write(path, updated).map_err(|e| e.to_string())
}

pub fn create_default_provider(input: NewProviderRequest) -> Result<ProviderProfile, String> {
    create_default_provider_with_format(input, requested_codex_profile_config_format())
}

pub fn create_default_provider_with_format(
    input: NewProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<ProviderProfile, String> {
    let workspace_name = if input.workspace_name.trim() == DEFAULT_PROVIDER_PROFILE_NAME {
        DEFAULT_PROVIDER_PROFILE_NAME.to_string()
    } else {
        workspace_name_or_default(&input.workspace_name, &input.name)?
    };
    validate_provider_name(&input.name)?;
    let provider = DefaultProviderProfile {
        name: input.name.trim().to_string(),
        provider_name: input.name.trim().to_string(),
        base_url: input.base_url.trim().to_string(),
        api_key: input.api_key.trim().to_string(),
        model: input.model.trim().to_string(),
        config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
    };

    if provider.base_url.is_empty() {
        return Err("base_url is required".to_string());
    }
    if provider.api_key.is_empty() {
        return Err("apikey is required".to_string());
    }
    if provider.model.is_empty() {
        return Err("model is required".to_string());
    }

    write_default_provider_profile(&provider, true, profile_config_format)?;

    let mut profile = provider.to_provider_profile();
    profile.name = workspace_name;
    profile.codex_profile_name = provider_codex_profile_name(&provider);
    profile.proxy_url = input.proxy_url.trim().to_string();
    apply_remote_frontend_options(
        &mut profile,
        &input.remote_frontend_mode,
        &input.remote_web_asset_registry_url,
        &input.remote_web_asset_version,
    );
    profile.bot = input.bot;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);

    if profile.name == DEFAULT_PROVIDER_PROFILE_NAME {
        let path = default_codex_config_path();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let updated = set_top_level_model_config(&content, &provider.model);
        std::fs::write(path, updated).map_err(|e| e.to_string())?;
    } else {
        write_provider_codex_home_config(&provider, true, profile_config_format)?;
        write_codex_home_config(
            &provider,
            &generated_codex_home(&profile),
            true,
            profile_config_format,
        )?;
    }

    Ok(profile)
}

pub fn create_next_ai_gateway_provider(
    input: NextAiGatewayProviderRequest,
) -> Result<ProviderProfile, String> {
    create_next_ai_gateway_provider_with_format(input, requested_codex_profile_config_format())
}

pub fn create_next_ai_gateway_provider_with_format(
    input: NextAiGatewayProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<ProviderProfile, String> {
    let workspace_name = workspace_name_or_default(&input.workspace_name, &input.name)?;
    let provider = next_ai_gateway_provider_profile(&input.name, &input.model)?;
    write_default_provider_profile(&provider, true, profile_config_format)?;
    write_provider_codex_home_config(&provider, true, profile_config_format)?;

    let mut profile = provider.to_provider_profile();
    profile.name = workspace_name;
    profile.codex_profile_name = provider_codex_profile_name(&provider);
    profile.proxy_url = input.proxy_url.trim().to_string();
    apply_remote_frontend_options(
        &mut profile,
        &input.remote_frontend_mode,
        &input.remote_web_asset_registry_url,
        &input.remote_web_asset_version,
    );
    profile.bot = input.bot;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_codex_home_config(
        &provider,
        &generated_codex_home(&profile),
        true,
        profile_config_format,
    )?;
    Ok(profile)
}

pub fn update_next_ai_gateway_provider_profile(
    input: UpdateNextAiGatewayProviderRequest,
) -> Result<ProviderProfile, String> {
    update_next_ai_gateway_provider_profile_with_format(
        input,
        requested_codex_profile_config_format(),
    )
}

pub fn update_next_ai_gateway_provider_profile_with_format(
    input: UpdateNextAiGatewayProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<ProviderProfile, String> {
    let original_selector = input.original_name.clone();
    let workspace_name = workspace_name_or_default(&input.workspace_name, &input.name)?;
    let provider = next_ai_gateway_provider_profile(&input.name, &input.model)?;
    write_default_provider_profile(&provider, true, profile_config_format)?;
    write_provider_codex_home_config(&provider, true, profile_config_format)?;

    let mut profile = provider.to_provider_profile();
    if is_uuid_like(&original_selector) {
        profile.id = original_selector;
    }
    profile.name = workspace_name;
    profile.codex_profile_name = provider_codex_profile_name(&provider);
    profile.proxy_url = input.proxy_url.trim().to_string();
    apply_remote_frontend_options(
        &mut profile,
        &input.remote_frontend_mode,
        &input.remote_web_asset_registry_url,
        &input.remote_web_asset_version,
    );
    profile.bot = input.bot;
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    write_codex_home_config(
        &provider,
        &generated_codex_home(&profile),
        true,
        profile_config_format,
    )?;
    Ok(profile)
}

fn next_ai_gateway_provider_profile(
    profile_name: &str,
    model: &str,
) -> Result<DefaultProviderProfile, String> {
    validate_provider_name(profile_name)?;
    let model = model.trim();
    if model.is_empty() {
        return Err("model is required".to_string());
    }

    Ok(DefaultProviderProfile {
        name: profile_name.trim().to_string(),
        provider_name: gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME.to_string(),
        base_url: gateway_config::codex_provider_base_url()?,
        api_key: gateway_config::codex_provider_api_key()?,
        model: model.to_string(),
        config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
    })
}

fn update_existing_default_provider(
    input: ExistingProviderRequest,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<DefaultProviderProfile, String> {
    let mut profile = read_default_provider_profiles()?
        .into_iter()
        .find(|profile| profile.name == input.profile_name)
        .ok_or_else(|| format!("Provider profile not found: {}", input.profile_name))?;

    let model = input.model.trim();
    if model.is_empty() {
        return Err("model is required".to_string());
    }
    profile.model = model.to_string();

    if let Some(base_url) = input.base_url {
        profile.base_url = base_url.trim().to_string();
    }
    if let Some(api_key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        profile.api_key = api_key.to_string();
    }

    write_default_provider_profile(&profile, false, profile_config_format)?;
    write_provider_codex_home_config(&profile, false, profile_config_format)?;
    Ok(profile)
}

fn workspace_profile(
    workspace_name: String,
    proxy_url: String,
    remote_frontend_mode: String,
    remote_web_asset_registry_url: String,
    remote_web_asset_version: String,
    bot: BotProfileConfig,
) -> Result<ProviderProfile, String> {
    let workspace_name = workspace_name_or_default(&workspace_name, "")?;
    let mut profile = ProviderProfile {
        id: new_uuid_v4(),
        name: workspace_name,
        codex_profile_name: String::new(),
        provider_name: String::new(),
        base_url: String::new(),
        model: String::new(),
        proxy_url: proxy_url.trim().to_string(),
        bot,
        ..ProviderProfile::default()
    };
    apply_remote_frontend_options(
        &mut profile,
        &remote_frontend_mode,
        &remote_web_asset_registry_url,
        &remote_web_asset_version,
    );
    ensure_provider_profile_identity(&mut profile);
    profile
        .bot
        .normalize_for_profile_instance(&profile.name, &profile.id);
    Ok(profile)
}

fn apply_remote_frontend_options(
    profile: &mut ProviderProfile,
    mode: &str,
    registry_url: &str,
    version: &str,
) {
    profile.remote_frontend_mode = normalized_remote_frontend_mode(mode);
    profile.remote_web_asset_registry_url = normalized_remote_web_asset_registry_url(registry_url);
    profile.remote_web_asset_version = normalized_remote_web_asset_version(version);
}

fn write_default_provider_profile(
    profile: &DefaultProviderProfile,
    force_defaults: bool,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    let path = default_codex_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_provider_profile_content_for_format(
        &content,
        profile,
        force_defaults,
        profile_config_format,
    );
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;

    if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles
        && !uses_top_level_provider_config(profile)
    {
        write_separate_profile_file(
            path.parent().unwrap_or_else(|| Path::new(".")),
            profile,
            &content,
            force_defaults,
        )?;
    }

    Ok(())
}

fn write_provider_codex_home_config(
    profile: &DefaultProviderProfile,
    force_defaults: bool,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    let provider_profile = profile.to_provider_profile();
    if is_default_provider(&provider_profile) {
        return Ok(());
    }

    let codex_home = generated_codex_home(&provider_profile);
    write_codex_home_config(profile, &codex_home, force_defaults, profile_config_format)
}

fn write_codex_home_config(
    profile: &DefaultProviderProfile,
    codex_home: &Path,
    force_defaults: bool,
    profile_config_format: CodexProfileConfigFormat,
) -> Result<(), String> {
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;

    let target_config_path = codex_home.join("config.toml");
    let content = if target_config_path.exists() {
        std::fs::read_to_string(&target_config_path).unwrap_or_default()
    } else {
        std::fs::read_to_string(default_codex_config_path()).unwrap_or_default()
    };

    let updated = upsert_provider_profile_content_for_format(
        &content,
        profile,
        force_defaults,
        profile_config_format,
    );
    let updated = if uses_top_level_provider_config(profile)
        || default_provider_uses_next_ai_gateway(profile)
        || profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles
    {
        set_top_level_model_provider_config(&updated, &profile.provider_name, &profile.model)
    } else {
        set_top_level_model_config(&updated, &profile.model)
    };
    let updated = set_next_ai_gateway_model_catalog_config(&updated, profile)?;
    std::fs::write(&target_config_path, updated).map_err(|e| e.to_string())?;

    if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles
        && !uses_top_level_provider_config(profile)
    {
        write_separate_profile_file(codex_home, profile, &content, force_defaults)?;
    }

    Ok(())
}

fn write_providerless_codex_home_config(profile: &ProviderProfile) -> Result<(), String> {
    let codex_home = generated_codex_home(profile);
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;

    let target_config_path = codex_home.join("config.toml");
    let content = if target_config_path.exists() {
        std::fs::read_to_string(&target_config_path).unwrap_or_default()
    } else {
        std::fs::read_to_string(default_codex_config_path()).unwrap_or_default()
    };
    let updated = clear_top_level_model_config(&content);
    std::fs::write(target_config_path, updated).map_err(|e| e.to_string())
}

fn clear_default_codex_home_top_level_model_config() -> Result<(), String> {
    let target_config_path = default_codex_config_path();
    if let Some(parent) = target_config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = std::fs::read_to_string(&target_config_path).unwrap_or_default();
    let updated = clear_top_level_model_config(&content);
    std::fs::write(target_config_path, updated).map_err(|e| e.to_string())
}

fn provider_profile_uses_next_ai_gateway(profile: &ProviderProfile) -> bool {
    profile.provider_name.trim() == gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME
}

fn default_provider_uses_next_ai_gateway(profile: &DefaultProviderProfile) -> bool {
    profile.provider_name.trim() == gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME
}

fn set_next_ai_gateway_model_catalog_config(
    content: &str,
    profile: &DefaultProviderProfile,
) -> Result<String, String> {
    if !default_provider_uses_next_ai_gateway(profile) {
        return Ok(clear_top_level_string_config(content, "model_catalog_json"));
    }

    let catalog_path = gateway_config::write_codex_model_catalog(&profile.model)?;
    Ok(set_top_level_string_config(
        content,
        "model_catalog_json",
        &catalog_path,
    ))
}

fn write_separate_profile_file(
    codex_home: &Path,
    profile: &DefaultProviderProfile,
    source_config_content: &str,
    force_defaults: bool,
) -> Result<(), String> {
    let profile_config_path = separate_profile_config_path(codex_home, &profile.name);
    let content = if profile_config_path.exists() {
        std::fs::read_to_string(&profile_config_path).unwrap_or_default()
    } else {
        legacy_profile_table_body(source_config_content, &profile.name).unwrap_or_else(|| {
            let default_profile_path =
                separate_profile_config_path(&PathBuf::from(default_codex_home()), &profile.name);
            std::fs::read_to_string(default_profile_path).unwrap_or_default()
        })
    };
    let updated = upsert_separate_profile_file_content(&content, profile, force_defaults)?;
    std::fs::write(profile_config_path, ensure_trailing_newline(&updated))
        .map_err(|e| e.to_string())
}

fn upsert_separate_profile_file_content(
    content: &str,
    profile: &DefaultProviderProfile,
    force_defaults: bool,
) -> Result<String, String> {
    let mut updated =
        set_top_level_model_provider_config(content, &profile.provider_name, &profile.model);
    if force_defaults || !top_level_assignment_exists(&updated, "model_reasoning_effort") {
        updated = set_top_level_string_config(&updated, "model_reasoning_effort", "xhigh");
    }
    set_next_ai_gateway_model_catalog_config(&updated, profile)
}

fn migrate_codex_home_profile_config(
    codex_home: &Path,
    profile: &ProviderProfile,
    create_missing_profile_file: bool,
) -> Result<(), String> {
    if is_providerless_workspace(profile)
        || is_default_provider(profile)
        || provider_profile_uses_top_level_config(profile)
    {
        return Ok(());
    }

    let profile_name = profile.codex_profile_name.trim();
    if profile_name.is_empty() || profile_name == DEFAULT_PROVIDER_PROFILE_NAME {
        return Ok(());
    }

    let config_path = codex_home.join("config.toml");
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let profile_config_path = separate_profile_config_path(codex_home, profile_name);

    if !profile_config_path.exists() {
        let profile_content =
            legacy_profile_table_body(&content, profile_name).unwrap_or_else(|| {
                if create_missing_profile_file {
                    format!(
                        "model = \"{}\"\nmodel_provider = \"{}\"",
                        toml_escape(&profile.model),
                        toml_escape(&profile.provider_name)
                    )
                } else {
                    String::new()
                }
            });
        if !profile_content.trim().is_empty() {
            std::fs::write(
                &profile_config_path,
                ensure_trailing_newline(&profile_content),
            )
            .map_err(|e| e.to_string())?;
        }
    }

    let updated = remove_legacy_profile_config(&content, profile_name);
    if updated != content {
        std::fs::write(config_path, ensure_trailing_newline(&updated))
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn sync_codex_home_app_server_defaults(
    codex_home: &Path,
    profile: &ProviderProfile,
) -> Result<(), String> {
    if is_providerless_workspace(profile)
        || is_default_provider(profile)
        || provider_profile_uses_top_level_config(profile)
    {
        return Ok(());
    }

    let profile_name = profile.codex_profile_name.trim();
    if profile_name.is_empty() || profile_name == DEFAULT_PROVIDER_PROFILE_NAME {
        return Ok(());
    }

    let config_path = codex_home.join("config.toml");
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut detail = provider_detail_from_default_config(profile);
    let profile_config_path = separate_profile_config_path(codex_home, profile_name);
    if let Ok(profile_content) = std::fs::read_to_string(profile_config_path) {
        if let Some(parsed) =
            parse_separate_provider_profile(profile_name, &content, &profile_content)
        {
            detail.provider_name = parsed.provider_name;
            detail.model = parsed.model;
            if !parsed.base_url.is_empty() {
                detail.base_url = parsed.base_url;
            }
            if !parsed.api_key.is_empty() {
                detail.api_key = parsed.api_key;
            }
        }
    }

    let updated = upsert_provider_definition_content(&content, &detail, false);
    let updated =
        set_top_level_model_provider_config(&updated, &detail.provider_name, &detail.model);
    let updated = set_next_ai_gateway_model_catalog_config(&updated, &detail)?;
    if updated != content {
        std::fs::write(config_path, ensure_trailing_newline(&updated))
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn separate_profile_config_path(codex_home: &Path, profile_name: &str) -> PathBuf {
    codex_home.join(format!("{}.config.toml", profile_name.trim()))
}

fn sync_provider_bot_media_mcp_config(
    profile: &ProviderProfile,
    codex_home: &Path,
    enabled: bool,
) -> Result<(), String> {
    let target_config_path = codex_home.join("config.toml");
    let content = std::fs::read_to_string(&target_config_path).unwrap_or_default();
    let without_existing = remove_bot_media_mcp_config(&content);
    let updated = if enabled {
        append_bot_media_mcp_config(&without_existing, profile)?
    } else {
        without_existing
    };
    if updated == content {
        return Ok(());
    }
    if updated.trim().is_empty() && !target_config_path.exists() {
        return Ok(());
    }
    if let Some(parent) = target_config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(target_config_path, ensure_trailing_newline(&updated)).map_err(|e| e.to_string())
}

pub fn sync_provider_bot_media_mcp_config_for_launch(
    profile: &ProviderProfile,
    codex_home: &str,
    enabled: bool,
) -> Result<(), String> {
    let codex_home = PathBuf::from(normalize_home_path(codex_home));
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;
    sync_provider_bot_media_mcp_config(profile, &codex_home, enabled)
}

fn append_bot_media_mcp_config(content: &str, profile: &ProviderProfile) -> Result<String, String> {
    let mut bot = profile.bot.clone();
    bot.normalize_for_profile_instance(&profile.name, &profile.id);
    if !bot.bridge_enabled() {
        return Ok(content.trim_end().to_string());
    }

    let command = std::env::current_exe().map_err(|e| e.to_string())?;
    let state_dir = if bot.state_dir.trim().is_empty() {
        generated_bot_gateway_state_dir(&profile.name)
    } else {
        PathBuf::from(normalize_home_path(&bot.state_dir))
    };
    let mut env = vec![
        ("CODEXL_BOT_GATEWAY_ENABLED", "true".to_string()),
        ("CODEXL_BOT_GATEWAY_PLATFORM", bot.platform),
        ("CODEXL_BOT_GATEWAY_TENANT_ID", bot.tenant_id),
        ("CODEXL_BOT_GATEWAY_INTEGRATION_ID", bot.integration_id),
        (
            "CODEXL_BOT_GATEWAY_FORWARD_ALL_CODEX_MESSAGES",
            if bot.forward_all_codex_messages {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ),
        (
            "CODEXL_BOT_HANDOFF_ENABLED",
            if bot.handoff.enabled {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ),
        (
            "CODEXL_BOT_HANDOFF_IDLE_SECONDS",
            bot.handoff.idle_seconds.to_string(),
        ),
        (
            "CODEXL_BOT_HANDOFF_SCREEN_LOCK",
            if bot.handoff.screen_lock {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ),
        (
            "CODEXL_BOT_HANDOFF_USER_IDLE",
            if bot.handoff.user_idle {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ),
        (
            "CODEXL_BOT_HANDOFF_PHONE_WIFI_TARGETS",
            bot.handoff.phone_wifi_targets.join("\n"),
        ),
        (
            "CODEXL_BOT_HANDOFF_PHONE_BLUETOOTH_TARGETS",
            bot.handoff.phone_bluetooth_targets.join("\n"),
        ),
        (
            "CODEXL_BOT_GATEWAY_STATE_DIR",
            state_dir.to_string_lossy().to_string(),
        ),
        (
            "BOT_GATEWAY_STATE_DIR",
            state_dir.to_string_lossy().to_string(),
        ),
        ("CODEXL_CODEX_PROFILE", profile.name.clone()),
    ];
    for name in BOT_MEDIA_MCP_OPTIONAL_ENV_NAMES {
        let Some(value) = std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        env.push((*name, value));
    }

    let mut output = content.trim_end().to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(BOT_MEDIA_MCP_MANAGED_BEGIN);
    output.push('\n');
    output.push_str(&format!(
        "[mcp_servers.{}]\n",
        toml_key(BOT_MEDIA_MCP_SERVER_NAME)
    ));
    output.push_str(&format!(
        "command = \"{}\"\n",
        toml_escape(&command.to_string_lossy())
    ));
    output.push_str(&format!(
        "args = [\"{}\"]\n",
        toml_escape(BOT_MEDIA_MCP_RUN_MODE_ARG)
    ));
    output.push_str("enabled = true\n");
    output.push_str("startup_timeout_sec = 30\n");
    output.push_str("tool_timeout_sec = 180\n\n");
    output.push_str(&format!(
        "[mcp_servers.{}.env]\n",
        toml_key(BOT_MEDIA_MCP_SERVER_NAME)
    ));
    for (key, value) in env {
        if value.trim().is_empty() {
            continue;
        }
        output.push_str(&format!("{} = \"{}\"\n", key, toml_escape(&value)));
    }
    output.push_str(BOT_MEDIA_MCP_MANAGED_END);
    Ok(output)
}

fn remove_bot_media_mcp_config(content: &str) -> String {
    let mut output = Vec::new();
    let mut in_managed_block = false;
    let mut in_target_table = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == BOT_MEDIA_MCP_MANAGED_BEGIN {
            in_managed_block = true;
            in_target_table = false;
            continue;
        }
        if in_managed_block {
            if trimmed == BOT_MEDIA_MCP_MANAGED_END {
                in_managed_block = false;
            }
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target_table = is_bot_media_mcp_table_header(trimmed);
            if in_target_table {
                continue;
            }
        }
        if in_target_table {
            continue;
        }

        output.push(line.to_string());
    }

    output.join("\n").trim_end().to_string()
}

fn is_bot_media_mcp_table_header(line: &str) -> bool {
    let inner = line.trim_start_matches('[').trim_end_matches(']').trim();
    [BOT_MEDIA_MCP_SERVER_NAME, LEGACY_BOT_MEDIA_MCP_SERVER_NAME]
        .iter()
        .any(|server_name| {
            inner == format!("mcp_servers.{}", server_name)
                || inner.starts_with(&format!("mcp_servers.{}.", server_name))
        })
}

pub fn remove_retired_builtin_mcp_configs_for_launch(codex_home: &str) -> Result<(), String> {
    let codex_home = PathBuf::from(normalize_home_path(codex_home));
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;
    remove_retired_builtin_mcp_configs(&codex_home)
}

fn remove_retired_builtin_mcp_configs(codex_home: &Path) -> Result<(), String> {
    let target_config_path = codex_home.join("config.toml");
    let content = std::fs::read_to_string(&target_config_path).unwrap_or_default();
    let updated = remove_retired_mcp_config(
        &content,
        RETIRED_DOUBAO_IME_MCP_SERVER_NAME,
        RETIRED_DOUBAO_IME_MCP_MANAGED_BEGIN,
        RETIRED_DOUBAO_IME_MCP_MANAGED_END,
    );
    if updated == content {
        return Ok(());
    }
    if updated.trim().is_empty() && !target_config_path.exists() {
        return Ok(());
    }
    if let Some(parent) = target_config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(target_config_path, ensure_trailing_newline(&updated)).map_err(|e| e.to_string())
}

fn remove_retired_mcp_config(
    content: &str,
    server_name: &str,
    managed_begin: &str,
    managed_end: &str,
) -> String {
    let mut output = Vec::new();
    let mut in_managed_block = false;
    let mut in_target_table = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == managed_begin {
            in_managed_block = true;
            in_target_table = false;
            continue;
        }
        if in_managed_block {
            if trimmed == managed_end {
                in_managed_block = false;
            }
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target_table = is_retired_mcp_table_header(trimmed, server_name);
            if in_target_table {
                continue;
            }
        }
        if in_target_table {
            continue;
        }

        output.push(line.to_string());
    }

    output.join("\n").trim_end().to_string()
}

fn is_retired_mcp_table_header(line: &str, server_name: &str) -> bool {
    let inner = line.trim_start_matches('[').trim_end_matches(']').trim();
    inner == format!("mcp_servers.{}", server_name)
        || inner.starts_with(&format!("mcp_servers.{}.", server_name))
}

fn ensure_trailing_newline(content: &str) -> String {
    if content.is_empty() || content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{}\n", content)
    }
}

pub fn normalize_home_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "~" {
        return user_home_dir()
            .map(|home| home.to_string_lossy().to_string())
            .unwrap_or_else(|| trimmed.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = user_home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~\\") {
        if let Some(home) = user_home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    trimmed.to_string()
}

fn codexl_home_dir() -> PathBuf {
    std::env::var("CODEXL_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| PathBuf::from(normalize_home_path(&value)))
        .unwrap_or_else(|| {
            if cfg!(windows) {
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

fn parse_default_provider_profiles(content: &str) -> Vec<DefaultProviderProfile> {
    use std::collections::HashMap;

    let mut providers: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut profiles: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current: Option<(String, String)> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some((section, name)) = parse_table_header(trimmed) {
            current = Some((section, name));
            continue;
        }

        let Some((section, name)) = current.as_ref() else {
            continue;
        };
        let Some((key, value)) = parse_string_assignment(trimmed) else {
            continue;
        };

        match section.as_str() {
            "model_providers" => {
                providers
                    .entry(name.clone())
                    .or_default()
                    .insert(key, value);
            }
            "profiles" => {
                profiles.entry(name.clone()).or_default().insert(key, value);
            }
            _ => {}
        }
    }

    let mut result = Vec::new();
    for (profile_name, profile_values) in profiles {
        let Some(provider_name) = profile_values.get("model_provider").cloned() else {
            continue;
        };
        let Some(model) = profile_values.get("model").cloned() else {
            continue;
        };
        let base_url = providers
            .get(&provider_name)
            .and_then(|values| values.get("base_url"))
            .cloned()
            .unwrap_or_default();
        let api_key = providers
            .get(&provider_name)
            .and_then(|values| values.get("experimental_bearer_token"))
            .cloned()
            .unwrap_or_default();
        result.push(DefaultProviderProfile {
            name: profile_name,
            provider_name,
            base_url,
            api_key,
            model,
            config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
        });
    }
    let (top_level_model, top_level_provider_name) = parse_top_level_model_config(content);
    if !top_level_model.is_empty()
        && !top_level_provider_name.is_empty()
        && !result.iter().any(|profile| {
            profile.name == top_level_provider_name
                || (profile.provider_name == top_level_provider_name
                    && profile.model == top_level_model)
        })
    {
        let base_url = providers
            .get(&top_level_provider_name)
            .and_then(|values| values.get("base_url"))
            .cloned()
            .unwrap_or_default();
        let api_key = providers
            .get(&top_level_provider_name)
            .and_then(|values| values.get("experimental_bearer_token"))
            .cloned()
            .unwrap_or_default();
        result.push(DefaultProviderProfile {
            name: top_level_provider_name.clone(),
            provider_name: top_level_provider_name,
            base_url,
            api_key,
            model: top_level_model,
            config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL.to_string(),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

fn read_provider_profiles_from_codex_home(
    codex_home: &Path,
) -> Result<Vec<DefaultProviderProfile>, String> {
    let config_path = codex_home.join("config.toml");
    let content = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let mut result = parse_default_provider_profiles(&content);

    if let Ok(entries) = std::fs::read_dir(codex_home) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(profile_name) = file_name.strip_suffix(".config.toml") else {
                continue;
            };
            if profile_name.is_empty() {
                continue;
            }
            let Ok(profile_content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(profile) =
                parse_separate_provider_profile(profile_name, &content, &profile_content)
            {
                if let Some(existing) = result.iter_mut().find(|item| item.name == profile.name) {
                    *existing = profile;
                } else {
                    result.push(profile);
                }
            }
        }
    }

    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn parse_separate_provider_profile(
    profile_name: &str,
    base_content: &str,
    profile_content: &str,
) -> Option<DefaultProviderProfile> {
    let (model, provider_name) = parse_top_level_model_config(profile_content);
    if model.is_empty() || provider_name.is_empty() {
        return None;
    }
    let base_url = provider_table_value(profile_content, &provider_name, "base_url")
        .or_else(|| provider_table_value(base_content, &provider_name, "base_url"))
        .unwrap_or_default();
    let api_key =
        provider_table_value(profile_content, &provider_name, "experimental_bearer_token")
            .or_else(|| {
                provider_table_value(base_content, &provider_name, "experimental_bearer_token")
            })
            .unwrap_or_default();
    Some(DefaultProviderProfile {
        name: profile_name.to_string(),
        provider_name,
        base_url,
        api_key,
        model,
        config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
    })
}

fn parse_table_header(line: &str) -> Option<(String, String)> {
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = line.trim_start_matches('[').trim_end_matches(']').trim();
    let (section, name) = inner.split_once('.')?;
    if section != "model_providers" && section != "profiles" {
        return None;
    }
    Some((section.to_string(), unquote_toml_key(name.trim())))
}

fn parse_string_assignment(line: &str) -> Option<(String, String)> {
    if line.starts_with('#') || line.is_empty() {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || !value.starts_with('"') {
        return None;
    }
    Some((key.to_string(), parse_toml_string(value)?))
}

fn parse_top_level_model_config(content: &str) -> (String, String) {
    let mut model = String::new();
    let mut provider_name = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            break;
        }
        let Some((key, value)) = parse_string_assignment(trimmed) else {
            continue;
        };
        match key.as_str() {
            "model" => model = value,
            "model_provider" => provider_name = value,
            _ => {}
        }
    }

    (model, provider_name)
}

fn provider_table_value(content: &str, provider_name: &str, target_key: &str) -> Option<String> {
    let mut in_target = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target = parse_table_header(trimmed)
                .map(|(section, name)| section == "model_providers" && name == provider_name)
                .unwrap_or(false);
            continue;
        }
        if !in_target {
            continue;
        }
        let Some((key, value)) = parse_string_assignment(trimmed) else {
            continue;
        };
        if key == target_key {
            return Some(value);
        }
    }
    None
}

fn parse_toml_string(value: &str) -> Option<String> {
    let mut chars = value.trim().chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            result.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(result),
            other => result.push(other),
        }
    }
    None
}

fn provider_detail_from_default_config(profile: &ProviderProfile) -> DefaultProviderProfile {
    let source_content = std::fs::read_to_string(default_codex_config_path()).unwrap_or_default();
    let codex_profile_name = profile.codex_profile_name.trim();
    let codex_profile_name = if codex_profile_name.is_empty() {
        profile.name.trim()
    } else {
        codex_profile_name
    };
    let mut detail = read_default_provider_profiles()
        .unwrap_or_else(|_| parse_default_provider_profiles(&source_content))
        .into_iter()
        .find(|item| item.name == codex_profile_name)
        .or_else(|| {
            read_default_provider_profiles()
                .unwrap_or_else(|_| parse_default_provider_profiles(&source_content))
                .into_iter()
                .find(|item| item.provider_name == profile.provider_name)
        })
        .unwrap_or_else(|| DefaultProviderProfile {
            name: codex_profile_name.to_string(),
            provider_name: profile.provider_name.clone(),
            base_url: profile.base_url.clone(),
            api_key: String::new(),
            model: profile.model.clone(),
            config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
        });

    detail.name = codex_profile_name.to_string();
    detail.provider_name = profile.provider_name.clone();
    detail.config_format = normalized_provider_config_format(&profile.provider_config_format);
    if !profile.base_url.is_empty() {
        detail.base_url = profile.base_url.clone();
    }
    detail.model = profile.model.clone();
    if provider_profile_uses_next_ai_gateway(profile) {
        if let Ok(base_url) = gateway_config::codex_provider_base_url() {
            detail.base_url = base_url;
        }
        if let Ok(api_key) = gateway_config::codex_provider_api_key() {
            detail.api_key = api_key;
        }
    }

    detail
}

fn upsert_provider_profile_content_for_format(
    content: &str,
    profile: &DefaultProviderProfile,
    force_defaults: bool,
    profile_config_format: CodexProfileConfigFormat,
) -> String {
    if profile_config_format == CodexProfileConfigFormat::SeparateProfileFiles
        && !uses_top_level_provider_config(profile)
    {
        let content = remove_legacy_profile_config(content, &profile.name);
        return upsert_provider_definition_content(&content, profile, force_defaults);
    }

    upsert_provider_profile_content(content, profile, force_defaults)
}

fn upsert_provider_profile_content(
    content: &str,
    profile: &DefaultProviderProfile,
    force_defaults: bool,
) -> String {
    if uses_top_level_provider_config(profile) {
        return upsert_top_level_provider_content(content, profile, force_defaults);
    }

    let provider_exists = table_exists(content, "model_providers", &profile.provider_name);
    let profile_exists = table_exists(content, "profiles", &profile.name);

    let mut provider_assignments = vec![
        ("name".to_string(), profile.provider_name.clone()),
        ("base_url".to_string(), profile.base_url.clone()),
    ];
    if !profile.api_key.is_empty() {
        provider_assignments.push((
            "experimental_bearer_token".to_string(),
            profile.api_key.clone(),
        ));
    }
    if force_defaults || !provider_exists {
        provider_assignments.push(("wire_api".to_string(), "responses".to_string()));
    }

    let mut updated = upsert_table_assignments(
        content,
        "model_providers",
        &profile.provider_name,
        &provider_assignments,
    );

    let mut profile_assignments = vec![
        ("model".to_string(), profile.model.clone()),
        ("model_provider".to_string(), profile.provider_name.clone()),
    ];
    if force_defaults || !profile_exists {
        profile_assignments.push(("model_reasoning_effort".to_string(), "xhigh".to_string()));
    }
    updated = upsert_table_assignments(&updated, "profiles", &profile.name, &profile_assignments);
    updated
}

fn upsert_provider_definition_content(
    content: &str,
    profile: &DefaultProviderProfile,
    force_defaults: bool,
) -> String {
    let provider_exists = table_exists(content, "model_providers", &profile.provider_name);
    let mut provider_assignments = vec![
        ("name".to_string(), profile.provider_name.clone()),
        ("base_url".to_string(), profile.base_url.clone()),
    ];
    if !profile.api_key.is_empty() {
        provider_assignments.push((
            "experimental_bearer_token".to_string(),
            profile.api_key.clone(),
        ));
    }
    if force_defaults || !provider_exists {
        provider_assignments.push(("wire_api".to_string(), "responses".to_string()));
    }

    upsert_table_assignments(
        content,
        "model_providers",
        &profile.provider_name,
        &provider_assignments,
    )
}

fn upsert_top_level_provider_content(
    content: &str,
    profile: &DefaultProviderProfile,
    force_defaults: bool,
) -> String {
    let provider_exists = table_exists(content, "model_providers", &profile.provider_name);
    let mut provider_assignments = vec![("name".to_string(), profile.provider_name.clone())];
    if !profile.base_url.is_empty()
        || provider_table_value(content, &profile.provider_name, "base_url").is_some()
    {
        provider_assignments.push(("base_url".to_string(), profile.base_url.clone()));
    }
    if !profile.api_key.is_empty()
        || provider_table_value(content, &profile.provider_name, "experimental_bearer_token")
            .is_some()
    {
        provider_assignments.push((
            "experimental_bearer_token".to_string(),
            profile.api_key.clone(),
        ));
    }
    if force_defaults || !provider_exists {
        provider_assignments.push(("wire_api".to_string(), "responses".to_string()));
    }

    let updated = upsert_table_assignments(
        content,
        "model_providers",
        &profile.provider_name,
        &provider_assignments,
    );
    set_top_level_model_provider_config(&updated, &profile.provider_name, &profile.model)
}

fn upsert_table_assignments(
    content: &str,
    section: &str,
    name: &str,
    assignments: &[(String, String)],
) -> String {
    let mut output = Vec::new();
    let mut in_target = false;
    let mut found_table = false;
    let mut seen_keys = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_target {
                push_missing_assignments(&mut output, assignments, &seen_keys);
                seen_keys.clear();
            }
            in_target = parse_table_header(trimmed)
                .map(|(table_section, table_name)| table_section == section && table_name == name)
                .unwrap_or(false);
            if in_target {
                found_table = true;
            }
        }

        if in_target {
            if let Some(key) = assignment_key(trimmed) {
                if let Some((_, value)) = assignments.iter().find(|(item, _)| item == &key) {
                    output.push(format!("{} = \"{}\"", key, toml_escape(value)));
                    seen_keys.push(key);
                    continue;
                }
            }
        }

        output.push(line.to_string());
    }

    if in_target {
        push_missing_assignments(&mut output, assignments, &seen_keys);
    }

    if !found_table {
        if !output.is_empty() && output.last().is_some_and(|line| !line.trim().is_empty()) {
            output.push(String::new());
        }
        output.push(format!("[{}.{}]", section, toml_key(name)));
        for (key, value) in assignments {
            output.push(format!("{} = \"{}\"", key, toml_escape(value)));
        }
    }

    output.join("\n")
}

fn push_missing_assignments(
    output: &mut Vec<String>,
    assignments: &[(String, String)],
    seen_keys: &[String],
) {
    for (key, value) in assignments {
        if !seen_keys.iter().any(|seen| seen == key) {
            output.push(format!("{} = \"{}\"", key, toml_escape(value)));
        }
    }
}

fn assignment_key(line: &str) -> Option<String> {
    if line.starts_with('#') || line.is_empty() {
        return None;
    }
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

fn table_exists(content: &str, section: &str, name: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        parse_table_header(trimmed)
            .map(|(table_section, table_name)| table_section == section && table_name == name)
            .unwrap_or(false)
    })
}

fn top_level_assignment_exists(content: &str, key: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return false;
        }
        if assignment_key(trimmed).is_some_and(|item| item == key) {
            return true;
        }
    }
    false
}

fn remove_legacy_profile_config(content: &str, profile_name: &str) -> String {
    let without_selector = remove_top_level_profile_selector(content, profile_name);
    remove_table(&without_selector, "profiles", profile_name)
}

fn remove_top_level_profile_selector(content: &str, profile_name: &str) -> String {
    let mut output = Vec::new();
    let mut in_top_level = true;

    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level {
            if let Some((key, value)) = parse_string_assignment(trimmed) {
                if key == "profile" && value == profile_name {
                    continue;
                }
            }
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn remove_table(content: &str, section: &str, name: &str) -> String {
    let mut output = Vec::new();
    let mut in_target = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_target = parse_table_header(trimmed)
                .map(|(table_section, table_name)| table_section == section && table_name == name)
                .unwrap_or(false);
            if in_target {
                continue;
            }
        }
        if in_target {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n").trim_end().to_string()
}

fn legacy_profile_table_body(content: &str, profile_name: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut in_target = false;
    let mut found = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_target {
                break;
            }
            in_target = parse_table_header(trimmed)
                .map(|(table_section, table_name)| {
                    table_section == "profiles" && table_name == profile_name
                })
                .unwrap_or(false);
            if in_target {
                found = true;
            }
            continue;
        }
        if in_target {
            lines.push(line.to_string());
        }
    }

    if found {
        Some(lines.join("\n").trim_end().to_string())
    } else {
        None
    }
}

fn set_top_level_string_config(content: &str, key: &str, value: &str) -> String {
    let mut output = Vec::new();
    output.push(format!("{} = \"{}\"", key, toml_escape(value)));

    let mut in_top_level = true;
    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level && assignment_key(trimmed).is_some_and(|item| item == key) {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn clear_top_level_string_config(content: &str, key: &str) -> String {
    let mut output = Vec::new();

    let mut in_top_level = true;
    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level && assignment_key(trimmed).is_some_and(|item| item == key) {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn set_top_level_model_config(content: &str, model: &str) -> String {
    let mut output = Vec::new();
    output.push(format!("model = \"{}\"", toml_escape(model)));

    let mut in_top_level = true;
    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level
            && (trimmed.starts_with("model =") || trimmed.starts_with("model_provider ="))
        {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn set_top_level_model_provider_config(content: &str, provider_name: &str, model: &str) -> String {
    let mut output = Vec::new();
    output.push(format!(
        "model_provider = \"{}\"",
        toml_escape(provider_name)
    ));
    output.push(format!("model = \"{}\"", toml_escape(model)));

    let mut in_top_level = true;
    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level
            && (trimmed.starts_with("model =") || trimmed.starts_with("model_provider ="))
        {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn clear_top_level_model_config(content: &str) -> String {
    let mut output = Vec::new();
    let mut in_top_level = true;
    for line in content.lines() {
        let trimmed = line.trim();
        if in_top_level && trimmed.starts_with('[') {
            in_top_level = false;
        }
        if in_top_level
            && (trimmed.starts_with("model =") || trimmed.starts_with("model_provider ="))
        {
            continue;
        }
        output.push(line.to_string());
    }

    output.join("\n")
}

fn dedupe_provider_profiles(profiles: Vec<ProviderProfile>) -> Vec<ProviderProfile> {
    let mut deduped = Vec::new();
    for mut profile in profiles {
        profile.id = profile.id.trim().to_string();
        profile.name = profile.name.trim().to_string();
        profile.codex_profile_name = profile.codex_profile_name.trim().to_string();
        profile.provider_name = profile.provider_name.trim().to_string();
        profile.provider_config_format =
            normalized_provider_config_format(&profile.provider_config_format);
        profile.base_url = profile.base_url.trim().to_string();
        profile.model = profile.model.trim().to_string();
        profile.proxy_url = profile.proxy_url.trim().to_string();
        profile.remote_frontend_mode =
            normalized_remote_frontend_mode(&profile.remote_frontend_mode);
        profile.remote_web_asset_registry_url =
            normalized_remote_web_asset_registry_url(&profile.remote_web_asset_registry_url);
        profile.remote_web_asset_version =
            normalized_remote_web_asset_version(&profile.remote_web_asset_version);
        profile.codex_home = normalize_home_path(&profile.codex_home);
        if profile.name.is_empty() {
            profile.name = profile.provider_name.clone();
        }
        if provider_profile_uses_top_level_config(&profile) {
            profile.codex_profile_name = DEFAULT_PROVIDER_PROFILE_NAME.to_string();
        } else if profile.codex_profile_name.is_empty() && !profile.provider_name.is_empty() {
            profile.codex_profile_name = profile.name.clone();
        }
        if profile.name == DEFAULT_PROVIDER_PROFILE_NAME {
            let id = profile.id.clone();
            let bot = profile.bot.clone();
            let proxy_url = profile.proxy_url.clone();
            let start_remote_on_launch = profile.start_remote_on_launch;
            let start_remote_cloud_on_launch = profile.start_remote_cloud_on_launch;
            let start_remote_e2ee_on_launch = profile.start_remote_e2ee_on_launch;
            let remote_e2ee_password = profile.remote_e2ee_password.clone();
            let remote_frontend_mode = profile.remote_frontend_mode.clone();
            let remote_web_asset_registry_url = profile.remote_web_asset_registry_url.clone();
            let remote_web_asset_version = profile.remote_web_asset_version.clone();
            profile = default_provider_profile();
            profile.id = id;
            profile.bot = bot;
            profile.proxy_url = proxy_url;
            profile.remote_frontend_mode = remote_frontend_mode;
            profile.remote_web_asset_registry_url = remote_web_asset_registry_url;
            profile.remote_web_asset_version = remote_web_asset_version;
            profile.start_remote_on_launch = start_remote_on_launch;
            profile.start_remote_cloud_on_launch = start_remote_cloud_on_launch;
            profile.start_remote_e2ee_on_launch = start_remote_e2ee_on_launch;
            profile.remote_e2ee_password = remote_e2ee_password;
        }
        if !profile.start_remote_on_launch {
            profile.start_remote_cloud_on_launch = false;
        }
        profile.start_remote_e2ee_on_launch =
            profile.start_remote_on_launch && profile.start_remote_cloud_on_launch;
        if !profile.start_remote_e2ee_on_launch {
            profile.remote_e2ee_password.clear();
        }
        ensure_provider_profile_identity(&mut profile);
        profile
            .bot
            .normalize_for_profile_instance(&profile.name, &profile.id);
        let has_provider = !profile.provider_name.is_empty() && !profile.model.is_empty();
        let is_providerless = profile.provider_name.is_empty() && profile.model.is_empty();
        if profile.name.is_empty() || (!has_provider && !is_providerless) {
            continue;
        }
        let profile_key = provider_profile_key(&profile);
        if !deduped.iter().any(|existing: &ProviderProfile| {
            provider_profile_key(existing) == profile_key
                || (is_default_provider(existing) && is_default_provider(&profile))
        }) {
            deduped.push(profile);
        }
    }
    deduped
}

fn normalize_saved_bot_configs(configs: Vec<SavedBotConfig>) -> Vec<SavedBotConfig> {
    let mut normalized = Vec::new();
    for config in configs {
        upsert_saved_bot_config(&mut normalized, config, false);
    }
    normalized
}

fn upsert_saved_bot_config(
    configs: &mut Vec<SavedBotConfig>,
    mut config: SavedBotConfig,
    preserve_existing_name: bool,
) {
    config.id = config.id.trim().to_string();
    config.name = config.name.trim().to_string();
    config.updated_at = config.updated_at.trim().to_string();
    if config.id.is_empty() {
        let saved_config_id = config.bot.saved_config_id.trim();
        config.id = if saved_config_id.is_empty() {
            config.bot.integration_id.trim().to_string()
        } else {
            saved_config_id.to_string()
        };
    }
    if config.id.is_empty() {
        config.id = new_uuid_v4();
    }
    if config.name.is_empty() {
        config.name = saved_bot_config_fallback_name(&config);
    }
    config.bot.saved_config_id = config.id.clone();
    config.bot.normalize_for_saved_config(&config.name);
    if !config.bot.bridge_enabled() || config.bot.integration_id.trim().is_empty() {
        return;
    }

    if let Some(existing) = configs.iter_mut().find(|item| item.id == config.id) {
        let existing_name = existing.name.clone();
        *existing = config;
        if preserve_existing_name && !existing_name.trim().is_empty() {
            existing.name = existing_name;
        }
        return;
    }
    configs.push(config);
}

fn saved_bot_config_from_profile(
    profile: &ProviderProfile,
    updated_at: Option<String>,
) -> Option<SavedBotConfig> {
    let mut bot = profile.bot.clone();
    bot.normalize_for_profile_instance(&profile.name, &profile.id);
    if !bot.bridge_enabled() {
        return None;
    }
    if bot.state_dir.trim().is_empty() {
        bot.state_dir = generated_bot_gateway_state_dir(&profile.name)
            .to_string_lossy()
            .to_string();
    }
    if bot.saved_config_id.trim().is_empty() {
        bot.saved_config_id = bot.integration_id.trim().to_string();
    }
    bot.normalize_for_saved_config(&profile.name);
    if bot.integration_id.trim().is_empty() {
        return None;
    }

    Some(SavedBotConfig {
        id: bot.saved_config_id.clone(),
        name: profile.name.clone(),
        bot,
        updated_at: updated_at
            .or_else(|| {
                let value = profile.bot.last_login_at.trim();
                if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            })
            .unwrap_or_default(),
    })
}

fn saved_bot_config_fallback_name(config: &SavedBotConfig) -> String {
    let platform = normalize_bot_platform(&config.bot.platform);
    let platform = if platform == BOT_PLATFORM_NONE {
        "Bot".to_string()
    } else {
        platform
    };
    let integration_id = config.bot.integration_id.trim();
    if integration_id.is_empty() {
        platform
    } else {
        format!("{} {}", platform, short_identifier(integration_id))
    }
}

fn short_identifier(value: &str) -> String {
    let value = value.trim();
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= 8 {
        value.to_string()
    } else {
        chars[chars.len() - 8..].iter().collect()
    }
}

fn normalize_bot_platform(platform: &str) -> String {
    match platform.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "off" | "disabled" => BOT_PLATFORM_NONE.to_string(),
        "slack" => BOT_PLATFORM_SLACK.to_string(),
        "discord" => BOT_PLATFORM_DISCORD.to_string(),
        "telegram" | "tg" => BOT_PLATFORM_TELEGRAM.to_string(),
        "line" => BOT_PLATFORM_LINE.to_string(),
        "feishu" | "lark" => BOT_PLATFORM_FEISHU.to_string(),
        "dingtalk" | "dingding" => BOT_PLATFORM_DINGTALK.to_string(),
        "wechat" | "weixin" | "wx" | "weixin-ilink" | "weixin_ilink" | "ilink" => {
            BOT_PLATFORM_WEIXIN_ILINK.to_string()
        }
        "wecom" | "wework" | "wechat-work" | "work-weixin" | "enterprise-wechat" => {
            BOT_PLATFORM_WECOM.to_string()
        }
        other => other.to_string(),
    }
}

fn normalize_bot_auth_type(platform: &str, auth_type: &str) -> String {
    let platform = normalize_bot_platform(platform);
    if platform == BOT_PLATFORM_NONE {
        return String::new();
    }

    let normalized = match auth_type.trim().to_ascii_lowercase().as_str() {
        "" | "default" | "auto" => default_bot_auth_type(&platform),
        "appsecret" | "app-secret" | "app_secret" => BOT_AUTH_APP_SECRET.to_string(),
        "bottoken" | "bot-token" | "bot_token" | "token" => BOT_AUTH_BOT_TOKEN.to_string(),
        "oauth" | "oauth2" | "oauth_2" | "oauth-2" => BOT_AUTH_OAUTH2.to_string(),
        "qr" | "qr_login" | "qr-login" | "qrcode" | "qr_code" => BOT_AUTH_QR_LOGIN.to_string(),
        "webhook" | "webhook_secret" | "webhook-secret" => BOT_AUTH_WEBHOOK_SECRET.to_string(),
        other => other.to_string(),
    };

    if bot_auth_type_supported(&platform, &normalized) {
        normalized
    } else {
        default_bot_auth_type(&platform)
    }
}

fn default_bot_auth_type(platform: &str) -> String {
    match platform {
        BOT_PLATFORM_WEIXIN_ILINK => BOT_AUTH_QR_LOGIN,
        BOT_PLATFORM_FEISHU | BOT_PLATFORM_DINGTALK | BOT_PLATFORM_WECOM => BOT_AUTH_APP_SECRET,
        BOT_PLATFORM_SLACK | BOT_PLATFORM_DISCORD | BOT_PLATFORM_TELEGRAM | BOT_PLATFORM_LINE => {
            BOT_AUTH_BOT_TOKEN
        }
        _ => "",
    }
    .to_string()
}

fn bot_auth_type_supported(platform: &str, auth_type: &str) -> bool {
    match platform {
        BOT_PLATFORM_WEIXIN_ILINK => matches!(auth_type, BOT_AUTH_QR_LOGIN | BOT_AUTH_BOT_TOKEN),
        BOT_PLATFORM_WECOM => matches!(auth_type, BOT_AUTH_APP_SECRET),
        BOT_PLATFORM_SLACK => matches!(
            auth_type,
            BOT_AUTH_BOT_TOKEN | BOT_AUTH_OAUTH2 | BOT_AUTH_WEBHOOK_SECRET
        ),
        BOT_PLATFORM_DISCORD => matches!(auth_type, BOT_AUTH_BOT_TOKEN | BOT_AUTH_OAUTH2),
        BOT_PLATFORM_TELEGRAM | BOT_PLATFORM_LINE => matches!(auth_type, BOT_AUTH_BOT_TOKEN),
        BOT_PLATFORM_FEISHU => matches!(auth_type, BOT_AUTH_APP_SECRET),
        BOT_PLATFORM_DINGTALK => matches!(auth_type, BOT_AUTH_APP_SECRET),
        _ => false,
    }
}

fn normalize_bot_auth_fields(
    platform: &str,
    fields: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    fields
        .into_iter()
        .filter(|(key, _)| bot_auth_field_supported(platform, key))
        .collect()
}

fn bot_auth_field_supported(platform: &str, key: &str) -> bool {
    match platform {
        BOT_PLATFORM_WECOM => matches!(key, "corpId" | "agentId" | "secret"),
        BOT_PLATFORM_TELEGRAM => matches!(key, "botToken"),
        BOT_PLATFORM_LINE => matches!(key, "channelAccessToken" | "channelSecret"),
        BOT_PLATFORM_DINGTALK => matches!(key, "appKey" | "appSecret" | "robotCode"),
        _ => true,
    }
}

fn is_uuid_like(value: &str) -> bool {
    let value = value.trim();
    if value.len() != 36 {
        return false;
    }
    for (index, ch) in value.chars().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if ch != '-' {
                    return false;
                }
            }
            _ => {
                if !ch.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

fn new_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if rand::rngs::OsRng.try_fill_bytes(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let counter = UUID_FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let fallback = nanos ^ ((std::process::id() as u128) << 64) ^ counter;
        bytes = fallback.to_be_bytes();
    }

    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn validate_provider_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    if name.eq_ignore_ascii_case(DEFAULT_PROVIDER_PROFILE_NAME) {
        return Err("Default is reserved".to_string());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err("name can only contain letters, numbers, '-' and '_'".to_string());
    }
    Ok(())
}

fn validate_workspace_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    if name == DEFAULT_PROVIDER_PROFILE_NAME {
        return Err("Default is reserved".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("name cannot contain control characters".to_string());
    }
    Ok(())
}

fn workspace_name_or_default(workspace_name: &str, fallback: &str) -> Result<String, String> {
    let name = workspace_name.trim();
    let name = if name.is_empty() {
        fallback.trim()
    } else {
        name
    };
    validate_workspace_name(name)?;
    Ok(name.to_string())
}

pub fn provider_profile_key(profile: &ProviderProfile) -> String {
    let id = profile.id.trim();
    if is_uuid_like(id) {
        id.to_string()
    } else {
        profile.name.trim().to_string()
    }
}

fn ensure_provider_profile_identity(profile: &mut ProviderProfile) {
    profile.id = profile.id.trim().to_string();
    if !is_uuid_like(&profile.id) {
        profile.id = if is_uuid_like(&profile.bot.integration_id) {
            profile.bot.integration_id.trim().to_string()
        } else {
            new_uuid_v4()
        };
    }
    profile.codex_home = normalize_home_path(&profile.codex_home);
    if profile.codex_home.is_empty() {
        profile.codex_home = profile_codex_home(profile);
    }
}

fn is_remote_control_token(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn normalized_remote_frontend_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        REMOTE_FRONTEND_MODE_CLI => REMOTE_FRONTEND_MODE_CLI.to_string(),
        REMOTE_FRONTEND_MODE_CLAUDE_CODE => REMOTE_FRONTEND_MODE_CLAUDE_CODE.to_string(),
        _ => REMOTE_FRONTEND_MODE_APP.to_string(),
    }
}

pub fn remote_frontend_mode_uses_cli(value: &str) -> bool {
    matches!(
        normalized_remote_frontend_mode(value).as_str(),
        REMOTE_FRONTEND_MODE_CLI | REMOTE_FRONTEND_MODE_CLAUDE_CODE
    )
}

pub fn normalized_remote_web_asset_registry_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub fn normalized_remote_web_asset_version(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "latest".to_string()
    } else {
        value.to_string()
    }
}

fn normalize_transcribe_base_url(value: &str) -> String {
    const TRANSCRIBE_ENDPOINT_SUFFIX: &str = "/audio/transcriptions";

    let mut base_url = value.trim().trim_end_matches('/').to_string();
    if base_url
        .to_ascii_lowercase()
        .ends_with(TRANSCRIBE_ENDPOINT_SUFFIX)
    {
        let next_len = base_url.len() - TRANSCRIBE_ENDPOINT_SUFFIX.len();
        base_url.truncate(next_len);
        base_url = base_url.trim_end_matches('/').to_string();
    }
    base_url
}

fn default_provider_profile() -> ProviderProfile {
    let mut profile = ProviderProfile {
        id: new_uuid_v4(),
        name: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
        codex_profile_name: String::new(),
        provider_name: String::new(),
        provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
        base_url: String::new(),
        model: String::new(),
        proxy_url: String::new(),
        remote_frontend_mode: default_remote_frontend_mode(),
        remote_web_asset_registry_url: String::new(),
        remote_web_asset_version: "latest".to_string(),
        codex_home: default_codex_home(),
        start_remote_on_launch: false,
        start_remote_cloud_on_launch: false,
        start_remote_e2ee_on_launch: false,
        remote_e2ee_password: String::new(),
        bot: BotProfileConfig::default(),
    };

    if let Ok(content) = std::fs::read_to_string(default_codex_config_path()) {
        let (model, provider_name) = parse_top_level_model_config(&content);
        if !provider_name.is_empty() {
            profile.provider_name = provider_name.clone();
            profile.provider_config_format = DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL.to_string();
            profile.base_url =
                provider_table_value(&content, &provider_name, "base_url").unwrap_or_default();
        }
        if !model.is_empty() {
            profile.model = model;
        }
        if profile.provider_name.is_empty() && !profile.model.is_empty() {
            if let Some(default_profile) = parse_default_provider_profiles(&content)
                .into_iter()
                .find(|item| item.model == profile.model)
            {
                profile.codex_profile_name = default_profile.name;
                profile.provider_name = default_profile.provider_name;
                profile.base_url = default_profile.base_url;
            }
        }
        if !profile.model.is_empty() && profile.provider_name.is_empty() {
            profile.provider_name = "default".to_string();
        }
    }

    profile
}

fn default_remote_frontend_mode() -> String {
    if cfg!(windows) {
        REMOTE_FRONTEND_MODE_CLI.to_string()
    } else {
        REMOTE_FRONTEND_MODE_APP.to_string()
    }
}

fn is_default_provider(profile: &ProviderProfile) -> bool {
    profile.name == DEFAULT_PROVIDER_PROFILE_NAME
}

fn is_providerless_workspace(profile: &ProviderProfile) -> bool {
    profile.provider_name.trim().is_empty() && profile.model.trim().is_empty()
}

fn uses_top_level_provider_config(profile: &DefaultProviderProfile) -> bool {
    profile
        .config_format
        .trim()
        .eq_ignore_ascii_case(DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL)
}

fn provider_codex_profile_name(profile: &DefaultProviderProfile) -> String {
    if uses_top_level_provider_config(profile) {
        DEFAULT_PROVIDER_PROFILE_NAME.to_string()
    } else {
        profile.name.clone()
    }
}

fn provider_profile_uses_top_level_config(profile: &ProviderProfile) -> bool {
    profile
        .provider_config_format
        .trim()
        .eq_ignore_ascii_case(DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL)
}

fn normalized_provider_config_format(value: &str) -> String {
    if value
        .trim()
        .eq_ignore_ascii_case(DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL)
    {
        DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL.to_string()
    } else {
        DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string()
    }
}

fn profile_codex_home(profile: &ProviderProfile) -> String {
    if is_default_provider(profile) {
        default_codex_home()
    } else {
        generated_codex_home(profile).to_string_lossy().to_string()
    }
}

fn slugify(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if slug.is_empty() {
        "default".to_string()
    } else {
        slug
    }
}

fn short_profile_id(id: &str) -> String {
    id.chars().filter(|ch| *ch != '-').take(8).collect()
}

fn toml_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn toml_key(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        value.to_string()
    } else {
        format!("\"{}\"", toml_escape(value))
    }
}

fn unquote_toml_key(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') {
        parse_toml_string(trimmed).unwrap_or_else(|| trimmed.to_string())
    } else {
        trimmed.to_string()
    }
}

fn env_string(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn env_u16(name: &str, fallback: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(fallback)
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn env_bool(name: &str, fallback: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(fallback)
}

fn requested_codex_profile_config_format() -> CodexProfileConfigFormat {
    codex_profile_config_format_from_env().unwrap_or_else(default_codex_profile_config_format)
}

fn default_codex_profile_config_format() -> CodexProfileConfigFormat {
    if cfg!(test) {
        CodexProfileConfigFormat::LegacyProfilesTable
    } else {
        CodexProfileConfigFormat::SeparateProfileFiles
    }
}

fn parse_codex_profile_config_format(value: &str) -> Option<CodexProfileConfigFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "legacy" | "profiles" | "profiles_table" | "profile_table" => {
            Some(CodexProfileConfigFormat::LegacyProfilesTable)
        }
        "separate" | "separate_profile_files" | "profile_files" | "profile_file" | "new" => {
            Some(CodexProfileConfigFormat::SeparateProfileFiles)
        }
        _ => None,
    }
}

fn codex_cli_version(executable: &str) -> Option<(u64, u64, u64)> {
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .ok()?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    parse_codex_cli_version(&text)
}

fn codex_profile_config_format_for_version(version: (u64, u64, u64)) -> CodexProfileConfigFormat {
    if version >= CODEX_PROFILE_CONFIG_FILES_MIN_VERSION {
        CodexProfileConfigFormat::SeparateProfileFiles
    } else {
        CodexProfileConfigFormat::LegacyProfilesTable
    }
}

fn parse_codex_cli_version(value: &str) -> Option<(u64, u64, u64)> {
    for token in
        value.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '+'))
    {
        let token = token.trim().trim_start_matches('v');
        let token = token
            .split_once('-')
            .map(|(version, _)| version)
            .unwrap_or(token);
        let token = token
            .split_once('+')
            .map(|(version, _)| version)
            .unwrap_or(token);
        let mut parts = token.split('.');
        let Some(major) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
            continue;
        };
        let Some(minor) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
            continue;
        };
        let Some(patch) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
            continue;
        };
        return Some((major, minor, patch));
    }
    None
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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
    fn claude_code_remote_frontend_mode_uses_cli_backend() {
        assert!(remote_frontend_mode_uses_cli(REMOTE_FRONTEND_MODE_CLI));
        assert!(remote_frontend_mode_uses_cli(
            REMOTE_FRONTEND_MODE_CLAUDE_CODE
        ));
        assert!(!remote_frontend_mode_uses_cli(REMOTE_FRONTEND_MODE_APP));
    }

    #[test]
    fn top_level_model_config_does_not_add_model_provider() {
        let content = r#"model = "old-model"
model_provider = "old-provider"

[profiles.custom]
model = "profile-model"
model_provider = "profile-provider"
"#;

        let updated = set_top_level_model_config(content, "new-model");
        let top_level = updated
            .split("[profiles.custom]")
            .next()
            .expect("top-level content");

        assert!(top_level.contains("model = \"new-model\""));
        assert!(!top_level.contains("model_provider ="));
        assert!(updated.contains("model_provider = \"profile-provider\""));
    }

    #[test]
    fn codex_cli_version_selects_profile_config_format() {
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.133.2"),
            Some((0, 133, 2))
        );
        assert_eq!(
            parse_codex_cli_version("codex 0.134.0-beta.1"),
            Some((0, 134, 0))
        );
        assert_eq!(
            codex_profile_config_format_for_version((0, 133, 2)),
            CodexProfileConfigFormat::LegacyProfilesTable
        );
        assert_eq!(
            codex_profile_config_format_for_version((0, 134, 0)),
            CodexProfileConfigFormat::SeparateProfileFiles
        );
    }

    #[test]
    fn top_level_string_config_preserves_profile_values() {
        let content = r#"model_catalog_json = "/old/catalog.json"

[profiles.nextai]
model_catalog_json = "/profile/catalog.json"
"#;

        let updated =
            set_top_level_string_config(content, "model_catalog_json", "/new/catalog.json");
        let top_level = updated
            .split("[profiles.nextai]")
            .next()
            .expect("top-level content");

        assert!(top_level.contains("model_catalog_json = \"/new/catalog.json\""));
        assert!(!top_level.contains("/old/catalog.json"));
        assert!(updated.contains("model_catalog_json = \"/profile/catalog.json\""));

        let cleared = clear_top_level_string_config(&updated, "model_catalog_json");
        let top_level = cleared
            .split("[profiles.nextai]")
            .next()
            .expect("top-level content");
        assert!(!top_level.contains("model_catalog_json = "));
        assert!(cleared.contains("model_catalog_json = \"/profile/catalog.json\""));
    }

    #[test]
    fn ccs_top_level_provider_is_discovered_and_preserved() {
        let content = r#"model_provider = "custom"
model = "gpt-5.5"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers]
[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
"#;

        let providers = parse_default_provider_profiles(content);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "custom");
        assert_eq!(providers[0].provider_name, "custom");
        assert_eq!(providers[0].model, "gpt-5.5");
        assert_eq!(
            providers[0].config_format,
            DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL
        );

        let mut provider = providers[0].clone();
        provider.model = "gpt-5.6".to_string();
        let updated = upsert_provider_profile_content(content, &provider, false);

        assert!(updated.contains("model_provider = \"custom\""));
        assert!(updated.contains("model = \"gpt-5.6\""));
        assert!(updated.contains("requires_openai_auth = true"));
        assert!(!updated.contains("[profiles.custom]"));
        assert!(!updated.contains("base_url = \"\""));
    }

    #[test]
    fn ccs_existing_provider_workspace_writes_top_level_config() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("ccs-workspace");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"model_provider = "custom"
model = "gpt-5.5"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers]
[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
"#,
        )
        .expect("write ccs config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = add_existing_provider_profile(ExistingProviderRequest {
            workspace_name: "workspace".to_string(),
            profile_name: "custom".to_string(),
            base_url: Some(String::new()),
            api_key: None,
            model: "gpt-5.6".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: String::new(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: String::new(),
            bot: BotProfileConfig::default(),
        })
        .expect("add ccs provider");

        assert_eq!(
            profile.provider_config_format,
            DEFAULT_PROVIDER_CONFIG_FORMAT_TOP_LEVEL
        );
        assert_eq!(profile.codex_profile_name, DEFAULT_PROVIDER_PROFILE_NAME);

        let content = std::fs::read_to_string(generated_codex_home(&profile).join("config.toml"))
            .expect("read generated ccs config");
        assert!(content.contains("model_provider = \"custom\""));
        assert!(content.contains("model = \"gpt-5.5\""));
        assert!(content.contains("requires_openai_auth = true"));
        assert!(!content.contains("[profiles.custom]"));
        assert!(!content.contains("base_url = \"\""));

        let config = AppConfig {
            active_provider: provider_profile_key(&profile),
            provider_profiles: vec![profile],
            ..AppConfig::default()
        };
        assert_eq!(config.active_cli_profile(), None);
        assert_eq!(config.active_cli_model_provider(), None);

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn next_ai_gateway_provider_writes_codex_model_catalog() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("nextai-gateway-catalog");
        let old_home = std::env::var("HOME").ok();
        let old_codexl_home = std::env::var("CODEXL_HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();
        let old_gateway_home = std::env::var("CODEXL_NEXT_AI_GATEWAY_HOME").ok();
        let old_gateway_config_path = std::env::var("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH").ok();
        let old_legacy_gateway_config_path = std::env::var("GATEWAY_CONFIG_PATH").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(root.join(".codex").join("config.toml"), "").expect("write default config");

        let codexl_home = root.join(".codexl");
        let gateway_home = root.join("gateway-home");
        let gateway_config_path = root.join("gateway.config.json");
        std::fs::write(
            &gateway_config_path,
            r#"{
  "host": "127.0.0.1",
  "port": 14589,
  "Providers": [
    { "name": "openai", "models": ["gpt-4.1", "openai/o3"] },
    { "name": "zhipu", "models": [{ "id": "glm-4.6" }, { "model": "/glm-4.5" }] }
  ]
}"#,
        )
        .expect("write gateway config");

        std::env::set_var("HOME", &root);
        std::env::set_var("CODEXL_HOME", &codexl_home);
        std::env::remove_var("CODEXL_CODEX_HOME");
        std::env::set_var("CODEXL_NEXT_AI_GATEWAY_HOME", &gateway_home);
        std::env::set_var("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH", &gateway_config_path);
        std::env::remove_var("GATEWAY_CONFIG_PATH");

        let profile = create_next_ai_gateway_provider(NextAiGatewayProviderRequest {
            workspace_name: "gateway-workspace".to_string(),
            name: "nextai".to_string(),
            model: "zhipu/glm-4.6".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: String::new(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: String::new(),
            bot: BotProfileConfig::default(),
        })
        .expect("create next ai gateway provider");

        let workspace_config_path = generated_codex_home(&profile).join("config.toml");
        let workspace_config =
            std::fs::read_to_string(&workspace_config_path).expect("read workspace config");
        assert!(workspace_config.contains("model = \"zhipu/glm-4.6\""));
        assert!(workspace_config.contains("model_provider = \"next-ai-gateway\""));
        assert!(workspace_config.contains("model_catalog_json = "));

        let default_config = std::fs::read_to_string(root.join(".codex").join("config.toml"))
            .expect("read default config");
        assert!(!default_config.contains("model_catalog_json"));

        let catalog_line = workspace_config
            .lines()
            .find(|line| line.trim_start().starts_with("model_catalog_json = "))
            .expect("model catalog config");
        let catalog_path = parse_toml_string(
            catalog_line
                .split_once('=')
                .expect("catalog assignment")
                .1
                .trim(),
        )
        .expect("catalog path string");
        let catalog = std::fs::read_to_string(&catalog_path).expect("read model catalog");
        let catalog_json: serde_json::Value =
            serde_json::from_str(&catalog).expect("parse model catalog");
        let models = catalog_json
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models array");
        let slugs = models
            .iter()
            .map(|model| {
                model
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .expect("model slug")
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(slugs.first().map(String::as_str), Some("zhipu/glm-4.6"));
        assert!(slugs.contains(&"openai/gpt-4.1".to_string()));
        assert!(slugs.contains(&"openai/o3".to_string()));
        assert!(slugs.contains(&"zhipu/glm-4.5".to_string()));
        assert_eq!(
            slugs
                .iter()
                .filter(|slug| slug.as_str() == "zhipu/glm-4.6")
                .count(),
            1
        );

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codexl_home {
            std::env::set_var("CODEXL_HOME", value);
        } else {
            std::env::remove_var("CODEXL_HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        if let Some(value) = old_gateway_home {
            std::env::set_var("CODEXL_NEXT_AI_GATEWAY_HOME", value);
        } else {
            std::env::remove_var("CODEXL_NEXT_AI_GATEWAY_HOME");
        }
        if let Some(value) = old_gateway_config_path {
            std::env::set_var("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH", value);
        } else {
            std::env::remove_var("CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH");
        }
        if let Some(value) = old_legacy_gateway_config_path {
            std::env::set_var("GATEWAY_CONFIG_PATH", value);
        } else {
            std::env::remove_var("GATEWAY_CONFIG_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_path_env_overrides_saved_config_on_load() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("codex-path-env");
        let old_home = std::env::var("HOME").ok();
        let old_config_path = std::env::var("CODEXL_CONFIG_PATH").ok();
        let old_codex_path = std::env::var("CODEXL_CODEX_PATH").ok();
        let config_path = root.join("config.json");
        let env_codex_path = root.join("Codex.exe");

        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(&env_codex_path, "").expect("write codex path");
        std::fs::write(
            &config_path,
            r#"{
  "codex_path": "/old/Codex.exe"
}"#,
        )
        .expect("write config");

        std::env::set_var("HOME", &root);
        std::env::set_var("CODEXL_CONFIG_PATH", &config_path);
        std::env::set_var("CODEXL_CODEX_PATH", &env_codex_path);

        let config = AppConfig::load();
        assert_eq!(config.codex_path, env_codex_path.to_string_lossy());

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_config_path {
            std::env::set_var("CODEXL_CONFIG_PATH", value);
        } else {
            std::env::remove_var("CODEXL_CONFIG_PATH");
        }
        if let Some(value) = old_codex_path {
            std::env::set_var("CODEXL_CODEX_PATH", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn initial_load_enables_builtin_extensions_when_user_node_available_once() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("initial-extension-defaults");
        let old_home = std::env::var("HOME").ok();
        let old_config_path = std::env::var("CODEXL_CONFIG_PATH").ok();
        let old_node_path = std::env::var("CODEXL_NODE_PATH").ok();
        let config_path = root.join("config.json");
        let node_path = root.join("node");

        std::fs::create_dir_all(&root).expect("create root");
        std::fs::write(&node_path, "#!/bin/sh\necho v20.0.0\n").expect("write fake node");
        let mut permissions = std::fs::metadata(&node_path)
            .expect("fake node metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&node_path, permissions).expect("make fake node executable");

        std::env::set_var("HOME", &root);
        std::env::set_var("CODEXL_CONFIG_PATH", &config_path);
        std::env::set_var("CODEXL_NODE_PATH", &node_path);

        let config = AppConfig::load();
        assert!(config.extensions.enabled);
        assert!(config.extensions.bot_gateway_enabled);
        assert!(config.extensions.next_ai_gateway_enabled);

        let mut manually_disabled = config.clone();
        manually_disabled.extensions.enabled = false;
        manually_disabled.extensions.bot_gateway_enabled = false;
        manually_disabled.extensions.next_ai_gateway_enabled = false;
        manually_disabled.save().expect("save disabled extensions");

        let reloaded = AppConfig::load();
        assert!(!reloaded.extensions.enabled);
        assert!(!reloaded.extensions.bot_gateway_enabled);
        assert!(!reloaded.extensions.next_ai_gateway_enabled);

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_config_path {
            std::env::set_var("CODEXL_CONFIG_PATH", value);
        } else {
            std::env::remove_var("CODEXL_CONFIG_PATH");
        }
        if let Some(value) = old_node_path {
            std::env::set_var("CODEXL_NODE_PATH", value);
        } else {
            std::env::remove_var("CODEXL_NODE_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_provider_does_not_inject_cli_profile_overrides() {
        let config = AppConfig {
            active_provider: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
            provider_profiles: vec![ProviderProfile {
                name: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
                codex_profile_name: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
                provider_name: "custom-provider".to_string(),
                model: "custom-model".to_string(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };

        assert_eq!(config.active_cli_profile(), None);
        assert_eq!(config.active_cli_model_provider(), None);
    }

    #[test]
    fn app_config_normalize_populates_device_uuid() {
        let mut config = AppConfig {
            device_uuid: "not-a-uuid".to_string(),
            ..AppConfig::default()
        };

        config.normalize();

        assert!(is_uuid_like(&config.device_uuid));
    }

    #[test]
    fn non_default_provider_injects_cli_profile_overrides() {
        let config = AppConfig {
            active_provider: "custom".to_string(),
            provider_profiles: vec![ProviderProfile {
                name: "custom".to_string(),
                codex_profile_name: "codex-profile".to_string(),
                provider_name: "custom-provider".to_string(),
                model: "custom-model".to_string(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };

        assert_eq!(
            config.active_cli_profile(),
            Some("codex-profile".to_string())
        );
        assert_eq!(
            config.active_cli_model_provider(),
            Some("custom-provider".to_string())
        );
    }

    #[test]
    fn next_ai_gateway_provider_uses_codex_home_top_level_config() {
        let config = AppConfig {
            active_provider: "gateway".to_string(),
            provider_profiles: vec![ProviderProfile {
                name: "gateway".to_string(),
                codex_profile_name: "nextai".to_string(),
                provider_name: gateway_config::NEXT_AI_GATEWAY_PROVIDER_NAME.to_string(),
                model: "openai/gpt-4.1".to_string(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };

        assert_eq!(config.active_cli_profile(), None);
        assert_eq!(config.active_cli_model_provider(), None);
    }

    #[test]
    fn providerless_workspace_does_not_inject_cli_overrides() {
        let mut config = AppConfig {
            active_provider: "workspace".to_string(),
            provider_profiles: vec![ProviderProfile {
                name: "workspace".to_string(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };

        config.normalize();

        let profile = config
            .provider_profile("workspace")
            .expect("providerless workspace should be kept");
        assert!(profile.codex_profile_name.is_empty());
        assert!(profile.provider_name.is_empty());
        assert!(profile.model.is_empty());
        assert_eq!(config.active_cli_profile(), None);
        assert_eq!(config.active_cli_model_provider(), None);
    }

    #[test]
    fn providerless_workspace_clears_top_level_model_config() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("workspace-providerless");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"model = "old-model"
model_provider = "old-provider"

[model_providers.old-provider]
name = "old-provider"
base_url = "https://api.example/v1"

[profiles.old-profile]
model = "profile-model"
model_provider = "old-provider"
"#,
        )
        .expect("write default config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = create_workspace_profile(WorkspaceRequest {
            workspace_name: "workspace-none".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: String::new(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: String::new(),
            bot: BotProfileConfig::default(),
        })
        .expect("create providerless workspace");

        assert_eq!(profile.name, "workspace-none");
        assert!(profile.codex_profile_name.is_empty());
        assert!(profile.provider_name.is_empty());
        assert!(profile.model.is_empty());

        let workspace_config_path = generated_codex_home(&profile).join("config.toml");
        let workspace_config =
            std::fs::read_to_string(workspace_config_path).expect("read workspace config");
        let top_level = workspace_config
            .split("[model_providers.old-provider]")
            .next()
            .expect("top-level content");
        assert!(!top_level.contains("model ="));
        assert!(!top_level.contains("model_provider ="));
        assert!(workspace_config.contains("[profiles.old-profile]"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_name_accepts_human_readable_text() {
        let profile = workspace_profile(
            "客户 Project #1".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            BotProfileConfig::default(),
        )
        .expect("workspace name should allow spaces and non-ascii text");

        assert_eq!(profile.name, "客户 Project #1");
    }

    #[test]
    fn app_config_keeps_duplicate_workspace_names_by_id() {
        let mut config = AppConfig {
            active_provider: DEFAULT_PROVIDER_PROFILE_NAME.to_string(),
            provider_profiles: vec![default_provider_profile()],
            ..AppConfig::default()
        };
        let first = workspace_profile(
            "Same Workspace".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            BotProfileConfig::default(),
        )
        .expect("first workspace");
        let second = workspace_profile(
            "Same Workspace".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            BotProfileConfig::default(),
        )
        .expect("second workspace");

        config.add_provider_profile(first);
        config.add_provider_profile(second);
        config.normalize();

        let duplicates = config
            .provider_profiles
            .iter()
            .filter(|profile| profile.name == "Same Workspace")
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 2);
        assert_ne!(duplicates[0].id, duplicates[1].id);
        assert!(config.provider_profile(&duplicates[0].id).is_some());
        assert!(config.provider_profile(&duplicates[1].id).is_some());
    }

    #[test]
    fn remote_control_token_stays_bound_to_provider_profile_id() {
        let profile_id = "11111111-1111-4111-8111-111111111111";
        let mut config = AppConfig {
            provider_profiles: vec![ProviderProfile {
                id: profile_id.to_string(),
                name: "workspace".to_string(),
                provider_name: String::new(),
                model: String::new(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };
        config.normalize();

        let (first, first_created) = config
            .remote_control_token_for_profile("workspace", || "a".repeat(64))
            .expect("first token");
        let (second, second_created) = config
            .remote_control_token_for_profile("workspace", || "b".repeat(64))
            .expect("second token");

        assert!(first_created);
        assert!(!second_created);
        assert_eq!(first, second);

        let profile_index = config
            .provider_profiles
            .iter()
            .position(|profile| profile.id == profile_id)
            .expect("profile index");
        config.provider_profiles[profile_index].name = "renamed-workspace".to_string();
        config.normalize();
        let (renamed, renamed_created) = config
            .remote_control_token_for_profile("renamed-workspace", || "c".repeat(64))
            .expect("renamed token");

        assert!(!renamed_created);
        assert_eq!(first, renamed);
        assert_eq!(
            config
                .remote_control_tokens
                .get(profile_id)
                .map(String::as_str),
            Some(first.as_str())
        );
    }

    #[test]
    fn remote_cloud_auth_requires_user_and_unexpired_token() {
        let mut auth = RemoteCloudAuthConfig {
            user_id: "user-1".to_string(),
            access_token: "token".to_string(),
            expires_at: now_unix_seconds() + 3600,
            ..RemoteCloudAuthConfig::default()
        };
        assert!(auth.is_logged_in());

        auth.user_id.clear();
        assert!(!auth.is_logged_in());

        auth.user_id = "user-1".to_string();
        auth.expires_at = now_unix_seconds().saturating_sub(1);
        assert!(!auth.is_logged_in());
    }

    #[test]
    fn cloud_remote_launch_options_force_e2ee() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("cloud-remote-force-e2ee");
        let old_home = std::env::var("HOME").ok();
        let old_config_path = std::env::var("CODEXL_CONFIG_PATH").ok();

        std::env::set_var("HOME", &root);
        std::env::set_var("CODEXL_CONFIG_PATH", root.join("config.json"));

        let mut config = AppConfig {
            provider_profiles: vec![ProviderProfile {
                name: "workspace".to_string(),
                codex_profile_name: "workspace".to_string(),
                provider_name: "provider".to_string(),
                model: "model".to_string(),
                ..ProviderProfile::default()
            }],
            ..AppConfig::default()
        };

        let err = config
            .set_remote_launch_options("workspace", true, true, None)
            .expect_err("cloud remote should require an encryption password");
        assert!(err.contains("requires a password"));
        let profile = config
            .provider_profile("workspace")
            .expect("workspace profile");
        assert!(!profile.start_remote_on_launch);
        assert!(!profile.start_remote_cloud_on_launch);
        assert!(!profile.start_remote_e2ee_on_launch);

        config
            .set_remote_launch_options("workspace", true, true, Some("secret".to_string()))
            .expect("enable cloud remote");
        let profile = config
            .provider_profile("workspace")
            .expect("workspace profile");
        assert!(profile.start_remote_on_launch);
        assert!(profile.start_remote_cloud_on_launch);
        assert!(profile.start_remote_e2ee_on_launch);
        assert_eq!(profile.remote_e2ee_password, "secret");

        config
            .set_remote_launch_options("workspace", true, false, None)
            .expect("disable cloud remote");
        let profile = config
            .provider_profile("workspace")
            .expect("workspace profile");
        assert!(profile.start_remote_on_launch);
        assert!(!profile.start_remote_cloud_on_launch);
        assert!(!profile.start_remote_e2ee_on_launch);
        assert!(profile.remote_e2ee_password.is_empty());

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_config_path {
            std::env::set_var("CODEXL_CONFIG_PATH", value);
        } else {
            std::env::remove_var("CODEXL_CONFIG_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn existing_provider_profile_uses_workspace_name_separately() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("workspace-existing-provider");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"[model_providers.nextai]
name = "nextai"
base_url = "https://api.example/v1"

[profiles.nextai]
model = "glm"
model_provider = "nextai"
"#,
        )
        .expect("write default config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = add_existing_provider_profile(ExistingProviderRequest {
            workspace_name: "workspace-a".to_string(),
            profile_name: "nextai".to_string(),
            base_url: None,
            api_key: None,
            model: "glm-4.6".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: String::new(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: String::new(),
            bot: BotProfileConfig::default(),
        })
        .expect("create workspace profile");

        assert_eq!(profile.name, "workspace-a");
        assert_eq!(profile.codex_profile_name, "nextai");
        assert_eq!(profile.provider_name, "nextai");
        let workspace_config_path = generated_codex_home(&profile).join("config.toml");
        let workspace_config =
            std::fs::read_to_string(workspace_config_path).expect("read workspace config");
        assert!(workspace_config.contains("[profiles.nextai]"));
        assert!(workspace_config.contains("model = \"glm\""));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn separate_profile_files_move_legacy_profile_table() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("separate-profile-files");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"profile = "bs"

[model_providers.bs]
name = "bs"
base_url = "https://api.example/v1"
experimental_bearer_token = "old-token"

[profiles.bs]
model = "old-model"
model_provider = "bs"
model_reasoning_effort = "high"
"#,
        )
        .expect("write default config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = add_existing_provider_profile_with_format(
            ExistingProviderRequest {
                workspace_name: "workspace-bs".to_string(),
                profile_name: "bs".to_string(),
                base_url: None,
                api_key: None,
                model: "new-model".to_string(),
                proxy_url: String::new(),
                remote_frontend_mode: String::new(),
                remote_web_asset_registry_url: String::new(),
                remote_web_asset_version: String::new(),
                bot: BotProfileConfig::default(),
            },
            CodexProfileConfigFormat::SeparateProfileFiles,
        )
        .expect("add provider with separate profile files");

        let default_config =
            std::fs::read_to_string(root.join(".codex").join("config.toml")).expect("read config");
        assert!(!default_config.contains("profile = \"bs\""));
        assert!(!default_config.contains("[profiles.bs]"));
        assert!(default_config.contains("[model_providers.bs]"));

        let default_profile_config =
            std::fs::read_to_string(root.join(".codex").join("bs.config.toml"))
                .expect("read profile config");
        assert!(default_profile_config.contains("model = \"old-model\""));
        assert!(default_profile_config.contains("model_provider = \"bs\""));
        assert!(default_profile_config.contains("model_reasoning_effort = \"high\""));

        let workspace_home = generated_codex_home(&profile);
        let workspace_config =
            std::fs::read_to_string(workspace_home.join("config.toml")).expect("read workspace");
        assert!(workspace_config.contains("model_provider = \"bs\""));
        assert!(workspace_config.contains("model = \"old-model\""));
        assert!(!workspace_config.contains("[profiles.bs]"));
        assert!(workspace_config.contains("[model_providers.bs]"));
        let workspace_profile_config =
            std::fs::read_to_string(workspace_home.join("bs.config.toml"))
                .expect("read workspace profile");
        assert!(workspace_profile_config.contains("model = \"old-model\""));
        assert!(workspace_profile_config.contains("model_provider = \"bs\""));

        let providers = read_default_provider_profiles().expect("read providers");
        let provider = providers
            .iter()
            .find(|item| item.name == "bs")
            .expect("provider from profile file");
        assert_eq!(provider.model, "old-model");
        assert_eq!(provider.provider_name, "bs");

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn normalizes_bot_gateway_platforms() {
        for platform in [
            BOT_PLATFORM_SLACK,
            BOT_PLATFORM_DISCORD,
            BOT_PLATFORM_TELEGRAM,
            BOT_PLATFORM_LINE,
            BOT_PLATFORM_FEISHU,
            BOT_PLATFORM_DINGTALK,
            BOT_PLATFORM_WEIXIN_ILINK,
            BOT_PLATFORM_WECOM,
        ] {
            assert_eq!(normalize_bot_platform(platform), platform);
        }

        assert_eq!(normalize_bot_platform("tg"), BOT_PLATFORM_TELEGRAM);
        assert_eq!(normalize_bot_platform("lark"), BOT_PLATFORM_FEISHU);
        assert_eq!(normalize_bot_platform("dingding"), BOT_PLATFORM_DINGTALK);
        assert_eq!(normalize_bot_platform("wechat"), BOT_PLATFORM_WEIXIN_ILINK);
        assert_eq!(normalize_bot_platform("wework"), BOT_PLATFORM_WECOM);
        assert_eq!(normalize_bot_platform("off"), BOT_PLATFORM_NONE);
    }

    #[test]
    fn normalizes_bot_gateway_auth_types_by_platform() {
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_WEIXIN_ILINK, ""),
            BOT_AUTH_QR_LOGIN
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_SLACK, ""),
            BOT_AUTH_BOT_TOKEN
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_WECOM, ""),
            BOT_AUTH_APP_SECRET
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_TELEGRAM, "webhook"),
            BOT_AUTH_BOT_TOKEN
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_LINE, "webhook"),
            BOT_AUTH_BOT_TOKEN
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_WECOM, "webhook_secret"),
            BOT_AUTH_APP_SECRET
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_DINGTALK, "webhook_secret"),
            BOT_AUTH_APP_SECRET
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_DISCORD, "webhook_secret"),
            BOT_AUTH_BOT_TOKEN
        );
        assert_eq!(
            normalize_bot_auth_type(BOT_PLATFORM_FEISHU, "webhook_secret"),
            BOT_AUTH_APP_SECRET
        );
        assert_eq!(normalize_bot_auth_type(BOT_PLATFORM_NONE, "bot_token"), "");
    }

    #[test]
    fn websocket_bot_configs_drop_webhook_fields() {
        for (platform, auth_type, kept_key) in [
            (BOT_PLATFORM_WECOM, BOT_AUTH_APP_SECRET, "corpId"),
            (BOT_PLATFORM_TELEGRAM, BOT_AUTH_BOT_TOKEN, "botToken"),
            (BOT_PLATFORM_LINE, BOT_AUTH_BOT_TOKEN, "channelAccessToken"),
            (BOT_PLATFORM_DINGTALK, BOT_AUTH_APP_SECRET, "appKey"),
        ] {
            let mut bot = BotProfileConfig {
                enabled: true,
                platform: platform.to_string(),
                auth_type: BOT_AUTH_WEBHOOK_SECRET.to_string(),
                auth_fields: BTreeMap::from([
                    (kept_key.to_string(), "kept".to_string()),
                    ("webhookSecret".to_string(), "old-secret".to_string()),
                    (
                        "outgoingWebhookUrl".to_string(),
                        "https://example.test/hook".to_string(),
                    ),
                ]),
                ..BotProfileConfig::default()
            };

            bot.normalize_for_profile("workspace");

            assert_eq!(bot.auth_type, auth_type);
            assert_eq!(bot.auth_fields.get(kept_key), Some(&"kept".to_string()));
            assert!(!bot.auth_fields.contains_key("webhookSecret"));
            assert!(!bot.auth_fields.contains_key("outgoingWebhookUrl"));
        }
    }

    #[test]
    fn bot_identity_is_derived_from_profile_and_instance_id() {
        let mut bot = BotProfileConfig {
            enabled: true,
            platform: BOT_PLATFORM_SLACK.to_string(),
            auth_type: String::new(),
            auth_fields: BTreeMap::new(),
            forward_all_codex_messages: true,
            handoff: BotHandoffConfig::default(),
            saved_config_id: String::new(),
            tenant_id: "old-tenant".to_string(),
            integration_id: "old-integration".to_string(),
            project_dir: "/tmp/bot-gateway".to_string(),
            state_dir: "/tmp/state".to_string(),
            codex_cwd: "/tmp/project".to_string(),
            status: String::new(),
            last_login_at: String::new(),
        };

        bot.normalize_for_profile_instance("nextai", "11111111-1111-4111-8111-111111111111");

        assert_eq!(bot.tenant_id, "nextai");
        assert_eq!(bot.integration_id, "11111111-1111-4111-8111-111111111111");
        assert!(bot.project_dir.is_empty());
        assert!(bot.state_dir.is_empty());
        assert!(bot.codex_cwd.is_empty());
        assert!(bot.forward_all_codex_messages);
    }

    #[test]
    fn saved_bot_config_preserves_identity_and_state_dir() {
        let mut bot = BotProfileConfig {
            enabled: true,
            platform: BOT_PLATFORM_WEIXIN_ILINK.to_string(),
            auth_type: BOT_AUTH_QR_LOGIN.to_string(),
            saved_config_id: "saved-weixin".to_string(),
            tenant_id: "tenant-1".to_string(),
            integration_id: "integration-1".to_string(),
            state_dir: "~/bot-state".to_string(),
            ..BotProfileConfig::default()
        };

        bot.normalize_for_profile_instance("workspace", "11111111-1111-4111-8111-111111111111");

        assert_eq!(bot.saved_config_id, "saved-weixin");
        assert_eq!(bot.tenant_id, "tenant-1");
        assert_eq!(bot.integration_id, "integration-1");
        assert!(bot.state_dir.ends_with("bot-state"));
    }

    #[test]
    fn app_config_collects_saved_bot_configs_from_profiles() {
        let mut config = AppConfig {
            provider_profiles: vec![ProviderProfile {
                id: "11111111-1111-4111-8111-111111111111".to_string(),
                name: "workspace".to_string(),
                codex_profile_name: "workspace".to_string(),
                provider_name: "provider".to_string(),
                provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
                base_url: "http://localhost:3000/v1".to_string(),
                model: "model".to_string(),
                proxy_url: String::new(),
                remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
                remote_web_asset_registry_url: String::new(),
                remote_web_asset_version: "latest".to_string(),
                codex_home: String::new(),
                start_remote_on_launch: false,
                start_remote_cloud_on_launch: false,
                start_remote_e2ee_on_launch: false,
                remote_e2ee_password: String::new(),
                bot: BotProfileConfig {
                    enabled: true,
                    platform: BOT_PLATFORM_FEISHU.to_string(),
                    auth_type: BOT_AUTH_APP_SECRET.to_string(),
                    auth_fields: BTreeMap::from([("appId".to_string(), "app-1".to_string())]),
                    status: "active".to_string(),
                    ..BotProfileConfig::default()
                },
            }],
            ..AppConfig::default()
        };

        config.normalize();

        assert_eq!(config.bot_configs.len(), 1);
        assert_eq!(config.bot_configs[0].name, "workspace");
        assert_eq!(
            config.bot_configs[0].bot.integration_id,
            "11111111-1111-4111-8111-111111111111"
        );
        assert!(!config.bot_configs[0].bot.state_dir.is_empty());
    }

    #[test]
    fn removing_workspace_preserves_saved_bot_config() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("delete-workspace-preserve-bot");
        let old_home = std::env::var("HOME").ok();
        let old_config_path = std::env::var("CODEXL_CONFIG_PATH").ok();

        std::env::set_var("HOME", &root);
        std::env::set_var("CODEXL_CONFIG_PATH", root.join("config.json"));

        let mut config = AppConfig {
            provider_profiles: vec![
                default_provider_profile(),
                ProviderProfile {
                    id: "11111111-1111-4111-8111-111111111111".to_string(),
                    name: "workspace".to_string(),
                    codex_profile_name: "workspace".to_string(),
                    provider_name: "provider".to_string(),
                    provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
                    base_url: "http://localhost:3000/v1".to_string(),
                    model: "model".to_string(),
                    proxy_url: String::new(),
                    remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
                    remote_web_asset_registry_url: String::new(),
                    remote_web_asset_version: "latest".to_string(),
                    codex_home: String::new(),
                    start_remote_on_launch: false,
                    start_remote_cloud_on_launch: false,
                    start_remote_e2ee_on_launch: false,
                    remote_e2ee_password: String::new(),
                    bot: BotProfileConfig {
                        enabled: true,
                        platform: BOT_PLATFORM_WEIXIN_ILINK.to_string(),
                        auth_type: BOT_AUTH_QR_LOGIN.to_string(),
                        saved_config_id: "saved-weixin".to_string(),
                        tenant_id: "tenant-1".to_string(),
                        integration_id: "integration-1".to_string(),
                        state_dir: "~/bot-state".to_string(),
                        forward_all_codex_messages: true,
                        handoff: BotHandoffConfig {
                            enabled: true,
                            ..BotHandoffConfig::default()
                        },
                        ..BotProfileConfig::default()
                    },
                },
            ],
            active_provider: "workspace".to_string(),
            ..AppConfig::default()
        };

        let removed = config
            .remove_provider_profile("workspace")
            .expect("remove workspace");

        assert_eq!(removed.name, "workspace");
        assert!(!config
            .provider_profiles
            .iter()
            .any(|profile| profile.name == "workspace"));
        let saved = config
            .bot_configs
            .iter()
            .find(|item| item.id == "saved-weixin")
            .expect("saved bot config");
        assert_eq!(saved.name, "workspace");
        assert_eq!(saved.bot.integration_id, "integration-1");
        assert!(saved.bot.state_dir.ends_with("bot-state"));
        assert!(!saved.bot.forward_all_codex_messages);
        assert!(!saved.bot.handoff.enabled);

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_config_path {
            std::env::set_var("CODEXL_CONFIG_PATH", value);
        } else {
            std::env::remove_var("CODEXL_CONFIG_PATH");
        }
    }

    #[test]
    fn ensure_provider_home_does_not_resync_existing_config() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("provider-home-no-resync");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"[model_providers.bs]
name = "bs"
base_url = "https://source.example/v1"
experimental_bearer_token = "source-token"

[profiles.bs]
model = "source-model"
model_provider = "bs"
"#,
        )
        .expect("write default config");

        let generated_home = root.join(".codexl").join("codex-homes").join("bs");
        std::fs::create_dir_all(&generated_home).expect("create generated home");
        std::fs::write(
            generated_home.join("config.toml"),
            r#"model = "existing-top-model"

[model_providers.bs]
name = "bs"
base_url = "https://existing.example/v1"
experimental_bearer_token = "existing-token"

[profiles.bs]
model = "existing-profile-model"
model_provider = "bs"
"#,
        )
        .expect("write generated config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = ProviderProfile {
            id: "bs".to_string(),
            name: "bs".to_string(),
            codex_profile_name: "bs".to_string(),
            provider_name: "bs".to_string(),
            provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
            base_url: "https://saved.example/v1".to_string(),
            model: "saved-model".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: "latest".to_string(),
            codex_home: String::new(),
            start_remote_on_launch: false,
            start_remote_cloud_on_launch: false,
            start_remote_e2ee_on_launch: false,
            remote_e2ee_password: String::new(),
            bot: BotProfileConfig::default(),
        };
        let path = ensure_provider_codex_home(&profile).expect("ensure provider home");
        let content = std::fs::read_to_string(PathBuf::from(path).join("config.toml"))
            .expect("read generated config");

        assert!(content.contains("https://existing.example/v1"));
        assert!(content.contains("existing-token"));
        assert!(content.contains("existing-profile-model"));
        assert!(!content.contains("https://source.example/v1"));
        assert!(!content.contains("source-token"));
        assert!(!content.contains("saved-model"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_provider_home_migrates_existing_legacy_profile_for_new_cli() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("provider-home-migrate-profile");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(root.join(".codex").join("config.toml"), "").expect("write default config");

        let generated_home = root.join(".codexl").join("codex-homes").join("bs");
        std::fs::create_dir_all(&generated_home).expect("create generated home");
        std::fs::write(
            generated_home.join("config.toml"),
            r#"profile = "bs"
model = "existing-top-model"

[model_providers.bs]
name = "bs"
base_url = "https://existing.example/v1"
experimental_bearer_token = "existing-token"

[profiles.bs]
model = "existing-profile-model"
model_provider = "bs"
"#,
        )
        .expect("write generated config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = ProviderProfile {
            id: "bs".to_string(),
            name: "bs".to_string(),
            codex_profile_name: "bs".to_string(),
            provider_name: "bs".to_string(),
            provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
            base_url: "https://saved.example/v1".to_string(),
            model: "saved-model".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: "latest".to_string(),
            codex_home: String::new(),
            start_remote_on_launch: false,
            start_remote_cloud_on_launch: false,
            start_remote_e2ee_on_launch: false,
            remote_e2ee_password: String::new(),
            bot: BotProfileConfig::default(),
        };
        let path = ensure_provider_codex_home_with_format(
            &profile,
            CodexProfileConfigFormat::SeparateProfileFiles,
        )
        .expect("ensure provider home");
        let codex_home = PathBuf::from(path);
        let content = std::fs::read_to_string(codex_home.join("config.toml")).expect("read config");
        let profile_content =
            std::fs::read_to_string(codex_home.join("bs.config.toml")).expect("read profile");

        assert!(!content.contains("profile = \"bs\""));
        assert!(!content.contains("[profiles.bs]"));
        assert!(content.contains("model_provider = \"bs\""));
        assert!(content.contains("model = \"existing-profile-model\""));
        assert!(content.contains("https://existing.example/v1"));
        assert!(content.contains("existing-token"));
        assert!(profile_content.contains("existing-profile-model"));
        assert!(!profile_content.contains("saved-model"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_provider_home_writes_bot_mcp_config_to_config_toml() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("provider-home-bot-mcp");
        let old_home = std::env::var("HOME").ok();
        let old_codex_home = std::env::var("CODEXL_CODEX_HOME").ok();

        std::fs::create_dir_all(root.join(".codex")).expect("create default codex home");
        std::fs::write(
            root.join(".codex").join("config.toml"),
            r#"[model_providers.nextai]
name = "nextai"
base_url = "http://localhost:3000/v1"

[profiles.nextai]
model = "glm"
model_provider = "nextai"
"#,
        )
        .expect("write default config");

        std::env::set_var("HOME", &root);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let profile = ProviderProfile {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            name: "nextai".to_string(),
            codex_profile_name: "nextai".to_string(),
            provider_name: "nextai".to_string(),
            provider_config_format: DEFAULT_PROVIDER_CONFIG_FORMAT_PROFILE.to_string(),
            base_url: "http://localhost:3000/v1".to_string(),
            model: "glm".to_string(),
            proxy_url: String::new(),
            remote_frontend_mode: REMOTE_FRONTEND_MODE_APP.to_string(),
            remote_web_asset_registry_url: String::new(),
            remote_web_asset_version: "latest".to_string(),
            codex_home: String::new(),
            start_remote_on_launch: false,
            start_remote_cloud_on_launch: false,
            start_remote_e2ee_on_launch: false,
            remote_e2ee_password: String::new(),
            bot: BotProfileConfig {
                enabled: true,
                platform: BOT_PLATFORM_FEISHU.to_string(),
                ..BotProfileConfig::default()
            },
        };

        let path = ensure_provider_codex_home(&profile).expect("ensure provider home");
        let content = std::fs::read_to_string(PathBuf::from(path).join("config.toml"))
            .expect("read generated config");

        assert!(content.contains("[mcp_servers.codexl_bot]"));
        assert!(content.contains("[mcp_servers.codexl_bot.env]"));
        assert!(content.contains("args = [\"--codexl-bot-media-mcp\"]"));
        assert!(content.contains("enabled = true"));
        assert!(content.contains("tool_timeout_sec = 180"));
        assert!(content.contains("CODEXL_BOT_GATEWAY_ENABLED = \"true\""));
        assert!(content.contains("CODEXL_BOT_GATEWAY_PLATFORM = \"feishu\""));
        assert!(content.contains("CODEXL_BOT_GATEWAY_FORWARD_ALL_CODEX_MESSAGES = \"false\""));
        assert!(content.contains("CODEXL_BOT_HANDOFF_ENABLED = \"false\""));
        assert!(content.contains("CODEXL_BOT_HANDOFF_IDLE_SECONDS = \"30\""));
        assert!(content.contains(
            "CODEXL_BOT_GATEWAY_INTEGRATION_ID = \"11111111-1111-4111-8111-111111111111\""
        ));
        assert!(!content.contains("mcp_servers.codexl_bot_media"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = old_codex_home {
            std::env::set_var("CODEXL_CODEX_HOME", value);
        } else {
            std::env::remove_var("CODEXL_CODEX_HOME");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bot_mcp_config_cleanup_removes_legacy_server_name() {
        let content = r#"model = "glm"

[mcp_servers.codexl_bot_media]
command = "/old"

[mcp_servers.codexl_bot_media.env]
CODEXL_BOT_GATEWAY_ENABLED = "true"

[profiles.nextai]
model = "glm"
"#;

        let cleaned = remove_bot_media_mcp_config(content);

        assert!(!cleaned.contains("codexl_bot_media"));
        assert!(cleaned.contains("[profiles.nextai]"));
    }

    #[test]
    fn retired_builtin_mcp_cleanup_removes_doubao_ime_server() {
        let root = test_dir("retired-builtins-mcp");
        std::fs::create_dir_all(&root).expect("create codex home");
        std::fs::write(
            root.join("config.toml"),
            r#"model = "glm"

[mcp_servers.codexl_doubao_ime]
command = "/Applications/Codex Launcher.app/Contents/MacOS/codex-launcher"
args = ["--codexl-doubao-ime-mcp"]
enabled = true

[profiles.default]
model = "glm"
"#,
        )
        .expect("write config");

        remove_retired_builtin_mcp_configs_for_launch(&root.to_string_lossy())
            .expect("cleanup retired mcp configs");
        let content = std::fs::read_to_string(root.join("config.toml")).expect("read config");

        assert!(!content.contains("codexl_doubao_ime"));
        assert!(!content.contains("--codexl-doubao-ime-mcp"));
        assert!(content.contains("[profiles.default]"));

        let _ = std::fs::remove_dir_all(root);
    }
}
