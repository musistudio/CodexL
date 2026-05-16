import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useTranslation } from "react-i18next";
import {
  Activity,
  AlertCircle,
  ChevronDown,
  CheckCircle2,
  CircleUserRound,
  Cloud,
  Copy,
  Cpu,
  Download,
  Eye,
  EyeOff,
  ExternalLink,
  FolderOpen,
  Globe,
  Languages,
  LayoutDashboard,
  LockKeyhole,
  LogOut,
  MessageCircle,
  Monitor,
  Moon,
  Palette,
  Pencil,
  Play,
  Plus,
  Puzzle,
  QrCode,
  Radio,
  RefreshCw,
  Search,
  Server,
  Settings,
  Smartphone,
  Square,
  Sun,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./components/ui/alert-dialog";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Card, CardContent, CardFooter, CardHeader } from "./components/ui/card";
import { Checkbox } from "./components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./components/ui/dropdown-menu";
import { Input } from "./components/ui/input";
import { Label } from "./components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { Switch } from "./components/ui/switch";
import { cn } from "./lib/utils";
import { createQrSvg } from "./qr";

type BotPlatformValue =
  | "weixin-ilink"
  | "wecom"
  | "slack"
  | "discord"
  | "telegram"
  | "line"
  | "feishu"
  | "dingtalk";
type BotPlatform = "none" | BotPlatformValue;
type BotAuthType = "qr_login" | "bot_token" | "app_secret" | "webhook_secret" | "oauth2";
type BotAuthInputType = "text" | "password" | "url";
type BotAuthFieldSpec = {
  key: string;
  label: string;
  type?: BotAuthInputType;
  placeholder?: string;
  required?: boolean;
};
type BotAuthSpec = {
  value: BotAuthType;
  label: string;
  fields: readonly BotAuthFieldSpec[];
};
type BotPlatformSpec = {
  value: BotPlatformValue;
  label: string;
  auth: readonly BotAuthSpec[];
};

const BOT_PLATFORM_SPECS: readonly BotPlatformSpec[] = [
  {
    value: "weixin-ilink",
    label: "Weixin iLink",
    auth: [
      { value: "qr_login", label: "QR Login", fields: [] },
      {
        value: "bot_token",
        label: "Bot Token",
        fields: [
          { key: "botToken", label: "Bot Token", type: "password", required: true },
          { key: "accountId", label: "Account ID" },
          { key: "userId", label: "User ID" },
        ],
      },
    ],
  },
  {
    value: "wecom",
    label: "WeCom",
    auth: [
      {
        value: "app_secret",
        label: "App Secret",
        fields: [
          { key: "corpId", label: "Corp ID", required: true },
          { key: "agentId", label: "Agent ID", required: true },
          { key: "secret", label: "Secret", type: "password", required: true },
        ],
      },
    ],
  },
  {
    value: "slack",
    label: "Slack",
    auth: [
      {
        value: "bot_token",
        label: "Bot Token",
        fields: [
          { key: "botToken", label: "Bot Token", type: "password", placeholder: "xoxb-...", required: true },
          { key: "signingSecret", label: "Signing Secret", type: "password" },
          { key: "appToken", label: "App Token", type: "password", placeholder: "xapp-..." },
        ],
      },
      {
        value: "oauth2",
        label: "OAuth 2.0",
        fields: [
          { key: "botToken", label: "OAuth Bot Token", type: "password", placeholder: "xoxb-...", required: true },
          { key: "signingSecret", label: "Signing Secret", type: "password" },
        ],
      },
      {
        value: "webhook_secret",
        label: "Webhook Secret",
        fields: [{ key: "signingSecret", label: "Signing Secret", type: "password", required: true }],
      },
    ],
  },
  {
    value: "discord",
    label: "Discord",
    auth: [
      {
        value: "bot_token",
        label: "Bot Token",
        fields: [
          { key: "botToken", label: "Bot Token", type: "password", required: true },
          { key: "applicationId", label: "Application ID" },
          { key: "publicKey", label: "Public Key" },
        ],
      },
      {
        value: "oauth2",
        label: "OAuth 2.0",
        fields: [
          { key: "botToken", label: "OAuth Access Token", type: "password", required: true },
          { key: "applicationId", label: "Application ID" },
          { key: "publicKey", label: "Public Key" },
        ],
      },
    ],
  },
  {
    value: "telegram",
    label: "Telegram",
    auth: [
      {
        value: "bot_token",
        label: "Bot Token",
        fields: [{ key: "botToken", label: "Bot Token", type: "password", required: true }],
      },
    ],
  },
  {
    value: "line",
    label: "LINE",
    auth: [
      {
        value: "bot_token",
        label: "Bot Token",
        fields: [
          { key: "channelAccessToken", label: "Channel Access Token", type: "password", required: true },
          { key: "channelSecret", label: "Channel Secret", type: "password" },
        ],
      },
    ],
  },
  {
    value: "feishu",
    label: "Feishu",
    auth: [
      {
        value: "app_secret",
        label: "App Secret",
        fields: [
          { key: "appId", label: "App ID", required: true },
          { key: "appSecret", label: "App Secret", type: "password", required: true },
          { key: "verificationToken", label: "Verification Token", type: "password" },
          { key: "domain", label: "Domain" },
        ],
      },
    ],
  },
  {
    value: "dingtalk",
    label: "DingTalk",
    auth: [
      {
        value: "app_secret",
        label: "App Secret",
        fields: [
          { key: "appKey", label: "App Key", required: true },
          { key: "appSecret", label: "App Secret", type: "password", required: true },
          { key: "robotCode", label: "Robot Code" },
        ],
      },
    ],
  },
] as const;

const BOT_PLATFORM_OPTIONS = BOT_PLATFORM_SPECS.map(({ value, label }) => ({ value, label }));
const NEXT_AI_GATEWAY_PROVIDER_NAME = "next-ai-gateway";
const WORKSPACE_PROVIDER_NONE_VALUE = "__workspace_provider_none__";

type ProviderProfile = {
  id: string;
  name: string;
  codex_profile_name: string;
  provider_name: string;
  base_url: string;
  model: string;
  proxy_url: string;
  codex_home: string;
  start_remote_on_launch: boolean;
  start_remote_cloud_on_launch: boolean;
  start_remote_e2ee_on_launch: boolean;
  bot: BotProfileConfig;
};

type BotProfileConfig = {
  enabled: boolean;
  platform: BotPlatform | string;
  auth_type: BotAuthType | string;
  auth_fields: Record<string, string>;
  forward_all_codex_messages: boolean;
  handoff: BotHandoffConfig;
  saved_config_id: string;
  tenant_id: string;
  integration_id: string;
  project_dir: string;
  state_dir: string;
  codex_cwd: string;
  status: string;
  last_login_at: string;
};

type SavedBotConfig = {
  id: string;
  name: string;
  bot: BotProfileConfig;
  updated_at: string;
};

type BotHandoffConfig = {
  enabled: boolean;
  idle_seconds: number;
  screen_lock: boolean;
  user_idle: boolean;
  phone_wifi_targets: string[];
  phone_bluetooth_targets: string[];
};

type BotHandoffScanTarget = {
  id: string;
  label: string;
  target: string;
  detail: string;
  source: string;
};

type BotHandoffScanState = {
  loading: boolean;
  error: string;
  results: BotHandoffScanTarget[];
};

type RemoteCloudAuthConfig = {
  user_id: string;
  display_name: string;
  email: string;
  avatar_url: string;
  is_pro: boolean;
  access_token: string;
  refresh_token: string;
  expires_at: number;
};

type DesktopAuthUser = {
  id: string;
  name: string;
  email: string;
  avatarUrl: string | null;
  role: string;
  hasSubscription: boolean;
};

type DesktopCloudAuth = {
  userId: string;
  displayName: string;
  email: string;
  avatarUrl: string | null;
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  relayUrl?: string | null;
  relay_url?: string | null;
  remoteRelayUrl?: string | null;
};

type DesktopAuthStartResponse = {
  code: string;
  loginUrl: string;
  expiresAt: string;
  expiresIn: number;
};

type DesktopAuthPollResponse =
  | { status: "pending"; expiresAt?: string }
  | {
      status: "authenticated";
      user: DesktopAuthUser;
      cloudAuth: DesktopCloudAuth | null;
      relayUrl?: string | null;
      relay_url?: string | null;
      remoteRelayUrl?: string | null;
    }
  | { status: "expired" | "invalid" };

type AccountLoginState = "idle" | "polling";

type AppConfig = {
  cdp_host: string;
  cdp_port: number;
  http_host: string;
  http_port: number;
  remote_control_host: string;
  remote_control_port: number;
  remote_relay_url: string;
  device_uuid: string;
  remote_cloud_auth: RemoteCloudAuthConfig;
  language: Language;
  appearance: Appearance;
  codex_path: string;
  codex_home: string;
  active_provider: string;
  provider_profiles: ProviderProfile[];
  bot_configs: SavedBotConfig[];
  auto_launch: boolean;
  extensions: ExtensionSettings;
};

type ExtensionSettings = {
  enabled: boolean;
  bot_gateway_enabled: boolean;
  next_ai_gateway_enabled: boolean;
};

type RuntimeStatus = {
  kind: string;
  executable: string;
  source: string;
  version: string;
  installed: boolean;
};

type BuiltinExtensionStatus = {
  id: string;
  name: string;
  description: string;
  version: string;
  runtime: RuntimeStatus;
  entryPath: string;
  ready: boolean;
  message: string;
};

type LaunchInfo = {
  running: boolean;
  pid: number | null;
  cdp_host: string;
  cdp_port: number;
  http_host: string;
  http_port: number;
  codex_path: string;
  codex_home: string;
  proxy_url: string;
  profile_name: string;
  cli_stdio_path: string | null;
};

type RemoteControlInfo = {
  running: boolean;
  profile_name: string;
  connection_mode: string;
  auth_mode: string;
  cloud_user_id: string | null;
  cloud_user_label: string | null;
  host: string;
  port: number;
  token: string;
  url: string;
  lan_url: string;
  relay_url: string | null;
  relay_connected: boolean;
  require_password: boolean;
  cdp_host: string;
  cdp_port: number;
  control_client_count: number;
  frame_client_count: number;
};

type InstanceStatus = LaunchInfo & {
  remote_control: RemoteControlInfo | null;
};

type NewProvider = {
  workspace_name: string;
  name: string;
  base_url: string;
  api_key: string;
  model: string;
  proxy_url: string;
  bot: BotProfileConfig;
};

type DefaultProviderProfile = {
  name: string;
  provider_name: string;
  base_url: string;
  api_key: string;
  model: string;
};

type ExistingProvider = {
  workspace_name: string;
  profile_name: string;
  base_url: string;
  api_key: string;
  model: string;
  proxy_url: string;
  bot: BotProfileConfig;
};

type UpdateProvider = ExistingProvider & {
  original_name: string;
};

type NextAiGatewayProvider = {
  workspace_name: string;
  name: string;
  model: string;
  proxy_url: string;
  bot: BotProfileConfig;
};

type UpdateNextAiGatewayProvider = NextAiGatewayProvider & {
  original_name: string;
};

type WorkspaceProvider = {
  workspace_name: string;
  proxy_url: string;
  bot: BotProfileConfig;
};

type UpdateWorkspaceProvider = WorkspaceProvider & {
  original_name: string;
};

type ProviderMode = "none" | "existing" | "new" | "gateway";
type DialogMode = "add" | "edit";
type AppSettingsSection = "general" | "extensions" | "bot" | "gateway" | "updates";
type AppUpdateStatus = "idle" | "checking" | "available" | "current" | "downloading" | "ready" | "error";
type AppUpdateState = {
  status: AppUpdateStatus;
  update: Update | null;
  error: string;
  downloadedBytes: number;
  contentLength: number | null;
};
type ToastState = {
  id: number;
  status: "loading" | "success" | "error";
  message: string;
};
type Language = "en" | "zh";
type Appearance = "system" | "light" | "dark";

type JsonObject = Record<string, unknown>;
type GatewayConfigFile = {
  path: string;
  config: JsonObject;
};
type GatewayProviderForm = {
  id: string;
  name: string;
  type: string;
  apiKey: string;
  baseUrl: string;
  models: string;
  raw: JsonObject;
};
type GatewayConfigForm = {
  host: string;
  port: string;
  providers: GatewayProviderForm[];
  rawConfig: JsonObject;
};
type GatewayProviderDialogState = {
  mode: "add" | "edit";
  provider: GatewayProviderForm;
};

type ProviderForm = {
  workspaceName: string;
  existingProfileName: string;
  existingBaseUrl: string;
  existingApiKey: string;
  existingModel: string;
  providerName: string;
  providerBaseUrl: string;
  providerApiKey: string;
  providerModel: string;
  gatewayModel: string;
  proxyUrl: string;
  botEnabled: boolean;
  botPlatform: BotPlatform;
  botAuthType: BotAuthType;
  botAuthFields: Record<string, string>;
  botConfigId: string;
  botTenantId: string;
  botIntegrationId: string;
  botStateDir: string;
  botStatus: string;
  botLastLoginAt: string;
  botForwardAllCodexMessages: boolean;
  botHandoffEnabled: boolean;
  botHandoffIdleSeconds: string;
  botHandoffPhoneWifiTargets: string;
  botHandoffPhoneBluetoothTargets: string;
};

type RemoteQrState = {
  profile: ProviderProfile;
  remote: RemoteControlInfo;
  url: string;
  markup: string;
};

type RemoteLaunchOptions = {
  startRemote: boolean;
  startCloud: boolean;
};

type RemotePasswordDialogState = {
  profileName: string;
  resolve: (password: string | null) => void;
};

type WeixinBotQrStart = {
  profileName: string;
  tenantId: string;
  integrationId: string;
  sessionId: string;
  qrCodeUrl: string;
  expiresAt: string;
  message: string;
};

type WeixinBotQrWait = {
  profileName: string;
  tenantId: string;
  integrationId: string;
  sessionId: string;
  status: string;
  message: string;
  confirmed: boolean;
};

type WeixinBotQrState = WeixinBotQrStart & {
  qrDisplay: QrDisplay;
  status: string;
  statusMessage: string;
};

type QrDisplay =
  | { kind: "webview"; src: string }
  | { kind: "image"; src: string }
  | { kind: "empty"; src: "" };

type AppStrings = ReturnType<typeof makeAppStrings>;

function useAppStrings() {
  const { t } = useTranslation();
  return useMemo(() => makeAppStrings(t), [t]);
}

function makeAppStrings(t: (key: string, options?: Record<string, unknown>) => string) {
  return {
    appTitle: t("app.title"),
    appSubtitle: t("app.subtitle"),
    searchPlaceholder: t("search.placeholder"),
    newInstance: t("actions.newInstance"),
    settings: t("settings.settings"),
    noInstancesTitle: t("search.emptyTitle"),
    noInstancesDescription: t("search.emptyDescription"),
    createInstance: t("actions.createInstance"),
    downloadUpdate: t("actions.downloadUpdate"),
    revealInFileExplorer: t("tooltips.revealInFileExplorer"),
    settingsTooltip: t("tooltips.settings"),
    downloadUpdateTooltip: t("tooltips.downloadUpdate"),
    showRemoteQr: t("remote.showQr"),
    editProfile: (name: string) => t("actions.editProfile", { name }),
    deleteProfile: (name: string) => t("actions.deleteProfile", { name }),
    stop: t("actions.stop"),
    start: t("actions.start"),
    launchOptions: t("remote.launchOptions"),
    remote: t("remote.remote"),
    cloudRemoteConnectedTooltip: t("remote.cloudRemoteConnectedTooltip"),
    startRemoteWithInstance: t("remote.startWithInstance"),
    connectCloudRelay: t("remote.connectCloudRelay"),
    encryptCloudRelay: t("remote.encryptCloudRelay"),
    endToEndEncryption: t("remote.endToEndEncryption"),
    encryptionPasswordPrompt: (name: string) => t("remote.encryptionPasswordPrompt", { name }),
    encryptionPasswordRequired: t("remote.encryptionPasswordRequired"),
    hidePassword: t("remote.hidePassword"),
    showPassword: t("remote.showPassword"),
    running: t("actions.running"),
    stopped: t("actions.stopped"),
    saving: t("actions.saving"),
    appSettingsTitle: t("settings.settings"),
    appSettingsDescription: t("settings.description"),
    general: t("settings.general"),
    extensions: t("settings.extensions"),
    remoteControl: t("remote.remoteControl"),
    remoteSettingsDescription: t("remote.settingsDescription"),
    cloudRemote: t("remote.cloudRemote"),
    relayUrl: t("remote.relayUrl"),
    cloudIdentity: t("remote.cloudIdentity"),
    cloudUserId: t("remote.cloudUserId"),
    displayName: t("remote.displayName"),
    accessToken: t("remote.accessToken"),
    refreshToken: t("remote.refreshToken"),
    expiresAt: t("remote.expiresAt"),
    signedIn: t("remote.signedIn"),
    signedOut: t("remote.signedOut"),
    clearCloudIdentity: t("remote.clearCloudIdentity"),
    gateway: t("gateway.title"),
    gatewaySettingsDescription: t("gateway.description"),
    botSettingsDescription: t("bot.settingsDescription"),
    addBot: t("bot.addBot"),
    associatedWorkspace: t("bot.associatedWorkspace"),
    botLinkedToWorkspace: t("bot.linkedToWorkspace"),
    deleteBot: t("bot.deleteBot"),
    editBot: t("bot.editBot"),
    noSavedBots: t("bot.noSavedBots"),
    notConfigured: t("bot.notConfigured"),
    status: t("bot.status"),
    updates: t("settings.updates"),
    updatesDescription: t("settings.updatesDescription"),
    extensionSettingsDescription: t("settings.extensionSettingsDescription"),
    enableExtensions: t("settings.enableExtensions"),
    botGatewayDescription: t("settings.botGatewayDescription"),
    nextAiGatewayDescription: t("settings.nextAiGatewayDescription"),
    ready: t("settings.ready"),
    notReady: t("settings.notReady"),
    preparingExtension: t("settings.preparingExtension"),
    language: t("settings.language"),
    languageDescription: t("settings.languageDescription"),
    english: t("settings.english"),
    chinese: t("settings.chinese"),
    appearance: t("settings.appearance"),
    appearanceDescription: t("settings.appearanceDescription"),
    system: t("settings.system"),
    light: t("settings.light"),
    dark: t("settings.dark"),
    cancel: t("actions.cancel"),
    checkForUpdates: t("actions.checkForUpdates"),
    checking: t("actions.checking"),
    installAndRestart: t("actions.installAndRestart"),
    installing: t("actions.installing"),
    save: t("actions.save"),
    saved: t("actions.saved"),
    manage: t("actions.manage"),
    createProfile: t("actions.createProfile"),
    newProfile: t("instanceDialog.newProfile"),
    configureInstance: t("instanceDialog.configure"),
    fromDefault: t("instanceDialog.fromDefault"),
    nextAiGatewayProvider: t("instanceDialog.nextAiGatewayProvider"),
    thirdPartyProvider: t("instanceDialog.thirdPartyProvider"),
    newProvider: t("instanceDialog.newProvider"),
    provider: t("instanceDialog.provider"),
    selectProvider: t("instanceDialog.selectProvider"),
    selectModel: t("instanceDialog.selectModel"),
    searchModel: t("instanceDialog.searchModel"),
    noModelsFound: t("instanceDialog.noModelsFound"),
    baseUrl: t("instanceDialog.baseUrl"),
    apiKey: t("instanceDialog.apiKey"),
    keepCurrentApiKey: t("instanceDialog.keepCurrentApiKey"),
    model: t("instanceDialog.model"),
    name: t("instanceDialog.name"),
    workspaceName: t("instanceDialog.workspaceName"),
    proxyUrl: t("instanceDialog.proxyUrl"),
    providerProfileName: t("instanceDialog.providerProfileName"),
    bot: t("bot.title"),
    authMethod: t("bot.authMethod"),
    savedBotConfig: t("bot.savedConfig"),
    customBotConfig: t("bot.customConfig"),
    enableBotIntegration: t("bot.enableIntegration"),
    forwardAllCodexMessages: t("bot.forwardAllCodexMessages"),
    handoffMode: t("bot.handoffMode"),
    handoffIdleSeconds: t("bot.handoffIdleSeconds"),
    handoffPhoneWifiTargets: t("bot.handoffPhoneWifiTargets"),
    handoffPhoneBluetoothTargets: t("bot.handoffPhoneBluetoothTargets"),
    refreshTargets: t("bot.refreshTargets"),
    scanningTargets: t("bot.scanningTargets"),
    selectScanTarget: t("bot.selectScanTarget"),
    noScanTargets: t("bot.noScanTargets"),
    platform: t("bot.platform"),
    selectPlatform: t("bot.selectPlatform"),
    none: t("common.none"),
    tenant: t("bot.tenant"),
    integrationId: t("bot.integrationId"),
    gatewayProject: t("bot.gatewayProject"),
    stateDir: t("bot.stateDir"),
    codexCwd: t("bot.codexCwd"),
    auto: t("common.auto"),
    optional: t("common.optional"),
    instanceName: t("instanceDialog.workspaceName"),
    deleteInstance: t("deleteDialog.title"),
    deleteInstanceConfirm: (name: string) => t("deleteDialog.confirm", { name }),
    alsoDeleteCodexHome: t("deleteDialog.removeCodexHome"),
    delete: t("actions.delete"),
    remoteQr: t("remote.remoteQr"),
    remoteUrl: t("remote.remoteUrl"),
    remotePasswordPrompt: (name: string) => t("remote.passwordPrompt", { name }),
    lanUrl: t("remote.lanUrl"),
    token: t("common.token"),
    copyUrl: t("actions.copyUrl"),
    copied: t("actions.copied"),
    open: t("actions.open"),
    weixinBotLogin: t("bot.loginTitle"),
    nativeWebview: t("bot.nativeWebview"),
    reopen: t("actions.reopen"),
    integration: t("bot.integration"),
    expires: t("bot.expires"),
    close: t("actions.close"),
    regenerate: t("actions.regenerate"),
    connected: t("actions.connected"),
    scanned: t("actions.scanned"),
    expired: t("actions.expired"),
    alreadyBound: t("actions.alreadyBound"),
    failed: t("actions.failed"),
    waiting: t("actions.waiting"),
    account: t("account.account"),
    signIn: t("account.signIn"),
    signingIn: t("account.signingIn"),
    signOut: t("account.signOut"),
    signedInAs: (name: string) => t("account.signedInAs", { name }),
    openDashboard: t("account.openDashboard"),
    loginFailed: t("account.loginFailed"),
    loginExpired: t("account.loginExpired"),
    scanQrInWeixin: t("bot.scanQrInWeixin"),
    noProviderFound: t("errors.noProviderFound"),
    clipboardUnavailable: t("errors.clipboardUnavailable"),
    nameRequired: t("errors.nameRequired"),
    baseUrlRequired: t("errors.baseUrlRequired"),
    apiKeyRequired: t("errors.apiKeyRequired"),
    modelRequired: t("errors.modelRequired"),
    providerRequired: t("errors.providerRequired"),
    botAuthRequired: (fields: string) => t("errors.botAuthRequired", { fields }),
    listen: t("gateway.listen"),
    port: t("gateway.port"),
    providers: t("gateway.providers"),
    providerType: t("gateway.providerType"),
    models: t("gateway.models"),
    addProvider: t("gateway.addProvider"),
    editProvider: t("gateway.editProvider"),
    providerDialogDescription: t("gateway.providerDialogDescription"),
    reload: t("actions.reload"),
    updateIdle: t("updates.idle"),
    updateCurrent: t("updates.current"),
    updateAvailable: (version: string) => t("updates.available", { version }),
    updateReady: t("updates.ready"),
    updateReleaseNotes: t("updates.releaseNotes"),
    updateCurrentVersion: t("updates.currentVersion"),
    updateNewVersion: t("updates.newVersion"),
    updatePublishedAt: t("updates.publishedAt"),
    updateDownloadedBytes: (downloaded: string) => t("updates.downloadedBytes", { downloaded }),
    updateProgress: (downloaded: string, total: string, percent: number) =>
      t("updates.progress", { downloaded, total, percent }),
  };
}

const emptyForm: ProviderForm = {
  workspaceName: "",
  existingProfileName: "",
  existingBaseUrl: "",
  existingApiKey: "",
  existingModel: "",
  providerName: "",
  providerBaseUrl: "",
  providerApiKey: "",
  providerModel: "",
  gatewayModel: "",
  proxyUrl: "",
  botEnabled: false,
  botPlatform: "none",
  botAuthType: "qr_login",
  botAuthFields: {},
  botConfigId: "",
  botTenantId: "",
  botIntegrationId: "",
  botStateDir: "",
  botStatus: "",
  botLastLoginAt: "",
  botForwardAllCodexMessages: false,
  botHandoffEnabled: false,
  botHandoffIdleSeconds: "30",
  botHandoffPhoneWifiTargets: "",
  botHandoffPhoneBluetoothTargets: "",
};

const emptyHandoffScanState: BotHandoffScanState = {
  loading: false,
  error: "",
  results: [],
};

const HANDOFF_TARGET_NONE_VALUE = "__codexl_handoff_target_none__";
const BOT_CONFIG_CUSTOM_VALUE = "__codexl_bot_config_custom__";
const WORKSPACE_PROVIDER_GATEWAY_VALUE = "__codexl_workspace_provider_gateway__";
let initialAppUpdateCheckStarted = false;

function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [instanceStatuses, setInstanceStatuses] = useState<Map<string, InstanceStatus>>(new Map());
  const [defaultProviders, setDefaultProviders] = useState<DefaultProviderProfile[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [appSettingsOpen, setAppSettingsOpen] = useState(false);
  const [settingsError, setSettingsError] = useState("");
  const [saveDisabled, setSaveDisabled] = useState(false);
  const [providerMode, setProviderMode] = useState<ProviderMode>("existing");
  const [dialogMode, setDialogMode] = useState<DialogMode>("add");
  const [editingProfileName, setEditingProfileName] = useState<string | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [gatewayModels, setGatewayModels] = useState<string[]>([]);
  const [pendingDeleteProfile, setPendingDeleteProfile] = useState<ProviderProfile | null>(null);
  const [removeCodexHome, setRemoveCodexHome] = useState(false);
  const [remoteQr, setRemoteQr] = useState<RemoteQrState | null>(null);
  const [remotePasswordDialog, setRemotePasswordDialog] = useState<RemotePasswordDialogState | null>(null);
  const [weixinBotQr, setWeixinBotQr] = useState<WeixinBotQrState | null>(null);
  const [accountLoginState, setAccountLoginState] = useState<AccountLoginState>("idle");
  const [accountError, setAccountError] = useState("");
  const [appUpdateState, setAppUpdateState] = useState<AppUpdateState>({
    status: "idle",
    update: null,
    error: "",
    downloadedBytes: 0,
    contentLength: null,
  });

  const existingProviderSelectRef = useRef<HTMLButtonElement>(null);
  const workspaceNameInputRef = useRef<HTMLInputElement>(null);
  const providerNameInputRef = useRef<HTMLInputElement>(null);
  const existingProviderBaseUrlRef = useRef<HTMLInputElement>(null);
  const existingProviderModelRef = useRef<HTMLInputElement>(null);
  const newProviderBaseUrlRef = useRef<HTMLInputElement>(null);
  const newProviderApiKeyRef = useRef<HTMLInputElement>(null);
  const newProviderModelRef = useRef<HTMLInputElement>(null);
  const gatewayModelTriggerRef = useRef<HTMLButtonElement>(null);

  const { i18n } = useTranslation();
  const strings = useAppStrings();
  const language = normalizeLanguage(config?.language);
  const appearance = normalizeAppearance(config?.appearance);
  const gatewayProfileEnabled = nextAiGatewayEnabled(config?.extensions);

  const showSettingsError = useCallback((error: unknown) => {
    const message = errorMessage(error).replace(/^Error:\s*/, "");
    setSettingsError(message);
    console.error(error);
  }, []);

  const checkForAppUpdate = useCallback(async () => {
    setAppUpdateState({
      status: "checking",
      update: null,
      error: "",
      downloadedBytes: 0,
      contentLength: null,
    });

    try {
      const nextUpdate = await check({ timeout: 30000 });
      setAppUpdateState((current) => ({
        ...current,
        status: nextUpdate ? "available" : "current",
        update: nextUpdate,
        error: "",
      }));
    } catch (checkError) {
      setAppUpdateState((current) => ({
        ...current,
        status: "error",
        update: null,
        error: errorMessage(checkError),
      }));
      console.error(checkError);
    }
  }, []);

  const installAppUpdate = useCallback(async () => {
    const update = appUpdateState.update;
    if (!update || appUpdateState.status === "downloading") return;

    let downloaded = 0;
    let totalBytes: number | null = null;
    setAppUpdateState((current) => ({
      ...current,
      status: "downloading",
      error: "",
      downloadedBytes: 0,
      contentLength: null,
    }));

    try {
      await update.downloadAndInstall(
        (event) => {
          if (event.event === "Started") {
            downloaded = 0;
            totalBytes = event.data.contentLength ?? null;
            setAppUpdateState((current) => ({
              ...current,
              downloadedBytes: 0,
              contentLength: totalBytes,
            }));
            return;
          }
          if (event.event === "Progress") {
            downloaded += event.data.chunkLength;
            setAppUpdateState((current) => ({
              ...current,
              downloadedBytes: downloaded,
            }));
            return;
          }
          setAppUpdateState((current) => ({
            ...current,
            downloadedBytes: totalBytes ?? downloaded,
          }));
        },
        { timeout: 120000 },
      );
      setAppUpdateState((current) => ({ ...current, status: "ready" }));
      await relaunch();
    } catch (installError) {
      setAppUpdateState((current) => ({
        ...current,
        status: "error",
        error: errorMessage(installError),
      }));
      console.error(installError);
    }
  }, [appUpdateState.status, appUpdateState.update]);

  const refreshConfig = useCallback(async () => {
    const nextConfig = await invoke<AppConfig>("get_config");
    setConfig(nextConfig);
    return nextConfig;
  }, []);

  const refreshStatus = useCallback(async () => {
    const statuses = await invoke<InstanceStatus[]>("get_instance_statuses");
    setInstanceStatuses(new Map(statuses.map((status) => [status.profile_name, status])));
  }, []);

  const loadDefaultProviders = useCallback(async () => {
    try {
      const providers = await invoke<DefaultProviderProfile[]>("get_default_providers");
      setDefaultProviders(providers);
      return providers;
    } catch {
      setDefaultProviders([]);
      return [];
    }
  }, []);

  const loadGatewayModels = useCallback(async () => {
    try {
      const result = await invoke<GatewayConfigFile>("get_gateway_config");
      const models = gatewayModelsFromConfig(result.config);
      setGatewayModels(models);
      return models;
    } catch {
      setGatewayModels([]);
      return [];
    }
  }, []);

  const saveRemoteCloudAuth = useCallback(async (remoteCloudAuth: RemoteCloudAuthConfig, remoteRelayUrl: string) => {
    const currentConfig = await invoke<AppConfig>("get_config");
    const nextConfig: AppConfig = {
      ...currentConfig,
      remote_relay_url: normalizeRemoteRelayUrl(remoteRelayUrl),
      remote_cloud_auth: remoteCloudAuth,
    };
    await invoke("update_config", { newConfig: nextConfig });
    setConfig(nextConfig);
  }, []);

  const beginDesktopLogin = useCallback(async () => {
    if (accountLoginState === "polling") {
      return;
    }

    setAccountLoginState("polling");
    setAccountError("");

    try {
      const login = await startDesktopLogin(language);
      const loginUrl = normalizeDesktopLoginUrl(login.loginUrl);
      try {
        await openUrl(loginUrl);
      } catch {
        window.open(loginUrl, "_blank", "noopener,noreferrer");
      }

      const parsedDeadline = Date.parse(login.expiresAt);
      const deadline = Number.isFinite(parsedDeadline)
        ? parsedDeadline
        : Date.now() + login.expiresIn * 1000;

      while (Date.now() < deadline) {
        await sleep(1500);
        const result = await pollDesktopLogin(login.code);

        if (result.status === "pending") {
          continue;
        }

        if (result.status === "authenticated") {
          await saveRemoteCloudAuth(
            remoteCloudAuthFromDesktopLogin(result),
            remoteRelayUrlFromDesktopLogin(result),
          );
          setAccountError("");
          return;
        }

        throw new Error(strings.loginExpired);
      }

      throw new Error(strings.loginExpired);
    } catch (error) {
      const message = `${strings.loginFailed}: ${errorMessage(error)}`;
      setAccountError(message);
      showSettingsError(message);
    } finally {
      setAccountLoginState("idle");
    }
  }, [accountLoginState, language, saveRemoteCloudAuth, showSettingsError, strings.loginExpired, strings.loginFailed]);

  const clearRemoteCloudAuth = useCallback(async () => {
    try {
      setAccountError("");
      await saveRemoteCloudAuth(emptyRemoteCloudAuth(), "");
    } catch (error) {
      showSettingsError(error);
    }
  }, [saveRemoteCloudAuth, showSettingsError]);

  const openAccountDashboard = useCallback(async () => {
    try {
      await openUrl(codexServerUrl("/dashboard"));
    } catch (error) {
      showSettingsError(error);
    }
  }, [showSettingsError]);

  useEffect(() => {
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
    if (i18n.language !== language) {
      void i18n.changeLanguage(language);
    }
  }, [i18n, language]);

  useEffect(() => {
    if (initialAppUpdateCheckStarted) {
      return;
    }
    initialAppUpdateCheckStarted = true;
    void checkForAppUpdate();
  }, [checkForAppUpdate]);

  useEffect(() => {
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      document.documentElement.dataset.theme =
        appearance === "system" ? (media?.matches ? "dark" : "light") : appearance;
    };

    applyTheme();
    if (appearance !== "system" || !media) {
      return;
    }

    media.addEventListener("change", applyTheme);
    return () => media.removeEventListener("change", applyTheme);
  }, [appearance]);

  useEffect(() => {
    let cancelled = false;
    let statusPoll: number | null = null;

    async function init() {
      try {
        const nextConfig = await invoke<AppConfig>("get_config");
        const statuses = await invoke<InstanceStatus[]>("get_instance_statuses");
        if (cancelled) return;
        setConfig(nextConfig);
        setInstanceStatuses(new Map(statuses.map((status) => [status.profile_name, status])));
        statusPoll = window.setInterval(() => {
          refreshStatus().catch(console.error);
        }, 2500);
      } catch (error) {
        if (!cancelled) {
          showSettingsError(error);
        }
      }
    }

    init().catch((error) => {
      if (!cancelled) {
        showSettingsError(error);
      }
    });

    return () => {
      cancelled = true;
      if (statusPoll !== null) {
        window.clearInterval(statusPoll);
      }
    };
  }, [refreshStatus, showSettingsError]);

  const profiles = useMemo(() => {
    if (!config) return [];
    return normalizedProfiles(config);
  }, [config]);

  const filteredProfiles = useMemo(() => {
    const query = searchQuery.toLowerCase();
    return profiles.filter(
      (profile) =>
        profile.name.toLowerCase().includes(query) ||
        profile.codex_profile_name.toLowerCase().includes(query) ||
        profile.provider_name.toLowerCase().includes(query) ||
        profile.model.toLowerCase().includes(query) ||
        profile.proxy_url.toLowerCase().includes(query),
    );
  }, [profiles, searchQuery]);

  const syncExistingProviderFields = useCallback(
    (profileName: string, providers = defaultProviders) => {
      const profile = providers.find((item) => item.name === profileName);
      setForm((current) => ({
        ...current,
        existingProfileName: profileName,
        existingBaseUrl: profile?.base_url || "",
        existingApiKey: profile?.api_key || "",
        existingModel: profile?.model || "",
      }));
    },
    [defaultProviders],
  );

  const openAddProviderDialog = useCallback(async () => {
    setDialogMode("add");
    setEditingProfileName(null);
    setSettingsError("");
    setSaveDisabled(false);
    const [providers, models] = await Promise.all([
      loadDefaultProviders(),
      gatewayProfileEnabled ? loadGatewayModels() : Promise.resolve([]),
    ]);
    const nextMode: ProviderMode = providers.length > 0 ? "existing" : "none";
    setForm({ ...emptyForm, gatewayModel: models[0] || "" });
    setProviderMode(nextMode);
    if (nextMode === "existing") {
      syncExistingProviderFields(providers[0].name, providers);
    }
    setSettingsOpen(true);
    window.requestAnimationFrame(() => {
      workspaceNameInputRef.current?.focus();
    });
  }, [gatewayProfileEnabled, loadDefaultProviders, loadGatewayModels, syncExistingProviderFields]);

  const openEditProviderDialog = useCallback(
    async (profile: ProviderProfile) => {
      setDialogMode("edit");
      setEditingProfileName(profile.name);
      setSettingsError("");
      setSaveDisabled(false);
      setForm(emptyForm);
      const isGatewayProfile =
        gatewayProfileEnabled && profile.provider_name === NEXT_AI_GATEWAY_PROVIDER_NAME;
      const [providers, models] = await Promise.all([
        loadDefaultProviders(),
        isGatewayProfile ? loadGatewayModels() : Promise.resolve([]),
      ]);

      if (isProviderlessWorkspace(profile)) {
        setProviderMode("none");
        setForm({
          ...emptyForm,
          workspaceName: profile.name,
          proxyUrl: profile.proxy_url || "",
          ...botFormFields(profile.bot, profile.name),
        });
        setSettingsOpen(true);
        window.requestAnimationFrame(() => {
          workspaceNameInputRef.current?.focus();
        });
        return;
      }

      if (isGatewayProfile) {
        setProviderMode("gateway");
        setForm({
          ...emptyForm,
          workspaceName: profile.name,
          providerName: profile.codex_profile_name || profile.name,
          gatewayModel: profile.model,
          proxyUrl: profile.proxy_url || "",
          ...botFormFields(profile.bot, profile.name),
        });
        if (models.length > 0 && !models.includes(profile.model)) {
          setGatewayModels([profile.model, ...models]);
        }
        setSettingsOpen(true);
        window.requestAnimationFrame(() => {
          gatewayModelTriggerRef.current?.focus();
        });
        return;
      }

      if (providers.length === 0) {
        setProviderMode("existing");
        setSettingsError(strings.noProviderFound);
        setSaveDisabled(false);
        setForm({
          ...emptyForm,
          workspaceName: profile.name,
          proxyUrl: profile.proxy_url || "",
          existingProfileName: profile.codex_profile_name || profile.name,
          existingBaseUrl: profile.base_url || "",
          existingModel:
            profile.model && profile.model !== "Default config"
              ? profile.model
              : "",
          ...botFormFields(profile.bot, profile.name),
        });
        setSettingsOpen(true);
        return;
      }

      const selected = selectProviderForProfile(profile, providers);
      setProviderMode("existing");
      setForm({
        ...emptyForm,
        workspaceName: profile.name,
        proxyUrl: profile.proxy_url || "",
        existingProfileName: selected.name,
        existingBaseUrl: profile.base_url || selected.base_url || "",
        existingApiKey: selected.api_key || "",
        existingModel:
          profile.model && profile.model !== "Default config"
            ? profile.model
            : selected.model || "",
        ...botFormFields(profile.bot, profile.name),
      });
      setSettingsOpen(true);
      window.requestAnimationFrame(() => {
        existingProviderSelectRef.current?.focus();
      });
    },
    [gatewayProfileEnabled, loadDefaultProviders, loadGatewayModels, strings.noProviderFound],
  );

  const closeSettingsDialog = useCallback(() => {
    setSettingsOpen(false);
    setSettingsError("");
    setEditingProfileName(null);
    setDialogMode("add");
    setSaveDisabled(false);
  }, []);

  const selectProviderMode = useCallback(
    (mode: ProviderMode) => {
      setSettingsError("");
      setSaveDisabled(false);
      if (mode === "none") {
        setProviderMode("none");
        return;
      }
      if (mode === "existing" && defaultProviders.length === 0) {
        setSettingsError(strings.noProviderFound);
        return;
      }
      if (mode === "existing") {
        const selectedProfileName = form.existingProfileName || defaultProviders[0]?.name || "";
        setProviderMode("existing");
        if (selectedProfileName) {
          syncExistingProviderFields(selectedProfileName);
        }
        return;
      }
      if (mode === "gateway" && !gatewayProfileEnabled) {
        return;
      }
      if (mode === "gateway" && gatewayModels.length === 0) {
        loadGatewayModels()
          .then((models) => {
            if (models.length > 0) {
              setForm((current) => ({ ...current, gatewayModel: current.gatewayModel || models[0] }));
            }
          })
          .catch(console.error);
      }
      if (mode === "gateway") {
        setForm((current) => ({
          ...current,
          providerName:
            current.providerName.trim() ||
            current.existingProfileName.trim() ||
            current.workspaceName.trim() ||
            "next-ai-gateway",
        }));
      }
      setProviderMode(mode);
    },
    [
      defaultProviders,
      form.existingProfileName,
      gatewayModels.length,
      gatewayProfileEnabled,
      loadGatewayModels,
      strings.noProviderFound,
      syncExistingProviderFields,
    ],
  );

  const openWeixinBotLogin = useCallback(
    async (profileName: string) => {
      const login = await invoke<WeixinBotQrStart>("start_weixin_bot_login", {
        profileName,
        force: true,
      });
      setWeixinBotQr({
        ...login,
        qrDisplay: normalizeQrDisplay(login.qrCodeUrl),
        status: "qr_pending",
        statusMessage: login.message || strings.scanQrInWeixin,
      });
    },
    [strings.scanQrInWeixin],
  );

  const saveProvider = useCallback(async () => {
    if (!config) return;

    try {
      setSettingsError("");
      let nextConfig: AppConfig;
      let savedProfileName = "";
      let savedBot: BotProfileConfig | null = null;
      const extensionsEnabled = botExtensionsEnabled(config.extensions);

      if (providerMode === "none") {
        const provider = readWorkspaceProviderForm(
          form,
          workspaceNameInputRef,
          strings,
          showSettingsError,
          extensionsEnabled,
        );
        if (!provider) return;
        savedProfileName = provider.workspace_name;
        savedBot = provider.bot;
        if (extensionsEnabled) {
          await prepareBotPluginIfNeeded(provider.bot);
        }

        if (dialogMode === "edit" && editingProfileName) {
          const update: UpdateWorkspaceProvider = { ...provider, original_name: editingProfileName };
          nextConfig = await invoke<AppConfig>("update_workspace", { provider: update });
        } else {
          nextConfig = await invoke<AppConfig>("create_workspace", { provider });
        }
      } else if (providerMode === "gateway") {
        const provider = readNextAiGatewayProviderForm(
          form,
          workspaceNameInputRef,
          providerNameInputRef,
          gatewayModelTriggerRef,
          strings,
          showSettingsError,
          extensionsEnabled,
        );
        if (!provider) return;
        savedProfileName = provider.workspace_name;
        savedBot = provider.bot;
        if (extensionsEnabled) {
          await prepareBotPluginIfNeeded(provider.bot);
        }
        await prepareNextAiGatewayPlugin();

        if (dialogMode === "edit" && editingProfileName) {
          const update: UpdateNextAiGatewayProvider = { ...provider, original_name: editingProfileName };
          nextConfig = await invoke<AppConfig>("update_next_ai_gateway_provider", { provider: update });
        } else {
          nextConfig = await invoke<AppConfig>("create_next_ai_gateway_provider", { provider });
        }
      } else if (providerMode === "existing") {
        const provider = readExistingProviderForm(
          form,
          workspaceNameInputRef,
          existingProviderSelectRef,
          existingProviderModelRef,
          strings,
          showSettingsError,
          extensionsEnabled,
        );
        if (!provider) return;
        savedProfileName = provider.workspace_name;
        savedBot = provider.bot;
        if (extensionsEnabled) {
          await prepareBotPluginIfNeeded(provider.bot);
        }

        if (dialogMode === "edit" && editingProfileName) {
          const update: UpdateProvider = { ...provider, original_name: editingProfileName };
          nextConfig = await invoke<AppConfig>("update_provider", { provider: update });
        } else {
          nextConfig = await invoke<AppConfig>("add_existing_provider", { provider });
        }
      } else {
        const provider = readNewProviderForm(
          form,
          workspaceNameInputRef,
          providerNameInputRef,
          newProviderBaseUrlRef,
          newProviderApiKeyRef,
          newProviderModelRef,
          strings,
          showSettingsError,
          extensionsEnabled,
        );
        if (!provider) return;
        savedProfileName = provider.workspace_name;
        savedBot = provider.bot;
        if (extensionsEnabled) {
          await prepareBotPluginIfNeeded(provider.bot);
        }
        nextConfig = await invoke<AppConfig>("create_provider", { provider });
      }

      const savedProfile = nextConfig.provider_profiles.find((profile) => profile.name === savedProfileName);
      savedBot = savedProfile?.bot ?? savedBot;
      if (extensionsEnabled && savedProfileName && isStaticAuthBot(savedBot)) {
        nextConfig = await invoke<AppConfig>("configure_bot_integration", {
          profileName: savedProfileName,
        });
        savedBot = nextConfig.provider_profiles.find((profile) => profile.name === savedProfileName)?.bot ?? savedBot;
      }

      setConfig(nextConfig);
      setSettingsOpen(false);
      setEditingProfileName(null);
      setDialogMode("add");
      setForm(emptyForm);
      await refreshStatus();
      if (extensionsEnabled && savedProfileName && shouldStartQrLogin(savedBot)) {
        await openWeixinBotLogin(savedProfileName);
      }
    } catch (error) {
      showSettingsError(error);
    }
  }, [
    config,
    dialogMode,
    editingProfileName,
    form,
    openWeixinBotLogin,
    providerMode,
    refreshStatus,
    showSettingsError,
    strings,
  ]);

  const saveAppSettings = useCallback(
    async (nextSettings: {
      language: Language;
      appearance: Appearance;
      extensions: ExtensionSettings;
      botConfigs?: SavedBotConfig[];
    }) => {
      if (!config) return;
      const nextBotConfigs = normalizeSavedBotConfigs(nextSettings.botConfigs ?? config.bot_configs);

      const nextConfig: AppConfig = {
        ...config,
        language: nextSettings.language,
        appearance: nextSettings.appearance,
        extensions: normalizeExtensionSettings(nextSettings.extensions),
        provider_profiles: mergeSavedBotConfigsIntoProfiles(config.provider_profiles, nextBotConfigs),
        bot_configs: nextBotConfigs,
      };
      await invoke("update_config", { newConfig: nextConfig });
      setConfig(nextConfig);
    },
    [config],
  );

  const saveBotConfigs = useCallback(
    async (botConfigs: SavedBotConfig[]) => {
      if (!config) return null;

      const nextConfig: AppConfig = {
        ...config,
        bot_configs: normalizeSavedBotConfigs(botConfigs),
      };
      nextConfig.provider_profiles = mergeSavedBotConfigsIntoProfiles(nextConfig.provider_profiles, nextConfig.bot_configs);
      await invoke("update_config", { newConfig: nextConfig });
      return refreshConfig();
    },
    [config, refreshConfig],
  );

  const requestRemoteE2eePassword = useCallback(
    (profileName: string) => {
      return new Promise<string | null>((resolve) => {
        setRemotePasswordDialog({ profileName, resolve });
      });
    },
    [],
  );

  const launchProfile = useCallback(
    async (profile: ProviderProfile, options: Partial<RemoteLaunchOptions> = {}) => {
      const startRemote = options.startRemote === true;
      const startCloud = startRemote && options.startCloud === true;
      const requireE2ee = startCloud;

      try {
        const info = await invoke<LaunchInfo>("launch_codex", {
          cdpPort: config?.cdp_port || null,
          codexPath: config?.codex_path || null,
          profileName: profile.name,
        });

        setInstanceStatuses((current) => {
          const next = new Map(current);
          const existing = next.get(profile.name);
          next.set(profile.name, {
            ...info,
            remote_control: existing?.remote_control || null,
          });
          return next;
        });
        setConfig((current) =>
          current
            ? {
                ...current,
                active_provider: profile.name,
                codex_home: info.codex_home,
              }
            : current,
        );

        if (startRemote) {
          await invoke<RemoteControlInfo>("start_remote_control", {
            profileName: profile.name,
            remotePassword: null,
            useCloudRelay: startCloud,
            requireE2ee,
          });
          await refreshStatus();
        }
      } catch (error) {
        showSettingsError(error);
        await refreshStatus().catch(console.error);
      }
    },
    [
      config?.cdp_port,
      config?.codex_path,
      refreshStatus,
      showSettingsError,
    ],
  );

  const stopCodex = useCallback(
    async (profile: ProviderProfile) => {
      try {
        await invoke("stop_codex", { profileName: profile.name });
        await refreshStatus();
      } catch (error) {
        showSettingsError(error);
      }
    },
    [refreshStatus, showSettingsError],
  );

  const toggleProfile = useCallback(
    async (profile: ProviderProfile, options: Partial<RemoteLaunchOptions> = {}) => {
      const isRunning = Boolean(instanceStatuses.get(profile.name)?.running);
      if (isRunning) {
        await stopCodex(profile);
        return;
      }
      await launchProfile(profile, options);
    },
    [instanceStatuses, launchProfile, stopCodex],
  );

  const setRemoteLaunchOptions = useCallback(
    async (profileName: string, options: Partial<RemoteLaunchOptions>) => {
      const profile = config?.provider_profiles.find((item) => item.name === profileName);
      if (!profile) {
        return;
      }

      const startRemote = options.startRemote ?? profile.start_remote_on_launch;
      const startCloud = startRemote
        ? options.startCloud ?? profile.start_remote_cloud_on_launch
        : false;
      const requireE2ee = startRemote && startCloud;
      let remoteE2eePassword: string | null = null;

      if (requireE2ee && !profile.start_remote_e2ee_on_launch) {
        const password = await requestRemoteE2eePassword(profileName);
        if (password === null) {
          return;
        }
        if (!password) {
          showSettingsError(strings.encryptionPasswordRequired);
          return;
        }
        remoteE2eePassword = password;
      }

      const nextConfig = await invoke<AppConfig>("set_remote_launch_options", {
        profileName,
        startRemote,
        startCloud,
        remoteE2eePassword,
      });
      setConfig(nextConfig);
    },
    [
      config?.provider_profiles,
      requestRemoteE2eePassword,
      showSettingsError,
      strings,
    ],
  );

  const openDeleteDialog = useCallback((profile: ProviderProfile) => {
    setPendingDeleteProfile(profile);
    setRemoveCodexHome(false);
  }, []);

  const confirmDelete = useCallback(async () => {
    if (!pendingDeleteProfile) return;

    try {
      const nextConfig = await invoke<AppConfig>("delete_provider", {
        name: pendingDeleteProfile.name,
        removeCodexHome,
      });
      setConfig(nextConfig);
      setPendingDeleteProfile(null);
      setRemoveCodexHome(false);
      await refreshStatus();
      await refreshConfig();
    } catch (error) {
      setPendingDeleteProfile(null);
      setRemoveCodexHome(false);
      showSettingsError(error);
    }
  }, [pendingDeleteProfile, refreshConfig, refreshStatus, removeCodexHome, showSettingsError]);

  const showRemoteQr = useCallback(
    (profile: ProviderProfile, remote: RemoteControlInfo) => {
      try {
        const url = remote.lan_url || remote.url;
        setRemoteQr({
          profile,
          remote,
          url,
          markup: createQrSvg(url, { moduleSize: 5, quietZone: 4 }),
        });
      } catch (error) {
        showSettingsError(error);
      }
    },
    [showSettingsError],
  );

  const closeWeixinBotLogin = useCallback(() => {
    const sessionId = weixinBotQr?.sessionId;
    setWeixinBotQr(null);
    if (sessionId) {
      invoke("cancel_weixin_bot_login", { sessionId }).catch(console.error);
      closeQrWebview(sessionId).catch(console.error);
    }
  }, [weixinBotQr?.sessionId]);

  const regenerateWeixinBotLogin = useCallback(async () => {
    if (!weixinBotQr) return;
    const current = weixinBotQr;
    setWeixinBotQr(null);
    await closeQrWebview(current.sessionId).catch(console.error);
    await invoke("cancel_weixin_bot_login", { sessionId: current.sessionId }).catch(console.error);
    await openWeixinBotLogin(current.profileName);
  }, [openWeixinBotLogin, weixinBotQr]);

  useEffect(() => {
    if (!weixinBotQr || isTerminalBotLoginStatus(weixinBotQr.status)) {
      return;
    }

    const activeLogin = weixinBotQr;
    let cancelled = false;
    let timer: number | null = null;

    async function poll() {
      try {
        const result = await invoke<WeixinBotQrWait>("wait_weixin_bot_login", {
          profileName: activeLogin.profileName,
          sessionId: activeLogin.sessionId,
        });
        if (cancelled) return;

        setWeixinBotQr((current) =>
          current && current.sessionId === result.sessionId
            ? {
                ...current,
                status: result.confirmed ? "confirmed" : result.status,
                statusMessage: result.message || current.statusMessage,
              }
            : current,
        );

        if (result.confirmed) {
          await closeQrWebview(activeLogin.sessionId).catch(console.error);
          await refreshConfig();
          return;
        }
        if (!isTerminalBotLoginStatus(result.status)) {
          timer = window.setTimeout(poll, 1200);
        }
      } catch (error) {
        if (cancelled) return;
        setWeixinBotQr((current) =>
          current
            ? {
                ...current,
                statusMessage: errorMessage(error),
              }
            : current,
        );
        timer = window.setTimeout(poll, 2500);
      }
    }

    timer = window.setTimeout(poll, 500);
    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, [refreshConfig, weixinBotQr?.profileName, weixinBotQr?.sessionId, weixinBotQr?.status]);

  return (
    <div className="h-screen min-h-screen flex flex-col overflow-hidden bg-background">
      <header className="relative flex h-12 shrink-0 items-center justify-end gap-2 border-b border-border bg-card/70 px-3 backdrop-blur-sm select-none sm:px-4">
        <div
          data-tauri-drag-region
          className="absolute inset-y-0 left-[5.5rem] right-0 z-0"
        />

        <div className="absolute left-1/2 top-1/2 z-10 w-40 -translate-x-1/2 -translate-y-1/2 sm:w-56 md:w-72 lg:w-88">
          <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="text"
            placeholder={strings.searchPlaceholder}
            className="h-7 rounded-md border-border/60 bg-background/70 pl-8 pr-2 text-xs"
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
          />
        </div>

        <div className="z-10 flex min-w-0 shrink-0 items-center justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            size="icon"
            title={strings.newInstance}
            aria-label={strings.newInstance}
            className="h-8 w-8"
            onClick={() => openAddProviderDialog().catch(showSettingsError)}
          >
            <Plus className="w-4 h-4" />
          </Button>
          {appUpdateState.update ? (
            <Tooltip label={strings.downloadUpdateTooltip} side="bottom">
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label={strings.downloadUpdate}
                className="h-8 w-8"
                disabled={appUpdateState.status === "checking" || appUpdateState.status === "downloading"}
                onClick={() => installAppUpdate().catch(console.error)}
              >
                {appUpdateState.status === "downloading" ? (
                  <RefreshCw className="w-4 h-4 animate-spin" />
                ) : (
                  <Download className="w-4 h-4" />
                )}
              </Button>
            </Tooltip>
          ) : null}
          <Tooltip label={strings.settingsTooltip} side="bottom">
            <Button
              type="button"
              variant="outline"
              size="icon"
              aria-label={strings.settings}
              className="h-8 w-8"
              onClick={() => setAppSettingsOpen(true)}
            >
              <Settings className="w-4 h-4" />
            </Button>
          </Tooltip>
          <AccountMenu
            auth={config?.remote_cloud_auth ?? emptyRemoteCloudAuth()}
            busy={accountLoginState === "polling"}
            error={accountError}
            strings={strings}
            onSignIn={() => beginDesktopLogin().catch(showSettingsError)}
            onSignOut={() => clearRemoteCloudAuth().catch(showSettingsError)}
            onOpenDashboard={() => openAccountDashboard().catch(showSettingsError)}
          />
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-auto p-6 md:p-8">
        {filteredProfiles.length > 0 ? (
          <div className="max-w-7xl mx-auto grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-6">
            {filteredProfiles.map((profile) => {
              const status = instanceStatuses.get(profile.name) || null;
              return (
                <ProfileCard
                  key={profile.name}
                  profile={profile}
                  status={status}
                  remoteLaunchOptions={{
                    startRemote: profile.start_remote_on_launch,
                    startCloud: profile.start_remote_cloud_on_launch,
                  }}
                  onToggleProfile={toggleProfile}
                  onRemoteLaunchOptionsChange={setRemoteLaunchOptions}
                  onEdit={openEditProviderDialog}
                  onDelete={openDeleteDialog}
                  onShowRemoteQr={showRemoteQr}
                  onError={showSettingsError}
                />
              );
            })}
          </div>
        ) : (
          <div className="max-w-7xl mx-auto flex flex-col items-center justify-center text-center py-20">
            <div className="w-16 h-16 bg-muted rounded-2xl flex items-center justify-center mb-4">
              <Server className="w-8 h-8 text-muted-foreground opacity-50" />
            </div>
            <h2 className="text-lg font-medium">{strings.noInstancesTitle}</h2>
            <p className="text-muted-foreground mt-1 max-w-sm">
              {strings.noInstancesDescription}
            </p>
            <Button
              type="button"
              variant="outline"
              className="mt-6"
              onClick={() => openAddProviderDialog().catch(showSettingsError)}
            >
              <Plus className="w-4 h-4" />
              {strings.createInstance}
            </Button>
          </div>
        )}
      </main>

      {appSettingsOpen && config ? (
        <AppSettingsDialog
          appearance={appearance}
          language={language}
          extensions={normalizeExtensionSettings(config.extensions)}
          botConfigs={config.bot_configs || []}
          profiles={profiles}
          onClose={() => setAppSettingsOpen(false)}
          onSave={saveAppSettings}
          onSaveBotConfigs={saveBotConfigs}
          appUpdateState={appUpdateState}
          onCheckForAppUpdate={checkForAppUpdate}
          onInstallAppUpdate={installAppUpdate}
        />
      ) : null}

      {settingsOpen ? (
        <SettingsDialog
          dialogMode={dialogMode}
          providerMode={providerMode}
          form={form}
          defaultProviders={defaultProviders}
          botConfigs={config?.bot_configs || []}
          settingsError={settingsError}
          saveDisabled={saveDisabled}
          editingProfileName={editingProfileName}
          existingProviderSelectRef={existingProviderSelectRef}
          workspaceNameInputRef={workspaceNameInputRef}
          providerNameInputRef={providerNameInputRef}
          existingProviderBaseUrlRef={existingProviderBaseUrlRef}
          existingProviderModelRef={existingProviderModelRef}
          newProviderBaseUrlRef={newProviderBaseUrlRef}
          newProviderApiKeyRef={newProviderApiKeyRef}
          newProviderModelRef={newProviderModelRef}
          gatewayModelTriggerRef={gatewayModelTriggerRef}
          gatewayEnabled={gatewayProfileEnabled}
          gatewayModels={gatewayModels}
          extensionsEnabled={botExtensionsEnabled(config?.extensions)}
          onClose={closeSettingsDialog}
          onSave={saveProvider}
          onSetForm={setForm}
          onSelectProviderMode={selectProviderMode}
          onSyncExistingProvider={syncExistingProviderFields}
        />
      ) : null}

      {pendingDeleteProfile ? (
        <DeleteDialog
          profile={pendingDeleteProfile}
          removeCodexHome={removeCodexHome}
          onRemoveCodexHomeChange={setRemoveCodexHome}
          onCancel={() => {
            setPendingDeleteProfile(null);
            setRemoveCodexHome(false);
          }}
          onConfirm={() => confirmDelete().catch(showSettingsError)}
        />
      ) : null}

      {remoteQr ? (
        <RemoteQrDialog
          remoteQr={remoteQr}
          onClose={() => setRemoteQr(null)}
          onError={showSettingsError}
        />
      ) : null}

      {remotePasswordDialog ? (
        <RemotePasswordDialog
          profileName={remotePasswordDialog.profileName}
          strings={strings}
          onCancel={() => {
            remotePasswordDialog.resolve(null);
            setRemotePasswordDialog(null);
          }}
          onConfirm={(password) => {
            remotePasswordDialog.resolve(password);
            setRemotePasswordDialog(null);
          }}
        />
      ) : null}

      {weixinBotQr ? (
        <WeixinBotQrDialog
          login={weixinBotQr}
          onRegenerate={() => regenerateWeixinBotLogin().catch(showSettingsError)}
          onClose={closeWeixinBotLogin}
        />
      ) : null}
    </div>
  );
}

function AccountMenu({
  auth,
  busy,
  error,
  strings,
  onSignIn,
  onSignOut,
  onOpenDashboard,
}: {
  auth: RemoteCloudAuthConfig;
  busy: boolean;
  error: string;
  strings: AppStrings;
  onSignIn: () => void;
  onSignOut: () => void;
  onOpenDashboard: () => void;
}) {
  const signedIn = hasRemoteCloudIdentity(auth);
  const label = remoteCloudDisplayName(auth);
  const email = remoteCloudEmail(auth);
  const avatarUrl = remoteCloudAvatarUrl(auth);
  const isPro = Boolean(auth.is_pro);

  if (!signedIn) {
    return (
      <Button
        type="button"
        variant="outline"
        size="icon"
        title={error || (busy ? strings.signingIn : strings.signIn)}
        aria-label={busy ? strings.signingIn : strings.signIn}
        className="h-8 w-8 rounded-full"
        disabled={busy}
        onClick={onSignIn}
      >
        {busy ? (
          <RefreshCw className="h-4 w-4 animate-spin" />
        ) : (
          <CircleUserRound className="h-4 w-4" />
        )}
      </Button>
    );
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="icon"
          title={strings.signedInAs(label)}
          aria-label={strings.account}
          className="h-8 w-8 rounded-full p-0"
        >
          <AccountAvatar label={label} avatarUrl={avatarUrl} premium={isPro} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <div className="px-2 py-2">
          <div className="truncate text-sm font-medium">{label}</div>
          {email ? (
            <div className="truncate text-xs text-muted-foreground">{email}</div>
          ) : null}
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onOpenDashboard}>
          <LayoutDashboard className="h-4 w-4" />
          {strings.openDashboard}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onSignOut}>
          <LogOut className="h-4 w-4" />
          {strings.signOut}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AccountAvatar({
  label,
  avatarUrl,
  premium,
}: {
  label: string;
  avatarUrl: string;
  premium: boolean;
}) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [avatarUrl]);

  const className = `account-avatar-shell ${premium ? "account-avatar-premium" : ""}`;

  if (avatarUrl && !failed) {
    return (
      <span className={className}>
        <img
          src={avatarUrl}
          alt=""
          className="relative z-10 h-7 w-7 rounded-full object-cover"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      </span>
    );
  }

  return (
    <span className={className}>
      <span className="relative z-10 flex h-7 w-7 items-center justify-center rounded-full bg-primary text-[11px] font-semibold text-primary-foreground">
        {accountInitials(label)}
      </span>
    </span>
  );
}

type ProfileCardProps = {
  profile: ProviderProfile;
  status: InstanceStatus | null;
  remoteLaunchOptions: RemoteLaunchOptions;
  onToggleProfile: (profile: ProviderProfile, options?: Partial<RemoteLaunchOptions>) => Promise<void>;
  onRemoteLaunchOptionsChange: (profileName: string, options: Partial<RemoteLaunchOptions>) => Promise<void>;
  onEdit: (profile: ProviderProfile) => Promise<void>;
  onDelete: (profile: ProviderProfile) => void;
  onShowRemoteQr: (profile: ProviderProfile, remote: RemoteControlInfo) => void;
  onError: (error: unknown) => void;
};

function ProfileCard({
  profile,
  status,
  remoteLaunchOptions,
  onToggleProfile,
  onRemoteLaunchOptionsChange,
  onEdit,
  onDelete,
  onShowRemoteQr,
  onError,
}: ProfileCardProps) {
  const strings = useAppStrings();
  const remote = status?.remote_control || null;
  const isRunning = Boolean(status?.running);
  const isRemoteRunning = Boolean(remote?.running);
  const showRemoteActions = isRunning && Boolean(remote?.url);
  const codexProfileName = profile.codex_profile_name || profile.name;
  const providerLine =
    isProviderlessWorkspace(profile)
      ? strings.none
      : profile.provider_name && profile.provider_name !== codexProfileName
      ? `${codexProfileName} / ${profile.provider_name}`
      : codexProfileName || profile.provider_name;
  const activeClass =
    isRunning || isRemoteRunning
      ? "border-emerald/40 shadow-[0_0_0_1px_oklch(0.7_0.17_163/0.1)]"
      : "border-border/60";

  return (
    <Card className={cn("flex flex-col transition-all hover:border-primary/40", activeClass)}>
      <CardHeader className="flex-row justify-between items-start gap-4 border-b border-border/50 pb-4">
        <div className="flex items-center gap-3 min-w-0">
          <div className="p-2.5 bg-primary/10 rounded-xl shrink-0">
            <Server className="w-5 h-5" />
          </div>
          <div className="min-w-0">
            <div className="text-base font-semibold truncate">{profile.name}</div>
            <div className="text-xs text-muted-foreground mt-0.5 truncate">{providerLine}</div>
          </div>
        </div>
        <StatusBadge
          isRunning={isRunning}
          isRemoteRunning={isRemoteRunning}
          isCloudRemote={
            remote?.running === true &&
            remote.connection_mode === "cloud" &&
            Boolean(remote.relay_url) &&
            remote.relay_connected === true
          }
        />
      </CardHeader>

      <CardContent className="pt-5 pb-5 flex-1 flex flex-col gap-4">
        {profile.model ? (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <Cpu className="w-4 h-4 shrink-0" />
            <span className="font-mono text-foreground truncate">{profile.model}</span>
          </div>
        ) : null}
        {profile.base_url ? (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <Globe className="w-4 h-4 shrink-0" />
            <span className="truncate text-foreground" title={profile.base_url}>
              {profile.base_url}
            </span>
          </div>
        ) : null}
        {profile.proxy_url ? (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <Radio className="w-4 h-4 shrink-0" />
            <span className="truncate text-foreground" title={profile.proxy_url}>
              {profile.proxy_url}
            </span>
          </div>
        ) : null}
        {profile.codex_home ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-auto w-full justify-start px-0 py-0 text-muted-foreground hover:bg-transparent hover:text-foreground"
            title={strings.revealInFileExplorer}
            onClick={() => revealItemInDir(profile.codex_home).catch(onError)}
          >
            <FolderOpen className="w-4 h-4 shrink-0" />
            <span className="text-xs font-mono text-foreground truncate">{profile.codex_home}</span>
          </Button>
        ) : null}
        {profile.bot?.enabled && profile.bot.platform !== "none" ? (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <MessageCircle className="w-4 h-4 shrink-0" />
            <span className="truncate text-foreground">
              {botPlatformLabel(profile.bot.platform)}
            </span>
            {profile.bot.status ? (
              <Badge variant={profile.bot.status === "active" ? "success" : "secondary"} className="shrink-0">
                {profile.bot.status}
              </Badge>
            ) : null}
          </div>
        ) : null}
      </CardContent>

      <CardFooter className="border-t border-border/50 bg-muted/10 pt-4 pb-4 justify-between">
        <LaunchMenuButton
          isRunning={isRunning}
          options={remoteLaunchOptions}
          onToggleProfile={() => onToggleProfile(profile, remoteLaunchOptions)}
          onOptionsChange={(options) => {
            onRemoteLaunchOptionsChange(profile.name, options).catch(onError);
          }}
          onError={onError}
        />

        <div className="flex gap-2">
          {showRemoteActions && remote?.url ? (
            <>
              <IconButton
                title={strings.showRemoteQr}
                onClick={() => onShowRemoteQr(profile, remote)}
              >
                <QrCode className="w-3.5 h-3.5" />
              </IconButton>
            </>
          ) : null}
          <IconButton title={strings.editProfile(profile.name)} onClick={() => onEdit(profile).catch(onError)}>
            <Pencil className="w-3.5 h-3.5" />
          </IconButton>
          <IconButton
            title={strings.deleteProfile(profile.name)}
            className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
            onClick={() => onDelete(profile)}
          >
            <Trash2 className="w-3.5 h-3.5" />
          </IconButton>
        </div>
      </CardFooter>
    </Card>
  );
}

type LaunchMenuButtonProps = {
  isRunning: boolean;
  options: RemoteLaunchOptions;
  onToggleProfile: () => Promise<void>;
  onOptionsChange: (options: Partial<RemoteLaunchOptions>) => void;
  onError: (error: unknown) => void;
};

function LaunchMenuButton({
  isRunning,
  options,
  onToggleProfile,
  onOptionsChange,
  onError,
}: LaunchMenuButtonProps) {
  const strings = useAppStrings();
  const variant = isRunning ? "dangerOutline" : "success";
  const startRemote = options.startRemote;
  const startCloud = startRemote && options.startCloud;

  if (isRunning) {
    return (
      <Button
        type="button"
        variant="dangerOutline"
        size="sm"
        onClick={() => onToggleProfile().catch(onError)}
      >
        <Square className="w-3.5 h-3.5" />
        {strings.stop}
      </Button>
    );
  }

  return (
    <div className="inline-flex rounded-md shadow-sm">
      <Button
        type="button"
        variant={variant}
        size="sm"
        className="rounded-r-none"
        onClick={() => onToggleProfile().catch(onError)}
      >
        <Play className="w-3.5 h-3.5" />
        {strings.start}
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant={variant}
            size="sm"
            className="rounded-l-none px-2"
            title={strings.launchOptions}
          >
            <ChevronDown className="w-3.5 h-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-60">
          <DropdownMenuLabel>{strings.launchOptions}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <div className="flex items-center justify-between gap-4 rounded-sm px-2 py-2">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-medium text-foreground">
                <Radio className="h-3.5 w-3.5 text-muted-foreground" />
                {strings.remote}
              </div>
              <div className="text-xs text-muted-foreground">
                {strings.startRemoteWithInstance}
              </div>
            </div>
            <Switch
              checked={startRemote}
              aria-label={strings.startRemoteWithInstance}
              onCheckedChange={(checked) => onOptionsChange({ startRemote: checked === true })}
            />
          </div>
          {startRemote ? (
            <div className="flex items-center justify-between gap-4 rounded-sm px-2 py-2">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-medium text-foreground">
                  <Cloud className="h-3.5 w-3.5 text-muted-foreground" />
                  {strings.cloudRemote}
                </div>
                <div className="text-xs text-muted-foreground">
                  {strings.connectCloudRelay}
                </div>
              </div>
              <Switch
                checked={startCloud}
                aria-label={strings.connectCloudRelay}
                onCheckedChange={(checked) => onOptionsChange({ startCloud: checked === true })}
              />
            </div>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

type IconButtonProps = {
  title: string;
  disabled?: boolean;
  tooltip?: string;
  className?: string;
  children: React.ReactNode;
  onClick: () => void;
};

function Tooltip({
  label,
  side = "top",
  children,
}: {
  label: string;
  side?: "top" | "bottom";
  children: React.ReactNode;
}) {
  return (
    <span className="group relative inline-flex">
      {children}
      <span
        className={cn(
          "pointer-events-none absolute z-[90] hidden w-max max-w-64 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs text-card-foreground shadow-xl group-hover:block group-focus-within:block",
          side === "bottom" ? "right-0 top-full mt-2" : "bottom-full right-0 mb-2",
        )}
      >
        {label}
      </span>
    </span>
  );
}

function IconButton({ title, disabled = false, tooltip, className = "", children, onClick }: IconButtonProps) {
  const button = (
    <Button
      type="button"
      variant="outline"
      size="icon"
      className={className}
      title={disabled && tooltip ? undefined : title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </Button>
  );

  if (!disabled || !tooltip) {
    return button;
  }

  return (
    <span
      className="group relative inline-flex"
      tabIndex={0}
      title={tooltip}
      aria-label={tooltip}
    >
      {button}
      <span className="pointer-events-none absolute bottom-full right-0 z-[90] mb-2 hidden w-max max-w-64 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs text-card-foreground shadow-xl group-hover:block group-focus:block">
        {tooltip}
      </span>
    </span>
  );
}

function AppSettingsDialog({
  appearance,
  language,
  extensions,
  botConfigs,
  profiles,
  appUpdateState,
  onClose,
  onSave,
  onSaveBotConfigs,
  onCheckForAppUpdate,
  onInstallAppUpdate,
}: {
  appearance: Appearance;
  language: Language;
  extensions: ExtensionSettings;
  botConfigs: SavedBotConfig[];
  profiles: ProviderProfile[];
  appUpdateState: AppUpdateState;
  onClose: () => void;
  onSave: (settings: {
    language: Language;
    appearance: Appearance;
    extensions: ExtensionSettings;
    botConfigs?: SavedBotConfig[];
  }) => Promise<void>;
  onSaveBotConfigs: (botConfigs: SavedBotConfig[]) => Promise<AppConfig | null>;
  onCheckForAppUpdate: () => Promise<void>;
  onInstallAppUpdate: () => Promise<void>;
}) {
  const strings = useAppStrings();
  const [activeSection, setActiveSection] = useState<AppSettingsSection>("general");
  const [draftLanguage, setDraftLanguage] = useState<Language>(language);
  const [draftAppearance, setDraftAppearance] = useState<Appearance>(appearance);
  const [draftExtensions, setDraftExtensions] = useState<ExtensionSettings>(normalizeExtensionSettings(extensions));
  const [draftBotConfigs, setDraftBotConfigs] = useState<SavedBotConfig[]>(normalizeSavedBotConfigs(botConfigs));
  const [botEditor, setBotEditor] = useState<{ mode: "add" | "edit"; config: SavedBotConfig | null } | null>(null);
  const [botSaving, setBotSaving] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const [preparingExtensions, setPreparingExtensions] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [extensionStatuses, setExtensionStatuses] = useState<BuiltinExtensionStatus[]>([]);
  const [extensionError, setExtensionError] = useState("");
  const [gatewayForm, setGatewayForm] = useState<GatewayConfigForm | null>(null);
  const [gatewayError, setGatewayError] = useState("");
  const botEnabled = draftExtensions.enabled && draftExtensions.bot_gateway_enabled;
  const gatewayEnabled = draftExtensions.enabled && draftExtensions.next_ai_gateway_enabled;

  const loadBuiltinExtensions = useCallback(async () => {
    try {
      setExtensionError("");
      const statuses = await invoke<BuiltinExtensionStatus[]>("get_builtin_extensions");
      setExtensionStatuses(statuses);
    } catch (error) {
      setExtensionError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    if (activeSection === "extensions") {
      loadBuiltinExtensions().catch(console.error);
    }
  }, [activeSection, loadBuiltinExtensions]);

  const handleExtensionsEnabledChange = useCallback(
    async (enabled: boolean) => {
      if (!enabled) {
        setExtensionError("");
        setDraftExtensions((current) => ({ ...current, enabled: false }));
        return;
      }

      setPreparingExtensions(true);
      setExtensionError("");
      try {
        await invoke<RuntimeStatus>("prepare_extensions_runtime");
        setDraftExtensions((current) => ({ ...current, enabled: true }));
        await loadBuiltinExtensions();
      } catch (error) {
        setDraftExtensions((current) => ({ ...current, enabled: false }));
        setExtensionError(errorMessage(error));
      } finally {
        setPreparingExtensions(false);
      }
    },
    [loadBuiltinExtensions],
  );

  const loadGatewayConfig = useCallback(async () => {
    try {
      setGatewayError("");
      const result = await invoke<GatewayConfigFile>("get_gateway_config");
      setGatewayForm(gatewayFormFromConfig(result));
    } catch (error) {
      setGatewayError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    if (activeSection === "gateway" && gatewayEnabled) {
      loadGatewayConfig().catch(console.error);
    }
  }, [activeSection, gatewayEnabled, loadGatewayConfig]);

  useEffect(() => {
    if (activeSection === "gateway" && !gatewayEnabled) {
      setActiveSection("extensions");
    }
  }, [activeSection, gatewayEnabled]);

  useEffect(() => {
    if (activeSection === "bot" && !botEnabled) {
      setActiveSection("extensions");
    }
  }, [activeSection, botEnabled]);

  const showToast = (status: ToastState["status"], message: string) => {
    const id = Date.now();
    setToast({ id, status, message });
    if (status !== "loading") {
      window.setTimeout(() => {
        setToast((current) => (current?.id === id ? null : current));
      }, 3200);
    }
  };

  const saveDraft = async () => {
    setSavingSettings(true);
    showToast(
      "loading",
      draftExtensions.enabled && !extensions.enabled ? strings.preparingExtension : strings.saving,
    );
    try {
      if (gatewayForm && gatewayEnabled) {
        await invoke<GatewayConfigFile>("update_gateway_config", {
          config: gatewayConfigFromForm(gatewayForm),
        });
      }
      await onSave({
        appearance: draftAppearance,
        language: draftLanguage,
        extensions: draftExtensions,
        botConfigs: draftBotConfigs,
      });
      if (activeSection === "extensions") {
        await loadBuiltinExtensions();
      }
      showToast("success", strings.saved);
    } catch (error) {
      if (!extensions.enabled && draftExtensions.enabled) {
        setDraftExtensions((current) => ({ ...current, enabled: false }));
      }
      showToast("error", `${strings.failed}: ${errorMessage(error)}`);
    } finally {
      setSavingSettings(false);
    }
  };

  const persistBotConfigs = async (nextConfigs: SavedBotConfig[]) => {
    const normalized = normalizeSavedBotConfigs(nextConfigs);
    setDraftBotConfigs(normalized);
    const persisted = await onSaveBotConfigs(normalized);
    if (persisted) {
      setDraftBotConfigs(normalizeSavedBotConfigs(persisted.bot_configs));
    }
  };

  const saveBotConfig = async (botConfig: SavedBotConfig) => {
    const currentConfigs = normalizeSavedBotConfigs(draftBotConfigs);
    const nextConfigs =
      botEditor?.mode === "edit"
        ? currentConfigs.map((item) => (item.id === botConfig.id ? botConfig : item))
        : [...currentConfigs, botConfig];
    setBotSaving(true);
    try {
      await persistBotConfigs(nextConfigs);
      setBotEditor(null);
    } finally {
      setBotSaving(false);
    }
  };

  const deleteBotConfig = async (botConfig: SavedBotConfig) => {
    if (associatedWorkspaceProfiles(botConfig, profiles).length > 0) {
      return;
    }
    setBotSaving(true);
    try {
      await persistBotConfigs(draftBotConfigs.filter((item) => item.id !== botConfig.id));
      showToast("success", strings.saved);
    } catch (error) {
      showToast("error", `${strings.failed}: ${errorMessage(error)}`);
    } finally {
      setBotSaving(false);
    }
  };

  const sectionTitle =
    activeSection === "extensions"
      ? strings.extensions
      : activeSection === "bot"
        ? strings.bot
      : activeSection === "gateway"
        ? strings.gateway
        : activeSection === "updates"
          ? strings.updates
          : strings.general;
  const sectionDescription =
    activeSection === "extensions"
      ? strings.extensionSettingsDescription
      : activeSection === "bot"
        ? strings.botSettingsDescription
      : activeSection === "gateway"
        ? strings.gatewaySettingsDescription
        : activeSection === "updates"
          ? strings.updatesDescription
          : strings.appSettingsDescription;

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="h-[80vh] w-[80vw] max-w-none grid-cols-[220px_1fr] gap-0 overflow-hidden p-0"
      >
        <aside className="flex min-h-0 flex-col border-r border-border bg-muted/20">
          <div className="border-b border-border px-5 py-4">
            <div className="flex items-center gap-2 text-base font-semibold">
              <Settings className="h-4 w-4" />
              {strings.appSettingsTitle}
            </div>
          </div>
          <nav className="flex-1 space-y-1 p-3">
            <SettingsNavButton
              active={activeSection === "general"}
              icon={<Settings className="h-4 w-4" />}
              label={strings.general}
              onClick={() => setActiveSection("general")}
            />
            <SettingsNavButton
              active={activeSection === "updates"}
              icon={<RefreshCw className="h-4 w-4" />}
              label={strings.updates}
              onClick={() => setActiveSection("updates")}
            />
            <SettingsNavButton
              active={activeSection === "extensions"}
              icon={<Puzzle className="h-4 w-4" />}
              label={strings.extensions}
              onClick={() => setActiveSection("extensions")}
            />
            {botEnabled ? (
              <SettingsNavButton
                active={activeSection === "bot"}
                icon={<MessageCircle className="h-4 w-4" />}
                label={strings.bot}
                onClick={() => setActiveSection("bot")}
              />
            ) : null}
            {gatewayEnabled ? (
              <SettingsNavButton
                active={activeSection === "gateway"}
                icon={<Server className="h-4 w-4" />}
                label={strings.gateway}
                onClick={() => setActiveSection("gateway")}
              />
            ) : null}
          </nav>
        </aside>

        <section className="flex min-h-0 flex-col">
          <DialogHeader className="flex-row items-center justify-between gap-4 border-b border-border px-6 py-4 pr-16">
            <div className="min-w-0">
              <DialogTitle className="text-base">{sectionTitle}</DialogTitle>
              <DialogDescription>{sectionDescription}</DialogDescription>
            </div>
            {activeSection === "extensions" ? (
              <div className="flex shrink-0 items-center gap-2">
                {preparingExtensions ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                ) : null}
                <span className="text-sm text-muted-foreground">{strings.enableExtensions}</span>
                <Switch
                  checked={draftExtensions.enabled}
                  disabled={preparingExtensions || savingSettings}
                  aria-label={strings.enableExtensions}
                  onCheckedChange={(checked) => void handleExtensionsEnabledChange(checked === true)}
                />
              </div>
            ) : activeSection === "bot" ? (
              <Button type="button" size="sm" onClick={() => setBotEditor({ mode: "add", config: null })}>
                <Plus className="h-4 w-4" />
                {strings.addBot}
              </Button>
            ) : null}
          </DialogHeader>

          <div className="flex-1 overflow-auto px-6 py-6">
            {activeSection === "general" ? (
              <div className="max-w-2xl space-y-7">
                <div className="grid grid-cols-[180px_1fr] items-start gap-6">
                  <div className="flex items-center gap-2 pt-2 text-sm font-medium">
                    <Languages className="h-4 w-4 text-muted-foreground" />
                    {strings.language}
                  </div>
                  <div className="grid gap-2">
                    <Select value={draftLanguage} onValueChange={(value) => setDraftLanguage(normalizeLanguage(value))}>
                      <SelectTrigger id="appLanguageSelect">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="en">{strings.english}</SelectItem>
                        <SelectItem value="zh">{strings.chinese}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                <div className="grid grid-cols-[180px_1fr] items-start gap-6">
                  <div className="flex items-center gap-2 pt-2 text-sm font-medium">
                    <Palette className="h-4 w-4 text-muted-foreground" />
                    {strings.appearance}
                  </div>
                  <div className="grid gap-2">
                    <Select value={draftAppearance} onValueChange={(value) => setDraftAppearance(normalizeAppearance(value))}>
                      <SelectTrigger id="appAppearanceSelect">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="system">
                          <span className="inline-flex items-center gap-2">
                            <Monitor className="h-3.5 w-3.5" />
                            {strings.system}
                          </span>
                        </SelectItem>
                        <SelectItem value="light">
                          <span className="inline-flex items-center gap-2">
                            <Sun className="h-3.5 w-3.5" />
                            {strings.light}
                          </span>
                        </SelectItem>
                        <SelectItem value="dark">
                          <span className="inline-flex items-center gap-2">
                            <Moon className="h-3.5 w-3.5" />
                            {strings.dark}
                          </span>
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              </div>
            ) : activeSection === "extensions" ? (
              <div className="max-w-3xl space-y-3">
                {extensionError ? (
                  <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
                    {extensionError}
                  </p>
                ) : null}
                {draftExtensions.enabled
                  ? extensionStatuses.map((extension) => (
                      <BuiltinExtensionRow
                        key={extension.id}
                        extension={extension}
                        extensionEnabled={extensionEnabledSetting(draftExtensions, extension.id)}
                        strings={strings}
                        onExtensionEnabledChange={(enabled) =>
                          setDraftExtensions((current) => setExtensionEnabledSetting(current, extension.id, enabled))
                        }
                      />
                    ))
                  : null}
              </div>
            ) : activeSection === "bot" ? (
              <BotSettingsPanel
                botConfigs={draftBotConfigs}
                profiles={profiles}
                strings={strings}
                onEditBotConfig={(config) => setBotEditor({ mode: "edit", config })}
                onDeleteBotConfig={(config) => void deleteBotConfig(config)}
              />
            ) : activeSection === "gateway" ? (
              <GatewaySettingsPanel
                form={gatewayForm}
                error={gatewayError}
                strings={strings}
                onReload={() => loadGatewayConfig().catch(console.error)}
                onChange={setGatewayForm}
              />
            ) : (
              <AppUpdatePanel
                strings={strings}
                updateState={appUpdateState}
                onCheckForAppUpdate={onCheckForAppUpdate}
                onInstallAppUpdate={onInstallAppUpdate}
              />
            )}
          </div>

          <DialogFooter className="border-t border-border px-6 py-4">
            <Button
              type="button"
              disabled={savingSettings || preparingExtensions}
              onClick={saveDraft}
            >
              {savingSettings ? <RefreshCw className="h-4 w-4 animate-spin" /> : null}
              {strings.save}
            </Button>
          </DialogFooter>
        </section>
        {toast ? <SettingsToast toast={toast} /> : null}
        {botEditor ? (
          <BotConfigDialog
            mode={botEditor.mode}
            config={botEditor.config}
            strings={strings}
            saving={botSaving}
            onClose={() => setBotEditor(null)}
            onSave={(config) => saveBotConfig(config)}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function SettingsToast({ toast }: { toast: ToastState }) {
  const icon =
    toast.status === "loading" ? (
      <RefreshCw className="h-4 w-4 animate-spin text-muted-foreground" />
    ) : toast.status === "success" ? (
      <CheckCircle2 className="h-4 w-4 text-emerald" />
    ) : (
      <AlertCircle className="h-4 w-4 text-destructive" />
    );

  return createPortal(
    <div
      role="status"
      aria-live="polite"
      className="fixed left-1/2 top-6 z-[80] flex w-[calc(100vw-2rem)] max-w-sm -translate-x-1/2 items-center gap-2 rounded-md border border-border bg-card px-3 py-2.5 text-sm text-card-foreground shadow-xl"
    >
      {icon}
      <span className="min-w-0 break-words">{toast.message}</span>
    </div>,
    document.body,
  );
}

function SettingsNavButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={cn(
        "flex h-10 w-full items-center gap-2 rounded-md px-3 text-left text-sm font-medium",
        active
          ? "bg-secondary text-secondary-foreground"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
      onClick={onClick}
    >
      {icon}
      {label}
    </button>
  );
}

function BotSettingsPanel({
  botConfigs,
  profiles,
  strings,
  onEditBotConfig,
  onDeleteBotConfig,
}: {
  botConfigs: SavedBotConfig[];
  profiles: ProviderProfile[];
  strings: AppStrings;
  onEditBotConfig: (config: SavedBotConfig) => void;
  onDeleteBotConfig: (config: SavedBotConfig) => void;
}) {
  const savedConfigs = useMemo(() => normalizeSavedBotConfigs(botConfigs), [botConfigs]);

  return (
    <div className="max-w-5xl">
      <div className="space-y-3">
        {savedConfigs.length > 0 ? (
          savedConfigs.map((config) => {
            const bot = normalizeBotConfig(config.bot, config.name);
            const status = bot.status || strings.ready;
            const linkedProfiles = associatedWorkspaceProfiles(config, profiles);
            const associatedWorkspace = associatedWorkspaceTextFromProfiles(linkedProfiles, strings.none);
            const deleteDisabled = linkedProfiles.length > 0;
            return (
              <div key={config.id} className="rounded-md border border-border bg-muted/10 px-3 py-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,0.9fr)_minmax(0,0.7fr)_minmax(0,0.85fr)_minmax(0,1fr)_minmax(0,0.75fr)]">
                    <GatewayProviderSummaryField label={strings.name} value={config.name || strings.bot} />
                    <GatewayProviderSummaryField label={strings.platform} value={botPlatformLabel(bot.platform)} />
                    <GatewayProviderSummaryField
                      label={strings.authMethod}
                      value={botAuthTypeLabel(bot.platform, bot.auth_type)}
                    />
                    <GatewayProviderSummaryField label={strings.associatedWorkspace} value={associatedWorkspace} />
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-muted-foreground">{strings.status}</div>
                      <div className="mt-1">
                        <Badge variant={bot.status === "active" ? "success" : "secondary"}>
                          {status}
                        </Badge>
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center justify-end gap-2">
                    <IconButton title={strings.editBot} onClick={() => onEditBotConfig(config)}>
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={deleteDisabled ? strings.botLinkedToWorkspace : strings.deleteBot}
                      tooltip={deleteDisabled ? strings.botLinkedToWorkspace : undefined}
                      disabled={deleteDisabled}
                      className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
                      onClick={() => onDeleteBotConfig(config)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </div>
              </div>
            );
          })
        ) : (
          <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
            {strings.noSavedBots}
          </div>
        )}
      </div>
    </div>
  );
}

function BotConfigDialog({
  mode,
  config,
  strings,
  saving,
  onClose,
  onSave,
}: {
  mode: "add" | "edit";
  config: SavedBotConfig | null;
  strings: AppStrings;
  saving: boolean;
  onClose: () => void;
  onSave: (config: SavedBotConfig) => Promise<void>;
}) {
  const nameRef = useRef<HTMLInputElement>(null);
  const [form, setForm] = useState<ProviderForm>(() => botConfigFormFields(config));
  const [error, setError] = useState("");
  const botAuthType = normalizeBotAuthType(form.botPlatform, form.botAuthType);
  const botAuthSpecs = authSpecsForPlatform(form.botPlatform);
  const botAuthFields = fieldsForBotAuth(form.botPlatform, botAuthType);

  const showError = (nextError: unknown) => setError(errorMessage(nextError));

  const save = async () => {
    setError("");
    const nextConfig = readSavedBotConfigForm(form, config, nameRef, strings, showError);
    if (!nextConfig) {
      return;
    }
    await onSave(nextConfig);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{mode === "add" ? strings.addBot : strings.editBot}</DialogTitle>
          <DialogDescription className="sr-only">{strings.botSettingsDescription}</DialogDescription>
        </DialogHeader>
        {error ? (
          <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
            {error}
          </p>
        ) : null}
        <div className="grid gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="botConfigNameInput">{strings.name}</Label>
            <Input
              id="botConfigNameInput"
              ref={nameRef}
              value={form.workspaceName}
              placeholder="my-bot"
              onChange={(event) =>
                setForm((current) => ({ ...current, workspaceName: event.target.value }))
              }
            />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="botConfigPlatformSelect">{strings.platform}</Label>
              <Select
                value={form.botPlatform}
                onValueChange={(value) =>
                  setForm((current) => {
                    const nextPlatform = normalizeBotPlatform(value);
                    const nextAuthType = defaultBotAuthType(nextPlatform);
                    return {
                      ...current,
                      botEnabled: true,
                      botPlatform: nextPlatform === "none" ? "weixin-ilink" : nextPlatform,
                      botAuthType: nextAuthType,
                      botAuthFields: pickBotAuthFields(current.botAuthFields, nextPlatform, nextAuthType),
                      botConfigId: config?.id || current.botConfigId,
                      botStatus: "",
                      botLastLoginAt: "",
                    };
                  })
                }
              >
                <SelectTrigger id="botConfigPlatformSelect">
                  <SelectValue placeholder={strings.selectPlatform} />
                </SelectTrigger>
                <SelectContent>
                  {BOT_PLATFORM_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {botAuthSpecs.length > 0 ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="botConfigAuthTypeSelect">{strings.authMethod}</Label>
                <Select
                  value={botAuthType}
                  onValueChange={(value) =>
                    setForm((current) => {
                      const nextAuthType = normalizeBotAuthType(current.botPlatform, value);
                      return {
                        ...current,
                        botAuthType: nextAuthType,
                        botAuthFields: pickBotAuthFields(
                          current.botAuthFields,
                          current.botPlatform,
                          nextAuthType,
                        ),
                        botConfigId: config?.id || current.botConfigId,
                        botStatus: "",
                        botLastLoginAt: "",
                      };
                    })
                  }
                >
                  <SelectTrigger id="botConfigAuthTypeSelect">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {botAuthSpecs.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            ) : null}
          </div>
          {botAuthFields.length > 0 ? (
            <div className="grid gap-3 sm:grid-cols-2">
              {botAuthFields.map((field) => (
                <div key={field.key} className="flex flex-col gap-1.5">
                  <Label htmlFor={`botAuthField-${field.key}`} className="flex items-center gap-1.5">
                    <span>{field.label}</span>
                    {field.required ? null : (
                      <span className="text-xs font-normal text-muted-foreground">{strings.optional}</span>
                    )}
                  </Label>
                  <Input
                    id={`botAuthField-${field.key}`}
                    type={field.type || "text"}
                    autoComplete="off"
                    placeholder={field.placeholder || ""}
                    value={form.botAuthFields[field.key] || ""}
                    onChange={(event) =>
                      setForm((current) => ({
                        ...current,
                        botAuthFields: {
                          ...current.botAuthFields,
                          [field.key]: event.target.value,
                        },
                        botConfigId: config?.id || current.botConfigId,
                        botStatus: "",
                        botLastLoginAt: "",
                      }))
                    }
                  />
                </div>
              ))}
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>
            {strings.cancel}
          </Button>
          <Button type="button" disabled={saving} onClick={() => save().catch(showError)}>
            {strings.save}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function BuiltinExtensionRow({
  extension,
  extensionEnabled,
  strings,
  onExtensionEnabledChange,
}: {
  extension: BuiltinExtensionStatus;
  extensionEnabled: boolean;
  strings: AppStrings;
  onExtensionEnabledChange: (enabled: boolean) => void;
}) {
  return (
    <div className="rounded-md border border-border bg-muted/20 px-3 py-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div className="text-sm font-medium text-foreground">{extension.name}</div>
            {extension.version ? (
              <span className="shrink-0 text-xs text-muted-foreground">v{extension.version}</span>
            ) : null}
            <Badge variant={extension.ready ? "success" : "secondary"} className="shrink-0">
              {extension.ready ? strings.ready : strings.notReady}
            </Badge>
          </div>
          <div className="mt-1 text-xs text-muted-foreground">{extensionDescription(extension, strings)}</div>
        </div>
        <Switch
          checked={extensionEnabled}
          aria-label={extension.name}
          onCheckedChange={(checked) => onExtensionEnabledChange(checked === true)}
        />
      </div>
    </div>
  );
}

function AppUpdatePanel({
  strings,
  updateState,
  onCheckForAppUpdate,
  onInstallAppUpdate,
}: {
  strings: AppStrings;
  updateState: AppUpdateState;
  onCheckForAppUpdate: () => Promise<void>;
  onInstallAppUpdate: () => Promise<void>;
}) {
  const { status, update, error, downloadedBytes, contentLength } = updateState;
  const checking = status === "checking";
  const downloading = status === "downloading";
  const progressPercent = contentLength
    ? Math.min(100, Math.round((downloadedBytes / contentLength) * 100))
    : null;
  const statusLabel =
    status === "checking"
      ? strings.checking
      : status === "available" && update
        ? strings.updateAvailable(update.version)
        : status === "current"
          ? strings.updateCurrent
          : status === "downloading"
            ? strings.installing
            : status === "ready"
              ? strings.updateReady
              : status === "error"
                ? strings.failed
                : strings.updateIdle;

  return (
    <div className="max-w-3xl space-y-4">
      <div className="rounded-md border border-border bg-muted/20 px-4 py-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <div className="flex items-center gap-2 text-sm font-medium">
                <RefreshCw className="h-4 w-4 text-muted-foreground" />
                {strings.updates}
              </div>
              <Badge variant={status === "current" || status === "ready" ? "success" : "secondary"}>
                {statusLabel}
              </Badge>
            </div>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{strings.updatesDescription}</p>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={checking || downloading}
            onClick={() => onCheckForAppUpdate().catch(console.error)}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", checking ? "animate-spin" : "")} />
            {strings.checkForUpdates}
          </Button>
        </div>

        {update ? (
          <div className="mt-4 grid gap-3 border-t border-border pt-4 text-sm">
            <div className="grid grid-cols-[140px_1fr] gap-3">
              <span className="text-muted-foreground">{strings.updateCurrentVersion}</span>
              <span className="font-medium">{update.currentVersion}</span>
            </div>
            <div className="grid grid-cols-[140px_1fr] gap-3">
              <span className="text-muted-foreground">{strings.updateNewVersion}</span>
              <span className="font-medium">{update.version}</span>
            </div>
            {update.date ? (
              <div className="grid grid-cols-[140px_1fr] gap-3">
                <span className="text-muted-foreground">{strings.updatePublishedAt}</span>
                <span>{update.date}</span>
              </div>
            ) : null}
            {update.body ? (
              <div className="grid grid-cols-[140px_1fr] gap-3">
                <span className="text-muted-foreground">{strings.updateReleaseNotes}</span>
                <p className="whitespace-pre-wrap leading-relaxed">{update.body}</p>
              </div>
            ) : null}
          </div>
        ) : null}

        {downloading ? (
          <div className="mt-4 space-y-2 border-t border-border pt-4">
            <div className="h-2 overflow-hidden rounded-full bg-secondary">
              <div
                className="h-full rounded-full bg-emerald transition-[width]"
                style={{ width: `${progressPercent ?? 12}%` }}
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {progressPercent === null
                ? strings.updateDownloadedBytes(formatBytes(downloadedBytes))
                : strings.updateProgress(formatBytes(downloadedBytes), formatBytes(contentLength || 0), progressPercent)}
            </p>
          </div>
        ) : null}

        {error ? (
          <p className="mt-4 rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
            {error}
          </p>
        ) : null}

        {update ? (
          <div className="mt-4 flex justify-end border-t border-border pt-4">
            <Button
              type="button"
              disabled={checking || downloading}
              onClick={() => onInstallAppUpdate().catch(console.error)}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", downloading ? "animate-spin" : "")} />
              {strings.installAndRestart}
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function GatewaySettingsPanel({
  form,
  error,
  strings,
  onReload,
  onChange,
}: {
  form: GatewayConfigForm | null;
  error: string;
  strings: AppStrings;
  onReload: () => void;
  onChange: React.Dispatch<React.SetStateAction<GatewayConfigForm | null>>;
}) {
  const [providerDialog, setProviderDialog] = useState<GatewayProviderDialogState | null>(null);

  if (!form) {
    return (
      <div className="max-w-5xl">
        {error ? (
          <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
            {error}
          </p>
        ) : (
          <Button type="button" variant="outline" onClick={onReload}>
            <RefreshCw className="h-4 w-4" />
            {strings.reload}
          </Button>
        )}
      </div>
    );
  }

  const update = (patch: Partial<GatewayConfigForm>) =>
    onChange((current) => (current ? { ...current, ...patch } : current));
  const openAddProviderDialog = () =>
    setProviderDialog({
      mode: "add",
      provider: createGatewayProviderForm(),
    });
  const openEditProviderDialog = (provider: GatewayProviderForm) =>
    setProviderDialog({
      mode: "edit",
      provider: cloneGatewayProviderForm(provider),
    });
  const updateDialogProvider = (patch: Partial<GatewayProviderForm>) =>
    setProviderDialog((current) =>
      current
        ? {
            ...current,
            provider: { ...current.provider, ...patch },
          }
        : current,
    );
  const saveProviderDialog = () => {
    if (!providerDialog) return;

    const nextProvider = cloneGatewayProviderForm(providerDialog.provider);
    onChange((current) => {
      if (!current) return current;

      if (providerDialog.mode === "add") {
        return {
          ...current,
          providers: [...current.providers, nextProvider],
        };
      }

      return {
        ...current,
        providers: current.providers.map((provider) =>
          provider.id === nextProvider.id ? nextProvider : provider,
        ),
      };
    });
    setProviderDialog(null);
  };

  return (
    <div className="max-w-5xl space-y-6">
      {error ? (
        <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
          {error}
        </p>
      ) : null}

      <section className="space-y-3">
        <SectionTitle icon={<Globe className="h-4 w-4" />} title={strings.listen} />
        <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
          <Field label="Host">
            <Input value={form.host} onChange={(event) => update({ host: event.target.value })} />
          </Field>
          <Field label={strings.port}>
            <Input value={form.port} inputMode="numeric" onChange={(event) => update({ port: event.target.value })} />
          </Field>
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <SectionTitle icon={<Cpu className="h-4 w-4" />} title={strings.providers} />
          <Button type="button" variant="outline" size="sm" onClick={openAddProviderDialog}>
            <Plus className="h-4 w-4" />
            {strings.addProvider}
          </Button>
        </div>
        <div className="space-y-3">
          {form.providers.length > 0 ? (
            form.providers.map((provider) => (
              <div key={provider.id} className="rounded-md border border-border bg-muted/10 px-3 py-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,0.8fr)_minmax(0,1.4fr)_minmax(0,1fr)]">
                    <GatewayProviderSummaryField label={strings.name} value={provider.name || strings.none} />
                    <GatewayProviderSummaryField label={strings.baseUrl} value={provider.baseUrl || strings.none} />
                    <GatewayProviderSummaryField label={strings.models} value={provider.models || strings.none} />
                  </div>
                  <div className="flex items-center justify-end gap-2">
                    <IconButton title={strings.editProvider} onClick={() => openEditProviderDialog(provider)}>
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={strings.delete}
                      className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
                      onClick={() => update({ providers: form.providers.filter((item) => item.id !== provider.id) })}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </div>
              </div>
            ))
          ) : (
            <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
              {strings.noProviderFound}
            </div>
          )}
        </div>
      </section>

      <Dialog open={providerDialog !== null} onOpenChange={(open) => !open && setProviderDialog(null)}>
        {providerDialog ? (
          <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
            <DialogHeader>
              <DialogTitle>{providerDialog.mode === "add" ? strings.addProvider : strings.editProvider}</DialogTitle>
              <DialogDescription className="sr-only">{strings.providerDialogDescription}</DialogDescription>
            </DialogHeader>
            <GatewayProviderEditor
              provider={providerDialog.provider}
              strings={strings}
              onChange={updateDialogProvider}
            />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setProviderDialog(null)}>
                {strings.cancel}
              </Button>
              <Button type="button" onClick={saveProviderDialog}>
                {strings.save}
              </Button>
            </DialogFooter>
          </DialogContent>
        ) : null}
      </Dialog>
    </div>
  );
}

function GatewayProviderSummaryField({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs font-medium text-muted-foreground">{label}</div>
      <div className="mt-1 break-words text-sm text-foreground">{value}</div>
    </div>
  );
}

function GatewayProviderEditor({
  provider,
  strings,
  onChange,
}: {
  provider: GatewayProviderForm;
  strings: AppStrings;
  onChange: (patch: Partial<GatewayProviderForm>) => void;
}) {
  return (
    <div className="grid gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={strings.name}>
          <Input
            autoFocus
            value={provider.name}
            onChange={(event) => onChange({ name: event.target.value })}
          />
        </Field>
        <Field label={strings.providerType}>
          <Select value={provider.type} onValueChange={(value) => onChange({ type: value })}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="openai_responses">openai_responses</SelectItem>
              <SelectItem value="openai_chat_completions">openai_chat_completions</SelectItem>
              <SelectItem value="anthropic_messages">anthropic_messages</SelectItem>
              <SelectItem value="gemini_generate_content">gemini_generate_content</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>
      <Field label={strings.baseUrl}>
        <Input value={provider.baseUrl} onChange={(event) => onChange({ baseUrl: event.target.value })} />
      </Field>
      <Field label={strings.apiKey}>
        <Input
          type="password"
          value={provider.apiKey}
          onChange={(event) => onChange({ apiKey: event.target.value })}
        />
      </Field>
      <Field label={strings.models}>
        <Input
          value={provider.models}
          placeholder="gpt-5.5, gpt-5.4"
          onChange={(event) => onChange({ models: event.target.value })}
        />
      </Field>
    </div>
  );
}

function SectionTitle({ icon, title }: { icon: React.ReactNode; title: string }) {
  return (
    <div className="flex items-center gap-2 text-sm font-medium">
      {icon}
      {title}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

function StatusBadge({
  isRunning,
  isRemoteRunning,
  isCloudRemote,
}: {
  isRunning: boolean;
  isRemoteRunning: boolean;
  isCloudRemote: boolean;
}) {
  const strings = useAppStrings();
  if (isRunning && isRemoteRunning) {
    const cloudRemoteTooltip = isCloudRemote ? strings.cloudRemoteConnectedTooltip : undefined;
    return (
      <Badge
        variant="success"
        className={cn("shrink-0", cloudRemoteTooltip ? "group relative" : "")}
        tabIndex={cloudRemoteTooltip ? 0 : undefined}
        aria-label={cloudRemoteTooltip ? `${strings.remote}. ${cloudRemoteTooltip}` : undefined}
      >
        <Radio className="w-3 h-3" />
        {strings.remote}
        {cloudRemoteTooltip ? <CloudRemoteIndicator tooltip={cloudRemoteTooltip} /> : null}
      </Badge>
    );
  }
  if (isRunning) {
    return (
      <Badge variant="success" className="shrink-0">
        <Activity className="w-3 h-3" />
        {strings.running}
      </Badge>
    );
  }
  return (
    <Badge variant="secondary" className="shrink-0">
      <Square className="w-3 h-3" />
      {strings.stopped}
    </Badge>
  );
}

function CloudRemoteIndicator({ tooltip }: { tooltip: string }) {
  return (
    <span className="inline-flex items-center gap-0.5" aria-hidden="true">
      <Cloud className="h-3 w-3" aria-hidden="true" />
      <LockKeyhole className="h-3 w-3" aria-hidden="true" />
      <span className="pointer-events-none absolute right-0 top-full z-[90] mt-2 hidden w-max max-w-64 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs normal-case text-card-foreground shadow-xl group-hover:block group-focus:block">
        {tooltip}
      </span>
    </span>
  );
}

function GatewayModelCombobox({
  id,
  value,
  options,
  triggerRef,
  strings,
  onValueChange,
}: {
  id: string;
  value: string;
  options: string[];
  triggerRef: React.RefObject<HTMLButtonElement | null>;
  strings: AppStrings;
  onValueChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const selected = value.trim();
  const allOptions = useMemo(() => {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const option of selected ? [selected, ...options] : options) {
      const normalized = option.trim();
      if (normalized && !seen.has(normalized)) {
        seen.add(normalized);
        result.push(normalized);
      }
    }
    return result;
  }, [options, selected]);
  const filteredOptions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return allOptions;
    }
    return allOptions.filter((option) => option.toLowerCase().includes(needle));
  }, [allOptions, query]);

  return (
    <DropdownMenu
      modal={false}
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (nextOpen) {
          setQuery("");
        }
      }}
    >
      <DropdownMenuTrigger asChild>
        <button
          id={id}
          ref={triggerRef}
          type="button"
          className={cn(
            "flex h-9 w-full items-center justify-between gap-2 rounded-md border border-input bg-background px-3 py-2 text-sm shadow-none transition-colors focus:outline-none focus:ring-2 focus:ring-ring",
            !selected && "text-muted-foreground",
          )}
        >
          <span className="min-w-0 truncate">{selected || strings.selectModel}</span>
          <ChevronDown className="h-4 w-4 shrink-0 opacity-50" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="w-[var(--radix-dropdown-menu-trigger-width)] p-0"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <div className="border-b border-border p-2">
          <div className="relative">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              autoFocus
              value={query}
              placeholder={strings.searchModel}
              className="h-8 pl-8"
              onChange={(event) => setQuery(event.target.value)}
              onClick={(event) => event.stopPropagation()}
              onKeyDown={(event) => event.stopPropagation()}
            />
          </div>
        </div>
        <div className="max-h-56 overflow-y-auto p-1">
          {filteredOptions.length > 0 ? (
            filteredOptions.map((option) => (
              <DropdownMenuItem
                key={option}
                className="justify-between gap-3"
                onSelect={(event) => {
                  event.preventDefault();
                  onValueChange(option);
                  setOpen(false);
                }}
              >
                <span className="min-w-0 truncate">{option}</span>
                {option === selected ? <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald" /> : null}
              </DropdownMenuItem>
            ))
          ) : (
            <div className="px-2 py-6 text-center text-sm text-muted-foreground">{strings.noModelsFound}</div>
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

type SettingsDialogProps = {
  dialogMode: DialogMode;
  providerMode: ProviderMode;
  form: ProviderForm;
  defaultProviders: DefaultProviderProfile[];
  botConfigs: SavedBotConfig[];
  settingsError: string;
  saveDisabled: boolean;
  editingProfileName: string | null;
  existingProviderSelectRef: React.RefObject<HTMLButtonElement | null>;
  workspaceNameInputRef: React.RefObject<HTMLInputElement | null>;
  providerNameInputRef: React.RefObject<HTMLInputElement | null>;
  existingProviderBaseUrlRef: React.RefObject<HTMLInputElement | null>;
  existingProviderModelRef: React.RefObject<HTMLInputElement | null>;
  newProviderBaseUrlRef: React.RefObject<HTMLInputElement | null>;
  newProviderApiKeyRef: React.RefObject<HTMLInputElement | null>;
  newProviderModelRef: React.RefObject<HTMLInputElement | null>;
  gatewayModelTriggerRef: React.RefObject<HTMLButtonElement | null>;
  gatewayEnabled: boolean;
  gatewayModels: string[];
  extensionsEnabled: boolean;
  onClose: () => void;
  onSave: () => Promise<void>;
  onSetForm: React.Dispatch<React.SetStateAction<ProviderForm>>;
  onSelectProviderMode: (mode: ProviderMode) => void;
  onSyncExistingProvider: (profileName: string) => void;
};

function SettingsDialog({
  dialogMode,
  providerMode,
  form,
  defaultProviders,
  botConfigs,
  settingsError,
  saveDisabled,
  editingProfileName,
  existingProviderSelectRef,
  workspaceNameInputRef,
  providerNameInputRef,
  existingProviderBaseUrlRef,
  existingProviderModelRef,
  newProviderBaseUrlRef,
  newProviderApiKeyRef,
  newProviderModelRef,
  gatewayModelTriggerRef,
  gatewayEnabled,
  gatewayModels,
  extensionsEnabled,
  onClose,
  onSave,
  onSetForm,
  onSelectProviderMode,
  onSyncExistingProvider,
}: SettingsDialogProps) {
  const strings = useAppStrings();
  const botAuthSpecs = authSpecsForPlatform(form.botPlatform);
  const botAuthType = normalizeBotAuthType(form.botPlatform, form.botAuthType);
  const botAuthFields = fieldsForBotAuth(form.botPlatform, botAuthType);
  const availableBotConfigs = useMemo(() => normalizeSavedBotConfigs(botConfigs), [botConfigs]);
  const selectedBotConfigId = availableBotConfigs.some((item) => item.id === form.botConfigId)
    ? form.botConfigId
    : BOT_CONFIG_CUSTOM_VALUE;
  const newProviderActive = providerMode === "new" || providerMode === "gateway";
  const isEditingDefaultWorkspace = dialogMode === "edit" && editingProfileName === "Default";
  const canChangeProviderMode = dialogMode === "add" || dialogMode === "edit";
  const [wifiScan, setWifiScan] = useState<BotHandoffScanState>(emptyHandoffScanState);
  const [bluetoothScan, setBluetoothScan] = useState<BotHandoffScanState>(emptyHandoffScanState);
  const autoHandoffScanRef = useRef(false);

  const scanHandoffTargets = useCallback(async (kind: "wifi" | "bluetooth") => {
    const setScan = kind === "wifi" ? setWifiScan : setBluetoothScan;
    const command =
      kind === "wifi" ? "scan_bot_handoff_wifi_targets" : "scan_bot_handoff_bluetooth_targets";
    setScan({ ...emptyHandoffScanState, loading: true });
    try {
      const results = await invoke<BotHandoffScanTarget[]>(command);
      setScan({
        loading: false,
        error: "",
        results,
      });
    } catch (error) {
      setScan({
        loading: false,
        error: errorMessage(error),
        results: [],
      });
    }
  }, []);

  const selectHandoffTarget = (kind: "wifi" | "bluetooth", targetValue: string) => {
    if (kind === "wifi") {
      onSetForm((current) => ({
        ...current,
        botHandoffPhoneWifiTargets: targetValue,
      }));
      return;
    }
    onSetForm((current) => ({
      ...current,
      botHandoffPhoneBluetoothTargets: targetValue,
    }));
  };

  useEffect(() => {
    if (!form.botEnabled || !form.botHandoffEnabled) {
      autoHandoffScanRef.current = false;
      return;
    }
    if (autoHandoffScanRef.current) {
      return;
    }
    autoHandoffScanRef.current = true;
    void scanHandoffTargets("wifi");
    void scanHandoffTargets("bluetooth");
  }, [form.botEnabled, form.botHandoffEnabled, scanHandoffTargets]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>
            {dialogMode === "edit" && editingProfileName ? strings.editProfile(editingProfileName) : strings.newProfile}
          </DialogTitle>
          <DialogDescription>{strings.configureInstance}</DialogDescription>
        </DialogHeader>
        {settingsError ? (
          <p className="bg-destructive/12 border border-destructive/50 rounded-md text-red-300 text-sm leading-relaxed px-3 py-2.5">
            {settingsError}
          </p>
        ) : null}
        {canChangeProviderMode ? (
          <div className="bg-background border border-border rounded-md grid grid-cols-3 p-0.5">
            <Button
              variant={providerMode === "none" ? "secondary" : "ghost"}
              size="sm"
              className={cn(
                "shadow-none",
                providerMode !== "none" && "text-muted-foreground hover:bg-transparent",
              )}
              type="button"
              onClick={() => onSelectProviderMode("none")}
            >
              {strings.none}
            </Button>
            <Button
              variant={providerMode === "existing" ? "secondary" : "ghost"}
              size="sm"
              className={cn(
                "shadow-none",
                providerMode !== "existing" && "text-muted-foreground hover:bg-transparent",
              )}
              type="button"
              disabled={defaultProviders.length === 0}
              onClick={() => onSelectProviderMode("existing")}
            >
              {strings.fromDefault}
            </Button>
            <Button
              variant={newProviderActive ? "secondary" : "ghost"}
              size="sm"
              className={cn(
                "shadow-none",
                !newProviderActive && "text-muted-foreground hover:bg-transparent",
              )}
              type="button"
              disabled={isEditingDefaultWorkspace || (dialogMode === "edit" && !gatewayEnabled)}
              onClick={() => onSelectProviderMode(gatewayEnabled ? "gateway" : "new")}
            >
              {strings.newProvider}
            </Button>
          </div>
        ) : null}

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="workspaceNameInput">{strings.workspaceName}</Label>
          <Input
            id="workspaceNameInput"
            ref={workspaceNameInputRef}
            type="text"
            placeholder="my-workspace"
            disabled={dialogMode === "edit" && editingProfileName === "Default"}
            value={form.workspaceName}
            onChange={(event) =>
              onSetForm((current) => ({ ...current, workspaceName: event.target.value }))
            }
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="workspaceProxyInput">{strings.proxyUrl}</Label>
          <Input
            id="workspaceProxyInput"
            type="text"
            placeholder="http://127.0.0.1:7890"
            value={form.proxyUrl}
            onChange={(event) =>
              onSetForm((current) => ({ ...current, proxyUrl: event.target.value }))
            }
          />
        </div>

        {providerMode === "existing" ? (
          <div className="flex flex-col gap-3.5">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="existingProviderSelect">{strings.provider}</Label>
              <Select
                value={form.existingProfileName}
                onValueChange={(value) => {
                  if (value === WORKSPACE_PROVIDER_NONE_VALUE) {
                    onSelectProviderMode("none");
                    return;
                  }
                  if (value === WORKSPACE_PROVIDER_GATEWAY_VALUE) {
                    onSelectProviderMode("gateway");
                    return;
                  }
                  onSyncExistingProvider(value);
                }}
              >
                <SelectTrigger id="existingProviderSelect" ref={existingProviderSelectRef}>
                  <SelectValue placeholder={strings.selectProvider} />
                </SelectTrigger>
                <SelectContent>
                  {canChangeProviderMode ? (
                    <SelectItem value={WORKSPACE_PROVIDER_NONE_VALUE}>{strings.none}</SelectItem>
                  ) : null}
                  {gatewayEnabled && canChangeProviderMode ? (
                    <SelectItem value={WORKSPACE_PROVIDER_GATEWAY_VALUE}>{strings.nextAiGatewayProvider}</SelectItem>
                  ) : null}
                  {defaultProviders.map((profile) => (
                    <SelectItem key={profile.name} value={profile.name}>
                      {profile.name} ({profile.provider_name} / {profile.model})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-3.5">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="existingProviderBaseUrlInput">{strings.baseUrl}</Label>
                <Input
                  id="existingProviderBaseUrlInput"
                  ref={existingProviderBaseUrlRef}
                  type="text"
                  placeholder="https://api.example.com/v1"
                  value={form.existingBaseUrl}
                  onChange={(event) =>
                    onSetForm((current) => ({ ...current, existingBaseUrl: event.target.value }))
                  }
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="existingProviderApiKeyInput">{strings.apiKey}</Label>
                <Input
                  id="existingProviderApiKeyInput"
                  type="password"
                  placeholder={strings.keepCurrentApiKey}
                  value={form.existingApiKey}
                  onChange={(event) =>
                    onSetForm((current) => ({ ...current, existingApiKey: event.target.value }))
                  }
                />
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="existingProviderModelInput">{strings.model}</Label>
              <Input
                id="existingProviderModelInput"
                ref={existingProviderModelRef}
                type="text"
                placeholder="gpt-5.5"
                value={form.existingModel}
                onChange={(event) =>
                  onSetForm((current) => ({ ...current, existingModel: event.target.value }))
                }
              />
            </div>
          </div>
        ) : null}

        {newProviderActive ? (
          <div className="flex flex-col gap-3.5">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="providerNameInput">{strings.providerProfileName}</Label>
              <Input
                id="providerNameInput"
                ref={providerNameInputRef}
                type="text"
                placeholder="nextai"
                disabled={isEditingDefaultWorkspace}
                value={form.providerName}
                onChange={(event) =>
                  onSetForm((current) => ({ ...current, providerName: event.target.value }))
                }
              />
            </div>
            {gatewayEnabled ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="providerTypeSelect">{strings.providerType}</Label>
                <Select
                  disabled={!canChangeProviderMode}
                  value={providerMode === "gateway" ? "gateway" : "third-party"}
                  onValueChange={(value) => {
                    if (value === "none") {
                      onSelectProviderMode("none");
                      return;
                    }
                    onSelectProviderMode(value === "gateway" ? "gateway" : "new");
                  }}
                >
                  <SelectTrigger id="providerTypeSelect">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {canChangeProviderMode ? (
                      <SelectItem value="none">{strings.none}</SelectItem>
                    ) : null}
                    <SelectItem
                      value="gateway"
                      disabled={
                        isEditingDefaultWorkspace ||
                        (dialogMode === "edit" && providerMode !== "gateway")
                      }
                    >
                      {strings.nextAiGatewayProvider}
                    </SelectItem>
                    <SelectItem
                      value="third-party"
                      disabled={
                        isEditingDefaultWorkspace ||
                        (dialogMode === "edit" && providerMode !== "new")
                      }
                    >
                      {strings.thirdPartyProvider}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            ) : null}
            {providerMode === "gateway" ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="gatewayModelInput">{strings.model}</Label>
                <GatewayModelCombobox
                  id="gatewayModelInput"
                  value={form.gatewayModel}
                  options={gatewayModels}
                  triggerRef={gatewayModelTriggerRef}
                  strings={strings}
                  onValueChange={(value) => onSetForm((current) => ({ ...current, gatewayModel: value }))}
                />
              </div>
            ) : (
              <>
                <div className="grid grid-cols-2 gap-3.5">
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="providerBaseUrlInput">{strings.baseUrl}</Label>
                    <Input
                      id="providerBaseUrlInput"
                      ref={newProviderBaseUrlRef}
                      type="text"
                      placeholder="https://api.example.com/v1"
                      value={form.providerBaseUrl}
                      onChange={(event) =>
                        onSetForm((current) => ({ ...current, providerBaseUrl: event.target.value }))
                      }
                    />
                  </div>
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="providerApiKeyInput">{strings.apiKey}</Label>
                    <Input
                      id="providerApiKeyInput"
                      ref={newProviderApiKeyRef}
                      type="password"
                      placeholder="sk-..."
                      value={form.providerApiKey}
                      onChange={(event) =>
                        onSetForm((current) => ({ ...current, providerApiKey: event.target.value }))
                      }
                    />
                  </div>
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="providerModelInput">{strings.model}</Label>
                  <Input
                    id="providerModelInput"
                    ref={newProviderModelRef}
                    type="text"
                    placeholder="gpt-5.5"
                    value={form.providerModel}
                    onChange={(event) =>
                      onSetForm((current) => ({ ...current, providerModel: event.target.value }))
                    }
                  />
                </div>
              </>
            )}
          </div>
        ) : null}

        {extensionsEnabled ? (
          <div className="border-t border-border pt-4 flex flex-col gap-3.5">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <Label className="text-sm">{strings.bot}</Label>
              </div>
              <Switch
                checked={form.botEnabled}
                aria-label={strings.enableBotIntegration}
                onCheckedChange={(checked) =>
                  onSetForm((current) => {
                    const enabled = checked === true;
                    const nextPlatform = enabled && current.botPlatform === "none" ? "weixin-ilink" : current.botPlatform;
                    const nextAuthType = normalizeBotAuthType(nextPlatform, current.botAuthType);
                    return {
                      ...current,
                      botEnabled: enabled,
                      botPlatform: nextPlatform,
                      botAuthType: nextAuthType,
                      botAuthFields: enabled
                        ? pickBotAuthFields(current.botAuthFields, nextPlatform, nextAuthType)
                        : {},
                      botConfigId: enabled ? current.botConfigId : "",
                      botTenantId: enabled ? current.botTenantId : "",
                      botIntegrationId: enabled ? current.botIntegrationId : "",
                      botStateDir: enabled ? current.botStateDir : "",
                      botStatus: enabled ? current.botStatus : "",
                      botLastLoginAt: enabled ? current.botLastLoginAt : "",
                      botForwardAllCodexMessages: enabled ? current.botForwardAllCodexMessages : false,
                      botHandoffEnabled: enabled ? current.botHandoffEnabled : false,
                    };
                  })
                }
              />
            </div>

            {form.botEnabled ? (
              <div className="flex flex-col gap-3.5">
                {availableBotConfigs.length > 0 ? (
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="botSavedConfigSelect">{strings.savedBotConfig}</Label>
                    <Select
                      value={selectedBotConfigId}
                      onValueChange={(value) =>
                        onSetForm((current) => {
                          if (value === BOT_CONFIG_CUSTOM_VALUE) {
                            return clearBotConfigSelection(current);
                          }
                          const saved = availableBotConfigs.find((item) => item.id === value);
                          return saved ? applySavedBotConfig(current, saved) : clearBotConfigSelection(current);
                        })
                      }
                    >
                      <SelectTrigger id="botSavedConfigSelect">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={BOT_CONFIG_CUSTOM_VALUE}>{strings.customBotConfig}</SelectItem>
                        {availableBotConfigs.map((saved) => (
                          <SelectItem key={saved.id} value={saved.id}>
                            {botConfigLabel(saved)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="botPlatformSelect">{strings.platform}</Label>
                  <Select
                    value={form.botPlatform}
                    onValueChange={(value) =>
                      onSetForm((current) => {
                        const nextPlatform = normalizeBotPlatform(value);
                        const nextAuthType = defaultBotAuthType(nextPlatform);
                        return {
                          ...current,
                          botPlatform: nextPlatform,
                          botEnabled: nextPlatform !== "none",
                          botAuthType: nextAuthType,
                          botAuthFields: pickBotAuthFields(current.botAuthFields, nextPlatform, nextAuthType),
                          botConfigId: "",
                          botTenantId: "",
                          botIntegrationId: "",
                          botStateDir: "",
                          botStatus: "",
                          botLastLoginAt: "",
                          botForwardAllCodexMessages:
                            nextPlatform !== "none" ? current.botForwardAllCodexMessages : false,
                          botHandoffEnabled: nextPlatform !== "none" ? current.botHandoffEnabled : false,
                        };
                      })
                    }
                  >
                    <SelectTrigger id="botPlatformSelect">
                      <SelectValue placeholder={strings.selectPlatform} />
                    </SelectTrigger>
                    <SelectContent>
                      {BOT_PLATFORM_OPTIONS.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                      <SelectItem value="none">{strings.none}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {botAuthSpecs.length > 0 ? (
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="botAuthTypeSelect">{strings.authMethod}</Label>
                    <Select
                      value={botAuthType}
                      onValueChange={(value) =>
                        onSetForm((current) => {
                          const nextAuthType = normalizeBotAuthType(current.botPlatform, value);
                          return {
                            ...current,
                            botAuthType: nextAuthType,
                            botAuthFields: pickBotAuthFields(
                              current.botAuthFields,
                              current.botPlatform,
                              nextAuthType,
                            ),
                            botConfigId: "",
                            botTenantId: "",
                            botIntegrationId: "",
                            botStateDir: "",
                            botStatus: "",
                            botLastLoginAt: "",
                          };
                        })
                      }
                    >
                      <SelectTrigger id="botAuthTypeSelect">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {botAuthSpecs.map((option) => (
                          <SelectItem key={option.value} value={option.value}>
                            {option.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
                {botAuthFields.length > 0 ? (
                  <div className="grid gap-3 sm:grid-cols-2">
                    {botAuthFields.map((field) => (
                      <div key={field.key} className="flex flex-col gap-1.5">
                        <Label htmlFor={`botAuthField-${field.key}`} className="flex items-center gap-1.5">
                          <span>{field.label}</span>
                          {field.required ? null : (
                            <span className="text-xs font-normal text-muted-foreground">{strings.optional}</span>
                          )}
                        </Label>
                        <Input
                          id={`botAuthField-${field.key}`}
                          type={field.type || "text"}
                          autoComplete="off"
                          placeholder={field.placeholder || ""}
                          value={form.botAuthFields[field.key] || ""}
                          onChange={(event) =>
                            onSetForm((current) => ({
                              ...current,
                              botAuthFields: {
                                ...current.botAuthFields,
                                [field.key]: event.target.value,
                              },
                              botConfigId: "",
                              botTenantId: "",
                              botIntegrationId: "",
                              botStateDir: "",
                              botStatus: "",
                              botLastLoginAt: "",
                            }))
                          }
                        />
                      </div>
                    ))}
                  </div>
                ) : null}
                <div className="flex items-center justify-between gap-4 rounded-md border border-border px-3 py-2">
                  <Label htmlFor="botForwardAllCodexMessagesSwitch" className="text-sm">
                    {strings.forwardAllCodexMessages}
                  </Label>
                  <Switch
                    id="botForwardAllCodexMessagesSwitch"
                    checked={form.botForwardAllCodexMessages}
                    aria-label={strings.forwardAllCodexMessages}
                    onCheckedChange={(checked) =>
                      onSetForm((current) => ({
                        ...current,
                        botForwardAllCodexMessages: checked === true,
                      }))
                    }
                  />
                </div>
                <div className="flex flex-col gap-3 rounded-md border border-border px-3 py-2.5">
                  <div className="flex items-center justify-between gap-4">
                    <Label htmlFor="botHandoffSwitch" className="text-sm">
                      {strings.handoffMode}
                    </Label>
                    <Switch
                      id="botHandoffSwitch"
                      checked={form.botHandoffEnabled}
                      aria-label={strings.handoffMode}
                      onCheckedChange={(checked) =>
                        onSetForm((current) => ({
                          ...current,
                          botHandoffEnabled: checked === true,
                        }))
                      }
                    />
                  </div>
                  {form.botHandoffEnabled ? (
                    <div className="grid gap-3 sm:grid-cols-2">
                      <div className="flex flex-col gap-1.5">
                        <Label htmlFor="botHandoffIdleSecondsInput">
                          {strings.handoffIdleSeconds}
                        </Label>
                        <Input
                          id="botHandoffIdleSecondsInput"
                          type="number"
                          min={30}
                          step={30}
                          value={form.botHandoffIdleSeconds}
                          onChange={(event) =>
                            onSetForm((current) => ({
                              ...current,
                              botHandoffIdleSeconds: event.target.value,
                            }))
                          }
                        />
                      </div>
                      <HandoffTargetPicker
                        id="botHandoffPhoneWifiTargetsInput"
                        label={strings.handoffPhoneWifiTargets}
                        selectedTarget={firstTarget(form.botHandoffPhoneWifiTargets)}
                        scan={wifiScan}
                        strings={strings}
                        onRefresh={() => scanHandoffTargets("wifi")}
                        onSelect={(targetValue) => selectHandoffTarget("wifi", targetValue)}
                      />
                      <HandoffTargetPicker
                        id="botHandoffPhoneBluetoothTargetsInput"
                        className="sm:col-span-2"
                        label={strings.handoffPhoneBluetoothTargets}
                        selectedTarget={firstTarget(form.botHandoffPhoneBluetoothTargets)}
                        scan={bluetoothScan}
                        strings={strings}
                        onRefresh={() => scanHandoffTargets("bluetooth")}
                        onSelect={(targetValue) => selectHandoffTarget("bluetooth", targetValue)}
                      />
                    </div>
                  ) : null}
                </div>
              </div>
            ) : null}
          </div>
        ) : null}

        <DialogFooter className="pt-1">
          <Button
            type="button"
            variant="outline"
            onClick={onClose}
          >
            {strings.cancel}
          </Button>
          <Button
            type="button"
            disabled={saveDisabled}
            onClick={() => onSave().catch(console.error)}
          >
            {dialogMode === "edit" ? strings.save : strings.createProfile}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type HandoffTargetPickerProps = {
  id: string;
  className?: string;
  label: string;
  selectedTarget: string;
  scan: BotHandoffScanState;
  strings: AppStrings;
  onRefresh: () => void;
  onSelect: (targetValue: string) => void;
};

function HandoffTargetPicker({
  id,
  className,
  label,
  selectedTarget,
  scan,
  strings,
  onRefresh,
  onSelect,
}: HandoffTargetPickerProps) {
  const options = selectedTarget && !scan.results.some((target) => handoffTargetMatchesSavedValue(target, selectedTarget))
    ? [
        {
          id: `selected:${selectedTarget}`,
          label: selectedTarget,
          target: selectedTarget,
          detail: "",
          source: "selected",
        },
        ...scan.results,
      ]
    : scan.results;
  const placeholderText = scan.loading
    ? strings.scanningTargets
    : options.length > 0
      ? strings.selectScanTarget
      : strings.noScanTargets;
  const selectedOption = options.find((target) => handoffTargetMatchesSavedValue(target, selectedTarget));
  const selectedDisplayText = selectedOption ? handoffTargetSelectionText(selectedOption) : "";

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div className="flex items-center justify-between gap-2">
        <Label htmlFor={id}>{label}</Label>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className="h-7 w-7"
          title={strings.refreshTargets}
          aria-label={strings.refreshTargets}
          disabled={scan.loading}
          onClick={onRefresh}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", scan.loading ? "animate-spin" : "")} />
        </Button>
      </div>
      <Select
        value={selectedTarget}
        onValueChange={(value) => onSelect(value === HANDOFF_TARGET_NONE_VALUE ? "" : value)}
      >
        <SelectTrigger id={id} disabled={scan.loading || options.length === 0}>
          <SelectValue placeholder={placeholderText}>
            {selectedDisplayText || undefined}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          {selectedTarget ? (
            <SelectItem value={HANDOFF_TARGET_NONE_VALUE}>{strings.none}</SelectItem>
          ) : null}
          {options.map((target) => (
            <SelectItem
              key={target.id}
              value={handoffTargetSavedValue(target)}
              textValue={handoffTargetSelectionText(target)}
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate">{handoffTargetOptionTitle(target)}</span>
                {target.detail ? (
                  <span className="truncate text-xs text-muted-foreground">{target.detail}</span>
                ) : null}
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {scan.error ? (
        <p className="text-xs text-destructive">{scan.error}</p>
      ) : null}
    </div>
  );
}

function handoffTargetOptionTitle(target: BotHandoffScanTarget): string {
  if (target.source === "bluetooth") {
    return handoffTargetSelectionText(target);
  }
  return target.label;
}

function handoffTargetSelectionText(target: BotHandoffScanTarget): string {
  if (target.source !== "bluetooth") {
    return target.label;
  }
  const label = target.label.trim();
  const value = target.target.trim();
  if (!label || !value || label === value || label.includes(value)) {
    return label || value;
  }
  return `${label}(${value})`;
}

function handoffTargetSavedValue(target: BotHandoffScanTarget): string {
  if (target.source === "bluetooth") {
    return handoffTargetSelectionText(target);
  }
  return target.target;
}

function handoffTargetMatchesSavedValue(target: BotHandoffScanTarget, savedValue: string): boolean {
  return target.target === savedValue || handoffTargetSavedValue(target) === savedValue;
}

type DeleteDialogProps = {
  profile: ProviderProfile;
  removeCodexHome: boolean;
  onRemoveCodexHomeChange: (remove: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
};

function DeleteDialog({
  profile,
  removeCodexHome,
  onRemoveCodexHomeChange,
  onCancel,
  onConfirm,
}: DeleteDialogProps) {
  const strings = useAppStrings();
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onCancel();
        }
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{strings.deleteInstance}</AlertDialogTitle>
          <AlertDialogDescription>
            {strings.deleteInstanceConfirm(profile.name)}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {profile.codex_home ? (
          <>
            <Label className="flex items-center gap-3 cursor-pointer select-none text-foreground">
              <Checkbox
                checked={removeCodexHome}
                onCheckedChange={(checked) => onRemoveCodexHomeChange(checked === true)}
              />
              <span className="text-sm">{strings.alsoDeleteCodexHome}</span>
            </Label>
            {removeCodexHome ? (
              <p className="text-xs text-muted-foreground font-mono bg-muted/50 rounded-md px-3 py-2">
                {profile.codex_home}
              </p>
            ) : null}
          </>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel onClick={onCancel}>
            {strings.cancel}
          </AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-white hover:bg-destructive/90"
            onClick={onConfirm}
          >
            {strings.delete}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function RemotePasswordDialog({
  profileName,
  strings,
  onCancel,
  onConfirm,
}: {
  profileName: string;
  strings: AppStrings;
  onCancel: () => void;
  onConfirm: (password: string) => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [showPassword, setShowPassword] = useState(false);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, []);

  const submit = () => {
    if (!password) {
      setError(strings.encryptionPasswordRequired);
      inputRef.current?.focus();
      return;
    }
    onConfirm(password);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>{strings.endToEndEncryption}</DialogTitle>
          <DialogDescription>{strings.encryptionPasswordPrompt(profileName)}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="remoteE2eePasswordInput">{strings.encryptCloudRelay}</Label>
          <div className="relative">
            <Input
              id="remoteE2eePasswordInput"
              ref={inputRef}
              type={showPassword ? "text" : "password"}
              autoComplete="new-password"
              className="pr-10"
              value={password}
              onChange={(event) => {
                setPassword(event.target.value);
                if (error) {
                  setError("");
                }
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  submit();
                }
              }}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              title={showPassword ? strings.hidePassword : strings.showPassword}
              aria-label={showPassword ? strings.hidePassword : strings.showPassword}
              onClick={() => {
                setShowPassword((current) => !current);
                window.requestAnimationFrame(() => inputRef.current?.focus());
              }}
            >
              {showPassword ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
            </Button>
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel}>
            {strings.cancel}
          </Button>
          <Button type="button" onClick={submit}>
            {strings.save}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RemoteQrDialog({
  remoteQr,
  onClose,
  onError,
}: {
  remoteQr: RemoteQrState;
  onClose: () => void;
  onError: (error: unknown) => void;
}) {
  const strings = useAppStrings();
  const [copySucceeded, setCopySucceeded] = useState(false);
  const copyResetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    };
  }, []);

  const handleCopyUrl = useCallback(async () => {
    try {
      await copyText(remoteQr.url);
      setCopySucceeded(true);
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
      copyResetTimerRef.current = window.setTimeout(() => {
        setCopySucceeded(false);
        copyResetTimerRef.current = null;
      }, 2000);
    } catch (error) {
      onError(error);
    }
  }, [onError, remoteQr.url]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent className="max-w-md overflow-hidden p-0">
        <DialogHeader className="border-b border-border px-5 py-4">
          <DialogTitle className="text-base">{strings.remoteQr}</DialogTitle>
          <DialogDescription>{remoteQr.profile.name}</DialogDescription>
        </DialogHeader>
        <div className="px-5 py-5 flex flex-col gap-4">
          <div
            className="mx-auto rounded-lg bg-white p-3 shadow-sm"
            dangerouslySetInnerHTML={{ __html: remoteQr.markup }}
          />
          <div className="space-y-2">
            <div className="text-[11px] font-semibold uppercase text-muted-foreground">{strings.lanUrl}</div>
            <div className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs font-mono break-all">
              {remoteQr.url}
            </div>
            <div className="text-[11px] font-semibold uppercase text-muted-foreground">{strings.token}</div>
            <div className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs font-mono break-all">
              {remoteQr.remote.token}
            </div>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              type="button"
              onClick={handleCopyUrl}
            >
              {copySucceeded ? <CheckCircle2 className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
              {copySucceeded ? strings.copied : strings.copyUrl}
            </Button>
            <Button
              type="button"
              onClick={() => openUrl(remoteQr.url).catch(onError)}
            >
              <ExternalLink className="w-3.5 h-3.5" />
              {strings.open}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function WeixinBotQrDialog({
  login,
  onRegenerate,
  onClose,
}: {
  login: WeixinBotQrState;
  onRegenerate: () => void;
  onClose: () => void;
}) {
  const strings = useAppStrings();
  const terminal = isTerminalBotLoginStatus(login.status);
  const confirmed = login.status === "confirmed";

  useEffect(() => {
    if (terminal || login.qrDisplay.kind !== "webview") {
      return;
    }
    openQrWebview(login).catch(console.error);
  }, [login.profileName, login.qrDisplay, login.sessionId, terminal]);

  useEffect(() => {
    if (!confirmed) {
      return;
    }
    closeQrWebview(login.sessionId).catch(console.error);
  }, [confirmed, login.sessionId]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent className="max-w-md overflow-hidden p-0">
        <DialogHeader className="border-b border-border px-5 py-4">
          <DialogTitle className="text-base">{strings.weixinBotLogin}</DialogTitle>
          <DialogDescription>{login.profileName}</DialogDescription>
        </DialogHeader>
        <div className="px-5 py-5 flex flex-col gap-4">
          <div className="mx-auto h-80 w-full max-w-sm rounded-lg border border-border bg-muted/30 p-3 shadow-sm flex items-center justify-center">
            {login.qrDisplay.kind === "webview" ? (
              <div className="flex flex-col items-center gap-3 text-center">
                <Smartphone className="h-12 w-12 text-muted-foreground" />
                <div className="text-sm font-medium">{strings.nativeWebview}</div>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => openQrWebview(login).catch(console.error)}
                >
                  <ExternalLink className="w-3.5 h-3.5" />
                  {strings.reopen}
                </Button>
              </div>
            ) : login.qrDisplay.kind === "image" ? (
              <img
                src={login.qrDisplay.src}
                alt="Weixin QR"
                className="h-full w-full object-contain"
              />
            ) : (
              <QrCode className="h-20 w-20 text-black/50" />
            )}
          </div>
          <div className="rounded-md border border-border bg-muted/30 px-3 py-2.5">
            <div className="flex items-center gap-2 text-sm font-medium">
              {confirmed ? (
                <CheckCircle2 className="h-4 w-4 text-emerald" />
              ) : terminal ? (
                <AlertCircle className="h-4 w-4 text-destructive" />
              ) : (
                <Smartphone className="h-4 w-4 text-muted-foreground" />
              )}
              <span>{botLoginStatusLabel(login.status, strings)}</span>
            </div>
            {login.statusMessage ? (
              <div className="mt-1 text-xs text-muted-foreground">{login.statusMessage}</div>
            ) : null}
          </div>
          <div className="space-y-2">
            <div className="text-[11px] font-semibold uppercase text-muted-foreground">{strings.integration}</div>
            <div className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs font-mono break-all">
              {login.integrationId}
            </div>
            {login.expiresAt ? (
              <>
                <div className="text-[11px] font-semibold uppercase text-muted-foreground">{strings.expires}</div>
                <div className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs font-mono break-all">
                  {login.expiresAt}
                </div>
              </>
            ) : null}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              type="button"
              onClick={onClose}
            >
              {strings.close}
            </Button>
            <Button
              type="button"
              disabled={!terminal && !confirmed}
              onClick={onRegenerate}
            >
              <RefreshCw className="w-3.5 h-3.5" />
              {strings.regenerate}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function codexServerBaseUrl() {
  return (import.meta.env.VITE_CODEXL_SERVER_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
}

function codexServerUrl(path: string) {
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  return `${codexServerBaseUrl()}${normalizedPath}`;
}

function normalizeDesktopLoginUrl(loginUrl: string) {
  try {
    const serverUrl = new URL(codexServerBaseUrl());
    const parsedLoginUrl = new URL(loginUrl);
    if (
      parsedLoginUrl.protocol === serverUrl.protocol &&
      parsedLoginUrl.hostname === serverUrl.hostname
    ) {
      parsedLoginUrl.port = serverUrl.port;
      return parsedLoginUrl.toString();
    }
  } catch {
    return loginUrl;
  }

  return loginUrl;
}

async function startDesktopLogin(language: Language): Promise<DesktopAuthStartResponse> {
  const response = await fetch(codexServerUrl("/api/desktop-auth/start"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      locale: language === "zh" ? "zh-CN" : "en",
      deviceName: "CodexL Launcher",
    }),
  });

  if (!response.ok) {
    throw new Error(`desktop login start failed: ${response.status}`);
  }

  return response.json();
}

async function pollDesktopLogin(code: string): Promise<DesktopAuthPollResponse> {
  const url = new URL(codexServerUrl("/api/desktop-auth/poll"));
  url.searchParams.set("code", code);
  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`desktop login poll failed: ${response.status}`);
  }

  return response.json();
}

function remoteCloudAuthFromDesktopLogin(
  result: Extract<DesktopAuthPollResponse, { status: "authenticated" }>,
): RemoteCloudAuthConfig {
  if (result.cloudAuth) {
    return {
      user_id: result.cloudAuth.userId,
      display_name: result.cloudAuth.displayName,
      email: result.cloudAuth.email,
      avatar_url: result.cloudAuth.avatarUrl ?? "",
      is_pro: result.user.hasSubscription,
      access_token: result.cloudAuth.accessToken,
      refresh_token: result.cloudAuth.refreshToken,
      expires_at: result.cloudAuth.expiresAt,
    };
  }

  return {
    user_id: result.user.id,
    display_name: result.user.name || result.user.email,
    email: result.user.email,
    avatar_url: result.user.avatarUrl ?? "",
    is_pro: result.user.hasSubscription,
    access_token: "",
    refresh_token: "",
    expires_at: 0,
  };
}

function remoteRelayUrlFromDesktopLogin(
  result: Extract<DesktopAuthPollResponse, { status: "authenticated" }>,
) {
  return normalizeRemoteRelayUrl(
    result.cloudAuth?.relayUrl ??
      result.cloudAuth?.relay_url ??
      result.cloudAuth?.remoteRelayUrl ??
      result.relayUrl ??
      result.relay_url ??
      result.remoteRelayUrl ??
      "",
  );
}

function normalizeRemoteRelayUrl(value: unknown) {
  return typeof value === "string" ? value.trim().replace(/\/+$/, "") : "";
}

function emptyRemoteCloudAuth(): RemoteCloudAuthConfig {
  return {
    user_id: "",
    display_name: "",
    email: "",
    avatar_url: "",
    is_pro: false,
    access_token: "",
    refresh_token: "",
    expires_at: 0,
  };
}

function hasRemoteCloudIdentity(auth: RemoteCloudAuthConfig | null | undefined) {
  if (!auth?.user_id.trim()) {
    return false;
  }

  return auth.expires_at === 0 || auth.expires_at > Math.floor(Date.now() / 1000) + 60;
}

function remoteCloudDisplayName(auth: RemoteCloudAuthConfig) {
  const claims = remoteCloudJwtClaims(auth);
  return (
    auth.display_name?.trim() ||
    stringClaim(claims, "name") ||
    auth.email?.trim() ||
    stringClaim(claims, "email") ||
    "CodexL"
  );
}

function remoteCloudEmail(auth: RemoteCloudAuthConfig) {
  return auth.email?.trim() || stringClaim(remoteCloudJwtClaims(auth), "email");
}

function remoteCloudAvatarUrl(auth: RemoteCloudAuthConfig) {
  const claims = remoteCloudJwtClaims(auth);
  return auth.avatar_url?.trim() || stringClaim(claims, "picture") || stringClaim(claims, "avatarUrl");
}

function remoteCloudJwtClaims(auth: RemoteCloudAuthConfig) {
  const parts = auth.access_token?.split(".") ?? [];

  if (parts.length < 2) {
    return null;
  }

  try {
    const normalized = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
    return JSON.parse(window.atob(padded)) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function stringClaim(claims: Record<string, unknown> | null, key: string) {
  const value = claims?.[key];
  return typeof value === "string" ? value.trim() : "";
}

function accountInitials(label: string) {
  const trimmed = label.trim();

  if (!trimmed) {
    return "CL";
  }

  const parts = trimmed.split(/\s+/).filter(Boolean);
  const initials = parts.length > 1 ? `${parts[0][0]}${parts[1][0]}` : trimmed.slice(0, 2);
  return initials.toUpperCase();
}

function sleep(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

async function copyText(text: string) {
  if (!navigator.clipboard?.writeText) {
    throw new Error("clipboard is not available");
  }
  await navigator.clipboard.writeText(text);
}

function readNewProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  nameRef: React.RefObject<HTMLInputElement | null>,
  baseUrlRef: React.RefObject<HTMLInputElement | null>,
  apiKeyRef: React.RefObject<HTMLInputElement | null>,
  modelRef: React.RefObject<HTMLInputElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
  extensionsEnabled: boolean,
): NewProvider | null {
  const provider = {
    workspace_name: form.workspaceName.trim(),
    name: form.providerName.trim(),
    base_url: form.providerBaseUrl.trim(),
    api_key: form.providerApiKey.trim(),
    model: form.providerModel.trim(),
    proxy_url: form.proxyUrl.trim(),
    bot: readBotConfig(form, form.workspaceName),
  };

  if (!provider.workspace_name) {
    showError(strings.nameRequired);
    workspaceNameRef.current?.focus();
    return null;
  }
  if (!provider.name) {
    showError(strings.nameRequired);
    nameRef.current?.focus();
    return null;
  }
  if (!provider.base_url) {
    showError(strings.baseUrlRequired);
    baseUrlRef.current?.focus();
    return null;
  }
  if (!provider.api_key) {
    showError(strings.apiKeyRequired);
    apiKeyRef.current?.focus();
    return null;
  }
  if (!provider.model) {
    showError(strings.modelRequired);
    modelRef.current?.focus();
    return null;
  }
  if (extensionsEnabled && !validateBotAuth(form, strings, showError)) {
    return null;
  }
  return provider;
}

function readWorkspaceProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
  extensionsEnabled: boolean,
): WorkspaceProvider | null {
  const provider = {
    workspace_name: form.workspaceName.trim(),
    proxy_url: form.proxyUrl.trim(),
    bot: readBotConfig(form, form.workspaceName),
  };

  if (!provider.workspace_name) {
    showError(strings.nameRequired);
    workspaceNameRef.current?.focus();
    return null;
  }
  if (extensionsEnabled && !validateBotAuth(form, strings, showError)) {
    return null;
  }
  return provider;
}

function readNextAiGatewayProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  nameRef: React.RefObject<HTMLInputElement | null>,
  modelRef: React.RefObject<HTMLButtonElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
  extensionsEnabled: boolean,
): NextAiGatewayProvider | null {
  const provider = {
    workspace_name: form.workspaceName.trim(),
    name: form.providerName.trim(),
    model: form.gatewayModel.trim(),
    proxy_url: form.proxyUrl.trim(),
    bot: readBotConfig(form, form.workspaceName),
  };

  if (!provider.workspace_name) {
    showError(strings.nameRequired);
    workspaceNameRef.current?.focus();
    return null;
  }
  if (!provider.name) {
    showError(strings.nameRequired);
    nameRef.current?.focus();
    return null;
  }
  if (!provider.model) {
    showError(strings.modelRequired);
    modelRef.current?.focus();
    return null;
  }
  if (extensionsEnabled && !validateBotAuth(form, strings, showError)) {
    return null;
  }
  return provider;
}

function readExistingProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  providerRef: React.RefObject<HTMLButtonElement | null>,
  modelRef: React.RefObject<HTMLInputElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
  extensionsEnabled: boolean,
): ExistingProvider | null {
  const provider = {
    workspace_name: form.workspaceName.trim(),
    profile_name: form.existingProfileName.trim(),
    base_url: form.existingBaseUrl.trim(),
    api_key: form.existingApiKey.trim(),
    model: form.existingModel.trim(),
    proxy_url: form.proxyUrl.trim(),
    bot: readBotConfig(form, form.workspaceName),
  };

  if (!provider.workspace_name) {
    showError(strings.nameRequired);
    workspaceNameRef.current?.focus();
    return null;
  }
  if (!provider.profile_name) {
    showError(strings.providerRequired);
    providerRef.current?.focus();
    return null;
  }
  if (!provider.model) {
    showError(strings.modelRequired);
    modelRef.current?.focus();
    return null;
  }
  if (extensionsEnabled && !validateBotAuth(form, strings, showError)) {
    return null;
  }
  return provider;
}

function normalizedProfiles(nextConfig: AppConfig): ProviderProfile[] {
  return dedupeProfiles(nextConfig.provider_profiles || []);
}

function mergeSavedBotConfigsIntoProfiles(
  profiles: ProviderProfile[],
  botConfigs: SavedBotConfig[],
): ProviderProfile[] {
  const configs = normalizeSavedBotConfigs(botConfigs);
  if (configs.length === 0) {
    return profiles;
  }
  return profiles.map((profile) => {
    const bot = normalizeBotConfig(profile.bot, profile.name);
    if (!bot.enabled || bot.platform === "none") {
      return profile;
    }
    const matched = configs.find((config) => {
      const configBot = normalizeBotConfig(config.bot, config.name);
      return Boolean(
        (bot.saved_config_id && config.id === bot.saved_config_id) ||
          (bot.integration_id && configBot.integration_id === bot.integration_id),
      );
    });
    if (!matched) {
      return profile;
    }
    const nextBot = normalizeBotConfig(
      {
        ...matched.bot,
        forward_all_codex_messages: bot.forward_all_codex_messages,
        handoff: bot.handoff,
        saved_config_id: matched.id,
        tenant_id: matched.bot.tenant_id || bot.tenant_id,
        integration_id: matched.bot.integration_id || bot.integration_id,
        state_dir: matched.bot.state_dir || bot.state_dir,
      },
      profile.name,
    );
    return {
      ...profile,
      bot: nextBot,
    };
  });
}

function readBotConfig(form: ProviderForm, profileName: string): BotProfileConfig {
  const normalizedProfileName = profileName.trim();
  const platform = form.botEnabled ? form.botPlatform : "none";
  const authType = normalizeBotAuthType(platform, form.botAuthType);
  return {
    enabled: form.botEnabled && platform !== "none",
    platform,
    auth_type: platform === "none" ? "" : authType,
    auth_fields: platform === "none" ? {} : pickBotAuthFields(form.botAuthFields, platform, authType),
    forward_all_codex_messages: form.botEnabled && platform !== "none" && form.botForwardAllCodexMessages,
    handoff: readBotHandoffConfig(form, form.botEnabled && platform !== "none"),
    saved_config_id: platform === "none" ? "" : form.botConfigId.trim(),
    tenant_id: platform === "none" ? "" : form.botTenantId.trim() || normalizedProfileName,
    integration_id: platform === "none" ? "" : form.botIntegrationId.trim(),
    project_dir: "",
    state_dir: platform === "none" ? "" : form.botStateDir.trim(),
    codex_cwd: "",
    status: platform === "none" ? "" : form.botStatus.trim(),
    last_login_at: platform === "none" ? "" : form.botLastLoginAt.trim(),
  };
}

function defaultBotConfig(profileName = ""): BotProfileConfig {
  return {
    enabled: false,
    platform: "none",
    auth_type: "",
    auth_fields: {},
    forward_all_codex_messages: false,
    handoff: defaultBotHandoffConfig(),
    saved_config_id: "",
    tenant_id: profileName,
    integration_id: "",
    project_dir: "",
    state_dir: "",
    codex_cwd: "",
    status: "",
    last_login_at: "",
  };
}

function normalizeBotConfig(bot: Partial<BotProfileConfig> | undefined, profileName: string): BotProfileConfig {
  const fallback = defaultBotConfig(profileName);
  const platform = normalizeBotPlatform(bot?.platform || fallback.platform);
  const enabled = Boolean(bot?.enabled) && platform !== "none";
  const authType = enabled ? normalizeBotAuthType(platform, bot?.auth_type || fallback.auth_type) : "";
  return {
    enabled,
    platform: enabled ? platform : "none",
    auth_type: authType,
    auth_fields: enabled ? pickBotAuthFields(bot?.auth_fields || {}, platform, authType) : {},
    forward_all_codex_messages: enabled ? Boolean(bot?.forward_all_codex_messages) : false,
    handoff: normalizeBotHandoffConfig(bot?.handoff, enabled),
    saved_config_id: enabled ? (bot?.saved_config_id || "").trim() : "",
    tenant_id: (bot?.tenant_id || fallback.tenant_id).trim(),
    integration_id: enabled ? (bot?.integration_id || "").trim() : "",
    project_dir: (bot?.project_dir || fallback.project_dir).trim(),
    state_dir: (bot?.state_dir || "").trim(),
    codex_cwd: (bot?.codex_cwd || "").trim(),
    status: (bot?.status || "").trim(),
    last_login_at: (bot?.last_login_at || "").trim(),
  };
}

function botFormFields(bot: Partial<BotProfileConfig> | undefined, profileName: string) {
  const normalized = normalizeBotConfig(bot, profileName);
  return {
    botEnabled: normalized.enabled,
    botPlatform: normalized.platform as BotPlatform,
    botAuthType: normalizeBotAuthType(normalized.platform, normalized.auth_type),
    botAuthFields: normalized.auth_fields,
    botConfigId: normalized.saved_config_id,
    botTenantId: normalized.tenant_id,
    botIntegrationId: normalized.integration_id,
    botStateDir: normalized.state_dir,
    botStatus: normalized.status,
    botLastLoginAt: normalized.last_login_at,
    botForwardAllCodexMessages: normalized.forward_all_codex_messages,
    botHandoffEnabled: normalized.handoff.enabled,
    botHandoffIdleSeconds: String(normalized.handoff.idle_seconds),
    botHandoffPhoneWifiTargets: normalized.handoff.phone_wifi_targets[0] || "",
    botHandoffPhoneBluetoothTargets: normalized.handoff.phone_bluetooth_targets[0] || "",
  };
}

function botConfigFormFields(config: SavedBotConfig | null): ProviderForm {
  if (!config) {
    const platform: BotPlatform = "weixin-ilink";
    const authType = defaultBotAuthType(platform);
    return {
      ...emptyForm,
      botEnabled: true,
      botPlatform: platform,
      botAuthType: authType,
      botAuthFields: {},
    };
  }

  const name = config.name.trim() || botPlatformLabel(config.bot.platform);
  return {
    ...emptyForm,
    workspaceName: name,
    ...botFormFields(config.bot, name),
    botEnabled: true,
    botConfigId: config.id,
    botForwardAllCodexMessages: false,
    botHandoffEnabled: false,
    botHandoffIdleSeconds: "30",
    botHandoffPhoneWifiTargets: "",
    botHandoffPhoneBluetoothTargets: "",
  };
}

function readSavedBotConfigForm(
  form: ProviderForm,
  existing: SavedBotConfig | null,
  nameRef: React.RefObject<HTMLInputElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
): SavedBotConfig | null {
  const name = form.workspaceName.trim();
  if (!name) {
    showError(strings.nameRequired);
    nameRef.current?.focus();
    return null;
  }
  if (normalizeBotPlatform(form.botPlatform) === "none") {
    showError(strings.selectPlatform);
    return null;
  }
  const botForm: ProviderForm = {
    ...form,
    botEnabled: true,
    botConfigId: existing?.id || form.botConfigId || newLocalId(),
  };
  if (!validateBotAuth(botForm, strings, showError)) {
    return null;
  }

  const id = botForm.botConfigId.trim() || newLocalId();
  const bot = {
    ...readBotConfig(botForm, name),
    enabled: true,
    saved_config_id: id,
    forward_all_codex_messages: false,
    handoff: defaultBotHandoffConfig(),
  };
  return {
    id,
    name,
    bot,
    updated_at: `unix:${Math.floor(Date.now() / 1000)}`,
  };
}

function normalizeSavedBotConfigs(configs: SavedBotConfig[] | undefined): SavedBotConfig[] {
  const seen = new Set<string>();
  const result: SavedBotConfig[] = [];
  for (const config of configs || []) {
    const id = String(config?.id || config?.bot?.saved_config_id || config?.bot?.integration_id || "").trim();
    if (!id || seen.has(id)) {
      continue;
    }
    const bot = normalizeBotConfig(
      {
        ...(config.bot || {}),
        saved_config_id: id,
      },
      config.name || "",
    );
    if (!bot.enabled || bot.platform === "none") {
      continue;
    }
    seen.add(id);
    result.push({
      id,
      name: String(config.name || "").trim() || botPlatformLabel(bot.platform),
      bot,
      updated_at: String(config.updated_at || "").trim(),
    });
  }
  return result.sort((a, b) => botConfigLabel(a).localeCompare(botConfigLabel(b)));
}

function applySavedBotConfig(current: ProviderForm, saved: SavedBotConfig): ProviderForm {
  const bot = normalizeBotConfig(
    {
      ...saved.bot,
      saved_config_id: saved.id || saved.bot.saved_config_id,
    },
    current.workspaceName || saved.name,
  );
  return {
    ...current,
    ...botFormFields(bot, current.workspaceName || saved.name),
    botEnabled: true,
    botConfigId: saved.id || bot.saved_config_id,
    botTenantId: bot.tenant_id,
    botIntegrationId: bot.integration_id,
    botStateDir: bot.state_dir,
    botStatus: bot.status,
    botLastLoginAt: bot.last_login_at,
  };
}

function clearBotConfigSelection(current: ProviderForm): ProviderForm {
  return {
    ...current,
    botConfigId: "",
    botTenantId: "",
    botIntegrationId: "",
    botStateDir: "",
    botStatus: "",
    botLastLoginAt: "",
  };
}

function botConfigLabel(config: SavedBotConfig): string {
  const name = String(config.name || "").trim() || botPlatformLabel(config.bot.platform);
  const platform = botPlatformLabel(config.bot.platform);
  const status = String(config.bot.status || "").trim();
  return [name, platform, status].filter(Boolean).join(" / ");
}

function readBotHandoffConfig(form: ProviderForm, botEnabled: boolean): BotHandoffConfig {
  const idleSeconds = Number.parseInt(form.botHandoffIdleSeconds, 10);
  return normalizeBotHandoffConfig(
    {
      enabled: botEnabled && form.botHandoffEnabled,
      idle_seconds: Number.isFinite(idleSeconds) ? idleSeconds : 30,
      screen_lock: true,
      user_idle: true,
      phone_wifi_targets: selectedTargetList(form.botHandoffPhoneWifiTargets),
      phone_bluetooth_targets: selectedTargetList(form.botHandoffPhoneBluetoothTargets),
    },
    botEnabled,
  );
}

function defaultBotHandoffConfig(): BotHandoffConfig {
  return {
    enabled: false,
    idle_seconds: 30,
    screen_lock: true,
    user_idle: true,
    phone_wifi_targets: [],
    phone_bluetooth_targets: [],
  };
}

function normalizeBotHandoffConfig(
  handoff: Partial<BotHandoffConfig> | undefined,
  botEnabled: boolean,
): BotHandoffConfig {
  const fallback = defaultBotHandoffConfig();
  const rawIdleSeconds = Number(handoff?.idle_seconds ?? fallback.idle_seconds);
  const idleSeconds = Number.isFinite(rawIdleSeconds)
    ? Math.min(86400, Math.max(30, Math.round(rawIdleSeconds)))
    : fallback.idle_seconds;
  return {
    enabled: botEnabled && Boolean(handoff?.enabled),
    idle_seconds: idleSeconds,
    screen_lock: handoff?.screen_lock ?? fallback.screen_lock,
    user_idle: handoff?.user_idle ?? fallback.user_idle,
    phone_wifi_targets: normalizeTargetList(handoff?.phone_wifi_targets).slice(0, 1),
    phone_bluetooth_targets: normalizeTargetList(handoff?.phone_bluetooth_targets).slice(0, 1),
  };
}

function splitTargets(value: string): string[] {
  return normalizeTargetList(value.split(/[,\n]/));
}

function firstTarget(value: string): string {
  return splitTargets(value)[0] || "";
}

function selectedTargetList(value: string): string[] {
  const target = firstTarget(value);
  return target ? [target] : [];
}

function normalizeTargetList(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const seen = new Set<string>();
  const targets: string[] = [];
  for (const item of value) {
    const target = String(item).trim();
    if (!target || seen.has(target)) {
      continue;
    }
    seen.add(target);
    targets.push(target);
  }
  return targets;
}

function normalizeBotPlatform(platform: string): BotPlatform {
  const normalized = platform.trim().toLowerCase();
  switch (normalized) {
    case "":
    case "none":
    case "off":
    case "disabled":
      return "none";
    case "wechat":
    case "weixin":
    case "wx":
    case "weixin-ilink":
    case "weixin_ilink":
    case "ilink":
      return "weixin-ilink";
    case "wecom":
    case "wework":
    case "wechat-work":
    case "work-weixin":
    case "enterprise-wechat":
      return "wecom";
    case "tg":
      return "telegram";
    case "lark":
      return "feishu";
    case "dingding":
      return "dingtalk";
    default:
      return BOT_PLATFORM_OPTIONS.some((option) => option.value === normalized)
        ? (normalized as BotPlatform)
        : "none";
  }
}

function authSpecsForPlatform(platform: BotPlatform | string): readonly BotAuthSpec[] {
  const normalized = normalizeBotPlatform(platform);
  if (normalized === "none") {
    return [];
  }
  return BOT_PLATFORM_SPECS.find((option) => option.value === normalized)?.auth || [];
}

function defaultBotAuthType(platform: BotPlatform | string): BotAuthType {
  return authSpecsForPlatform(platform)[0]?.value || "qr_login";
}

function normalizeBotAuthType(platform: BotPlatform | string, authType: string): BotAuthType {
  const normalized = authType.trim().toLowerCase().replace(/-/g, "_");
  const aliases: Record<string, BotAuthType> = {
    appsecret: "app_secret",
    app_secret: "app_secret",
    bottoken: "bot_token",
    bot_token: "bot_token",
    oauth: "oauth2",
    oauth2: "oauth2",
    oauth_2: "oauth2",
    qr: "qr_login",
    qr_code: "qr_login",
    qr_login: "qr_login",
    qrcode: "qr_login",
    token: "bot_token",
    webhook: "webhook_secret",
    webhook_secret: "webhook_secret",
  };
  const value = aliases[normalized] || defaultBotAuthType(platform);
  return authSpecsForPlatform(platform).some((option) => option.value === value)
    ? value
    : defaultBotAuthType(platform);
}

function fieldsForBotAuth(platform: BotPlatform | string, authType: string): readonly BotAuthFieldSpec[] {
  const normalizedAuthType = normalizeBotAuthType(platform, authType);
  return authSpecsForPlatform(platform).find((option) => option.value === normalizedAuthType)?.fields || [];
}

function pickBotAuthFields(
  fields: Partial<Record<string, string>> | undefined,
  platform: BotPlatform | string,
  authType: string,
): Record<string, string> {
  const allowedKeys = new Set(fieldsForBotAuth(platform, authType).map((field) => field.key));
  if (allowedKeys.size === 0) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(fields || {})
      .map(([key, value]) => [key.trim(), String(value ?? "").trim()] as const)
      .filter(([key, value]) => allowedKeys.has(key) && value.length > 0),
  );
}

function validateBotAuth(
  form: ProviderForm,
  strings: AppStrings,
  showError: (error: unknown) => void,
): boolean {
  if (!form.botEnabled || form.botPlatform === "none") {
    return true;
  }
  const authType = normalizeBotAuthType(form.botPlatform, form.botAuthType);
  const missing = fieldsForBotAuth(form.botPlatform, authType).filter(
    (field) => field.required && !form.botAuthFields[field.key]?.trim(),
  );
  if (missing.length === 0) {
    return true;
  }
  showError(strings.botAuthRequired(missing.map((field) => field.label).join(", ")));
  window.requestAnimationFrame(() => {
    document.getElementById(`botAuthField-${missing[0]?.key}`)?.focus();
  });
  return false;
}

function isQrLoginBot(bot: BotProfileConfig | null): boolean {
  return Boolean(
    bot?.enabled &&
      normalizeBotPlatform(bot.platform) === "weixin-ilink" &&
      normalizeBotAuthType(bot.platform, bot.auth_type || "") === "qr_login",
  );
}

function shouldStartQrLogin(bot: BotProfileConfig | null): boolean {
  return isQrLoginBot(bot) && !hasReusableBotConfig(bot);
}

function hasReusableBotConfig(bot: BotProfileConfig | null): boolean {
  return Boolean(bot?.saved_config_id?.trim() && bot?.integration_id?.trim() && bot?.state_dir?.trim());
}

function isStaticAuthBot(bot: BotProfileConfig | null): boolean {
  return Boolean(
    bot?.enabled &&
      normalizeBotPlatform(bot.platform) !== "none" &&
      normalizeBotAuthType(bot.platform, bot.auth_type || "") !== "qr_login",
  );
}

async function prepareBotPluginIfNeeded(bot: BotProfileConfig | null): Promise<void> {
  if (!bot?.enabled || normalizeBotPlatform(bot.platform) === "none") {
    return;
  }
  await invoke("prepare_builtin_extension", { extensionId: "bot-gateway" });
}

async function prepareNextAiGatewayPlugin(): Promise<void> {
  await invoke("prepare_builtin_extension", { extensionId: "next-ai-gateway" });
}

function botPlatformLabel(platform: string): string {
  const normalized = normalizeBotPlatform(platform);
  if (normalized === "none") {
    return "Bot";
  }
  return BOT_PLATFORM_OPTIONS.find((option) => option.value === normalized)?.label || "Bot";
}

function botAuthTypeLabel(platform: string, authType: string): string {
  const normalized = normalizeBotAuthType(platform, authType);
  return authSpecsForPlatform(platform).find((option) => option.value === normalized)?.label || normalized;
}

function associatedWorkspaceProfiles(config: SavedBotConfig, profiles: ProviderProfile[]): ProviderProfile[] {
  const configBot = normalizeBotConfig(config.bot, config.name);
  const savedConfigId = config.id.trim() || configBot.saved_config_id.trim();
  const integrationId = configBot.integration_id.trim();
  return profiles.filter((profile) => {
    const profileBot = normalizeBotConfig(profile.bot, profile.name);
    if (!profileBot.enabled || profileBot.platform === "none") {
      return false;
    }
    return Boolean(
      (savedConfigId && profileBot.saved_config_id === savedConfigId) ||
        (integrationId && profileBot.integration_id === integrationId),
    );
  });
}

function associatedWorkspaceTextFromProfiles(profiles: ProviderProfile[], fallback: string): string {
  const names = profiles.map((profile) => profile.name);
  return names.length > 0 ? names.join(", ") : fallback;
}

function extensionDescription(extension: BuiltinExtensionStatus, strings: AppStrings): string {
  if (extension.id === "bot-gateway") {
    return strings.botGatewayDescription;
  }
  if (extension.id === "next-ai-gateway") {
    return strings.nextAiGatewayDescription;
  }
  return extension.description;
}

function gatewayFormFromConfig(file: GatewayConfigFile): GatewayConfigForm {
  const config = file.config || {};
  return {
    host: stringValue(config.host, "127.0.0.1"),
    port: numberString(config.port, "14589"),
    providers: arrayValue(config.Providers ?? config.providers).map((item) =>
      gatewayProviderFormFromRaw(objectValue(item)),
    ),
    rawConfig: config,
  };
}

function gatewayModelsFromConfig(config: JsonObject): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  for (const item of arrayValue(config.Providers ?? config.providers)) {
    const provider = objectValue(item);
    const providerName = stringValue(provider.name, "").trim();
    for (const model of gatewayProviderModels(provider)) {
      const option = gatewayModelOption(providerName, model);
      if (option && !seen.has(option)) {
        seen.add(option);
        models.push(option);
      }
    }
  }
  return models;
}

function gatewayModelOption(providerName: string, modelName: string): string {
  const provider = providerName.trim();
  const model = modelName.trim().replace(/^\/+/, "");
  if (!provider || !model) {
    return "";
  }
  return model.startsWith(`${provider}/`) ? model : `${provider}/${model}`;
}

function gatewayProviderModels(provider: JsonObject): string[] {
  const rawModels = provider.models;
  if (Array.isArray(rawModels)) {
    return rawModels.map(gatewayModelName).filter(Boolean);
  }
  if (typeof rawModels === "string") {
    return commaList(rawModels);
  }
  return [];
}

function gatewayModelName(item: unknown): string {
  if (typeof item === "string") {
    return item.trim();
  }
  const model = objectValue(item);
  return stringValue(model.name ?? model.id ?? model.model, "").trim();
}

function gatewayProviderFormFromRaw(raw: JsonObject): GatewayProviderForm {
  return {
    id: newLocalId(),
    name: stringValue(raw.name, ""),
    type: stringValue(raw.type ?? raw.provider, "openai_responses"),
    apiKey: stringValue(raw.apikey ?? raw.apiKey, ""),
    baseUrl: stringValue(raw.baseurl ?? raw.baseUrl, ""),
    models: gatewayProviderModels(raw).join(", "),
    raw,
  };
}

function createGatewayProviderForm(): GatewayProviderForm {
  return {
    id: newLocalId(),
    name: "",
    type: "openai_responses",
    apiKey: "",
    baseUrl: "https://api.openai.com/v1",
    models: "",
    raw: {},
  };
}

function cloneGatewayProviderForm(provider: GatewayProviderForm): GatewayProviderForm {
  return {
    ...provider,
    raw: cloneJsonObject(provider.raw),
  };
}

function gatewayConfigFromForm(form: GatewayConfigForm): JsonObject {
  const config = cloneJsonObject(form.rawConfig);
  config.host = form.host.trim() || "127.0.0.1";
  config.port = integerValue(form.port, 14589);
  config.bodyLimitBytes = 52428800;
  delete config.bodyLimit;
  delete config.providers;
  config.Providers = form.providers.map(gatewayProviderConfigFromForm);

  return config;
}

function gatewayProviderConfigFromForm(provider: GatewayProviderForm): JsonObject {
  const raw = cloneJsonObject(provider.raw);
  raw.name = provider.name.trim();
  raw.type = provider.type;
  raw.apikey = provider.apiKey.trim();
  raw.baseurl = provider.baseUrl.trim();
  raw.models = commaList(provider.models);
  delete raw.apiKey;
  delete raw.baseUrl;
  delete raw.provider;
  return raw;
}

function objectValue(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonObject) : {};
}

function arrayValue(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function stringValue(value: unknown, fallback: string): string {
  return typeof value === "string" ? value : fallback;
}

function numberString(value: unknown, fallback: string): string {
  return typeof value === "number" && Number.isFinite(value) ? String(value) : fallback;
}

function integerValue(value: unknown, fallback: number): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.trunc(value);
  }
  const parsed = Number.parseInt(String(value || ""), 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function commaList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function cloneJsonObject(value: JsonObject): JsonObject {
  return JSON.parse(JSON.stringify(value || {})) as JsonObject;
}

function newLocalId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function dedupeProfiles(profiles: ProviderProfile[]) {
  const seen = new Set<string>();
  const result: ProviderProfile[] = [];
  for (const profile of profiles) {
    const name = profile.name.trim();
    if (!name || seen.has(name)) continue;
    seen.add(name);
    result.push({
      id: profile.id || "",
      name,
      codex_profile_name: (profile.codex_profile_name || "").trim(),
      provider_name: (profile.provider_name || "").trim(),
      base_url: profile.base_url.trim(),
      model: profile.model.trim(),
      proxy_url: (profile.proxy_url || "").trim(),
      codex_home: profile.codex_home.trim(),
      start_remote_on_launch: Boolean(profile.start_remote_on_launch),
      start_remote_cloud_on_launch:
        Boolean(profile.start_remote_on_launch) && Boolean(profile.start_remote_cloud_on_launch),
      start_remote_e2ee_on_launch:
        Boolean(profile.start_remote_on_launch) &&
        Boolean(profile.start_remote_cloud_on_launch),
      bot: normalizeBotConfig(profile.bot, name),
    });
  }
  return result;
}

function isProviderlessWorkspace(profile: ProviderProfile) {
  return !profile.provider_name.trim() && !profile.model.trim();
}

function selectProviderForProfile(profile: ProviderProfile, providers: DefaultProviderProfile[]) {
  const exactMatch = providers.find((item) => item.name === (profile.codex_profile_name || profile.name));
  const providerMatch = providers.find((item) => item.provider_name === profile.provider_name);
  const fallback = providers[0];
  return exactMatch || providerMatch || fallback;
}

function errorMessage(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const digits = unitIndex === 0 || value >= 10 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

function normalizeLanguage(value: unknown): Language {
  const language = String(value || "").trim().toLowerCase();
  return language === "zh" || language === "zh-cn" ? "zh" : "en";
}

function normalizeAppearance(value: unknown): Appearance {
  const appearance = String(value || "").trim().toLowerCase();
  if (appearance === "light" || appearance === "dark") {
    return appearance;
  }
  return "system";
}

function normalizeExtensionSettings(value: Partial<ExtensionSettings> | undefined | null): ExtensionSettings {
  const raw = value as
    | (Partial<ExtensionSettings> & { botGatewayEnabled?: boolean; nextAiGatewayEnabled?: boolean })
    | undefined
    | null;
  return {
    enabled: Boolean(value?.enabled),
    bot_gateway_enabled: raw?.bot_gateway_enabled ?? raw?.botGatewayEnabled ?? false,
    next_ai_gateway_enabled: raw?.next_ai_gateway_enabled ?? raw?.nextAiGatewayEnabled ?? false,
  };
}

function botExtensionsEnabled(value: Partial<ExtensionSettings> | undefined | null): boolean {
  const settings = normalizeExtensionSettings(value);
  return settings.enabled && settings.bot_gateway_enabled;
}

function nextAiGatewayEnabled(value: Partial<ExtensionSettings> | undefined | null): boolean {
  const settings = normalizeExtensionSettings(value);
  return settings.enabled && settings.next_ai_gateway_enabled;
}

function extensionEnabledSetting(settings: ExtensionSettings, extensionId: string): boolean {
  if (extensionId === "bot-gateway") {
    return settings.bot_gateway_enabled;
  }
  if (extensionId === "next-ai-gateway") {
    return settings.next_ai_gateway_enabled;
  }
  return false;
}

function setExtensionEnabledSetting(
  settings: ExtensionSettings,
  extensionId: string,
  enabled: boolean,
): ExtensionSettings {
  if (extensionId === "bot-gateway") {
    return { ...settings, bot_gateway_enabled: enabled };
  }
  if (extensionId === "next-ai-gateway") {
    return { ...settings, next_ai_gateway_enabled: enabled };
  }
  return settings;
}

function normalizeQrDisplay(raw: string): QrDisplay {
  const value = raw.trim();
  if (!value) return { kind: "empty", src: "" };
  if (value.startsWith("http://") || value.startsWith("https://")) {
    return { kind: "webview", src: value };
  }
  if (value.startsWith("data:")) {
    return { kind: "image", src: value };
  }
  if (value.startsWith("<svg")) {
    return { kind: "image", src: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(value)}` };
  }
  return { kind: "image", src: `data:image/png;base64,${value}` };
}

async function openQrWebview(login: WeixinBotQrState) {
  if (login.qrDisplay.kind !== "webview") {
    return;
  }
  const label = qrWebviewLabel(login.sessionId);
  try {
    const existing = await WebviewWindow.getByLabel(label);
    if (existing) {
      qrWebviewWindows.set(label, existing);
      await existing.show();
      await existing.setFocus();
      return;
    }

    const webview = new WebviewWindow(label, {
      url: login.qrDisplay.src,
      title: `Weixin Login - ${login.profileName}`,
      width: 430,
      height: 720,
      minWidth: 360,
      minHeight: 560,
      center: true,
      resizable: true,
      focus: true,
    });
    await new Promise<void>((resolve, reject) => {
      webview.once("tauri://created", () => resolve()).catch(reject);
      webview.once("tauri://error", (event) => reject(event.payload)).catch(reject);
    });
    qrWebviewWindows.set(label, webview);
  } catch (error) {
    const fallback = window.open(login.qrDisplay.src, "_blank", "noopener,noreferrer");
    if (!fallback) {
      throw error;
    }
  }
}

async function closeQrWebview(sessionId: string) {
  const label = qrWebviewLabel(sessionId);
  const tracked = qrWebviewWindows.get(label);
  if (tracked) {
    qrWebviewWindows.delete(label);
    await tracked.close().catch(() => undefined);
  }

  try {
    const existing = await WebviewWindow.getByLabel(label);
    await existing?.close();
  } catch {
    // The QR window may already be closed, or this may be running in browser dev mode.
  }
}

function qrWebviewLabel(sessionId: string) {
  const safe = sessionId.replace(/[^a-zA-Z0-9_:-]/g, "-");
  return `weixin-bot-qr-${safe}`;
}

const qrWebviewWindows = new Map<string, WebviewWindow>();

function isTerminalBotLoginStatus(status: string): boolean {
  return ["confirmed", "expired", "already_bound", "failed"].includes(status);
}

function botLoginStatusLabel(status: string, strings: AppStrings): string {
  switch (status) {
    case "confirmed":
      return strings.connected;
    case "scanned":
      return strings.scanned;
    case "expired":
      return strings.expired;
    case "already_bound":
      return strings.alreadyBound;
    case "failed":
      return strings.failed;
    default:
      return strings.waiting;
  }
}

export default App;
