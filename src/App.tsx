import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useTranslation } from "react-i18next";
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  XAxis,
  YAxis,
} from "recharts";
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
  FileCog,
  FolderOpen,
  Globe,
  ImageIcon,
  Languages,
  LayoutDashboard,
  LockKeyhole,
  LogOut,
  Maximize2,
  MessageCircle,
  Mic,
  Minimize2,
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
  Terminal,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AlertDialog,
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
const DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL = "https://web.codexl.io";
const DEFAULT_CODEX_WEB_ASSET_VERSION = "latest";
const DEFAULT_TRANSCRIBE_MODEL = "gpt-4o-mini-transcribe";

type RemoteFrontendMode = "app" | "cli" | "claude-code";

type ProviderProfile = {
  id: string;
  name: string;
  codex_profile_name: string;
  provider_name: string;
  provider_config_format: string;
  base_url: string;
  model: string;
  proxy_url: string;
  remote_frontend_mode: RemoteFrontendMode | string;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
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
  subscription_expires_at: number;
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

type DesktopAuthRefreshResponse =
  | {
      status: "refreshed";
      user: DesktopAuthUser;
      cloudAuth: DesktopCloudAuth;
      relayUrl?: string | null;
      relay_url?: string | null;
      remoteRelayUrl?: string | null;
    }
  | { status: "invalid" | "unavailable"; error?: string };

type AccountLoginState = "idle" | "polling";

type AppConfig = {
  cdp_host: string;
  cdp_port: number;
  http_host: string;
  http_port: number;
  remote_control_host: string;
  remote_control_port: number;
  remote_relay_url: string;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
  remote_transcribe_base_url?: string;
  remote_transcribe_api_url: string;
  remote_transcribe_api_key: string;
  remote_transcribe_model: string;
  device_uuid: string;
  remote_cloud_auth: RemoteCloudAuthConfig;
  remote_control_tokens?: Record<string, string>;
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
  core_mode: RemoteFrontendMode | string;
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
  web_asset_mode: string;
  web_asset_base_url: string | null;
  web_asset_version: string;
  cdp_host: string;
  cdp_port: number;
  cdp_ready: boolean;
  control_client_count: number;
  frame_client_count: number;
};

type CodexWebAssetVersions = {
  latest: string;
  versions: string[];
};

type ProviderModelsProbeResponse = {
  models: string[];
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
  remote_frontend_mode: RemoteFrontendMode;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
  bot: BotProfileConfig;
};

type DefaultProviderProfile = {
  name: string;
  provider_name: string;
  base_url: string;
  api_key: string;
  model: string;
  config_format?: string;
};

type ExistingProvider = {
  workspace_name: string;
  profile_name: string;
  base_url: string;
  api_key: string;
  model: string;
  proxy_url: string;
  remote_frontend_mode: RemoteFrontendMode;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
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
  remote_frontend_mode: RemoteFrontendMode;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
  bot: BotProfileConfig;
};

type UpdateNextAiGatewayProvider = NextAiGatewayProvider & {
  original_name: string;
};

type WorkspaceProvider = {
  workspace_name: string;
  proxy_url: string;
  remote_frontend_mode: RemoteFrontendMode;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
  bot: BotProfileConfig;
};

type UpdateWorkspaceProvider = WorkspaceProvider & {
  original_name: string;
};

type ProviderMode = "none" | "existing" | "new" | "gateway";
type DialogMode = "add" | "edit";
type AppSettingsSection =
  | "general"
  | "profiles"
  | "transcribe"
  | "extensions"
  | "bot"
  | "gateway"
  | "usage"
  | "updates";
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
type GatewayUsageTotals = {
  requestCount: number;
  successCount: number;
  errorCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  lastReceivedAtUnix: number | null;
};
type GatewayUsageDaily = {
  day: string;
  requestCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
};
type GatewayUsageBreakdown = {
  label: string;
  provider: string;
  providerName: string;
  model: string;
  requestCount: number;
  successCount: number;
  errorCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
};
type GatewayUsageSessionBreakdown = {
  sessionId: string;
  label: string;
  projectPath: string;
  projectLabel: string;
  requestCount: number;
  successCount: number;
  errorCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  firstReceivedAtUnix: number | null;
  lastReceivedAtUnix: number | null;
};
type GatewayUsageProjectBreakdown = {
  projectPath: string;
  label: string;
  sessionCount: number;
  requestCount: number;
  successCount: number;
  errorCount: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  firstReceivedAtUnix: number | null;
  lastReceivedAtUnix: number | null;
};
type GatewayUsageRequestEvent = {
  eventId: string;
  requestId: string;
  emittedAt: string;
  receivedAtUnix: number;
  clientSessionId: string;
  clientSessionLabel: string;
  clientProjectPath: string;
  clientProjectLabel: string;
  route: string;
  provider: string;
  providerName: string;
  model: string;
  status: string;
  statusCode: number | null;
  latencyMs: number | null;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
};
type GatewayUsageSummary = {
  databasePath: string;
  windowDays: number;
  windowHours: number;
  startDate: string;
  endDate: string;
  generatedAtUnix: number;
  totals: GatewayUsageTotals;
  daily: GatewayUsageDaily[];
  byProvider: GatewayUsageBreakdown[];
  byModel: GatewayUsageBreakdown[];
  bySession: GatewayUsageSessionBreakdown[];
  byProject: GatewayUsageProjectBreakdown[];
  requests: GatewayUsageRequestEvent[];
};
type GatewayUsageBreakdownMode = "model" | "session" | "project";
type GatewayUsageViewMode = "overview" | "details";
type GatewayUsageOverviewMode = "daily" | "weekly" | "cumulative";
type GatewayUsageHeatmapTooltipState = {
  placement: "above" | "below";
  text: string;
  x: number;
  y: number;
};
type GatewayUsageDateRange = {
  startDate: string;
  endDate: string;
  hours?: number;
};
type GatewayProviderForm = {
  id: string;
  name: string;
  type: string;
  apiKey: string;
  baseUrl: string;
  models: string;
  thinkingEffortModels: string[];
  raw: JsonObject;
};
type GatewayMcpServerTransport = "stdio" | "websocket";
type GatewayMcpServerStdioMessageMode = "newline-json" | "content-length";
type GatewayMcpServerForm = {
  id: string;
  name: string;
  enabled: boolean;
  transport: GatewayMcpServerTransport;
  stdioMessageMode: GatewayMcpServerStdioMessageMode;
  command: string;
  args: string;
  cwd: string;
  url: string;
  headersJson: string;
  envJson: string;
  apiKey: string;
  apiKeyEnv: string;
  protocolVersion: string;
  startupTimeoutMs: string;
  requestTimeoutMs: string;
  raw: JsonObject;
};
type GatewayAvailableTool = {
  name: string;
  description: string;
  inputSchema?: JsonObject;
};
type GatewayToolsResponse = {
  tools?: unknown[];
};
type GatewayVirtualToolVisibility = "internal" | "client";
type GatewayVirtualBaseModelMode = "request" | "fixed" | "strip_prefix" | "strip_suffix";
type GatewayVirtualToolForm = {
  id: string;
  name: string;
  description: string;
  visibility: GatewayVirtualToolVisibility;
  inputSchemaJson: string;
  raw: JsonObject;
};
type GatewayVirtualProfileForm = {
  id: string;
  profileId: string;
  key: string;
  displayName: string;
  description: string;
  enabled: boolean;
  exactAliases: string;
  prefixes: string;
  suffixes: string;
  baseModelMode: GatewayVirtualBaseModelMode;
  fixedModel: string;
  matchMultimodal: boolean;
  matchWebSearch: boolean;
  maxTurns: string;
  maxToolCalls: string;
  clientToolsPolicy: "allow" | "deny";
  includeInGatewayModels: boolean;
  tools: GatewayVirtualToolForm[];
  raw: JsonObject;
};
type GatewayConfigForm = {
  host: string;
  port: string;
  usageCaptureEnabled: boolean;
  requestLoggingEnabled: boolean;
  providers: GatewayProviderForm[];
  mcpServers: GatewayMcpServerForm[];
  virtualModelProfiles: GatewayVirtualProfileForm[];
  rawConfig: JsonObject;
};
type GatewayProviderDialogState = {
  mode: "add" | "edit";
  provider: GatewayProviderForm;
  initialSignature: string;
};
type GatewayMcpServerDialogState = {
  mode: "add" | "edit";
  server: GatewayMcpServerForm;
  initialSignature: string;
};
type GatewayVirtualProfileDialogState = {
  mode: "add" | "edit";
  profile: GatewayVirtualProfileForm;
  initialSignature: string;
};
type DefaultProviderDialogState = {
  mode: "add" | "edit";
  profile: DefaultProviderProfile;
  initialSignature: string;
};
type GatewaySettingsTab = "settings" | "providers" | "mcp" | "tools";

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
  remoteFrontendMode: RemoteFrontendMode;
  remoteWebAssetRegistryUrl: string;
  remoteWebAssetVersion: string;
  remoteWebAssetVersions: string[];
  remoteWebAssetVersionsLoading: boolean;
  remoteWebAssetRegistryError: string;
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
  defaultUrlKind: RemoteQrUrlKind;
};

type RemoteQrUrlKind = "remote" | "lan";

type RemoteQrUrlOption = {
  kind: RemoteQrUrlKind;
  url: string;
};

type RemoteLaunchOptions = {
  startRemote: boolean;
  startCloud: boolean;
};

type WorkspaceOperationKind = "start" | "stop" | "options";

type WorkspaceOperation = {
  key: string;
  kind: WorkspaceOperationKind;
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
    loadingWorkspacesTitle: t("search.loadingTitle"),
    loadingWorkspacesDescription: t("search.loadingDescription"),
    newInstance: t("actions.newInstance"),
    settings: t("settings.settings"),
    noInstancesTitle: t("search.emptyTitle"),
    noInstancesDescription: t("search.emptyDescription"),
    emptyCreateTitle: t("search.emptyCreateTitle"),
    emptyCreateDescription: t("search.emptyCreateDescription"),
    emptyProfilesTitle: t("search.emptyProfilesTitle"),
    emptyProfilesDescription: t("search.emptyProfilesDescription"),
    emptyGatewayTitle: t("search.emptyGatewayTitle"),
    emptyGatewayDescription: t("search.emptyGatewayDescription"),
    desktopRuntimeUnavailableTitle: t("errors.desktopRuntimeUnavailableTitle"),
    desktopRuntimeUnavailableDescription: t("errors.desktopRuntimeUnavailableDescription"),
    createInstance: t("actions.createInstance"),
    clearSearch: t("actions.clearSearch"),
    downloadUpdate: t("actions.downloadUpdate"),
    revealInFileExplorer: t("tooltips.revealInFileExplorer"),
    settingsTooltip: t("tooltips.settings"),
    downloadUpdateTooltip: t("tooltips.downloadUpdate"),
    showRemoteQr: t("remote.showQr"),
    editProfile: (name: string) => t("actions.editProfile", { name }),
    deleteProfile: (name: string) => t("actions.deleteProfile", { name }),
    stop: t("actions.stop"),
    stopping: t("actions.stopping"),
    start: t("actions.start"),
    starting: t("actions.starting"),
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
    discardSettingsChangesDescription: t("settings.discardChangesDescription"),
    discardSettingsChangesTitle: t("settings.discardChangesTitle"),
    general: t("settings.general"),
    profiles: t("settings.profiles"),
    manageProfiles: t("settings.manageProfiles"),
    profileSettingsDescription: t("settings.profileSettingsDescription"),
    addProfileConfig: t("settings.addProfileConfig"),
    editProfileConfig: t("settings.editProfileConfig"),
    noProfileConfigs: t("settings.noProfileConfigs"),
    profileUsedByWorkspace: (workspace: string) => t("settings.profileUsedByWorkspace", { workspace }),
    extensions: t("settings.extensions"),
    transcribe: t("settings.transcribe"),
    transcribeSettingsDescription: t("settings.transcribeSettingsDescription"),
    transcribeApiUrl: t("settings.transcribeApiUrl"),
    transcribeApiUrlDescription: t("settings.transcribeApiUrlDescription"),
    transcribeApiKey: t("settings.transcribeApiKey"),
    transcribeApiKeyDescription: t("settings.transcribeApiKeyDescription"),
    transcribeModel: t("settings.transcribeModel"),
    transcribeModelDescription: t("settings.transcribeModelDescription"),
    invalidTranscribeApiUrl: t("settings.invalidTranscribeApiUrl"),
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
    gatewayUsage: t("gateway.usage"),
    gatewayUsageSettingsDescription: t("gateway.usageSettingsDescription"),
    gatewayUsageCapture: t("gateway.usageCapture"),
    gatewayUsageCaptureDescription: t("gateway.usageCaptureDescription"),
    gatewayRequestLogging: t("gateway.requestLogging"),
    gatewayRequestLoggingDescription: t("gateway.requestLoggingDescription"),
    gatewayUsageDashboard: t("gateway.usageDashboard"),
    gatewayUsageOverview: t("gateway.usageOverview"),
    gatewayUsageDetails: t("gateway.usageDetails"),
    gatewayUsageDateRange: t("gateway.usageDateRange"),
    gatewayUsageEnterFullscreen: t("gateway.usageEnterFullscreen"),
    gatewayUsageExitFullscreen: t("gateway.usageExitFullscreen"),
    gatewayUsageLast24Hours: t("gateway.usageLast24Hours"),
    gatewayUsageLast7Days: t("gateway.usageLast7Days"),
    gatewayUsageLast30Days: t("gateway.usageLast30Days"),
    gatewayUsageLast90Days: t("gateway.usageLast90Days"),
    gatewayUsageStartDate: t("gateway.usageStartDate"),
    gatewayUsageEndDate: t("gateway.usageEndDate"),
    gatewayUsageRequests: t("gateway.usageRequests"),
    gatewayUsageSuccessRate: t("gateway.usageSuccessRate"),
    gatewayUsageTokens: t("gateway.usageTokens"),
    gatewayUsageTotal: t("gateway.usageTotal"),
    gatewayUsageCache: t("gateway.usageCache"),
    gatewayUsageCacheRate: t("gateway.usageCacheRate"),
    gatewayUsageCacheRead: t("gateway.usageCacheRead"),
    gatewayUsageCacheWrite: t("gateway.usageCacheWrite"),
    gatewayUsageTime: t("gateway.usageTime"),
    gatewayUsageLatency: t("gateway.usageLatency"),
    gatewayUsageDaily: t("gateway.usageDaily"),
    gatewayUsageByProvider: t("gateway.usageByProvider"),
    gatewayUsageByModel: t("gateway.usageByModel"),
    gatewayUsageBySession: t("gateway.usageBySession"),
    gatewayUsageByProject: t("gateway.usageByProject"),
    gatewayUsageGroupBy: t("gateway.usageGroupBy"),
    gatewayUsageRequestList: t("gateway.usageRequestList"),
    gatewayUsageSession: t("gateway.usageSession"),
    gatewayUsageProject: t("gateway.usageProject"),
    gatewayUsageUnknownSession: t("gateway.usageUnknownSession"),
    gatewayUsageUnknownProject: t("gateway.usageUnknownProject"),
    gatewayUsageFirstSeen: t("gateway.usageFirstSeen"),
    gatewayUsageLastSeen: t("gateway.usageLastSeen"),
    gatewayUsageNoData: t("gateway.usageNoData"),
    gatewayUsageDatabase: t("gateway.usageDatabase"),
    gatewayUsageInput: t("gateway.usageInput"),
    gatewayUsageOutput: t("gateway.usageOutput"),
    gatewayUsageRefresh: t("gateway.usageRefresh"),
    gatewayUsageLifetimeTokens: t("gateway.usageLifetimeTokens"),
    gatewayUsagePeakTokens: t("gateway.usagePeakTokens"),
    gatewayUsageLongestTask: t("gateway.usageLongestTask"),
    gatewayUsageCurrentStreak: t("gateway.usageCurrentStreak"),
    gatewayUsageLongestStreak: t("gateway.usageLongestStreak"),
    gatewayUsageTokenActivity: t("gateway.usageTokenActivity"),
    gatewayUsageDailyMode: t("gateway.usageDailyMode"),
    gatewayUsageWeeklyMode: t("gateway.usageWeeklyMode"),
    gatewayUsageCumulativeMode: t("gateway.usageCumulativeMode"),
    gatewayUsageDayUnit: t("gateway.usageDayUnit"),
    botSettingsDescription: t("bot.settingsDescription"),
    addBot: t("bot.addBot"),
    associatedWorkspace: t("bot.associatedWorkspace"),
    botLinkedToWorkspace: t("bot.linkedToWorkspace"),
    deleteBot: t("bot.deleteBot"),
    deleteBotConfirm: (name: string) => t("bot.deleteBotConfirm", { name }),
    editBot: t("bot.editBot"),
    noSavedBots: t("bot.noSavedBots"),
    notConfigured: t("bot.notConfigured"),
    status: t("bot.status"),
    updates: t("settings.updates"),
    updatesDescription: t("settings.updatesDescription"),
    extensionSettingsDescription: t("settings.extensionSettingsDescription"),
    enableExtensions: t("settings.enableExtensions"),
    configureExtensions: t("settings.configureExtensions"),
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
    deleting: t("actions.deleting"),
    discardChanges: t("actions.discardChanges"),
    manage: t("actions.manage"),
    createProfile: t("actions.createProfile"),
    newProfile: t("instanceDialog.newProfile"),
    configureInstance: t("instanceDialog.configure"),
    fromDefault: t("instanceDialog.fromDefault"),
    nextAiGatewayProvider: t("instanceDialog.nextAiGatewayProvider"),
    thirdPartyProvider: t("instanceDialog.thirdPartyProvider"),
    newProvider: t("instanceDialog.newProvider"),
    providerSource: t("instanceDialog.providerSource"),
    providerSourceNone: t("instanceDialog.providerSourceNone"),
    providerSourceNoneDescription: t("instanceDialog.providerSourceNoneDescription"),
    providerSourceDefault: t("instanceDialog.providerSourceDefault"),
    providerSourceDefaultDescription: t("instanceDialog.providerSourceDefaultDescription"),
    providerSourceGateway: t("instanceDialog.providerSourceGateway"),
    providerSourceGatewayDescription: t("instanceDialog.providerSourceGatewayDescription"),
    providerSourceDefaultUnavailable: t("instanceDialog.providerSourceDefaultUnavailable"),
    providerSourceGatewayDefaultUnavailable: t("instanceDialog.providerSourceGatewayDefaultUnavailable"),
    providerlessWorkspace: t("instanceDialog.providerlessWorkspace"),
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
    remoteFrontendMode: t("instanceDialog.remoteFrontendMode"),
    remoteFrontendApp: t("instanceDialog.remoteFrontendApp"),
    remoteFrontendCli: t("instanceDialog.remoteFrontendCli"),
    remoteFrontendClaudeCode: t("instanceDialog.remoteFrontendClaudeCode"),
    codexAppDetected: (path: string) => t("instanceDialog.codexAppDetected", { path }),
    codexAppNotFound: t("instanceDialog.codexAppNotFound"),
    registryUrl: t("instanceDialog.registryUrl"),
    registryVersion: t("instanceDialog.registryVersion"),
    loadingVersions: t("instanceDialog.loadingVersions"),
    providerProfileName: t("instanceDialog.providerProfileName"),
    bot: t("bot.title"),
    authMethod: t("bot.authMethod"),
    savedBotConfig: t("bot.savedConfig"),
    customBotConfig: t("bot.customConfig"),
    enableBotIntegration: t("bot.enableIntegration"),
    botOptionsDescription: t("bot.optionsDescription"),
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
    deleteProfileConfig: t("settings.deleteProfileConfig"),
    deleteProfileConfigConfirm: (name: string) => t("settings.deleteProfileConfigConfirm", { name }),
    remoteQr: t("remote.remoteQr"),
    remoteQrStartRequired: t("remote.remoteQrStartRequired"),
    remoteQrUnavailable: t("remote.remoteQrUnavailable"),
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
    pro: t("account.pro"),
    proExpiresAt: (date: string) => t("account.proExpiresAt", { date }),
    loginFailed: t("account.loginFailed"),
    loginExpired: t("account.loginExpired"),
    sessionExpired: t("account.sessionExpired"),
    refreshFailed: t("account.refreshFailed"),
    scanQrInWeixin: t("bot.scanQrInWeixin"),
    noProviderFound: t("errors.noProviderFound"),
    clipboardUnavailable: t("errors.clipboardUnavailable"),
    nameRequired: t("errors.nameRequired"),
    baseUrlRequired: t("errors.baseUrlRequired"),
    apiKeyRequired: t("errors.apiKeyRequired"),
    modelRequired: t("errors.modelRequired"),
    providerRequired: t("errors.providerRequired"),
    registryUrlRequired: t("errors.registryUrlRequired"),
    registryVersionRequired: t("errors.registryVersionRequired"),
    registryVersionsUnavailable: t("errors.registryVersionsUnavailable"),
    botAuthRequired: (fields: string) => t("errors.botAuthRequired", { fields }),
    fieldRequired: (field: string) => t("errors.fieldRequired", { field }),
    listen: t("gateway.listen"),
    port: t("gateway.port"),
    providers: t("gateway.providers"),
    providerType: t("gateway.providerType"),
    models: t("gateway.models"),
    adaptThinkingEffort: t("gateway.adaptThinkingEffort"),
    addProvider: t("gateway.addProvider"),
    editProvider: t("gateway.editProvider"),
    providerDialogDescription: t("gateway.providerDialogDescription"),
    mcpServers: t("gateway.mcpServers"),
    addMcpServer: t("gateway.addMcpServer"),
    editMcpServer: t("gateway.editMcpServer"),
    mcpServerDialogDescription: t("gateway.mcpServerDialogDescription"),
    availableMcpTools: t("gateway.availableMcpTools"),
    gatewayToolsLoading: t("gateway.gatewayToolsLoading"),
    noGatewayTools: t("gateway.noGatewayTools"),
    unavailableTool: t("gateway.unavailableTool"),
    transport: t("gateway.transport"),
    stdioMessageMode: t("gateway.stdioMessageMode"),
    command: t("gateway.command"),
    args: t("gateway.args"),
    cwd: t("gateway.cwd"),
    url: t("gateway.url"),
    envJson: t("gateway.envJson"),
    headersJson: t("gateway.headersJson"),
    apiKeyEnv: t("gateway.apiKeyEnv"),
    protocolVersion: t("gateway.protocolVersion"),
    startupTimeoutMs: t("gateway.startupTimeoutMs"),
    requestTimeoutMs: t("gateway.requestTimeoutMs"),
    toolInjection: t("gateway.toolInjection"),
    virtualModelProfiles: t("gateway.virtualModelProfiles"),
    addVirtualProfile: t("gateway.addVirtualProfile"),
    editVirtualProfile: t("gateway.editVirtualProfile"),
    virtualProfileDialogDescription: t("gateway.virtualProfileDialogDescription"),
    profileKey: t("gateway.profileKey"),
    description: t("gateway.descriptionField"),
    disabled: t("gateway.disabled"),
    enabled: t("gateway.enabled"),
    exactAliases: t("gateway.exactAliases"),
    prefixes: t("gateway.prefixes"),
    suffixes: t("gateway.suffixes"),
    baseModelMode: t("gateway.baseModelMode"),
    fixedModel: t("gateway.fixedModel"),
    matchMultimodal: t("gateway.matchMultimodal"),
    gatewayMatchRequired: t("gateway.matchRequired"),
    matchWebSearch: t("gateway.matchWebSearch"),
    maxTurns: t("gateway.maxTurns"),
    maxToolCalls: t("gateway.maxToolCalls"),
    clientToolsPolicy: t("gateway.clientToolsPolicy"),
    includeInGatewayModels: t("gateway.includeInGatewayModels"),
    tools: t("gateway.tools"),
    addTool: t("gateway.addTool"),
    visibility: t("gateway.visibility"),
    inputSchemaJson: t("gateway.inputSchemaJson"),
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
  remoteFrontendMode: "app",
  remoteWebAssetRegistryUrl: DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL,
  remoteWebAssetVersion: DEFAULT_CODEX_WEB_ASSET_VERSION,
  remoteWebAssetVersions: [],
  remoteWebAssetVersionsLoading: false,
  remoteWebAssetRegistryError: "",
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
const DEFAULT_CODEXL_SERVER_URL = "https://codexl.io";
const MAX_AUTH_STATUS_REFRESH_DELAY_MS = 2_147_483_647;
const AUTH_REFRESH_SKEW_MS = 5 * 60_000;
const AUTH_REFRESH_RETRY_DELAY_MS = 30_000;
let initialAppUpdateCheckStarted = false;
const providerModelProbeCache = new Map<string, string[]>();
const providerModelProbeInFlight = new Map<string, Promise<string[]>>();
const providerModelProbeFailureUntil = new Map<string, number>();
const PROVIDER_MODEL_PROBE_FAILURE_COOLDOWN_MS = 30_000;

function isEditableTextTarget(target: EventTarget | null) {
  if (!(target instanceof Element)) {
    return false;
  }

  const editable = target.closest("input, textarea, [contenteditable]");
  if (!editable) {
    return false;
  }

  if (editable instanceof HTMLInputElement || editable instanceof HTMLTextAreaElement) {
    return !editable.disabled;
  }

  return editable.getAttribute("contenteditable") !== "false";
}

function useProviderModelProbe(baseUrl: string, apiKey: string, enabled = true, providerHint = "") {
  const [models, setModels] = useState<string[]>([]);

  useEffect(() => {
    const normalizedBaseUrl = baseUrl.trim();
    const normalizedApiKey = apiKey.trim();
    const normalizedProviderHint = providerHint.trim();
    if (!enabled || !isHttpUrl(normalizedBaseUrl)) {
      setModels([]);
      return;
    }

    const key = providerModelProbeKey(normalizedBaseUrl, normalizedApiKey, normalizedProviderHint);
    const cached = providerModelProbeCache.get(key);
    if (cached) {
      setModels(cached);
      return;
    }
    const failureUntil = providerModelProbeFailureUntil.get(key);
    if (failureUntil && failureUntil > Date.now()) {
      setModels([]);
      return;
    }

    let cancelled = false;
    const timer = window.setTimeout(() => {
      loadProviderModelsProbe(key, normalizedBaseUrl, normalizedApiKey, normalizedProviderHint)
        .then((nextModels) => {
          if (!cancelled) {
            setModels(nextModels);
          }
        })
        .catch(() => {
          if (!cancelled) {
            setModels([]);
          }
        });
    }, 900);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [apiKey, baseUrl, enabled, providerHint]);

  return models;
}

function providerModelProbeKey(baseUrl: string, apiKey: string, providerHint: string) {
  return `${baseUrl.trim()}\n${apiKey.trim()}\n${providerHint.trim()}`;
}

async function loadProviderModelsProbe(key: string, baseUrl: string, apiKey: string, providerHint: string) {
  const existing = providerModelProbeInFlight.get(key);
  if (existing) {
    return existing;
  }

  const request = invoke<ProviderModelsProbeResponse>("probe_provider_models", {
    request: {
      base_url: baseUrl,
      api_key: apiKey,
      provider_hint: providerHint,
    },
  })
    .then((response) => normalizeModelOptions(response.models))
    .then((models) => {
      providerModelProbeFailureUntil.delete(key);
      providerModelProbeCache.set(key, models);
      return models;
    })
    .catch((error) => {
      providerModelProbeFailureUntil.set(key, Date.now() + PROVIDER_MODEL_PROBE_FAILURE_COOLDOWN_MS);
      throw error;
    })
    .finally(() => {
      providerModelProbeInFlight.delete(key);
    });

  providerModelProbeInFlight.set(key, request);
  return request;
}

function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [initializing, setInitializing] = useState(true);
  const [runtimeUnsupported, setRuntimeUnsupported] = useState(false);
  const [instanceStatuses, setInstanceStatuses] = useState<Map<string, InstanceStatus>>(new Map());
  const [defaultProviders, setDefaultProviders] = useState<DefaultProviderProfile[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [appSettingsOpen, setAppSettingsOpen] = useState(false);
  const [appSettingsInitialSection, setAppSettingsInitialSection] = useState<AppSettingsSection>("general");
  const [settingsError, setSettingsError] = useState("");
  const [saveDisabled, setSaveDisabled] = useState(false);
  const [workspaceSavePending, setWorkspaceSavePending] = useState(false);
  const [workspaceDeletePending, setWorkspaceDeletePending] = useState(false);
  const [providerMode, setProviderMode] = useState<ProviderMode>("existing");
  const [dialogMode, setDialogMode] = useState<DialogMode>("add");
  const [editingProfileName, setEditingProfileName] = useState<string | null>(null);
  const [editingProfileKey, setEditingProfileKey] = useState<string | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [codexAppPath, setCodexAppPath] = useState("");
  const [gatewayModels, setGatewayModels] = useState<string[]>([]);
  const [pendingDeleteProfile, setPendingDeleteProfile] = useState<ProviderProfile | null>(null);
  const [removeCodexHome, setRemoveCodexHome] = useState(false);
  const [remoteQr, setRemoteQr] = useState<RemoteQrState | null>(null);
  const [workspaceOperation, setWorkspaceOperation] = useState<WorkspaceOperation | null>(null);
  const [remotePasswordDialog, setRemotePasswordDialog] = useState<RemotePasswordDialogState | null>(null);
  const [weixinBotQr, setWeixinBotQr] = useState<WeixinBotQrState | null>(null);
  const [accountLoginState, setAccountLoginState] = useState<AccountLoginState>("idle");
  const [accountError, setAccountError] = useState("");
  const [authStatusRefreshTick, setAuthStatusRefreshTick] = useState(0);
  const [authRefreshRetryAt, setAuthRefreshRetryAt] = useState<number | null>(null);
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
  const newProviderBaseUrlRef = useRef<HTMLInputElement>(null);
  const newProviderApiKeyRef = useRef<HTMLInputElement>(null);
  const newProviderModelRef = useRef<HTMLInputElement>(null);
  const gatewayModelTriggerRef = useRef<HTMLButtonElement>(null);
  const workspaceSavePendingRef = useRef(false);
  const workspaceDeletePendingRef = useRef(false);

  const { i18n } = useTranslation();
  const strings = useAppStrings();
  const language = normalizeLanguage(config?.language);
  const appearance = normalizeAppearance(config?.appearance);
  const gatewayProfileEnabled = nextAiGatewayEnabled(config?.extensions);
  const workspaceDefaultProviders = useMemo(
    () => workspaceSelectableDefaultProviders(defaultProviders, gatewayProfileEnabled),
    [defaultProviders, gatewayProfileEnabled],
  );

  useEffect(() => {
    if (import.meta.env.DEV) {
      return;
    }

    const preventWebViewContextMenu = (event: MouseEvent) => {
      if (!isEditableTextTarget(event.target)) {
        event.preventDefault();
      }
    };

    document.addEventListener("contextmenu", preventWebViewContextMenu, { capture: true });
    return () => {
      document.removeEventListener("contextmenu", preventWebViewContextMenu, { capture: true });
    };
  }, []);

  const showSettingsError = useCallback((error: unknown) => {
    const message = userFacingErrorMessage(error, strings);
    setSettingsError(message);
    console.error(error);
  }, [strings]);

  useEffect(() => {
    const showRuntimeError = (error: unknown) => {
      setSettingsError(userFacingErrorMessage(error, strings));
    };
    const handleError = (event: ErrorEvent) => {
      showRuntimeError(event.error || event.message);
    };
    const handleRejection = (event: PromiseRejectionEvent) => {
      showRuntimeError(event.reason || event);
    };
    window.addEventListener("error", handleError);
    window.addEventListener("unhandledrejection", handleRejection);
    return () => {
      window.removeEventListener("error", handleError);
      window.removeEventListener("unhandledrejection", handleRejection);
    };
  }, [strings]);

  const checkForAppUpdate = useCallback(async () => {
    if (!isTauriRuntime()) {
      return;
    }

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

  const detectCodexAppPath = useCallback(async () => {
    if (!isTauriRuntime()) {
      setCodexAppPath("");
      return "";
    }

    try {
      const path = await invoke<string>("find_codex");
      setCodexAppPath(path);
      return path;
    } catch {
      setCodexAppPath("");
      return "";
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

  const openAppSettingsDialog = useCallback((section: AppSettingsSection = "general") => {
    setAppSettingsInitialSection(section);
    setAppSettingsOpen(true);
    loadDefaultProviders().catch(showSettingsError);
  }, [loadDefaultProviders, showSettingsError]);

  const saveRemoteCloudAuth = useCallback(async (remoteCloudAuth: RemoteCloudAuthConfig, remoteRelayUrl: string) => {
    const nextConfig = await invoke<AppConfig>("update_remote_cloud_auth", {
      remoteCloudAuth,
      remoteRelayUrl: normalizeRemoteRelayUrl(remoteRelayUrl),
    });
    setAuthRefreshRetryAt(null);
    setConfig(nextConfig);
  }, []);

  const beginDesktopLogin = useCallback(async () => {
    if (accountLoginState === "polling") {
      return;
    }
    if (!isTauriRuntime()) {
      setAccountError(strings.desktopRuntimeUnavailableTitle);
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
  }, [
    accountLoginState,
    language,
    saveRemoteCloudAuth,
    showSettingsError,
    strings.desktopRuntimeUnavailableTitle,
    strings.loginExpired,
    strings.loginFailed,
  ]);

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

  const refreshRemoteCloudAuth = useCallback(
    async (auth: RemoteCloudAuthConfig) => {
      const refreshToken = auth.refresh_token.trim();
      if (!refreshToken) {
        setAuthStatusRefreshTick((current) => current + 1);
        return;
      }

      try {
        const result = await refreshDesktopAuth(refreshToken);
        const refreshedRelayUrl =
          remoteRelayUrlFromDesktopRefresh(result) || config?.remote_relay_url || "";
        await saveRemoteCloudAuth(remoteCloudAuthFromDesktopRefresh(result), refreshedRelayUrl);
        setAccountError("");
      } catch (error) {
        if (error instanceof DesktopAuthHttpError && [401, 403].includes(error.status)) {
          setAccountError(strings.sessionExpired);
          await saveRemoteCloudAuth(emptyRemoteCloudAuth(), "");
          return;
        }

        setAccountError(strings.refreshFailed);
        setAuthRefreshRetryAt(Date.now() + AUTH_REFRESH_RETRY_DELAY_MS);
        console.warn("Desktop auth refresh failed; will retry.", error);
      }
    },
    [config?.remote_relay_url, saveRemoteCloudAuth, strings.refreshFailed, strings.sessionExpired],
  );

  useEffect(() => {
    const auth = config?.remote_cloud_auth;
    const delay = remoteCloudAuthStatusRefreshDelay(auth, authRefreshRetryAt);
    if (delay === null) {
      return;
    }

    const timer = window.setTimeout(() => {
      if (auth?.refresh_token.trim()) {
        refreshRemoteCloudAuth(auth).catch(console.error);
        return;
      }
      setAuthStatusRefreshTick((current) => current + 1);
    }, delay);

    return () => {
      window.clearTimeout(timer);
    };
  }, [
    config?.remote_cloud_auth.access_token,
    config?.remote_cloud_auth.expires_at,
    config?.remote_cloud_auth.refresh_token,
    config?.remote_cloud_auth.user_id,
    authRefreshRetryAt,
    authStatusRefreshTick,
    refreshRemoteCloudAuth,
  ]);

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
    void detectCodexAppPath();
  }, [detectCodexAppPath]);

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
      if (!isTauriRuntime()) {
        if (!cancelled) {
          setRuntimeUnsupported(true);
          setInitializing(false);
        }
        return;
      }

      try {
        setRuntimeUnsupported(false);
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
      } finally {
        if (!cancelled) {
          setInitializing(false);
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
  const workspaceSearchDisabled = initializing || runtimeUnsupported || profiles.length === 0;
  const headerActionsDisabled = initializing || runtimeUnsupported;
  const headerDisabledReason = runtimeUnsupported
    ? strings.desktopRuntimeUnavailableTitle
    : initializing
      ? strings.loadingWorkspacesTitle
      : "";

  const syncExistingProviderFields = useCallback(
    (profileName: string, providers = workspaceDefaultProviders) => {
      const profile = providers.find((item) => item.name === profileName);
      setForm((current) => ({
        ...current,
        existingProfileName: profileName,
        existingBaseUrl: profile?.base_url || "",
        existingApiKey: profile?.api_key || "",
        existingModel: profile?.model || "",
      }));
    },
    [workspaceDefaultProviders],
  );

  const openAddProviderDialog = useCallback(async () => {
    setDialogMode("add");
    setEditingProfileName(null);
    setEditingProfileKey(null);
    setSettingsError("");
    setSaveDisabled(false);
    setWorkspaceSavePending(false);
    workspaceSavePendingRef.current = false;
    const [providers, models, detectedCodexAppPath] = await Promise.all([
      loadDefaultProviders(),
      gatewayProfileEnabled ? loadGatewayModels() : Promise.resolve([]),
      detectCodexAppPath(),
    ]);
    const selectableProviders = workspaceSelectableDefaultProviders(providers, gatewayProfileEnabled);
    const nextMode: ProviderMode = selectableProviders.length > 0 ? "existing" : "none";
    setForm({
      ...emptyForm,
      ...defaultRemoteFrontendFormFields(detectedCodexAppPath),
      gatewayModel: models[0] || "",
    });
    setProviderMode(nextMode);
    if (nextMode === "existing") {
      syncExistingProviderFields(selectableProviders[0].name, selectableProviders);
    }
    setSettingsOpen(true);
    window.requestAnimationFrame(() => {
      workspaceNameInputRef.current?.focus();
    });
  }, [
    detectCodexAppPath,
    gatewayProfileEnabled,
    loadDefaultProviders,
    loadGatewayModels,
    syncExistingProviderFields,
  ]);

  const openEditProviderDialog = useCallback(
    async (profile: ProviderProfile) => {
      setDialogMode("edit");
      setEditingProfileName(profile.name);
      setEditingProfileKey(profile.name === "Default" ? profile.name : profileKey(profile));
      setSettingsError("");
      setSaveDisabled(false);
      setWorkspaceSavePending(false);
      workspaceSavePendingRef.current = false;
      setForm(emptyForm);
      const isGatewayProfile = gatewayProfileEnabled && isNextAiGatewayProvider(profile);
      const [providers, detectedCodexAppPath, models] = await Promise.all([
        loadDefaultProviders(),
        detectCodexAppPath(),
        isGatewayProfile ? loadGatewayModels() : Promise.resolve([]),
      ]);
      const selectableProviders = workspaceSelectableDefaultProviders(providers, gatewayProfileEnabled);

      if (isProviderlessWorkspace(profile)) {
        setProviderMode("none");
        setForm({
          ...emptyForm,
          workspaceName: profile.name,
          proxyUrl: profile.proxy_url || "",
          ...profileRemoteFrontendFormFields(profile, detectedCodexAppPath),
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
          ...profileRemoteFrontendFormFields(profile, detectedCodexAppPath),
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

      if (selectableProviders.length === 0) {
        setProviderMode("existing");
        setSettingsError(strings.noProviderFound);
        setSaveDisabled(false);
        setForm({
          ...emptyForm,
          workspaceName: profile.name,
          proxyUrl: profile.proxy_url || "",
          ...profileRemoteFrontendFormFields(profile, detectedCodexAppPath),
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

      const selected = selectProviderForProfile(profile, selectableProviders);
      setProviderMode("existing");
      setForm({
        ...emptyForm,
        workspaceName: profile.name,
        proxyUrl: profile.proxy_url || "",
        ...profileRemoteFrontendFormFields(profile, detectedCodexAppPath),
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
    [
      detectCodexAppPath,
      gatewayProfileEnabled,
      loadDefaultProviders,
      loadGatewayModels,
      strings.noProviderFound,
    ],
  );

  const closeSettingsDialog = useCallback(() => {
    setSettingsOpen(false);
    setSettingsError("");
    setEditingProfileName(null);
    setEditingProfileKey(null);
    setDialogMode("add");
    setSaveDisabled(false);
    setWorkspaceSavePending(false);
    workspaceSavePendingRef.current = false;
  }, []);

  const selectProviderMode = useCallback(
    (mode: ProviderMode) => {
      setSettingsError("");
      setSaveDisabled(false);
      if (mode === "none") {
        setProviderMode("none");
        return;
      }
      if (mode === "existing" && workspaceDefaultProviders.length === 0) {
        setSettingsError(strings.noProviderFound);
        return;
      }
      if (mode === "existing") {
        const selectedProfileName = workspaceDefaultProviders.some(
          (profile) => profile.name === form.existingProfileName,
        )
          ? form.existingProfileName
          : workspaceDefaultProviders[0]?.name || "";
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
      if (mode === "gateway" && dialogMode === "add") {
        setForm((current) =>
          current.providerName ? { ...current, providerName: "" } : current,
        );
      }
      setProviderMode(mode);
    },
    [
      dialogMode,
      form.existingProfileName,
      gatewayModels.length,
      gatewayProfileEnabled,
      loadGatewayModels,
      strings.noProviderFound,
      syncExistingProviderFields,
      workspaceDefaultProviders,
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
    if (workspaceSavePendingRef.current) return;

    workspaceSavePendingRef.current = true;
    setWorkspaceSavePending(true);
    try {
      setSettingsError("");
      let nextConfig: AppConfig;
      let savedProfileName = "";
      let savedProfileKey = "";
      let savedBot: BotProfileConfig | null = null;
      const extensionsEnabled = botExtensionsEnabled(config.extensions);
      const originalProfileKey = editingProfileKey || editingProfileName || "";

      if (providerMode === "none" || form.remoteFrontendMode === "claude-code") {
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

        if (dialogMode === "edit" && originalProfileKey) {
          const update: UpdateWorkspaceProvider = { ...provider, original_name: originalProfileKey };
          nextConfig = await invoke<AppConfig>("update_workspace", { provider: update });
        } else {
          nextConfig = await invoke<AppConfig>("create_workspace", { provider });
        }
      } else if (providerMode === "gateway") {
        const provider = readNextAiGatewayProviderForm(
          form,
          workspaceNameInputRef,
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

        if (dialogMode === "edit" && originalProfileKey) {
          const update: UpdateNextAiGatewayProvider = { ...provider, original_name: originalProfileKey };
          nextConfig = await invoke<AppConfig>("update_next_ai_gateway_provider", { provider: update });
        } else {
          nextConfig = await invoke<AppConfig>("create_next_ai_gateway_provider", { provider });
        }
      } else if (providerMode === "existing") {
        const provider = readExistingProviderForm(
          form,
          workspaceNameInputRef,
          existingProviderSelectRef,
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

        if (dialogMode === "edit" && originalProfileKey) {
          const update: UpdateProvider = { ...provider, original_name: originalProfileKey };
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

      savedProfileKey = nextConfig.active_provider || originalProfileKey;
      const savedProfile =
        nextConfig.provider_profiles.find((profile) => profileKey(profile) === savedProfileKey) ||
        nextConfig.provider_profiles.find((profile) => profile.name === savedProfileName);
      savedProfileKey = savedProfile ? profileKey(savedProfile) : savedProfileKey;
      savedBot = savedProfile?.bot ?? savedBot;
      if (extensionsEnabled && savedProfileKey && isStaticAuthBot(savedBot)) {
        nextConfig = await invoke<AppConfig>("configure_bot_integration", {
          profileName: savedProfileKey,
        });
        savedBot =
          nextConfig.provider_profiles.find((profile) => profileKey(profile) === savedProfileKey)
            ?.bot ?? savedBot;
      }

      setConfig(nextConfig);
      setSettingsOpen(false);
      setEditingProfileName(null);
      setEditingProfileKey(null);
      setDialogMode("add");
      setForm(emptyForm);
      await refreshStatus();
      if (extensionsEnabled && savedProfileKey && shouldStartQrLogin(savedBot)) {
        await openWeixinBotLogin(savedProfileKey);
      }
    } catch (error) {
      showSettingsError(error);
    } finally {
      workspaceSavePendingRef.current = false;
      setWorkspaceSavePending(false);
    }
  }, [
    config,
    dialogMode,
    editingProfileKey,
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
      transcribeBaseUrl: string;
      transcribeApiKey: string;
      transcribeModel: string;
      botConfigs?: SavedBotConfig[];
    }) => {
      if (!config) return;
      const nextBotConfigs = normalizeSavedBotConfigs(nextSettings.botConfigs ?? config.bot_configs);
      const transcribeSettings = normalizeTranscribeSettings(nextSettings);

      const nextConfig: AppConfig = {
        ...config,
        language: nextSettings.language,
        appearance: nextSettings.appearance,
        extensions: normalizeExtensionSettings(nextSettings.extensions),
        remote_transcribe_base_url: transcribeSettings.transcribeBaseUrl,
        remote_transcribe_api_url: transcribeSettings.transcribeBaseUrl,
        remote_transcribe_api_key: transcribeSettings.transcribeApiKey,
        remote_transcribe_model: transcribeSettings.transcribeModel,
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

  const saveDefaultProviderProfile = useCallback(
    async (provider: DefaultProviderProfile) => {
      const nextConfig = await invoke<AppConfig>("save_default_provider_profile", { provider });
      setConfig(nextConfig);
      await loadDefaultProviders();
      await refreshStatus();
      return nextConfig;
    },
    [loadDefaultProviders, refreshStatus],
  );

  const deleteDefaultProviderProfile = useCallback(
    async (name: string) => {
      const nextConfig = await invoke<AppConfig>("delete_default_provider_profile", { name });
      setConfig(nextConfig);
      await loadDefaultProviders();
      await refreshStatus();
      return nextConfig;
    },
    [loadDefaultProviders, refreshStatus],
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
      const key = profileKey(profile);
      const startRemote = options.startRemote === true;
      const startCloud = startRemote && options.startCloud === true;
      const requireE2ee = startCloud;

      setWorkspaceOperation({ key, kind: "start" });
      try {
        const info = await invoke<LaunchInfo>("launch_codex", {
          cdpPort: config?.cdp_port || null,
          codexPath: config?.codex_path || null,
          profileName: key,
        });

        setInstanceStatuses((current) => {
          const next = new Map(current);
          const existing = next.get(key);
          next.set(key, {
            ...info,
            remote_control: existing?.remote_control || null,
          });
          return next;
        });
        setConfig((current) =>
          current
            ? {
                ...current,
                active_provider: key,
                codex_home: info.codex_home,
              }
            : current,
        );

        if (startRemote) {
          await invoke<RemoteControlInfo>("start_remote_control", {
            profileName: key,
            remotePassword: null,
            useCloudRelay: startCloud,
            requireE2ee,
          });
          await refreshStatus();
        }
      } catch (error) {
        showSettingsError(error);
        await refreshStatus().catch(console.error);
      } finally {
        setWorkspaceOperation((current) =>
          current?.key === key && current.kind === "start" ? null : current,
        );
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
      const key = profileKey(profile);
      setWorkspaceOperation({ key, kind: "stop" });
      try {
        await invoke("stop_codex", { profileName: key });
        await refreshStatus();
      } catch (error) {
        showSettingsError(error);
      } finally {
        setWorkspaceOperation((current) =>
          current?.key === key && current.kind === "stop" ? null : current,
        );
      }
    },
    [refreshStatus, showSettingsError],
  );

  const toggleProfile = useCallback(
    async (profile: ProviderProfile, options: Partial<RemoteLaunchOptions> = {}) => {
      const key = profileKey(profile);
      if (workspaceOperation?.key === key) {
        return;
      }
      const status = instanceStatuses.get(key);
      const isRunning = Boolean(status?.running || status?.remote_control?.running);
      if (isRunning) {
        await stopCodex(profile);
        return;
      }
      await launchProfile(profile, options);
    },
    [instanceStatuses, launchProfile, stopCodex, workspaceOperation],
  );

  const setRemoteLaunchOptions = useCallback(
    async (profileName: string, options: Partial<RemoteLaunchOptions>) => {
      const profile = config?.provider_profiles.find((item) => profileKey(item) === profileName || item.name === profileName);
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

      const key = profileKey(profile);
      setWorkspaceOperation({ key, kind: "options" });
      try {
        const nextConfig = await invoke<AppConfig>("set_remote_launch_options", {
          profileName,
          startRemote,
          startCloud,
          remoteE2eePassword,
        });
        setConfig(nextConfig);
      } finally {
        setWorkspaceOperation((current) =>
          current?.key === key && current.kind === "options" ? null : current,
        );
      }
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
    setWorkspaceDeletePending(false);
    workspaceDeletePendingRef.current = false;
  }, []);

  const confirmDelete = useCallback(async () => {
    if (!pendingDeleteProfile) return;
    if (workspaceDeletePendingRef.current) return;

    workspaceDeletePendingRef.current = true;
    setWorkspaceDeletePending(true);
    try {
      const nextConfig = await invoke<AppConfig>("delete_provider", {
        name: profileKey(pendingDeleteProfile),
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
    } finally {
      workspaceDeletePendingRef.current = false;
      setWorkspaceDeletePending(false);
    }
  }, [pendingDeleteProfile, refreshConfig, refreshStatus, removeCodexHome, showSettingsError]);

  const showRemoteQr = useCallback(
    (profile: ProviderProfile, remote: RemoteControlInfo) => {
      try {
        if (!remoteControlReadyForQr(remote)) {
          return;
        }
        const urlOptions = remoteQrUrlOptions(remote);
        if (urlOptions.length === 0) {
          return;
        }
        setRemoteQr({
          profile,
          remote,
          defaultUrlKind: urlOptions[0].kind,
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
            disabled={workspaceSearchDisabled}
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
            disabled={headerActionsDisabled}
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
              disabled={headerActionsDisabled}
              onClick={() => openAppSettingsDialog()}
            >
              <Settings className="w-4 h-4" />
            </Button>
          </Tooltip>
          <AccountMenu
            auth={config?.remote_cloud_auth ?? emptyRemoteCloudAuth()}
            busy={accountLoginState === "polling"}
            disabled={headerActionsDisabled}
            disabledReason={headerDisabledReason}
            error={accountError}
            language={language}
            strings={strings}
            onSignIn={() => beginDesktopLogin().catch(showSettingsError)}
            onSignOut={() => clearRemoteCloudAuth().catch(showSettingsError)}
            onOpenDashboard={() => openAccountDashboard().catch(showSettingsError)}
          />
        </div>
      </header>

      {settingsError && !settingsOpen ? (
        <div className="fixed left-1/2 top-14 z-50 flex w-[min(42rem,calc(100vw-2rem))] -translate-x-1/2 items-start gap-2 rounded-md border border-destructive/50 bg-background px-3 py-2.5 text-sm shadow-lg">
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
          <span className="min-w-0 flex-1 break-words text-foreground">{settingsError}</span>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="-mr-1 -mt-1 h-7 w-7 shrink-0"
            onClick={() => setSettingsError("")}
            aria-label={strings.close}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      ) : null}

      <main className="min-h-0 flex-1 overflow-auto p-6 md:p-8">
        {initializing ? (
          <WorkspaceLoadingState strings={strings} />
        ) : runtimeUnsupported ? (
          <RuntimeUnsupportedState strings={strings} />
        ) : filteredProfiles.length > 0 ? (
          <div className="max-w-7xl mx-auto grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-6">
            {filteredProfiles.map((profile) => {
              const key = profileKey(profile);
              const status = instanceStatuses.get(key) || instanceStatuses.get(profile.name) || null;
              const operationKind = workspaceOperation?.key === key ? workspaceOperation.kind : null;
              return (
                <ProfileCard
                  key={key}
                  profile={profile}
                  status={status}
                  operationKind={operationKind}
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
        ) : profiles.length > 0 && searchQuery.trim() ? (
          <WorkspaceSearchEmptyState strings={strings} onClearSearch={() => setSearchQuery("")} />
        ) : (
          <WorkspaceEmptyState
            strings={strings}
            onCreate={() => openAddProviderDialog().catch(showSettingsError)}
            onManageProfiles={() => openAppSettingsDialog("profiles")}
            onConfigureExtensions={() => openAppSettingsDialog("extensions")}
          />
        )}
      </main>

      {appSettingsOpen && config ? (
        <AppSettingsDialog
          appearance={appearance}
          language={language}
          remoteCloudAuth={config.remote_cloud_auth ?? emptyRemoteCloudAuth()}
          extensions={normalizeExtensionSettings(config.extensions)}
          transcribeBaseUrl={config.remote_transcribe_base_url || config.remote_transcribe_api_url || ""}
          transcribeApiKey={config.remote_transcribe_api_key || ""}
          transcribeModel={config.remote_transcribe_model || DEFAULT_TRANSCRIBE_MODEL}
          botConfigs={config.bot_configs || []}
          defaultProviders={defaultProviders}
          profiles={profiles}
          onClose={() => setAppSettingsOpen(false)}
          onSave={saveAppSettings}
          onSaveBotConfigs={saveBotConfigs}
          onSaveDefaultProvider={saveDefaultProviderProfile}
          onDeleteDefaultProvider={deleteDefaultProviderProfile}
          appUpdateState={appUpdateState}
          initialSection={appSettingsInitialSection}
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
          codexAppPath={codexAppPath}
          settingsError={settingsError}
          saveDisabled={saveDisabled}
          saving={workspaceSavePending}
          editingProfileName={editingProfileName}
          existingProviderSelectRef={existingProviderSelectRef}
          workspaceNameInputRef={workspaceNameInputRef}
          providerNameInputRef={providerNameInputRef}
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
          busy={workspaceDeletePending}
          onRemoveCodexHomeChange={setRemoveCodexHome}
          onCancel={() => {
            if (workspaceDeletePendingRef.current) return;
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
  disabled,
  disabledReason,
  error,
  language,
  strings,
  onSignIn,
  onSignOut,
  onOpenDashboard,
}: {
  auth: RemoteCloudAuthConfig;
  busy: boolean;
  disabled: boolean;
  disabledReason: string;
  error: string;
  language: Language;
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
  const proExpiresAt = isPro ? remoteCloudSubscriptionExpiresAt(auth) : 0;
  const proExpiresAtText = proExpiresAt ? formatAccountDate(proExpiresAt, language) : "";
  const accountTitle = disabled ? disabledReason : error || (busy ? strings.signingIn : strings.signIn);

  if (!signedIn) {
    return (
      <Button
        type="button"
        variant="outline"
        size="icon"
        title={accountTitle}
        aria-label={busy ? strings.signingIn : strings.signIn}
        className="h-8 w-8 rounded-full"
        disabled={busy || disabled}
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
            title={disabled ? disabledReason : strings.signedInAs(label)}
            aria-label={strings.account}
            className="h-8 w-8 rounded-full p-0"
            disabled={disabled}
          >
          <AccountAvatar label={label} avatarUrl={avatarUrl} premium={isPro} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <div className="px-2 py-2">
          <div className="flex min-w-0 items-center gap-2">
            <div className="min-w-0 truncate text-sm font-medium">{label}</div>
            {isPro ? (
              <Badge className="account-pro-badge shrink-0 px-1.5 py-0 text-[10px] leading-4">
                {strings.pro}
              </Badge>
            ) : null}
          </div>
          {proExpiresAtText ? (
            <div className="mt-0.5 truncate text-xs text-muted-foreground">
              {strings.proExpiresAt(proExpiresAtText)}
            </div>
          ) : null}
          {email ? (
            <div className="mt-0.5 truncate text-xs text-muted-foreground">{email}</div>
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
  const avatarSizeClassName = premium ? "h-[1.625rem] w-[1.625rem]" : "h-7 w-7";

  if (avatarUrl && !failed) {
    return (
      <span className={className}>
        <img
          src={avatarUrl}
          alt=""
          className={cn("relative z-10 rounded-full object-cover", avatarSizeClassName)}
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      </span>
    );
  }

  return (
    <span className={className}>
      <span
        className={cn(
          "relative z-10 flex items-center justify-center rounded-full bg-primary text-[11px] font-semibold text-primary-foreground",
          avatarSizeClassName,
        )}
      >
        {accountInitials(label)}
      </span>
    </span>
  );
}

function WorkspaceLoadingState({ strings }: { strings: AppStrings }) {
  return (
    <div className="mx-auto flex max-w-7xl flex-col gap-6">
      <div className="flex flex-col items-center justify-center py-20 text-center">
        <div className="mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-muted">
          <RefreshCw className="h-7 w-7 animate-spin text-muted-foreground" />
        </div>
        <h2 className="text-lg font-medium">{strings.loadingWorkspacesTitle}</h2>
        <p className="mt-1 max-w-sm text-muted-foreground">{strings.loadingWorkspacesDescription}</p>
      </div>
      <div className="grid grid-cols-1 gap-6 opacity-60 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        {Array.from({ length: 4 }).map((_, index) => (
          <div key={index} className="rounded-lg border border-border/60 bg-card p-5">
            <div className="flex items-center gap-3">
              <div className="h-10 w-10 rounded-xl bg-muted" />
              <div className="min-w-0 flex-1 space-y-2">
                <div className="h-4 w-2/3 rounded bg-muted" />
                <div className="h-3 w-1/2 rounded bg-muted" />
              </div>
            </div>
            <div className="mt-6 space-y-3">
              <div className="h-3 w-3/4 rounded bg-muted" />
              <div className="h-3 w-1/2 rounded bg-muted" />
              <div className="h-8 w-full rounded bg-muted" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function RuntimeUnsupportedState({ strings }: { strings: AppStrings }) {
  return (
    <div className="mx-auto flex max-w-2xl flex-col items-center justify-center py-20 text-center">
      <div className="mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-destructive/10">
        <AlertCircle className="h-8 w-8 text-destructive" />
      </div>
      <h2 className="text-lg font-medium">{strings.desktopRuntimeUnavailableTitle}</h2>
      <p className="mt-2 max-w-md text-sm leading-relaxed text-muted-foreground">
        {strings.desktopRuntimeUnavailableDescription}
      </p>
    </div>
  );
}

function WorkspaceSearchEmptyState({
  strings,
  onClearSearch,
}: {
  strings: AppStrings;
  onClearSearch: () => void;
}) {
  return (
    <div className="mx-auto flex max-w-2xl flex-col items-center justify-center py-20 text-center">
      <div className="mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-muted">
        <Search className="h-8 w-8 text-muted-foreground opacity-70" />
      </div>
      <h2 className="text-lg font-medium">{strings.noInstancesTitle}</h2>
      <p className="mt-1 max-w-sm text-muted-foreground">{strings.noInstancesDescription}</p>
      <Button type="button" variant="outline" className="mt-6" onClick={onClearSearch}>
        <X className="h-4 w-4" />
        {strings.clearSearch}
      </Button>
    </div>
  );
}

function WorkspaceEmptyState({
  strings,
  onCreate,
  onManageProfiles,
  onConfigureExtensions,
}: {
  strings: AppStrings;
  onCreate: () => void;
  onManageProfiles: () => void;
  onConfigureExtensions: () => void;
}) {
  const setupItems = [
    {
      icon: <Plus className="h-4 w-4" />,
      title: strings.emptyCreateTitle,
      description: strings.emptyCreateDescription,
      action: strings.createInstance,
      onClick: onCreate,
      primary: true,
    },
    {
      icon: <FileCog className="h-4 w-4" />,
      title: strings.emptyProfilesTitle,
      description: strings.emptyProfilesDescription,
      action: strings.manageProfiles,
      onClick: onManageProfiles,
      primary: false,
    },
    {
      icon: <Puzzle className="h-4 w-4" />,
      title: strings.emptyGatewayTitle,
      description: strings.emptyGatewayDescription,
      action: strings.configureExtensions,
      onClick: onConfigureExtensions,
      primary: false,
    },
  ];

  return (
    <div className="mx-auto flex max-w-5xl flex-col items-center justify-center py-14">
      <div className="mb-8 max-w-xl text-center">
        <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-muted">
          <Server className="h-8 w-8 text-muted-foreground opacity-70" />
        </div>
        <h2 className="text-xl font-semibold">{strings.noInstancesTitle}</h2>
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{strings.noInstancesDescription}</p>
      </div>

      <div className="grid w-full gap-3 md:grid-cols-3">
        {setupItems.map((item) => (
          <button
            key={item.title}
            type="button"
            className={cn(
              "flex min-h-44 flex-col rounded-lg border p-4 text-left transition-colors hover:border-primary/40 hover:bg-muted/20",
              item.primary ? "border-emerald/40 bg-emerald/5" : "border-border bg-card",
            )}
            onClick={item.onClick}
          >
            <span
              className={cn(
                "mb-4 flex h-9 w-9 items-center justify-center rounded-md border",
                item.primary ? "border-emerald/30 bg-emerald/10 text-emerald" : "border-border bg-muted/40",
              )}
            >
              {item.icon}
            </span>
            <span className="text-sm font-medium text-foreground">{item.title}</span>
            <span className="mt-2 flex-1 text-sm leading-relaxed text-muted-foreground">{item.description}</span>
            <span className="mt-4 inline-flex items-center gap-2 text-sm font-medium text-foreground">
              {item.action}
              <ChevronDown className="-rotate-90 h-3.5 w-3.5" />
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

type ProfileCardProps = {
  profile: ProviderProfile;
  status: InstanceStatus | null;
  operationKind: WorkspaceOperationKind | null;
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
  operationKind,
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
  const isRunning = Boolean(status?.running || remote?.running);
  const isRemoteRunning = Boolean(remote?.running);
  const isBusy = operationKind !== null;
  const showRemoteActions = isRunning && remoteControlReadyForQr(remote);
  const remoteQrDisabledTooltip = isBusy
    ? strings.saving
    : isRunning
      ? strings.remoteQrUnavailable
      : strings.remoteQrStartRequired;
  const codexProfileName =
    profile.provider_config_format === "top_level"
      ? profile.provider_name || profile.name
      : profile.codex_profile_name || profile.name;
  const remoteFrontendMode = normalizeRemoteFrontendMode(profile.remote_frontend_mode);
  const providerLine =
    isProviderlessWorkspace(profile)
      ? strings.providerlessWorkspace
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
        <div className="flex items-center gap-3 text-sm text-muted-foreground">
          {remoteFrontendMode === "claude-code" ? (
            <Cpu className="w-4 h-4 shrink-0" />
          ) : remoteFrontendModeUsesCli(remoteFrontendMode) ? (
            <Terminal className="w-4 h-4 shrink-0" />
          ) : (
            <Monitor className="w-4 h-4 shrink-0" />
          )}
          <span className="truncate text-foreground">
            {remoteFrontendMode === "claude-code"
              ? strings.remoteFrontendClaudeCode
              : remoteFrontendMode === "cli"
                ? `${strings.remoteFrontendCli} / ${strings.registryVersion}: ${profile.remote_web_asset_version || DEFAULT_CODEX_WEB_ASSET_VERSION}`
                : strings.remoteFrontendApp}
          </span>
        </div>
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
          operationKind={operationKind}
          options={remoteLaunchOptions}
          onToggleProfile={() => onToggleProfile(profile, remoteLaunchOptions)}
          onOptionsChange={(options) => {
            onRemoteLaunchOptionsChange(profileKey(profile), options).catch(onError);
          }}
          onError={onError}
        />

        <div className="flex gap-2">
          <IconButton
            title={strings.showRemoteQr}
            disabled={isBusy || !showRemoteActions || !(remote?.url || remote?.lan_url)}
            tooltip={remoteQrDisabledTooltip}
            onClick={() => {
              if (remote) {
                onShowRemoteQr(profile, remote);
              }
            }}
          >
            <QrCode className="w-3.5 h-3.5" />
          </IconButton>
          <IconButton
            title={strings.editProfile(profile.name)}
            disabled={isBusy}
            tooltip={strings.saving}
            onClick={() => onEdit(profile).catch(onError)}
          >
            <Pencil className="w-3.5 h-3.5" />
          </IconButton>
          <IconButton
            title={strings.deleteProfile(profile.name)}
            disabled={isBusy}
            tooltip={strings.saving}
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
  operationKind: WorkspaceOperationKind | null;
  options: RemoteLaunchOptions;
  onToggleProfile: () => Promise<void>;
  onOptionsChange: (options: Partial<RemoteLaunchOptions>) => void;
  onError: (error: unknown) => void;
};

function LaunchMenuButton({
  isRunning,
  operationKind,
  options,
  onToggleProfile,
  onOptionsChange,
  onError,
}: LaunchMenuButtonProps) {
  const strings = useAppStrings();
  const variant = isRunning ? "dangerOutline" : "success";
  const startRemote = options.startRemote;
  const startCloud = startRemote && options.startCloud;
  const isBusy = operationKind !== null;
  const isStarting = operationKind === "start";
  const isStopping = operationKind === "stop";
  const isSavingOptions = operationKind === "options";
  const busyFallbackLabel = operationKind === "options" ? strings.saving : "";

  if (isRunning) {
    return (
      <Button
        type="button"
        variant="dangerOutline"
        size="sm"
        disabled={isBusy}
        onClick={() => onToggleProfile().catch(onError)}
      >
        {isBusy ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Square className="w-3.5 h-3.5" />}
        {isStopping ? strings.stopping : busyFallbackLabel || strings.stop}
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
        disabled={isBusy}
        onClick={() => onToggleProfile().catch(onError)}
      >
        {isBusy ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
        {isStarting ? strings.starting : busyFallbackLabel || strings.start}
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant={variant}
            size="sm"
            className="rounded-l-none px-2"
            title={strings.launchOptions}
            disabled={isBusy}
          >
            {isSavingOptions ? (
              <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <ChevronDown className="w-3.5 h-3.5" />
            )}
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
              disabled={isBusy}
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
                disabled={isBusy}
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
  remoteCloudAuth,
  extensions,
  transcribeBaseUrl,
  transcribeApiKey,
  transcribeModel,
  botConfigs,
  defaultProviders,
  profiles,
  appUpdateState,
  initialSection,
  onClose,
  onSave,
  onSaveBotConfigs,
  onSaveDefaultProvider,
  onDeleteDefaultProvider,
  onCheckForAppUpdate,
  onInstallAppUpdate,
}: {
  appearance: Appearance;
  language: Language;
  remoteCloudAuth: RemoteCloudAuthConfig;
  extensions: ExtensionSettings;
  transcribeBaseUrl: string;
  transcribeApiKey: string;
  transcribeModel: string;
  botConfigs: SavedBotConfig[];
  defaultProviders: DefaultProviderProfile[];
  profiles: ProviderProfile[];
  appUpdateState: AppUpdateState;
  initialSection: AppSettingsSection;
  onClose: () => void;
  onSave: (settings: {
    language: Language;
    appearance: Appearance;
    extensions: ExtensionSettings;
    transcribeBaseUrl: string;
    transcribeApiKey: string;
    transcribeModel: string;
    botConfigs?: SavedBotConfig[];
  }) => Promise<void>;
  onSaveBotConfigs: (botConfigs: SavedBotConfig[]) => Promise<AppConfig | null>;
  onSaveDefaultProvider: (provider: DefaultProviderProfile) => Promise<AppConfig>;
  onDeleteDefaultProvider: (name: string) => Promise<AppConfig>;
  onCheckForAppUpdate: () => Promise<void>;
  onInstallAppUpdate: () => Promise<void>;
}) {
  const strings = useAppStrings();
  const [activeSection, setActiveSection] = useState<AppSettingsSection>(initialSection);
  const [draftLanguage, setDraftLanguage] = useState<Language>(language);
  const [draftAppearance, setDraftAppearance] = useState<Appearance>(appearance);
  const [draftExtensions, setDraftExtensions] = useState<ExtensionSettings>(normalizeExtensionSettings(extensions));
  const [draftTranscribeBaseUrl, setDraftTranscribeBaseUrl] = useState(transcribeBaseUrl);
  const [draftTranscribeApiKey, setDraftTranscribeApiKey] = useState(transcribeApiKey);
  const [draftTranscribeModel, setDraftTranscribeModel] = useState(transcribeModel || DEFAULT_TRANSCRIBE_MODEL);
  const [draftBotConfigs, setDraftBotConfigs] = useState<SavedBotConfig[]>(normalizeSavedBotConfigs(botConfigs));
  const [botEditor, setBotEditor] = useState<{ mode: "add" | "edit"; config: SavedBotConfig | null } | null>(null);
  const [pendingDeleteBotConfig, setPendingDeleteBotConfig] = useState<SavedBotConfig | null>(null);
  const [botSaving, setBotSaving] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const [preparingExtensions, setPreparingExtensions] = useState(false);
  const [discardSettingsConfirmOpen, setDiscardSettingsConfirmOpen] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [extensionStatuses, setExtensionStatuses] = useState<BuiltinExtensionStatus[]>([]);
  const [extensionError, setExtensionError] = useState("");
  const [gatewayForm, setGatewayForm] = useState<GatewayConfigForm | null>(null);
  const [savedGatewayConfigSignature, setSavedGatewayConfigSignature] = useState<string | null>(null);
  const [gatewayError, setGatewayError] = useState("");
  const [usageViewMode, setUsageViewMode] = useState<GatewayUsageViewMode>("overview");
  const [usageOverviewSummary, setUsageOverviewSummary] = useState<GatewayUsageSummary | null>(null);
  const [usageOverviewLoading, setUsageOverviewLoading] = useState(false);
  const [usageOverviewError, setUsageOverviewError] = useState("");
  const [usageSummary, setUsageSummary] = useState<GatewayUsageSummary | null>(null);
  const [usageLoading, setUsageLoading] = useState(false);
  const [usageError, setUsageError] = useState("");
  const [usageDateRange, setUsageDateRange] = useState<GatewayUsageDateRange>(() =>
    gatewayUsageDateRangeForHours(24),
  );
  const [usageFullscreen, setUsageFullscreen] = useState(false);
  const botEnabled = draftExtensions.enabled && draftExtensions.bot_gateway_enabled;
  const gatewayEnabled = draftExtensions.enabled && draftExtensions.next_ai_gateway_enabled;
  const gatewayUsageEnabled = gatewayEnabled && Boolean(gatewayForm?.usageCaptureEnabled);
  const usageFullscreenActive = activeSection === "usage" && usageFullscreen;

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
      const nextForm = gatewayFormFromConfig(result);
      setGatewayForm(nextForm);
      setSavedGatewayConfigSignature(gatewayConfigFormSignature(nextForm));
    } catch (error) {
      setGatewayError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    if (gatewayEnabled && gatewayForm === null) {
      loadGatewayConfig().catch(console.error);
    }
  }, [gatewayEnabled, gatewayForm, loadGatewayConfig]);

  useEffect(() => {
    if ((activeSection === "gateway" || activeSection === "usage") && !gatewayEnabled) {
      setActiveSection("extensions");
    }
  }, [activeSection, gatewayEnabled]);

  useEffect(() => {
    if (activeSection === "usage" && gatewayForm && !gatewayForm.usageCaptureEnabled) {
      setActiveSection("gateway");
    }
  }, [activeSection, gatewayForm]);

  useEffect(() => {
    if (activeSection !== "usage" && usageFullscreen) {
      setUsageFullscreen(false);
    }
  }, [activeSection, usageFullscreen]);

  useEffect(() => {
    if (usageViewMode === "overview" && usageFullscreen) {
      setUsageFullscreen(false);
    }
  }, [usageFullscreen, usageViewMode]);

  useEffect(() => {
    if (activeSection === "bot" && !botEnabled) {
      setActiveSection("extensions");
    }
  }, [activeSection, botEnabled]);

  const loadGatewayUsageOverview = useCallback(async () => {
    setUsageOverviewLoading(true);
    setUsageOverviewError("");
    try {
      const summary = await invoke<GatewayUsageSummary>("get_gateway_usage_summary", {
        days: 365,
        hours: undefined,
        startDate: undefined,
        endDate: undefined,
      });
      setUsageOverviewSummary(summary);
    } catch (error) {
      setUsageOverviewError(errorMessage(error));
    } finally {
      setUsageOverviewLoading(false);
    }
  }, []);

  const loadGatewayUsage = useCallback(async () => {
    setUsageLoading(true);
    setUsageError("");
    try {
      const summary = await invoke<GatewayUsageSummary>("get_gateway_usage_summary", {
        days: usageDateRange.hours ? undefined : 30,
        hours: usageDateRange.hours,
        startDate: usageDateRange.hours ? undefined : usageDateRange.startDate,
        endDate: usageDateRange.hours ? undefined : usageDateRange.endDate,
      });
      setUsageSummary(summary);
    } catch (error) {
      setUsageError(errorMessage(error));
    } finally {
      setUsageLoading(false);
    }
  }, [usageDateRange]);

  useEffect(() => {
    if (activeSection === "usage" && gatewayUsageEnabled && usageViewMode === "overview") {
      loadGatewayUsageOverview().catch(console.error);
    }
  }, [activeSection, gatewayUsageEnabled, loadGatewayUsageOverview, usageViewMode]);

  useEffect(() => {
    if (activeSection === "usage" && gatewayUsageEnabled && usageViewMode === "details") {
      loadGatewayUsage().catch(console.error);
    }
  }, [activeSection, gatewayUsageEnabled, loadGatewayUsage, usageViewMode]);

  const showToast = (status: ToastState["status"], message: string) => {
    const id = Date.now();
    setToast({ id, status, message });
    if (status !== "loading") {
      window.setTimeout(() => {
        setToast((current) => (current?.id === id ? null : current));
      }, 3200);
    }
  };

  const settingsDraftChanged = useMemo(() => {
    const currentTranscribeSettings = normalizeTranscribeSettings({
      transcribeBaseUrl,
      transcribeApiKey,
      transcribeModel,
    });
    const draftTranscribeSettings = normalizeTranscribeSettings({
      transcribeBaseUrl: draftTranscribeBaseUrl,
      transcribeApiKey: draftTranscribeApiKey,
      transcribeModel: draftTranscribeModel,
    });
    const draftGatewayConfigSignature = gatewayForm ? gatewayConfigFormSignatureOrNull(gatewayForm) : null;
    const gatewayChanged =
      gatewayEnabled &&
      gatewayForm !== null &&
      (draftGatewayConfigSignature === null ||
        savedGatewayConfigSignature === null ||
        draftGatewayConfigSignature !== savedGatewayConfigSignature);

    return (
      draftLanguage !== language ||
      draftAppearance !== appearance ||
      !extensionSettingsEqual(draftExtensions, extensions) ||
      !transcribeSettingsEqual(draftTranscribeSettings, currentTranscribeSettings) ||
      !savedBotConfigsEqual(draftBotConfigs, botConfigs) ||
      gatewayChanged
    );
  }, [
    appearance,
    botConfigs,
    draftAppearance,
    draftBotConfigs,
    draftExtensions,
    draftLanguage,
    draftTranscribeApiKey,
    draftTranscribeBaseUrl,
    draftTranscribeModel,
    extensions,
    gatewayEnabled,
    gatewayForm,
    language,
    savedGatewayConfigSignature,
    transcribeApiKey,
    transcribeBaseUrl,
    transcribeModel,
  ]);

  const settingsBusy = savingSettings || preparingExtensions || botSaving;
  const requestSettingsClose = () => {
    if (settingsBusy) {
      return;
    }
    if (settingsDraftChanged) {
      setDiscardSettingsConfirmOpen(true);
      return;
    }
    onClose();
  };

  const saveDraft = async () => {
    if (!settingsDraftChanged) {
      return;
    }

    setSavingSettings(true);
    showToast(
      "loading",
      draftExtensions.enabled && !extensions.enabled ? strings.preparingExtension : strings.saving,
    );
    try {
      const transcribeSettings = normalizeTranscribeSettings({
        transcribeBaseUrl: draftTranscribeBaseUrl,
        transcribeApiKey: draftTranscribeApiKey,
        transcribeModel: draftTranscribeModel,
      });
      if (transcribeSettings.transcribeBaseUrl && !isHttpUrl(transcribeSettings.transcribeBaseUrl)) {
        showToast("error", strings.invalidTranscribeApiUrl);
        return;
      }
      let nextSavedGatewayConfigSignature: string | null = null;
      if (gatewayForm && gatewayEnabled) {
        const nextGatewayConfig = gatewayConfigFromForm(gatewayForm);
        await invoke<GatewayConfigFile>("update_gateway_config", {
          config: nextGatewayConfig,
        });
        nextSavedGatewayConfigSignature = jsonSignature(nextGatewayConfig);
      }
      await onSave({
        appearance: draftAppearance,
        language: draftLanguage,
        extensions: draftExtensions,
        transcribeBaseUrl: transcribeSettings.transcribeBaseUrl,
        transcribeApiKey: transcribeSettings.transcribeApiKey,
        transcribeModel: transcribeSettings.transcribeModel,
        botConfigs: draftBotConfigs,
      });
      if (nextSavedGatewayConfigSignature !== null) {
        setSavedGatewayConfigSignature(nextSavedGatewayConfigSignature);
      }
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
      setPendingDeleteBotConfig(null);
      return;
    }
    setBotSaving(true);
    try {
      await persistBotConfigs(draftBotConfigs.filter((item) => item.id !== botConfig.id));
      setPendingDeleteBotConfig(null);
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
      : activeSection === "transcribe"
        ? strings.transcribe
      : activeSection === "profiles"
        ? strings.profiles
      : activeSection === "bot"
        ? strings.bot
      : activeSection === "gateway"
        ? strings.gateway
      : activeSection === "usage"
        ? strings.gatewayUsage
        : activeSection === "updates"
          ? strings.updates
          : strings.appSettingsTitle;
  const sectionDescription =
    activeSection === "extensions"
      ? strings.extensionSettingsDescription
      : activeSection === "transcribe"
        ? strings.transcribeSettingsDescription
      : activeSection === "profiles"
        ? strings.profileSettingsDescription
      : activeSection === "bot"
        ? strings.botSettingsDescription
      : activeSection === "gateway"
        ? strings.gatewaySettingsDescription
      : activeSection === "usage"
        ? strings.gatewayUsageSettingsDescription
        : activeSection === "updates"
          ? strings.updatesDescription
          : strings.appSettingsDescription;

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          requestSettingsClose();
        }
      }}
    >
      <DialogContent
        className={cn(
          "max-w-none gap-0 overflow-hidden p-0",
          usageFullscreenActive
            ? "h-[100dvh] w-[100dvw] grid-cols-[1fr] rounded-none border-0"
            : "h-[calc(100dvh-24px)] w-[calc(100dvw-24px)] grid-cols-[56px_minmax(0,1fr)] sm:h-[88dvh] sm:w-[92dvw] sm:grid-cols-[180px_minmax(0,1fr)] lg:h-[80vh] lg:w-[80vw] lg:grid-cols-[220px_minmax(0,1fr)]",
	        )}
	        closeLabel={strings.close}
	        showCloseButton={!settingsBusy}
	      >
        {!usageFullscreenActive ? (
        <aside className="flex min-h-0 flex-col border-r border-border bg-muted/20">
          <div className="border-b border-border px-2 py-4 sm:px-4 lg:px-5">
            <div className="flex items-center justify-center gap-2 text-base font-semibold sm:justify-start">
              <Settings className="h-4 w-4" />
              <span className="hidden min-w-0 truncate sm:inline">{strings.appSettingsTitle}</span>
            </div>
          </div>
          <nav className="flex-1 space-y-1 p-2 sm:p-3">
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
              active={activeSection === "profiles"}
              icon={<FileCog className="h-4 w-4" />}
              label={strings.profiles}
              onClick={() => setActiveSection("profiles")}
            />
            <SettingsNavButton
              active={activeSection === "transcribe"}
              icon={<Mic className="h-4 w-4" />}
              label={strings.transcribe}
              onClick={() => setActiveSection("transcribe")}
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
            {gatewayUsageEnabled ? (
              <SettingsNavButton
                active={activeSection === "usage"}
                icon={<LayoutDashboard className="h-4 w-4" />}
                label={strings.gatewayUsage}
                onClick={() => setActiveSection("usage")}
              />
            ) : null}
          </nav>
        </aside>
        ) : null}

        <section className="flex min-h-0 min-w-0 flex-col">
          <DialogHeader className="flex-row flex-wrap items-start justify-between gap-3 border-b border-border px-3 py-3 pr-14 sm:items-center sm:px-6 sm:py-4 sm:pr-16">
            <div className="min-w-0 flex-1 basis-full sm:basis-0">
              <DialogTitle className="text-base">{sectionTitle}</DialogTitle>
              <DialogDescription>{sectionDescription}</DialogDescription>
            </div>
            {activeSection === "extensions" ? (
              <div className="flex min-w-0 max-w-full items-center gap-2">
                {preparingExtensions ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                ) : null}
                <span className="min-w-0 text-sm text-muted-foreground">{strings.enableExtensions}</span>
                <Switch
                  checked={draftExtensions.enabled}
                  disabled={preparingExtensions || savingSettings}
                  aria-label={strings.enableExtensions}
                  onCheckedChange={(checked) => void handleExtensionsEnabledChange(checked === true)}
                />
              </div>
            ) : activeSection === "bot" ? (
              <Button
                type="button"
                size="sm"
                className="max-w-full"
                disabled={botSaving}
                onClick={() => setBotEditor({ mode: "add", config: null })}
              >
                <Plus className="h-4 w-4" />
                {strings.addBot}
              </Button>
            ) : activeSection === "usage" ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="max-w-full"
                onClick={() =>
                  setUsageViewMode((current) => (current === "overview" ? "details" : "overview"))
                }
              >
                {usageViewMode === "overview" ? (
                  <Activity className="h-4 w-4" />
                ) : (
                  <LayoutDashboard className="h-4 w-4" />
                )}
                {usageViewMode === "overview" ? strings.gatewayUsageDetails : strings.gatewayUsageOverview}
              </Button>
            ) : null}
          </DialogHeader>

          <div
            className={cn(
              "flex-1 overflow-auto px-3 py-4 sm:px-6 sm:py-6",
              usageFullscreenActive && "sm:px-8",
            )}
          >
            {activeSection === "general" ? (
              <div className="max-w-2xl space-y-7">
                <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-6">
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

                <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-6">
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
            ) : activeSection === "transcribe" ? (
              <div className="max-w-2xl space-y-7">
                <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-6">
                  <div className="flex items-center gap-2 pt-2 text-sm font-medium">
                    <Server className="h-4 w-4 text-muted-foreground" />
                    {strings.transcribeApiUrl}
                  </div>
                  <div className="grid gap-2">
                    <Input
                      id="transcribeApiUrl"
                      value={draftTranscribeBaseUrl}
                      placeholder="https://api.openai.com/v1"
                      onChange={(event) => setDraftTranscribeBaseUrl(event.target.value)}
                    />
                    <p className="text-xs leading-relaxed text-muted-foreground">
                      {strings.transcribeApiUrlDescription}
                    </p>
                  </div>
                </div>

                <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-6">
                  <div className="flex items-center gap-2 pt-2 text-sm font-medium">
                    <LockKeyhole className="h-4 w-4 text-muted-foreground" />
                    {strings.transcribeApiKey}
                  </div>
                  <div className="grid gap-2">
                    <Input
                      id="transcribeApiKey"
                      type="password"
                      value={draftTranscribeApiKey}
                      placeholder="sk-..."
                      onChange={(event) => setDraftTranscribeApiKey(event.target.value)}
                    />
                    <p className="text-xs leading-relaxed text-muted-foreground">
                      {strings.transcribeApiKeyDescription}
                    </p>
                  </div>
                </div>

                <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-6">
                  <div className="flex items-center gap-2 pt-2 text-sm font-medium">
                    <Mic className="h-4 w-4 text-muted-foreground" />
                    {strings.transcribeModel}
                  </div>
                  <div className="grid gap-2">
                    <Input
                      id="transcribeModel"
                      value={draftTranscribeModel}
                      placeholder={DEFAULT_TRANSCRIBE_MODEL}
                      onChange={(event) => setDraftTranscribeModel(event.target.value)}
                    />
                    <p className="text-xs leading-relaxed text-muted-foreground">
                      {strings.transcribeModelDescription}
                    </p>
                  </div>
                </div>
              </div>
            ) : activeSection === "profiles" ? (
              <ProfileSettingsPanel
                defaultProviders={defaultProviders}
                workspaceProfiles={profiles}
                strings={strings}
                onSaveProfile={onSaveDefaultProvider}
                onDeleteProfile={onDeleteDefaultProvider}
              />
            ) : activeSection === "bot" ? (
              <BotSettingsPanel
                botConfigs={draftBotConfigs}
                profiles={profiles}
                saving={botSaving}
                strings={strings}
                onEditBotConfig={(config) => setBotEditor({ mode: "edit", config })}
                onDeleteBotConfig={(config) => setPendingDeleteBotConfig(config)}
              />
            ) : activeSection === "gateway" ? (
              <GatewaySettingsPanel
                form={gatewayForm}
                error={gatewayError}
                strings={strings}
                onReload={() => loadGatewayConfig().catch(console.error)}
                onChange={setGatewayForm}
              />
            ) : activeSection === "usage" ? (
              <div className={cn(usageViewMode === "overview" ? "max-w-none" : "max-w-5xl", usageFullscreenActive && "max-w-none")}>
                {usageViewMode === "overview" ? (
                  <GatewayUsageOverviewDashboard
                    auth={remoteCloudAuth}
                    summary={usageOverviewSummary}
                    loading={usageOverviewLoading}
                    error={usageOverviewError || gatewayError}
                    strings={strings}
                    language={language}
                    onRefresh={() => loadGatewayUsageOverview().catch(console.error)}
                  />
                ) : (
                  <GatewayUsageDashboard
                    summary={usageSummary}
                    loading={usageLoading}
                    error={usageError || gatewayError}
                    strings={strings}
                    dateRange={usageDateRange}
                    onDateRangeChange={setUsageDateRange}
                    onPresetHours={(hours) => setUsageDateRange(gatewayUsageDateRangeForHours(hours))}
                    onPresetRange={(days) => setUsageDateRange(gatewayUsageDateRangeForDays(days))}
                    fullscreen={usageFullscreenActive}
                    onToggleFullscreen={() => setUsageFullscreen((current) => !current)}
                    onRefresh={() => loadGatewayUsage().catch(console.error)}
                  />
                )}
              </div>
            ) : (
              <AppUpdatePanel
                strings={strings}
                updateState={appUpdateState}
                onCheckForAppUpdate={onCheckForAppUpdate}
                onInstallAppUpdate={onInstallAppUpdate}
              />
            )}
          </div>

          {!usageFullscreenActive ? (
            <DialogFooter className="border-t border-border px-6 py-4">
              <Button
                type="button"
                disabled={settingsBusy || !settingsDraftChanged}
                onClick={saveDraft}
              >
                {savingSettings ? <RefreshCw className="h-4 w-4 animate-spin" /> : null}
                {strings.save}
              </Button>
            </DialogFooter>
          ) : null}
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
        {pendingDeleteBotConfig ? (
          <AlertDialog
            open
            onOpenChange={(open) => {
              if (!open && !botSaving) {
                setPendingDeleteBotConfig(null);
              }
            }}
          >
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{strings.deleteBot}</AlertDialogTitle>
                <AlertDialogDescription>
                  {strings.deleteBotConfirm(pendingDeleteBotConfig.name || strings.bot)}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel disabled={botSaving} onClick={() => setPendingDeleteBotConfig(null)}>
                  {strings.cancel}
                </AlertDialogCancel>
                <Button
                  type="button"
                  variant="destructive"
                  disabled={botSaving}
                  onClick={() => deleteBotConfig(pendingDeleteBotConfig).catch(console.error)}
                >
                  {botSaving ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : null}
                  {botSaving ? strings.deleting : strings.delete}
                </Button>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        ) : null}
        <AlertDialog open={discardSettingsConfirmOpen} onOpenChange={setDiscardSettingsConfirmOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{strings.discardSettingsChangesTitle}</AlertDialogTitle>
              <AlertDialogDescription>{strings.discardSettingsChangesDescription}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{strings.cancel}</AlertDialogCancel>
              <Button
                type="button"
                variant="destructive"
                onClick={() => {
                  setDiscardSettingsConfirmOpen(false);
                  onClose();
                }}
              >
                {strings.discardChanges}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
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
        "flex h-10 w-full items-center justify-center gap-2 rounded-md px-2 text-left text-sm font-medium sm:justify-start sm:px-3",
        active
          ? "bg-secondary text-secondary-foreground"
          : "text-muted-foreground hover:bg-muted hover:text-foreground",
      )}
      title={label}
      onClick={onClick}
    >
      {icon}
      <span className="hidden min-w-0 truncate sm:inline">{label}</span>
    </button>
  );
}

function ProfileSettingsPanel({
  defaultProviders,
  workspaceProfiles,
  strings,
  onSaveProfile,
  onDeleteProfile,
}: {
  defaultProviders: DefaultProviderProfile[];
  workspaceProfiles: ProviderProfile[];
  strings: AppStrings;
  onSaveProfile: (profile: DefaultProviderProfile) => Promise<AppConfig>;
  onDeleteProfile: (name: string) => Promise<AppConfig>;
}) {
  const profiles = useMemo(
    () => profileManagementDefaultProviders(defaultProviders),
    [defaultProviders],
  );
  const [dialog, setDialog] = useState<DefaultProviderDialogState | null>(null);
  const [pendingDeleteProfileConfig, setPendingDeleteProfileConfig] =
    useState<DefaultProviderProfile | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [discardProfileDialogConfirmOpen, setDiscardProfileDialogConfirmOpen] = useState(false);
  const profileModelOptions = useProviderModelProbe(
    dialog?.profile.base_url ?? "",
    dialog?.profile.api_key ?? "",
    dialog !== null,
    dialog?.profile.provider_name ?? "",
  );
  const profileDialogDirty =
    dialog !== null &&
    defaultProviderProfileDraftSignature(dialog.profile) !== dialog.initialSignature;

  const openAddProfileDialog = () => {
    const profile = createDefaultProviderProfileForm();
    setError("");
    setShowApiKey(false);
    setDiscardProfileDialogConfirmOpen(false);
    setDialog({
      mode: "add",
      profile,
      initialSignature: defaultProviderProfileDraftSignature(profile),
    });
  };

  const openEditProfileDialog = (profile: DefaultProviderProfile) => {
    const nextProfile = cloneDefaultProviderProfile(profile);
    setError("");
    setShowApiKey(false);
    setDiscardProfileDialogConfirmOpen(false);
    setDialog({
      mode: "edit",
      profile: nextProfile,
      initialSignature: defaultProviderProfileDraftSignature(nextProfile),
    });
  };

  const closeProfileDialog = () => {
    setShowApiKey(false);
    setDiscardProfileDialogConfirmOpen(false);
    setDialog(null);
    setError("");
  };

  const requestCloseProfileDialog = () => {
    if (saving) {
      return;
    }
    if (profileDialogDirty) {
      setDiscardProfileDialogConfirmOpen(true);
      return;
    }
    closeProfileDialog();
  };

  const updateDialogProfile = (patch: Partial<DefaultProviderProfile>) =>
    setDialog((current) =>
      current
        ? {
            ...current,
            profile: { ...current.profile, ...patch },
          }
        : current,
    );

  const saveDialogProfile = async () => {
    if (!dialog) return;
    setSaving(true);
    setError("");
    try {
      await onSaveProfile(normalizeDefaultProviderProfileForm(dialog.profile));
      closeProfileDialog();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  const confirmDeleteProfileConfig = async () => {
    if (!pendingDeleteProfileConfig) return;
    const linkedWorkspace = workspaceProfilesUsingDefaultProvider(pendingDeleteProfileConfig, workspaceProfiles)[0];
    if (linkedWorkspace) {
      setError(strings.profileUsedByWorkspace(linkedWorkspace.name));
      setPendingDeleteProfileConfig(null);
      return;
    }
    setSaving(true);
    setError("");
    try {
      await onDeleteProfile(pendingDeleteProfileConfig.name);
      setPendingDeleteProfileConfig(null);
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="max-w-5xl space-y-4">
      {error ? (
        <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
          {error}
        </p>
      ) : null}

      <div className="flex items-center justify-between gap-3">
        <SectionTitle icon={<FileCog className="h-4 w-4" />} title={strings.profiles} />
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={saving}
          onClick={openAddProfileDialog}
        >
          <Plus className="h-4 w-4" />
          {strings.addProfileConfig}
        </Button>
      </div>

      <div className="space-y-3">
        {profiles.length > 0 ? (
          profiles.map((profile) => {
            const linkedWorkspace = workspaceProfilesUsingDefaultProvider(profile, workspaceProfiles)[0];
            const deleteDisabled = Boolean(linkedWorkspace);
            return (
              <div key={profile.name} className="rounded-md border border-border bg-muted/10 px-3 py-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,0.8fr)_minmax(0,0.8fr)_minmax(0,1fr)_minmax(0,1.2fr)_100px]">
                    <GatewayProviderSummaryField label={strings.providerProfileName} value={profile.name} />
                    <GatewayProviderSummaryField label={strings.provider} value={profile.provider_name || strings.none} />
                    <GatewayProviderSummaryField label={strings.model} value={profile.model || strings.none} />
                    <GatewayProviderSummaryField label={strings.baseUrl} value={profile.base_url || strings.none} />
                    <GatewayProviderSummaryField
                      label={strings.apiKey}
                      value={profile.api_key ? strings.ready : strings.notConfigured}
                    />
                  </div>
                  <div className="flex items-center justify-end gap-2">
                    <IconButton
                      title={strings.editProfileConfig}
                      disabled={saving}
                      onClick={() => openEditProfileDialog(profile)}
                    >
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={
                        linkedWorkspace
                          ? strings.profileUsedByWorkspace(linkedWorkspace.name)
                          : strings.delete
                      }
                      tooltip={
                        linkedWorkspace
                          ? strings.profileUsedByWorkspace(linkedWorkspace.name)
                          : undefined
                      }
                      disabled={deleteDisabled || saving}
                      className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
                      onClick={() => {
                        setError("");
                        setPendingDeleteProfileConfig(profile);
                      }}
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
            {strings.noProfileConfigs}
          </div>
        )}
      </div>

      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => {
          if (!open) {
            requestCloseProfileDialog();
          }
        }}
      >
        {dialog ? (
          <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto" closeLabel={strings.close} showCloseButton={!saving}>
            <DialogHeader>
              <DialogTitle>
                {dialog.mode === "add" ? strings.addProfileConfig : strings.editProfileConfig}
              </DialogTitle>
              <DialogDescription className="sr-only">{strings.profileSettingsDescription}</DialogDescription>
            </DialogHeader>
            <fieldset className="grid gap-4" disabled={saving}>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="defaultProfileNameInput">{strings.providerProfileName}</Label>
                  <Input
                    id="defaultProfileNameInput"
                    value={dialog.profile.name}
                    disabled={dialog.mode === "edit"}
                    placeholder="my-profile"
                    onChange={(event) => updateDialogProfile({ name: event.target.value })}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="defaultProfileProviderInput">{strings.provider}</Label>
                  <Input
                    id="defaultProfileProviderInput"
                    value={dialog.profile.provider_name}
                    placeholder="openai"
                    onChange={(event) => updateDialogProfile({ provider_name: event.target.value })}
                  />
                </div>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="defaultProfileBaseUrlInput">{strings.baseUrl}</Label>
                  <Input
                    id="defaultProfileBaseUrlInput"
                    value={dialog.profile.base_url}
                    placeholder="https://api.example.com/v1"
                    onChange={(event) => updateDialogProfile({ base_url: event.target.value })}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="defaultProfileApiKeyInput">{strings.apiKey}</Label>
                  <div className="relative">
                    <Input
                      id="defaultProfileApiKeyInput"
                      type={showApiKey ? "text" : "password"}
                      className="pr-10"
                      value={dialog.profile.api_key}
                      placeholder="sk-..."
                      onChange={(event) => updateDialogProfile({ api_key: event.target.value })}
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                      title={showApiKey ? strings.hidePassword : strings.showPassword}
                      aria-label={showApiKey ? strings.hidePassword : strings.showPassword}
                      onClick={() => setShowApiKey((current) => !current)}
                    >
                      {showApiKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                    </Button>
                  </div>
                </div>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="defaultProfileModelInput">{strings.model}</Label>
                <div className="relative">
                  <Input
                    id="defaultProfileModelInput"
                    className={profileModelOptions.length > 0 ? "pr-10" : undefined}
                    value={dialog.profile.model}
                    placeholder="gpt-5.5"
                    onChange={(event) => updateDialogProfile({ model: event.target.value })}
                  />
                  <ModelOptionsDropdown
                    options={profileModelOptions}
                    selectedValues={[dialog.profile.model]}
                    strings={strings}
                    onSelect={(model) => updateDialogProfile({ model })}
                  />
                </div>
              </div>
            </fieldset>
            <DialogFooter>
              <Button type="button" variant="outline" disabled={saving} onClick={requestCloseProfileDialog}>
                {strings.cancel}
              </Button>
              <Button type="button" disabled={saving} onClick={() => saveDialogProfile().catch(console.error)}>
                {saving ? <RefreshCw className="h-4 w-4 animate-spin" /> : null}
                {strings.save}
              </Button>
            </DialogFooter>
          </DialogContent>
        ) : null}
      </Dialog>
      <AlertDialog open={discardProfileDialogConfirmOpen} onOpenChange={setDiscardProfileDialogConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{strings.discardSettingsChangesTitle}</AlertDialogTitle>
            <AlertDialogDescription>{strings.discardSettingsChangesDescription}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{strings.cancel}</AlertDialogCancel>
            <Button type="button" variant="destructive" onClick={closeProfileDialog}>
              {strings.discardChanges}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {pendingDeleteProfileConfig ? (
        <AlertDialog
          open
          onOpenChange={(open) => {
            if (!open && !saving) {
              setPendingDeleteProfileConfig(null);
              setError("");
            }
          }}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{strings.deleteProfileConfig}</AlertDialogTitle>
              <AlertDialogDescription>
                {strings.deleteProfileConfigConfirm(pendingDeleteProfileConfig.name)}
              </AlertDialogDescription>
            </AlertDialogHeader>
            {error ? (
              <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
                {error}
              </p>
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel
                disabled={saving}
                onClick={() => {
                  setPendingDeleteProfileConfig(null);
                  setError("");
                }}
              >
                {strings.cancel}
              </AlertDialogCancel>
              <Button
                type="button"
                variant="destructive"
                disabled={saving}
                onClick={() => confirmDeleteProfileConfig().catch(console.error)}
              >
                {saving ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : null}
                {saving ? strings.deleting : strings.delete}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </div>
  );
}

function BotSettingsPanel({
  botConfigs,
  profiles,
  saving,
  strings,
  onEditBotConfig,
  onDeleteBotConfig,
}: {
  botConfigs: SavedBotConfig[];
  profiles: ProviderProfile[];
  saving: boolean;
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
                    <IconButton title={strings.editBot} disabled={saving} onClick={() => onEditBotConfig(config)}>
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={deleteDisabled ? strings.botLinkedToWorkspace : strings.deleteBot}
                      tooltip={deleteDisabled ? strings.botLinkedToWorkspace : undefined}
                      disabled={saving || deleteDisabled}
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
  const initialFormSignatureRef = useRef(botConfigDraftSignature(botConfigFormFields(config)));
  const [form, setForm] = useState<ProviderForm>(() => botConfigFormFields(config));
  const [error, setError] = useState("");
  const [discardConfirmOpen, setDiscardConfirmOpen] = useState(false);
  const botAuthType = normalizeBotAuthType(form.botPlatform, form.botAuthType);
  const botAuthSpecs = authSpecsForPlatform(form.botPlatform);
  const botAuthFields = fieldsForBotAuth(form.botPlatform, botAuthType);
  const dirty = botConfigDraftSignature(form) !== initialFormSignatureRef.current;

  const showError = (nextError: unknown) => setError(errorMessage(nextError));
  const requestClose = () => {
    if (saving) {
      return;
    }
    if (dirty) {
      setDiscardConfirmOpen(true);
      return;
    }
    onClose();
  };

  const save = async () => {
    setError("");
    const nextConfig = readSavedBotConfigForm(form, config, nameRef, strings, showError);
    if (!nextConfig) {
      return;
    }
    await onSave(nextConfig);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && requestClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto" closeLabel={strings.close} showCloseButton={!saving}>
        <DialogHeader>
          <DialogTitle>{mode === "add" ? strings.addBot : strings.editBot}</DialogTitle>
          <DialogDescription className="sr-only">{strings.botSettingsDescription}</DialogDescription>
        </DialogHeader>
        {error ? (
          <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
            {error}
          </p>
        ) : null}
        <fieldset className="grid gap-4" disabled={saving}>
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
                disabled={saving}
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
                  disabled={saving}
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
        </fieldset>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={saving} onClick={requestClose}>
            {strings.cancel}
          </Button>
          <Button type="button" disabled={saving} onClick={() => save().catch(showError)}>
            {saving ? <RefreshCw className="h-4 w-4 animate-spin" /> : null}
            {strings.save}
          </Button>
        </DialogFooter>
        <AlertDialog open={discardConfirmOpen} onOpenChange={setDiscardConfirmOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{strings.discardSettingsChangesTitle}</AlertDialogTitle>
              <AlertDialogDescription>{strings.discardSettingsChangesDescription}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{strings.cancel}</AlertDialogCancel>
              <Button
                type="button"
                variant="destructive"
                onClick={() => {
                  setDiscardConfirmOpen(false);
                  onClose();
                }}
              >
                {strings.discardChanges}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
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
            <div className="grid grid-cols-1 gap-1 sm:grid-cols-[140px_minmax(0,1fr)] sm:gap-3">
              <span className="text-muted-foreground">{strings.updateCurrentVersion}</span>
              <span className="font-medium">{update.currentVersion}</span>
            </div>
            <div className="grid grid-cols-1 gap-1 sm:grid-cols-[140px_minmax(0,1fr)] sm:gap-3">
              <span className="text-muted-foreground">{strings.updateNewVersion}</span>
              <span className="font-medium">{update.version}</span>
            </div>
            {update.date ? (
              <div className="grid grid-cols-1 gap-1 sm:grid-cols-[140px_minmax(0,1fr)] sm:gap-3">
                <span className="text-muted-foreground">{strings.updatePublishedAt}</span>
                <span>{update.date}</span>
              </div>
            ) : null}
            {update.body ? (
              <div className="grid grid-cols-1 gap-1 sm:grid-cols-[140px_minmax(0,1fr)] sm:gap-3">
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
  const [mcpServerDialog, setMcpServerDialog] = useState<GatewayMcpServerDialogState | null>(null);
  const [virtualProfileDialog, setVirtualProfileDialog] =
    useState<GatewayVirtualProfileDialogState | null>(null);
  const [pendingGatewayDialogClose, setPendingGatewayDialogClose] =
    useState<null | "provider" | "mcp" | "virtual">(null);
  const [activeGatewayTab, setActiveGatewayTab] = useState<GatewaySettingsTab>("settings");
  const [availableTools, setAvailableTools] = useState<GatewayAvailableTool[]>([]);
  const [availableToolsLoading, setAvailableToolsLoading] = useState(false);
  const [availableToolsError, setAvailableToolsError] = useState("");
  const [availableToolsLoaded, setAvailableToolsLoaded] = useState(false);

  const loadGatewayTools = useCallback(async () => {
    setAvailableToolsLoading(true);
    setAvailableToolsError("");
    try {
      const response = await invoke<GatewayToolsResponse>("get_gateway_tools");
      setAvailableTools(gatewayAvailableToolsFromResponse(response));
      setAvailableToolsLoaded(true);
    } catch (error) {
      setAvailableToolsError(errorMessage(error));
      setAvailableTools([]);
      setAvailableToolsLoaded(true);
    } finally {
      setAvailableToolsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (activeGatewayTab === "tools" && !availableToolsLoaded && !availableToolsLoading) {
      loadGatewayTools().catch(console.error);
    }
  }, [activeGatewayTab, availableToolsLoaded, availableToolsLoading, loadGatewayTools]);

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
  const openAddProviderDialog = () => {
    const provider = createGatewayProviderForm();
    setProviderDialog({
      mode: "add",
      provider,
      initialSignature: gatewayProviderDraftSignature(provider),
    });
  };
  const openEditProviderDialog = (provider: GatewayProviderForm) => {
    const nextProvider = cloneGatewayProviderForm(provider);
    setProviderDialog({
      mode: "edit",
      provider: nextProvider,
      initialSignature: gatewayProviderDraftSignature(nextProvider),
    });
  };
  const updateDialogProvider = (patch: Partial<GatewayProviderForm>) =>
    setProviderDialog((current) =>
      current
        ? {
            ...current,
            provider: { ...current.provider, ...patch },
          }
        : current,
    );
  const requestDeleteProvider = (provider: GatewayProviderForm) => {
    update({ providers: form.providers.filter((item) => item.id !== provider.id) });
  };
  const openAddMcpServerDialog = () => {
    const server = createGatewayMcpServerForm();
    setMcpServerDialog({
      mode: "add",
      server,
      initialSignature: gatewayMcpServerDraftSignature(server),
    });
  };
  const openEditMcpServerDialog = (server: GatewayMcpServerForm) => {
    const nextServer = cloneGatewayMcpServerForm(server);
    setMcpServerDialog({
      mode: "edit",
      server: nextServer,
      initialSignature: gatewayMcpServerDraftSignature(nextServer),
    });
  };
  const updateDialogMcpServer = (patch: Partial<GatewayMcpServerForm>) =>
    setMcpServerDialog((current) =>
      current
        ? {
            ...current,
            server: { ...current.server, ...patch },
          }
        : current,
    );
  const requestDeleteMcpServer = (server: GatewayMcpServerForm) => {
    setAvailableToolsLoaded(false);
    update({ mcpServers: form.mcpServers.filter((item) => item.id !== server.id) });
  };
  const openAddVirtualProfileDialog = () => {
    const profile = createGatewayVirtualProfileForm(availableTools);
    setVirtualProfileDialog({
      mode: "add",
      profile,
      initialSignature: gatewayVirtualProfileDraftSignature(profile),
    });
  };
  const openEditVirtualProfileDialog = (profile: GatewayVirtualProfileForm) => {
    const nextProfile = cloneGatewayVirtualProfileForm(profile);
    setVirtualProfileDialog({
      mode: "edit",
      profile: nextProfile,
      initialSignature: gatewayVirtualProfileDraftSignature(nextProfile),
    });
  };
  const updateDialogVirtualProfile = (patch: Partial<GatewayVirtualProfileForm>) =>
    setVirtualProfileDialog((current) =>
      current
        ? {
            ...current,
            profile: { ...current.profile, ...patch },
          }
        : current,
    );
  const requestDeleteVirtualProfile = (profile: GatewayVirtualProfileForm) => {
    update({
      virtualModelProfiles: form.virtualModelProfiles.filter((item) => item.id !== profile.id),
    });
  };
  const providerDialogError = providerDialog
    ? gatewayProviderDialogError(providerDialog.provider, strings)
    : "";
  const mcpServerDialogError = mcpServerDialog
    ? gatewayMcpServerDialogError(mcpServerDialog.server, strings)
    : "";
  const virtualProfileDialogError = virtualProfileDialog
    ? gatewayVirtualProfileDialogError(virtualProfileDialog.profile, strings)
    : "";
  const providerDialogDirty =
    providerDialog !== null &&
    gatewayProviderDraftSignature(providerDialog.provider) !== providerDialog.initialSignature;
  const mcpServerDialogDirty =
    mcpServerDialog !== null &&
    gatewayMcpServerDraftSignature(mcpServerDialog.server) !== mcpServerDialog.initialSignature;
  const virtualProfileDialogDirty =
    virtualProfileDialog !== null &&
    gatewayVirtualProfileDraftSignature(virtualProfileDialog.profile) !==
      virtualProfileDialog.initialSignature;
  const requestCloseGatewayProviderDialog = () => {
    if (providerDialogDirty) {
      setPendingGatewayDialogClose("provider");
      return;
    }
    setProviderDialog(null);
  };
  const requestCloseGatewayMcpServerDialog = () => {
    if (mcpServerDialogDirty) {
      setPendingGatewayDialogClose("mcp");
      return;
    }
    setMcpServerDialog(null);
  };
  const requestCloseGatewayVirtualProfileDialog = () => {
    if (virtualProfileDialogDirty) {
      setPendingGatewayDialogClose("virtual");
      return;
    }
    setVirtualProfileDialog(null);
  };
  const discardPendingGatewayDialogChanges = () => {
    if (pendingGatewayDialogClose === "provider") {
      setProviderDialog(null);
    } else if (pendingGatewayDialogClose === "mcp") {
      setMcpServerDialog(null);
    } else if (pendingGatewayDialogClose === "virtual") {
      setVirtualProfileDialog(null);
    }
    setPendingGatewayDialogClose(null);
  };
  const saveProviderDialog = () => {
    if (!providerDialog) return;
    if (providerDialogError) return;

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
  const saveMcpServerDialog = () => {
    if (!mcpServerDialog) return;
    if (mcpServerDialogError) return;

    const nextServer = cloneGatewayMcpServerForm(mcpServerDialog.server);
    onChange((current) => {
      if (!current) return current;

      if (mcpServerDialog.mode === "add") {
        return {
          ...current,
          mcpServers: [...current.mcpServers, nextServer],
        };
      }

      return {
        ...current,
        mcpServers: current.mcpServers.map((server) =>
          server.id === nextServer.id ? nextServer : server,
        ),
      };
    });
    setAvailableToolsLoaded(false);
    setMcpServerDialog(null);
  };
  const saveVirtualProfileDialog = () => {
    if (!virtualProfileDialog) return;
    if (virtualProfileDialogError) return;

    const nextProfile = cloneGatewayVirtualProfileForm(virtualProfileDialog.profile);
    onChange((current) => {
      if (!current) return current;

      if (virtualProfileDialog.mode === "add") {
        return {
          ...current,
          virtualModelProfiles: [...current.virtualModelProfiles, nextProfile],
        };
      }

      return {
        ...current,
        virtualModelProfiles: current.virtualModelProfiles.map((profile) =>
          profile.id === nextProfile.id ? nextProfile : profile,
        ),
      };
    });
    setVirtualProfileDialog(null);
  };
  const gatewayTabs: Array<{
    value: GatewaySettingsTab;
    label: string;
    icon: React.ReactNode;
  }> = [
    { value: "settings", label: strings.settings, icon: <Settings className="h-4 w-4" /> },
    { value: "providers", label: strings.providers, icon: <Cpu className="h-4 w-4" /> },
    { value: "mcp", label: strings.mcpServers, icon: <Server className="h-4 w-4" /> },
    { value: "tools", label: strings.toolInjection, icon: <Wrench className="h-4 w-4" /> },
  ];

  return (
    <div className="max-w-5xl space-y-6">
      {error ? (
        <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
          {error}
        </p>
      ) : null}

      <div
        className="inline-flex max-w-full flex-wrap gap-1 rounded-md border border-border bg-muted/10 p-1"
        role="tablist"
        aria-label={strings.gateway}
      >
        {gatewayTabs.map((tab) => {
          const active = activeGatewayTab === tab.value;
          return (
            <button
              key={tab.value}
              type="button"
              role="tab"
              aria-selected={active}
              className={cn(
                "inline-flex h-9 items-center gap-2 rounded px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
                active && "bg-background text-foreground shadow-xs",
              )}
              onClick={() => setActiveGatewayTab(tab.value)}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          );
        })}
      </div>

      {activeGatewayTab === "settings" ? (
        <section className="space-y-5">
          <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
            <Field label="Host">
              <Input value={form.host} onChange={(event) => update({ host: event.target.value })} />
            </Field>
            <Field label={strings.port}>
              <Input
                value={form.port}
                inputMode="numeric"
                onChange={(event) => update({ port: event.target.value })}
              />
            </Field>
          </div>

          <div className="overflow-hidden rounded-md border border-border bg-muted/10">
            <div className="flex flex-wrap items-center justify-between gap-4 px-3 py-3">
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium">{strings.gatewayUsageCapture}</div>
                <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                  {strings.gatewayUsageCaptureDescription}
                </p>
              </div>
              <Switch
                checked={form.usageCaptureEnabled}
                aria-label={strings.gatewayUsageCapture}
                onCheckedChange={(checked) => update({ usageCaptureEnabled: checked === true })}
              />
            </div>
            <div className="flex flex-wrap items-center justify-between gap-4 border-t border-border px-3 py-3">
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium">{strings.gatewayRequestLogging}</div>
                <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                  {strings.gatewayRequestLoggingDescription}
                </p>
              </div>
              <Switch
                checked={form.requestLoggingEnabled}
                aria-label={strings.gatewayRequestLogging}
                onCheckedChange={(checked) => update({ requestLoggingEnabled: checked === true })}
              />
            </div>
          </div>
        </section>
      ) : null}

      {activeGatewayTab === "providers" ? (
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
                      onClick={() => requestDeleteProvider(provider)}
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
      ) : null}

      {activeGatewayTab === "mcp" ? (
        <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <SectionTitle icon={<Server className="h-4 w-4" />} title={strings.mcpServers} />
          <Button type="button" variant="outline" size="sm" onClick={openAddMcpServerDialog}>
            <Plus className="h-4 w-4" />
            {strings.addMcpServer}
          </Button>
        </div>
        <div className="space-y-3">
          {form.mcpServers.length > 0 ? (
            form.mcpServers.map((server) => (
              <div key={server.id} className="rounded-md border border-border bg-muted/10 px-3 py-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,0.8fr)_88px_120px_minmax(0,1.4fr)]">
                    <GatewayProviderSummaryField label={strings.name} value={server.name || strings.none} />
                    <GatewayProviderSummaryField
                      label={strings.enabled}
                      value={server.enabled ? strings.enabled : strings.disabled}
                    />
                    <GatewayProviderSummaryField label={strings.transport} value={server.transport} />
                    <GatewayProviderSummaryField
                      label={server.transport === "websocket" ? strings.url : strings.command}
                      value={gatewayMcpServerTarget(server) || strings.none}
                    />
                  </div>
                  <div className="flex items-center justify-end gap-2">
                    <Switch
                      checked={server.enabled}
                      aria-label={`${strings.enabled}: ${server.name || strings.mcpServers}`}
                      onCheckedChange={(checked) => {
                        setAvailableToolsLoaded(false);
                        update({
                          mcpServers: form.mcpServers.map((item) =>
                            item.id === server.id ? { ...item, enabled: checked === true } : item,
                          ),
                        });
                      }}
                    />
                    <IconButton title={strings.editMcpServer} onClick={() => openEditMcpServerDialog(server)}>
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={strings.delete}
                      className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
                      onClick={() => requestDeleteMcpServer(server)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </div>
              </div>
            ))
          ) : (
            <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
              {strings.none}
            </div>
          )}
        </div>
        </section>
      ) : null}

      {activeGatewayTab === "tools" ? (
        <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <SectionTitle icon={<Wrench className="h-4 w-4" />} title={strings.toolInjection} />
          <Button type="button" variant="outline" size="sm" onClick={openAddVirtualProfileDialog}>
            <Plus className="h-4 w-4" />
            {strings.addVirtualProfile}
          </Button>
        </div>
        <div className="space-y-3">
          {form.virtualModelProfiles.length > 0 ? (
            form.virtualModelProfiles.map((profile) => (
              <div key={profile.id} className="rounded-md border border-border bg-muted/10 px-3 py-3">
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center">
                  <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)_minmax(0,1fr)]">
                    <GatewayProviderSummaryField
                      label={strings.displayName}
                      value={profile.displayName || strings.none}
                    />
                    <GatewayProviderSummaryField
                      label={strings.suffixes}
                      value={profile.suffixes || profile.prefixes || profile.exactAliases || strings.none}
                    />
                    <GatewayProviderSummaryField
                      label={strings.tools}
                      value={profile.tools.map((tool) => tool.name).filter(Boolean).join(", ") || strings.none}
                    />
                  </div>
                  <div className="flex items-center justify-end gap-2">
                    <IconButton
                      title={strings.editVirtualProfile}
                      onClick={() => openEditVirtualProfileDialog(profile)}
                    >
                      <Pencil className="h-4 w-4" />
                    </IconButton>
                    <IconButton
                      title={strings.delete}
                      className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive hover:border-destructive/30"
                      onClick={() => requestDeleteVirtualProfile(profile)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                </div>
              </div>
            ))
          ) : (
            <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
              {strings.none}
            </div>
          )}
        </div>
        </section>
      ) : null}

      <Dialog open={providerDialog !== null} onOpenChange={(open) => !open && requestCloseGatewayProviderDialog()}>
        {providerDialog ? (
          <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto" closeLabel={strings.close}>
            <DialogHeader>
              <DialogTitle>{providerDialog.mode === "add" ? strings.addProvider : strings.editProvider}</DialogTitle>
              <DialogDescription className="sr-only">{strings.providerDialogDescription}</DialogDescription>
            </DialogHeader>
            <GatewayProviderEditor
              provider={providerDialog.provider}
              strings={strings}
              onChange={updateDialogProvider}
            />
            {providerDialogError ? <GatewayDialogValidationMessage message={providerDialogError} /> : null}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={requestCloseGatewayProviderDialog}>
                {strings.cancel}
              </Button>
              <Button type="button" disabled={Boolean(providerDialogError)} onClick={saveProviderDialog}>
                {strings.save}
              </Button>
            </DialogFooter>
          </DialogContent>
        ) : null}
      </Dialog>

      <Dialog open={mcpServerDialog !== null} onOpenChange={(open) => !open && requestCloseGatewayMcpServerDialog()}>
        {mcpServerDialog ? (
          <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto" closeLabel={strings.close}>
            <DialogHeader>
              <DialogTitle>
                {mcpServerDialog.mode === "add" ? strings.addMcpServer : strings.editMcpServer}
              </DialogTitle>
              <DialogDescription className="sr-only">{strings.mcpServerDialogDescription}</DialogDescription>
            </DialogHeader>
            <GatewayMcpServerEditor
              server={mcpServerDialog.server}
              strings={strings}
              onChange={updateDialogMcpServer}
            />
            {mcpServerDialogError ? <GatewayDialogValidationMessage message={mcpServerDialogError} /> : null}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={requestCloseGatewayMcpServerDialog}>
                {strings.cancel}
              </Button>
              <Button type="button" disabled={Boolean(mcpServerDialogError)} onClick={saveMcpServerDialog}>
                {strings.save}
              </Button>
            </DialogFooter>
          </DialogContent>
        ) : null}
      </Dialog>

      <Dialog
        open={virtualProfileDialog !== null}
        onOpenChange={(open) => !open && requestCloseGatewayVirtualProfileDialog()}
      >
        {virtualProfileDialog ? (
          <DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto" closeLabel={strings.close}>
            <DialogHeader>
              <DialogTitle>
                {virtualProfileDialog.mode === "add"
                  ? strings.addVirtualProfile
                  : strings.editVirtualProfile}
              </DialogTitle>
              <DialogDescription className="sr-only">{strings.virtualProfileDialogDescription}</DialogDescription>
            </DialogHeader>
            <GatewayVirtualProfileEditor
              profile={virtualProfileDialog.profile}
              availableTools={availableTools}
              availableToolsLoading={availableToolsLoading}
              availableToolsError={availableToolsError}
              strings={strings}
              onChange={updateDialogVirtualProfile}
              onRefreshTools={loadGatewayTools}
            />
            {virtualProfileDialogError ? <GatewayDialogValidationMessage message={virtualProfileDialogError} /> : null}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={requestCloseGatewayVirtualProfileDialog}>
                {strings.cancel}
              </Button>
              <Button type="button" disabled={Boolean(virtualProfileDialogError)} onClick={saveVirtualProfileDialog}>
                {strings.save}
              </Button>
            </DialogFooter>
          </DialogContent>
        ) : null}
      </Dialog>
      <AlertDialog
        open={pendingGatewayDialogClose !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPendingGatewayDialogClose(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{strings.discardSettingsChangesTitle}</AlertDialogTitle>
            <AlertDialogDescription>{strings.discardSettingsChangesDescription}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{strings.cancel}</AlertDialogCancel>
            <Button type="button" variant="destructive" onClick={discardPendingGatewayDialogChanges}>
              {strings.discardChanges}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function GatewayUsageOverviewDashboard({
  auth,
  summary,
  loading,
  error,
  strings,
  language,
  onRefresh,
}: {
  auth: RemoteCloudAuthConfig;
  summary: GatewayUsageSummary | null;
  loading: boolean;
  error: string;
  strings: AppStrings;
  language: Language;
  onRefresh: () => void;
}) {
  const [activityMode, setActivityMode] = useState<GatewayUsageOverviewMode>("daily");
  const signedIn = hasRemoteCloudIdentity(auth);
  const displayName = remoteCloudDisplayName(auth);
  const avatarUrl = signedIn ? remoteCloudAvatarUrl(auth) : "";
  const isPro = signedIn && Boolean(auth.is_pro);
  const metrics = useMemo(() => gatewayUsageOverviewMetrics(summary), [summary]);
  const hasUsage = metrics.lifetimeTokens > 0;

  const metricItems = [
    {
      label: strings.gatewayUsageLifetimeTokens,
      value: formatTokenCount(metrics.lifetimeTokens),
    },
    {
      label: strings.gatewayUsagePeakTokens,
      value: formatTokenCount(metrics.peakTokens),
    },
    {
      label: strings.gatewayUsageLongestTask,
      value: formatDurationCompact(metrics.longestTaskSeconds),
    },
    {
      label: strings.gatewayUsageCurrentStreak,
      value: formatUsageDays(metrics.currentStreakDays, strings),
    },
    {
      label: strings.gatewayUsageLongestStreak,
      value: formatUsageDays(metrics.longestStreakDays, strings),
    },
  ];

  return (
    <section
      className={cn(
        "mx-auto flex min-h-[min(760px,calc(100dvh-220px))] max-w-6xl flex-col items-center px-1 pb-8",
        signedIn ? "pt-8 sm:pt-12" : "pt-2 sm:pt-4",
      )}
    >
      {signedIn ? (
        <div className="flex flex-col items-center text-center">
          <GatewayUsageOverviewAvatar label={displayName} avatarUrl={avatarUrl} />
          <div className="mt-6 flex max-w-full flex-wrap items-center justify-center gap-2">
            <h2 className="max-w-[min(28rem,100%)] truncate text-3xl font-semibold tracking-normal text-foreground">
              {displayName}
            </h2>
            {isPro ? (
              <Badge className="account-pro-badge h-6 rounded-full px-2 text-xs font-medium">
                {strings.pro}
              </Badge>
            ) : null}
          </div>
        </div>
      ) : null}

      {error ? (
        <p className="mt-8 w-full max-w-4xl rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
          {error}
        </p>
      ) : null}

      <div
        className={cn(
          "grid w-full max-w-4xl overflow-hidden rounded-xl border border-border bg-muted/10 sm:grid-cols-5",
          signedIn ? "mt-12" : "mt-2 sm:mt-4",
        )}
      >
        {metricItems.map((item, index) => (
          <div
            key={item.label}
            className={cn(
              "min-w-0 px-4 py-4 text-center",
              index > 0 && "border-t border-border sm:border-l sm:border-t-0",
            )}
          >
            <div className="truncate text-lg font-semibold tabular-nums text-foreground">{item.value}</div>
            <div className="mt-1 truncate text-sm text-muted-foreground">{item.label}</div>
          </div>
        ))}
      </div>

      <div className="mt-12 w-full max-w-5xl">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
          <h3 className="text-base font-semibold text-foreground">{strings.gatewayUsageTokenActivity}</h3>
          <div className="flex items-center gap-2">
            <GatewayUsageOverviewModeTabs mode={activityMode} strings={strings} onModeChange={setActivityMode} />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              title={strings.gatewayUsageRefresh}
              aria-label={strings.gatewayUsageRefresh}
              onClick={onRefresh}
              disabled={loading}
            >
              <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
            </Button>
          </div>
        </div>
        <GatewayUsageActivityHeatmap
          summary={summary}
          mode={activityMode}
          language={language}
        />
        {!hasUsage && !loading ? (
          <p className="mt-6 rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
            {strings.gatewayUsageNoData}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function GatewayUsageOverviewAvatar({ label, avatarUrl }: { label: string; avatarUrl: string }) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [avatarUrl]);

  const fallback = (
    <span className="flex h-28 w-28 items-center justify-center rounded-full bg-sky-600 text-3xl font-medium text-white shadow-sm">
      {accountInitials(label)}
    </span>
  );

  if (!avatarUrl || failed) {
    return fallback;
  }

  return (
    <img
      src={avatarUrl}
      alt=""
      className="h-28 w-28 rounded-full object-cover shadow-sm"
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
    />
  );
}

function GatewayUsageOverviewModeTabs({
  mode,
  strings,
  onModeChange,
}: {
  mode: GatewayUsageOverviewMode;
  strings: AppStrings;
  onModeChange: (mode: GatewayUsageOverviewMode) => void;
}) {
  const options: Array<{ value: GatewayUsageOverviewMode; label: string }> = [
    { value: "daily", label: strings.gatewayUsageDailyMode },
    { value: "weekly", label: strings.gatewayUsageWeeklyMode },
    { value: "cumulative", label: strings.gatewayUsageCumulativeMode },
  ];

  return (
    <div className="flex items-center gap-4">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={cn(
            "text-sm font-medium text-muted-foreground transition-colors hover:text-foreground",
            mode === option.value && "text-foreground",
          )}
          onClick={() => onModeChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function GatewayUsageActivityHeatmap({
  summary,
  mode,
  language,
}: {
  summary: GatewayUsageSummary | null;
  mode: GatewayUsageOverviewMode;
  language: Language;
}) {
  const [activeTooltip, setActiveTooltip] = useState<GatewayUsageHeatmapTooltipState | null>(null);
  const heatmap = useMemo(
    () => buildGatewayUsageHeatmap(summary, mode, language),
    [language, mode, summary],
  );
  const showTooltip = useCallback((event: React.PointerEvent<HTMLDivElement>, text: string) => {
    setActiveTooltip(positionGatewayUsageHeatmapTooltip(text, event.clientX, event.clientY));
  }, []);

  return (
    <div className="w-full pb-1">
      <div className="mx-auto w-full">
        <div
          className="grid grid-flow-col gap-[3px] sm:gap-1"
          style={{
            gridTemplateColumns: `repeat(${heatmap.weekCount}, minmax(0, 1fr))`,
            gridTemplateRows: `repeat(${heatmap.rowCount}, minmax(0, 1fr))`,
          }}
        >
          {heatmap.cells.map((cell) => (
            <div
              key={cell.key}
              className={cn(
                "group relative aspect-square w-full rounded-[2px] sm:rounded-[3px]",
                !cell.inRange && "pointer-events-none opacity-0",
              )}
              aria-label={cell.tooltip}
              onPointerEnter={(event) => showTooltip(event, cell.tooltip)}
              onPointerMove={(event) => showTooltip(event, cell.tooltip)}
              onPointerLeave={() => setActiveTooltip(null)}
              onPointerCancel={() => setActiveTooltip(null)}
              style={{
                gridColumn: cell.weekIndex + 1,
                gridRow: cell.dayIndex + 1,
                backgroundColor: gatewayUsageHeatmapColor(cell.value, heatmap.maxValue),
              }}
            />
          ))}
        </div>
        <div
          className="mt-3 grid gap-[3px] text-sm text-muted-foreground sm:gap-1"
          style={{ gridTemplateColumns: `repeat(${heatmap.weekCount}, minmax(0, 1fr))` }}
        >
          {heatmap.monthLabels.map((month) => (
            <span
              key={`${month.label}-${month.weekIndex}`}
              className="truncate"
              style={{ gridColumn: `${month.weekIndex + 1} / span 4` }}
            >
              {month.label}
            </span>
          ))}
        </div>
      </div>
      {activeTooltip && typeof document !== "undefined"
        ? createPortal(
            <div
              className="pointer-events-none fixed z-[1000] w-max rounded-md border border-border bg-card px-2.5 py-1.5 text-center text-xs text-card-foreground shadow-xl"
              style={{
                left: activeTooltip.x,
                maxWidth: "min(16rem, calc(100vw - 16px))",
                top: activeTooltip.y,
                transform:
                  activeTooltip.placement === "above" ? "translate(-50%, -100%)" : "translateX(-50%)",
              }}
            >
              {activeTooltip.text}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}

function GatewayUsageDashboard({
  summary,
  loading,
  error,
  strings,
  dateRange,
  onDateRangeChange,
  onPresetHours,
  onPresetRange,
  fullscreen,
  onToggleFullscreen,
  onRefresh,
}: {
  summary: GatewayUsageSummary | null;
  loading: boolean;
  error: string;
  strings: AppStrings;
  dateRange: GatewayUsageDateRange;
  onDateRangeChange: React.Dispatch<React.SetStateAction<GatewayUsageDateRange>>;
  onPresetHours: (hours: number) => void;
  onPresetRange: (days: number) => void;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
  onRefresh: () => void;
}) {
  const totals = summary?.totals;
  const requestCount = totals?.requestCount ?? 0;
  const successRate = requestCount > 0 ? (totals?.successCount ?? 0) / requestCount : 0;
  const cacheTokens = gatewayUsageCacheTokens(totals);
  const cacheRate = gatewayUsageCacheRate(totals);
  const hasUsage = requestCount > 0;
  const [breakdownMode, setBreakdownMode] = useState<GatewayUsageBreakdownMode>("model");

  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
        <SectionTitle icon={<Activity className="h-4 w-4" />} title={strings.gatewayUsageDashboard} />
        <div className="grid w-full gap-2 sm:grid-cols-[auto_minmax(0,1fr)_auto] xl:w-auto xl:flex xl:flex-wrap xl:items-center xl:justify-end">
          <div
            className="grid grid-cols-[repeat(auto-fit,minmax(58px,1fr))] gap-1 rounded-md border border-border bg-muted/10 p-1 sm:inline-flex"
            aria-label={strings.gatewayUsageDateRange}
          >
            <button
              type="button"
              className={cn(
                "h-8 rounded px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
                usageDateRangeMatchesHours(dateRange, 24) && "bg-background text-foreground shadow-xs",
              )}
              onClick={() => onPresetHours(24)}
            >
              {strings.gatewayUsageLast24Hours}
            </button>
            {[7, 30, 90].map((days) => (
              <button
                key={days}
                type="button"
                className={cn(
                  "h-8 rounded px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
                  usageDateRangeMatchesDays(dateRange, days) && "bg-background text-foreground shadow-xs",
                )}
                onClick={() => onPresetRange(days)}
              >
                {days === 7
                  ? strings.gatewayUsageLast7Days
                  : days === 30
                    ? strings.gatewayUsageLast30Days
                    : strings.gatewayUsageLast90Days}
              </button>
            ))}
          </div>
          <div className="grid min-w-0 grid-cols-2 gap-2">
            <Input
              type="date"
              className="h-9 min-w-0"
              value={dateRange.startDate}
              aria-label={strings.gatewayUsageStartDate}
              onChange={(event) =>
                onDateRangeChange((current) => ({
                  ...current,
                  hours: undefined,
                  startDate: event.target.value,
                }))
              }
            />
            <Input
              type="date"
              className="h-9 min-w-0"
              value={dateRange.endDate}
              aria-label={strings.gatewayUsageEndDate}
              onChange={(event) =>
                onDateRangeChange((current) => ({
                  ...current,
                  hours: undefined,
                  endDate: event.target.value,
                }))
              }
            />
          </div>
          <div className="grid grid-cols-2 gap-2 sm:flex">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="min-w-0"
              title={strings.gatewayUsageRefresh}
              onClick={onRefresh}
              disabled={loading}
            >
              <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
              <span className="hidden lg:inline">{strings.gatewayUsageRefresh}</span>
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="min-w-0"
              title={fullscreen ? strings.gatewayUsageExitFullscreen : strings.gatewayUsageEnterFullscreen}
              onClick={onToggleFullscreen}
            >
              {fullscreen ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}
              <span className="hidden lg:inline">
                {fullscreen ? strings.gatewayUsageExitFullscreen : strings.gatewayUsageEnterFullscreen}
              </span>
            </Button>
          </div>
        </div>
      </div>

      {error ? (
        <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
          {error}
        </p>
      ) : null}

      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(168px,100%),1fr))] gap-3">
        <GatewayUsageMetric
          label={strings.gatewayUsageRequests}
          value={formatCompactNumber(requestCount)}
          detail={summary ? `${formatPercent(successRate)} / ${gatewayUsageWindowLabel(summary)}` : ""}
        />
        <GatewayUsageMetric
          label={strings.gatewayUsageInput}
          value={formatTokenCount(totals?.inputTokens ?? 0)}
          detail={`${strings.gatewayUsageTotal} ${formatTokenCount(totals?.totalTokens ?? 0)}`}
        />
        <GatewayUsageMetric
          label={strings.gatewayUsageOutput}
          value={formatTokenCount(totals?.outputTokens ?? 0)}
          detail={formatUnixDateTime(totals?.lastReceivedAtUnix)}
        />
        <GatewayUsageMetric
          label={strings.gatewayUsageCache}
          value={formatTokenCount(cacheTokens)}
          detail={`${strings.gatewayUsageCacheRead} ${formatTokenCount(totals?.cacheReadTokens ?? 0)} / ${strings.gatewayUsageCacheWrite} ${formatTokenCount(totals?.cacheWriteTokens ?? 0)}`}
        />
        <GatewayUsageMetric
          label={strings.gatewayUsageCacheRate}
          value={formatPercent(cacheRate)}
          detail={`${strings.gatewayUsageCacheRead} ${formatTokenCount(totals?.cacheReadTokens ?? 0)} / ${strings.gatewayUsageInput} ${formatTokenCount(gatewayUsageCacheRateBase(totals))}`}
        />
      </div>

      {!hasUsage && !loading ? (
        <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
          {strings.gatewayUsageNoData}
        </div>
      ) : null}

      {hasUsage || loading ? (
        <div className="space-y-4">
          <GatewayUsageDailyChart daily={summary?.daily || []} strings={strings} />

          <GatewayUsageBreakdownModePicker
            mode={breakdownMode}
            onModeChange={setBreakdownMode}
            strings={strings}
          />

          {breakdownMode === "model" ? (
            <GatewayUsageModelComparison items={summary?.byModel || []} strings={strings} />
          ) : breakdownMode === "session" ? (
            <GatewayUsageSessionAnalysis items={summary?.bySession || []} strings={strings} />
          ) : (
            <GatewayUsageProjectAnalysis items={summary?.byProject || []} strings={strings} />
          )}

          <GatewayUsageRequestTable items={summary?.requests || []} strings={strings} />
        </div>
      ) : null}

      {summary?.databasePath ? (
        <p className="truncate text-xs text-muted-foreground">
          {strings.gatewayUsageDatabase}: {summary.databasePath}
        </p>
      ) : null}
    </section>
  );
}

function GatewayUsageMetric({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <div className="min-w-0 rounded-md border border-border bg-muted/10 p-3">
      <div className="text-xs font-medium text-muted-foreground">{label}</div>
      <div className="mt-1 truncate text-xl font-semibold tabular-nums sm:text-2xl">{value}</div>
      {detail ? <div className="mt-1 truncate text-xs text-muted-foreground">{detail}</div> : null}
    </div>
  );
}

function GatewayUsageDailyChart({ daily, strings }: { daily: GatewayUsageDaily[]; strings: AppStrings }) {
  const chartData = daily.map((item) => ({
    day: item.day.slice(5),
    input: item.inputTokens,
    output: item.outputTokens,
    cache: gatewayUsageCacheTokens(item),
    total: item.totalTokens,
  }));

  return (
    <section className="rounded-md border border-border bg-muted/10 p-3 sm:p-4">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">{strings.gatewayUsageDaily}</h3>
        <span className="text-xs text-muted-foreground">{strings.gatewayUsageTokens}</span>
      </div>
      {chartData.length > 0 ? (
        <div className="h-[200px] sm:h-[260px]">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={chartData} margin={{ top: 8, right: 18, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="rgba(148, 163, 184, 0.18)" vertical={false} />
              <XAxis
                dataKey="day"
                tickLine={false}
                axisLine={false}
                tick={{ fill: "currentColor", fontSize: 12 }}
              />
              <YAxis
                width={56}
                tickLine={false}
                axisLine={false}
                tick={{ fill: "currentColor", fontSize: 12 }}
                tickFormatter={(value) => formatTokenCount(Number(value))}
              />
              <RechartsTooltip
                formatter={(value, name) => [
                  formatTokenCount(Number(value)),
                  gatewayUsageSeriesLabel(String(name), strings),
                ]}
                labelFormatter={(label) => String(label)}
                contentStyle={{
                  background: "hsl(var(--card))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: 6,
                  color: "hsl(var(--card-foreground))",
                }}
              />
              <Legend formatter={(value) => gatewayUsageSeriesLabel(String(value), strings)} />
              <Line type="monotone" dataKey="input" stroke="#38bdf8" strokeWidth={2} dot={false} />
              <Line type="monotone" dataKey="output" stroke="#34d399" strokeWidth={2} dot={false} />
              <Line type="monotone" dataKey="cache" stroke="#f59e0b" strokeWidth={2} dot={false} />
              <Line type="monotone" dataKey="total" stroke="hsl(var(--primary))" strokeWidth={2.5} dot={false} />
            </LineChart>
          </ResponsiveContainer>
        </div>
      ) : (
        <p className="py-6 text-center text-sm text-muted-foreground">{strings.gatewayUsageNoData}</p>
      )}
    </section>
  );
}

function GatewayUsageBreakdownModePicker({
  mode,
  onModeChange,
  strings,
}: {
  mode: GatewayUsageBreakdownMode;
  onModeChange: (mode: GatewayUsageBreakdownMode) => void;
  strings: AppStrings;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border bg-muted/10 px-3 py-2 sm:px-4">
      <span className="text-sm font-semibold">{strings.gatewayUsageGroupBy}</span>
      <Select value={mode} onValueChange={(value) => onModeChange(value as GatewayUsageBreakdownMode)}>
        <SelectTrigger className="h-9 w-[180px]">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="model">{strings.gatewayUsageByModel}</SelectItem>
          <SelectItem value="session">{strings.gatewayUsageBySession}</SelectItem>
          <SelectItem value="project">{strings.gatewayUsageByProject}</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

function GatewayUsageModelComparison({ items, strings }: { items: GatewayUsageBreakdown[]; strings: AppStrings }) {
  const maxTokens = Math.max(...items.map((item) => item.totalTokens), 1);

  return (
    <section className="rounded-md border border-border bg-muted/10 p-3 sm:p-4">
      <h3 className="mb-3 text-sm font-semibold">{strings.gatewayUsageByModel}</h3>
      {items.length > 0 ? (
        <div className="space-y-3">
          {items.map((item) => {
            const total = Math.max(0, item.totalTokens);
            const barWidth = `${Math.max(3, Math.round((total / maxTokens) * 100))}%`;
            const inputWidth = total > 0 ? `${(item.inputTokens / total) * 100}%` : "0%";
            const outputWidth = total > 0 ? `${(item.outputTokens / total) * 100}%` : "0%";
            const cacheWidth = total > 0 ? `${(gatewayUsageCacheTokens(item) / total) * 100}%` : "0%";

            return (
              <div key={`${item.provider}-${item.providerName}-${item.model || item.label}`} className="space-y-1.5">
                <div className="grid min-w-0 gap-1 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">
                      {item.label || item.model || item.providerName || item.provider || strings.none}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatCompactNumber(item.requestCount)} {strings.gatewayUsageRequests}
                    </div>
                  </div>
                  <div className="text-sm tabular-nums sm:shrink-0 sm:text-right">{formatTokenCount(item.totalTokens)}</div>
                </div>
                <div className="h-2 overflow-hidden rounded-full bg-muted" aria-label={item.label}>
                  <div className="flex h-full min-w-1 overflow-hidden rounded-full" style={{ width: barWidth }}>
                    {item.inputTokens > 0 ? <div className="h-full bg-sky-400" style={{ width: inputWidth }} /> : null}
                    {gatewayUsageCacheTokens(item) > 0 ? (
                      <div className="h-full bg-amber-500" style={{ width: cacheWidth }} />
                    ) : null}
                    {item.outputTokens > 0 ? (
                      <div className="h-full bg-emerald-400" style={{ width: outputWidth }} />
                    ) : null}
                  </div>
                </div>
                <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  <span>{strings.gatewayUsageInput} {formatTokenCount(item.inputTokens)}</span>
                  <span>{strings.gatewayUsageCache} {formatTokenCount(gatewayUsageCacheTokens(item))}</span>
                  <span>{strings.gatewayUsageOutput} {formatTokenCount(item.outputTokens)}</span>
                  <span>{strings.gatewayUsageCacheRate} {formatPercent(gatewayUsageCacheRate(item))}</span>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="py-6 text-center text-sm text-muted-foreground">{strings.gatewayUsageNoData}</p>
      )}
    </section>
  );
}

function GatewayUsageSessionAnalysis({
  items,
  strings,
}: {
  items: GatewayUsageSessionBreakdown[];
  strings: AppStrings;
}) {
  const maxTokens = Math.max(...items.map((item) => item.totalTokens), 1);

  return (
    <section className="rounded-md border border-border bg-muted/10 p-3 sm:p-4">
      <h3 className="mb-3 text-sm font-semibold">{strings.gatewayUsageBySession}</h3>
      {items.length > 0 ? (
        <div className="space-y-3">
          {items.map((item) => {
            const total = Math.max(0, item.totalTokens);
            const barWidth = `${Math.max(3, Math.round((total / maxTokens) * 100))}%`;
            const inputWidth = total > 0 ? `${(item.inputTokens / total) * 100}%` : "0%";
            const outputWidth = total > 0 ? `${(item.outputTokens / total) * 100}%` : "0%";
            const cacheWidth = total > 0 ? `${(gatewayUsageCacheTokens(item) / total) * 100}%` : "0%";
            const sessionLabel = item.label || gatewayUsageSessionLabel(item.sessionId, strings);
            const projectLabel = item.projectLabel || strings.gatewayUsageUnknownProject;

            return (
              <div key={item.sessionId || item.label} className="space-y-1.5">
                <div className="grid min-w-0 gap-1 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium" title={sessionLabel}>
                      {compactGatewayUsageLabel(sessionLabel)}
                    </div>
                    <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
                      <span>
                        {formatCompactNumber(item.requestCount)} {strings.gatewayUsageRequests}
                      </span>
                      <span className="truncate" title={item.projectPath || projectLabel}>
                        {projectLabel}
                      </span>
                      <span>
                        {strings.gatewayUsageLastSeen} {formatUnixDateTime(item.lastReceivedAtUnix)}
                      </span>
                    </div>
                  </div>
                  <div className="space-y-0.5 text-sm tabular-nums sm:shrink-0 sm:text-right">
                    <div>{formatTokenCount(item.totalTokens)}</div>
                    <div className="text-xs text-muted-foreground">
                      {strings.gatewayUsageCacheRate} {formatPercent(gatewayUsageCacheRate(item))}
                    </div>
                  </div>
                </div>
                <div className="h-2 overflow-hidden rounded-full bg-muted" aria-label={sessionLabel}>
                  <div className="flex h-full min-w-1 overflow-hidden rounded-full" style={{ width: barWidth }}>
                    {item.inputTokens > 0 ? <div className="h-full bg-sky-400" style={{ width: inputWidth }} /> : null}
                    {item.outputTokens > 0 ? (
                      <div className="h-full bg-emerald-400" style={{ width: outputWidth }} />
                    ) : null}
                    {gatewayUsageCacheTokens(item) > 0 ? (
                      <div className="h-full bg-amber-500" style={{ width: cacheWidth }} />
                    ) : null}
                  </div>
                </div>
                <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  <span>{strings.gatewayUsageInput} {formatTokenCount(item.inputTokens)}</span>
                  <span>{strings.gatewayUsageOutput} {formatTokenCount(item.outputTokens)}</span>
                  <span>{strings.gatewayUsageCache} {formatTokenCount(gatewayUsageCacheTokens(item))}</span>
                  <span>{strings.gatewayUsageFirstSeen} {formatUnixDateTime(item.firstReceivedAtUnix)}</span>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="py-6 text-center text-sm text-muted-foreground">{strings.gatewayUsageNoData}</p>
      )}
    </section>
  );
}

function GatewayUsageProjectAnalysis({
  items,
  strings,
}: {
  items: GatewayUsageProjectBreakdown[];
  strings: AppStrings;
}) {
  const maxTokens = Math.max(...items.map((item) => item.totalTokens), 1);

  return (
    <section className="rounded-md border border-border bg-muted/10 p-3 sm:p-4">
      <h3 className="mb-3 text-sm font-semibold">{strings.gatewayUsageByProject}</h3>
      {items.length > 0 ? (
        <div className="space-y-3">
          {items.map((item) => {
            const total = Math.max(0, item.totalTokens);
            const barWidth = `${Math.max(3, Math.round((total / maxTokens) * 100))}%`;
            const inputWidth = total > 0 ? `${(item.inputTokens / total) * 100}%` : "0%";
            const outputWidth = total > 0 ? `${(item.outputTokens / total) * 100}%` : "0%";
            const cacheWidth = total > 0 ? `${(gatewayUsageCacheTokens(item) / total) * 100}%` : "0%";
            const label = item.label || strings.gatewayUsageUnknownProject;

            return (
              <div key={item.projectPath || label} className="space-y-1.5">
                <div className="grid min-w-0 gap-1 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium" title={item.projectPath || label}>
                      {compactGatewayUsageLabel(label)}
                    </div>
                    <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
                      <span>
                        {formatCompactNumber(item.sessionCount)} {strings.gatewayUsageSession}
                      </span>
                      <span>
                        {formatCompactNumber(item.requestCount)} {strings.gatewayUsageRequests}
                      </span>
                      <span>
                        {strings.gatewayUsageLastSeen} {formatUnixDateTime(item.lastReceivedAtUnix)}
                      </span>
                    </div>
                  </div>
                  <div className="space-y-0.5 text-sm tabular-nums sm:shrink-0 sm:text-right">
                    <div>{formatTokenCount(item.totalTokens)}</div>
                    <div className="text-xs text-muted-foreground">
                      {strings.gatewayUsageCacheRate} {formatPercent(gatewayUsageCacheRate(item))}
                    </div>
                  </div>
                </div>
                <div className="h-2 overflow-hidden rounded-full bg-muted" aria-label={label}>
                  <div className="flex h-full min-w-1 overflow-hidden rounded-full" style={{ width: barWidth }}>
                    {item.inputTokens > 0 ? <div className="h-full bg-sky-400" style={{ width: inputWidth }} /> : null}
                    {item.outputTokens > 0 ? (
                      <div className="h-full bg-emerald-400" style={{ width: outputWidth }} />
                    ) : null}
                    {gatewayUsageCacheTokens(item) > 0 ? (
                      <div className="h-full bg-amber-500" style={{ width: cacheWidth }} />
                    ) : null}
                  </div>
                </div>
                <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  <span>{strings.gatewayUsageInput} {formatTokenCount(item.inputTokens)}</span>
                  <span>{strings.gatewayUsageOutput} {formatTokenCount(item.outputTokens)}</span>
                  <span>{strings.gatewayUsageCache} {formatTokenCount(gatewayUsageCacheTokens(item))}</span>
                  <span>{strings.gatewayUsageFirstSeen} {formatUnixDateTime(item.firstReceivedAtUnix)}</span>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="py-6 text-center text-sm text-muted-foreground">{strings.gatewayUsageNoData}</p>
      )}
    </section>
  );
}

function GatewayUsageRequestTable({ items, strings }: { items: GatewayUsageRequestEvent[]; strings: AppStrings }) {
  return (
    <section className="rounded-md border border-border bg-muted/10 p-3 sm:p-4">
      <h3 className="mb-3 text-sm font-semibold">{strings.gatewayUsageRequestList}</h3>
      {items.length > 0 ? (
        <div className="overflow-x-auto rounded border border-border/70">
          <div className="min-w-[960px]">
            <div className="grid grid-cols-[116px_160px_minmax(180px,1fr)_86px_70px_70px_70px_70px_66px] bg-muted/20 px-2 py-2 text-xs font-medium text-muted-foreground sm:px-3">
              <span>{strings.gatewayUsageTime}</span>
              <span>{strings.gatewayUsageSession}</span>
              <span>{strings.model}</span>
              <span>{strings.status}</span>
              <span className="text-right">{strings.gatewayUsageInput}</span>
              <span className="text-right">{strings.gatewayUsageOutput}</span>
              <span className="text-right">{strings.gatewayUsageCache}</span>
              <span className="text-right">{strings.gatewayUsageTotal}</span>
              <span className="text-right">{strings.gatewayUsageLatency}</span>
            </div>
            {items.map((event) => {
              const sessionLabel = event.clientSessionLabel || gatewayUsageSessionLabel(event.clientSessionId, strings);
              const projectLabel = event.clientProjectLabel || strings.gatewayUsageUnknownProject;

              return (
                <div
                  key={event.eventId}
                  className="grid grid-cols-[116px_160px_minmax(180px,1fr)_86px_70px_70px_70px_70px_66px] items-center border-t border-border/70 px-2 py-2 text-sm sm:px-3"
                >
                  <span className="truncate text-xs text-muted-foreground">{formatUnixDateTime(event.receivedAtUnix)}</span>
                  <div className="min-w-0 text-xs text-muted-foreground" title={sessionLabel}>
                    <div className="truncate">{compactGatewayUsageLabel(sessionLabel)}</div>
                    <div className="truncate" title={event.clientProjectPath || projectLabel}>
                      {projectLabel}
                    </div>
                  </div>
                  <div className="min-w-0">
                    <div className="truncate font-medium">
                      {event.model || event.providerName || event.provider || strings.none}
                    </div>
                    <div className="truncate text-xs text-muted-foreground">
                      {event.route || event.requestId || event.eventId}
                    </div>
                  </div>
                  <span>
                    <Badge className={gatewayUsageStatusClass(event.status)}>{event.status || strings.none}</Badge>
                  </span>
                  <span className="text-right tabular-nums text-muted-foreground">
                    {formatTokenCount(event.inputTokens)}
                  </span>
                  <span className="text-right tabular-nums text-muted-foreground">
                    {formatTokenCount(event.outputTokens)}
                  </span>
                  <span className="text-right tabular-nums text-muted-foreground">
                    {formatTokenCount(gatewayUsageCacheTokens(event))}
                  </span>
                  <span className="text-right tabular-nums text-muted-foreground">
                    {formatTokenCount(event.totalTokens)}
                  </span>
                  <span className="text-right text-xs tabular-nums text-muted-foreground">
                    {event.latencyMs !== null ? formatLatency(event.latencyMs) : "-"}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      ) : (
        <p className="py-6 text-center text-sm text-muted-foreground">{strings.gatewayUsageNoData}</p>
      )}
    </section>
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

function GatewayDialogValidationMessage({ message }: { message: string }) {
  return (
    <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2.5 text-sm leading-relaxed text-red-300">
      {message}
    </p>
  );
}

function gatewayProviderDraftSignature(provider: GatewayProviderForm): string {
  return jsonSignature({
    name: provider.name.trim(),
    type: provider.type,
    apiKey: provider.apiKey.trim(),
    baseUrl: provider.baseUrl.trim(),
    models: commaList(provider.models),
    thinkingEffortModels: normalizeModelOptions(provider.thinkingEffortModels),
  });
}

function gatewayMcpServerDraftSignature(server: GatewayMcpServerForm): string {
  return jsonSignature({
    name: server.name.trim(),
    enabled: server.enabled,
    transport: server.transport,
    stdioMessageMode: server.stdioMessageMode,
    command: server.command.trim(),
    args: server.args.trim(),
    cwd: server.cwd.trim(),
    url: server.url.trim(),
    headersJson: server.headersJson.trim(),
    envJson: server.envJson.trim(),
    apiKey: server.apiKey.trim(),
    apiKeyEnv: server.apiKeyEnv.trim(),
    protocolVersion: server.protocolVersion.trim(),
    startupTimeoutMs: server.startupTimeoutMs.trim(),
    requestTimeoutMs: server.requestTimeoutMs.trim(),
  });
}

function gatewayVirtualProfileDraftSignature(profile: GatewayVirtualProfileForm): string {
  return jsonSignature({
    profileId: profile.profileId.trim(),
    key: profile.key.trim(),
    displayName: profile.displayName.trim(),
    description: profile.description.trim(),
    enabled: profile.enabled,
    exactAliases: profile.exactAliases.trim(),
    prefixes: profile.prefixes.trim(),
    suffixes: profile.suffixes.trim(),
    baseModelMode: profile.baseModelMode,
    fixedModel: profile.fixedModel.trim(),
    matchMultimodal: profile.matchMultimodal,
    matchWebSearch: profile.matchWebSearch,
    maxTurns: profile.maxTurns.trim(),
    maxToolCalls: profile.maxToolCalls.trim(),
    clientToolsPolicy: profile.clientToolsPolicy,
    includeInGatewayModels: profile.includeInGatewayModels,
    tools: profile.tools.map(gatewayVirtualToolDraftSignature),
  });
}

function gatewayVirtualToolDraftSignature(tool: GatewayVirtualToolForm) {
  return {
    name: tool.name.trim(),
    description: tool.description.trim(),
    visibility: tool.visibility,
    inputSchemaJson: tool.inputSchemaJson.trim(),
  };
}

function gatewayProviderDialogError(provider: GatewayProviderForm, strings: AppStrings): string {
  if (!provider.name.trim()) {
    return strings.fieldRequired(strings.name);
  }
  if (!provider.baseUrl.trim()) {
    return strings.fieldRequired(strings.baseUrl);
  }
  return "";
}

function gatewayMcpServerDialogError(server: GatewayMcpServerForm, strings: AppStrings): string {
  if (!server.name.trim()) {
    return strings.fieldRequired(strings.name);
  }
  if (server.transport === "websocket") {
    if (!server.url.trim()) {
      return strings.fieldRequired(strings.url);
    }
    try {
      jsonObjectFromText(server.headersJson, strings.headersJson);
    } catch (error) {
      return errorMessage(error);
    }
    return "";
  }

  if (!server.command.trim()) {
    return strings.fieldRequired(strings.command);
  }
  try {
    jsonObjectFromText(server.envJson, strings.envJson);
  } catch (error) {
    return errorMessage(error);
  }
  return "";
}

function gatewayVirtualProfileDialogError(profile: GatewayVirtualProfileForm, strings: AppStrings): string {
  if (!profile.key.trim()) {
    return strings.fieldRequired(strings.profileKey);
  }
  if (
    !profile.exactAliases.trim() &&
    !profile.prefixes.trim() &&
    !profile.suffixes.trim()
  ) {
    return strings.gatewayMatchRequired;
  }
  if (profile.baseModelMode === "fixed" && !profile.fixedModel.trim()) {
    return strings.fieldRequired(strings.fixedModel);
  }
  return "";
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
  const probedModels = useProviderModelProbe(provider.baseUrl, provider.apiKey, true, provider.type);
  const deepSeekV4Models = deepSeekV4ModelsFromProvider(provider);
  const selectedThinkingEffortModels = normalizeThinkingEffortModels(
    provider.thinkingEffortModels,
    provider.models,
  );
  const selectedModels = commaList(provider.models);
  const updateModels = (models: string) => {
    onChange({
      models,
      thinkingEffortModels: normalizeThinkingEffortModels(provider.thinkingEffortModels, models),
    });
  };
  const appendModel = (model: string) => {
    const nextModels = normalizeModelOptions([...selectedModels, model]).join(", ");
    updateModels(nextModels);
  };
  const updateThinkingEffortModel = (model: string, enabled: boolean) => {
    const selected = new Set(selectedThinkingEffortModels);
    if (enabled) {
      selected.add(model);
    } else {
      selected.delete(model);
    }
    onChange({
      thinkingEffortModels: Array.from(selected).filter((item) => deepSeekV4Models.includes(item)),
      ...(enabled ? { type: "openai_chat_completions" } : {}),
    });
  };
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
        <div className="relative">
          <Input
            value={provider.models}
            className={probedModels.length > 0 ? "pr-10" : undefined}
            placeholder="gpt-5.5, gpt-5.4"
            onChange={(event) => updateModels(event.target.value)}
          />
          <ModelOptionsDropdown
            options={probedModels}
            selectedValues={selectedModels}
            strings={strings}
            onSelect={appendModel}
          />
        </div>
      </Field>
      {deepSeekV4Models.length > 0 ? (
        <div className="rounded-md border border-border bg-muted/10 px-3 py-2.5">
          <div className="text-sm font-medium">{strings.adaptThinkingEffort}</div>
          <div className="mt-2 grid gap-2">
            {deepSeekV4Models.map((model) => (
              <div key={model} className="flex items-center justify-between gap-3">
                <div className="min-w-0 break-words text-sm text-foreground">{model}</div>
                <Switch
                  checked={selectedThinkingEffortModels.includes(model)}
                  aria-label={`${strings.adaptThinkingEffort}: ${model}`}
                  onCheckedChange={(checked) => updateThinkingEffortModel(model, checked === true)}
                />
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

const gatewayTextareaClassName =
  "min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm font-mono leading-relaxed shadow-xs outline-none transition-[color,box-shadow] placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50";

function GatewayMcpServerEditor({
  server,
  strings,
  onChange,
}: {
  server: GatewayMcpServerForm;
  strings: AppStrings;
  onChange: (patch: Partial<GatewayMcpServerForm>) => void;
}) {
  return (
    <div className="grid gap-4">
      <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-muted/10 px-3 py-2.5">
        <div className="text-sm font-medium">{strings.enabled}</div>
        <Switch checked={server.enabled} onCheckedChange={(checked) => onChange({ enabled: checked === true })} />
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={strings.name}>
          <Input autoFocus value={server.name} onChange={(event) => onChange({ name: event.target.value })} />
        </Field>
        <Field label={strings.transport}>
          <Select
            value={server.transport}
            onValueChange={(value) => onChange({ transport: value === "websocket" ? "websocket" : "stdio" })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="stdio">stdio</SelectItem>
              <SelectItem value="websocket">websocket</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>

      {server.transport === "websocket" ? (
        <>
          <Field label={strings.url}>
            <Input value={server.url} onChange={(event) => onChange({ url: event.target.value })} />
          </Field>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label={strings.apiKey}>
              <Input
                type="password"
                value={server.apiKey}
                onChange={(event) => onChange({ apiKey: event.target.value })}
              />
            </Field>
            <Field label={strings.apiKeyEnv}>
              <Input value={server.apiKeyEnv} onChange={(event) => onChange({ apiKeyEnv: event.target.value })} />
            </Field>
          </div>
          <Field label={strings.headersJson}>
            <textarea
              className={gatewayTextareaClassName}
              spellCheck={false}
              autoCapitalize="none"
              value={server.headersJson}
              onChange={(event) => onChange({ headersJson: event.target.value })}
            />
          </Field>
        </>
      ) : (
        <>
          <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_180px]">
            <Field label={strings.command}>
              <Input
                value={server.command}
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                onChange={(event) => onChange({ command: event.target.value })}
              />
            </Field>
            <Field label={strings.stdioMessageMode}>
              <Select
                value={server.stdioMessageMode}
                onValueChange={(value) =>
                  onChange({
                    stdioMessageMode: value === "content-length" ? "content-length" : "newline-json",
                  })
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="newline-json">newline-json</SelectItem>
                  <SelectItem value="content-length">content-length</SelectItem>
                </SelectContent>
              </Select>
            </Field>
          </div>
          <Field label={strings.args}>
            <Input
              value={server.args}
              placeholder="-y, @modelcontextprotocol/server-filesystem, ."
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              onChange={(event) => onChange({ args: event.target.value })}
            />
          </Field>
          <Field label={strings.cwd}>
            <Input
              value={server.cwd}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              onChange={(event) => onChange({ cwd: event.target.value })}
            />
          </Field>
          <Field label={strings.envJson}>
            <textarea
              className={gatewayTextareaClassName}
              spellCheck={false}
              autoCapitalize="none"
              value={server.envJson}
              onChange={(event) => onChange({ envJson: event.target.value })}
            />
          </Field>
        </>
      )}

      <div className="grid gap-3 sm:grid-cols-3">
        <Field label={strings.protocolVersion}>
          <Input
            value={server.protocolVersion}
            onChange={(event) => onChange({ protocolVersion: event.target.value })}
          />
        </Field>
        <Field label={strings.startupTimeoutMs}>
          <Input
            inputMode="numeric"
            value={server.startupTimeoutMs}
            onChange={(event) => onChange({ startupTimeoutMs: event.target.value })}
          />
        </Field>
        <Field label={strings.requestTimeoutMs}>
          <Input
            inputMode="numeric"
            value={server.requestTimeoutMs}
            onChange={(event) => onChange({ requestTimeoutMs: event.target.value })}
          />
        </Field>
      </div>
    </div>
  );
}

function GatewayVirtualProfileEditor({
  profile,
  availableTools,
  availableToolsLoading,
  availableToolsError,
  strings,
  onChange,
  onRefreshTools,
}: {
  profile: GatewayVirtualProfileForm;
  availableTools: GatewayAvailableTool[];
  availableToolsLoading: boolean;
  availableToolsError: string;
  strings: AppStrings;
  onChange: (patch: Partial<GatewayVirtualProfileForm>) => void;
  onRefreshTools: () => Promise<void>;
}) {
  const availableToolByName = useMemo(
    () => new Map(availableTools.map((tool) => [tool.name, tool])),
    [availableTools],
  );
  const selectedToolByName = useMemo(
    () => new Map(profile.tools.map((tool) => [tool.name, tool])),
    [profile.tools],
  );
  const unavailableSelectedTools = profile.tools.filter((tool) => tool.name && !availableToolByName.has(tool.name));
  const setToolSelected = (tool: GatewayAvailableTool, selected: boolean) => {
    if (selected) {
      if (selectedToolByName.has(tool.name)) return;
      onChange({ tools: [...profile.tools, gatewayVirtualToolFormFromAvailableTool(tool)] });
      return;
    }
    onChange({ tools: profile.tools.filter((item) => item.name !== tool.name) });
  };
  const updateToolVisibility = (toolName: string, visibility: GatewayVirtualToolVisibility) =>
    onChange({
      tools: profile.tools.map((tool) => (tool.name === toolName ? { ...tool, visibility } : tool)),
    });

  return (
    <div className="grid gap-5">
      <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-muted/10 px-3 py-2.5">
        <div className="text-sm font-medium">{strings.enabled}</div>
        <Switch checked={profile.enabled} onCheckedChange={(checked) => onChange({ enabled: checked })} />
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={strings.profileKey}>
          <Input autoFocus value={profile.key} onChange={(event) => onChange({ key: event.target.value })} />
        </Field>
        <Field label={strings.displayName}>
          <Input
            value={profile.displayName}
            onChange={(event) => onChange({ displayName: event.target.value })}
          />
        </Field>
      </div>
      <Field label={strings.description}>
        <Input value={profile.description} onChange={(event) => onChange({ description: event.target.value })} />
      </Field>

      <div className="grid gap-3 sm:grid-cols-3">
        <Field label={strings.exactAliases}>
          <Input value={profile.exactAliases} onChange={(event) => onChange({ exactAliases: event.target.value })} />
        </Field>
        <Field label={strings.prefixes}>
          <Input value={profile.prefixes} onChange={(event) => onChange({ prefixes: event.target.value })} />
        </Field>
        <Field label={strings.suffixes}>
          <Input
            value={profile.suffixes}
            placeholder=":vision-search"
            onChange={(event) => onChange({ suffixes: event.target.value })}
          />
        </Field>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={strings.baseModelMode}>
          <Select
            value={profile.baseModelMode}
            onValueChange={(value) =>
              onChange({ baseModelMode: normalizeGatewayVirtualBaseModelMode(value) })
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="strip_suffix">strip_suffix</SelectItem>
              <SelectItem value="strip_prefix">strip_prefix</SelectItem>
              <SelectItem value="request">request</SelectItem>
              <SelectItem value="fixed">fixed</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field label={strings.fixedModel}>
          <Input value={profile.fixedModel} onChange={(event) => onChange({ fixedModel: event.target.value })} />
        </Field>
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        <GatewaySwitchField
          label={strings.matchMultimodal}
          checked={profile.matchMultimodal}
          onCheckedChange={(checked) => onChange({ matchMultimodal: checked })}
        />
        <GatewaySwitchField
          label={strings.matchWebSearch}
          checked={profile.matchWebSearch}
          onCheckedChange={(checked) => onChange({ matchWebSearch: checked })}
        />
        <GatewaySwitchField
          label={strings.includeInGatewayModels}
          checked={profile.includeInGatewayModels}
          onCheckedChange={(checked) => onChange({ includeInGatewayModels: checked })}
        />
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        <Field label={strings.maxTurns}>
          <Input
            inputMode="numeric"
            value={profile.maxTurns}
            onChange={(event) => onChange({ maxTurns: event.target.value })}
          />
        </Field>
        <Field label={strings.maxToolCalls}>
          <Input
            inputMode="numeric"
            value={profile.maxToolCalls}
            onChange={(event) => onChange({ maxToolCalls: event.target.value })}
          />
        </Field>
        <Field label={strings.clientToolsPolicy}>
          <Select
            value={profile.clientToolsPolicy}
            onValueChange={(value) => onChange({ clientToolsPolicy: value === "deny" ? "deny" : "allow" })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="allow">allow</SelectItem>
              <SelectItem value="deny">deny</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <SectionTitle icon={<ImageIcon className="h-4 w-4" />} title={strings.availableMcpTools} />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onRefreshTools().catch(console.error)}
            disabled={availableToolsLoading}
          >
            <RefreshCw className={cn("h-4 w-4", availableToolsLoading && "animate-spin")} />
            {strings.reload}
          </Button>
        </div>
        {availableToolsError ? (
          <p className="rounded-md border border-destructive/50 bg-destructive/12 px-3 py-2 text-sm leading-relaxed text-red-300">
            {availableToolsError}
          </p>
        ) : null}
        {availableToolsLoading ? (
          <div className="rounded-md border border-border bg-muted/10 px-3 py-3 text-sm text-muted-foreground">
            {strings.gatewayToolsLoading}
          </div>
        ) : null}
        <div className="space-y-2">
          {availableTools.map((tool) => {
            const selectedTool = selectedToolByName.get(tool.name);
            const selected = Boolean(selectedTool);
            return (
              <div
                key={tool.name}
                className="grid gap-3 rounded-md border border-border bg-muted/10 px-3 py-3 sm:grid-cols-[minmax(0,1fr)_150px] sm:items-center"
              >
                <label className="flex min-w-0 items-start gap-3">
                  <Checkbox
                    checked={selected}
                    onCheckedChange={(checked) => setToolSelected(tool, checked === true)}
                  />
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium">{tool.name}</span>
                    {tool.description ? (
                      <span className="mt-1 line-clamp-2 block text-xs leading-relaxed text-muted-foreground">
                        {tool.description}
                      </span>
                    ) : null}
                  </span>
                </label>
                <Select
                  value={selectedTool?.visibility || "internal"}
                  onValueChange={(value) =>
                    updateToolVisibility(tool.name, value === "client" ? "client" : "internal")
                  }
                  disabled={!selected}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="internal">internal</SelectItem>
                    <SelectItem value="client">client</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            );
          })}
          {unavailableSelectedTools.map((tool) => (
            <div
              key={tool.id}
              className="grid gap-3 rounded-md border border-border bg-muted/10 px-3 py-3 opacity-75 sm:grid-cols-[minmax(0,1fr)_150px] sm:items-center"
            >
              <label className="flex min-w-0 items-start gap-3">
                <Checkbox
                  checked
                  onCheckedChange={(checked) => {
                    if (checked !== true) {
                      onChange({ tools: profile.tools.filter((item) => item.id !== tool.id) });
                    }
                  }}
                />
                <span className="min-w-0">
                  <span className="block truncate text-sm font-medium">{tool.name}</span>
                  <span className="mt-1 block text-xs leading-relaxed text-muted-foreground">
                    {strings.unavailableTool}
                  </span>
                </span>
              </label>
              <Select
                value={tool.visibility}
                onValueChange={(value) => updateToolVisibility(tool.name, value === "client" ? "client" : "internal")}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="internal">internal</SelectItem>
                  <SelectItem value="client">client</SelectItem>
                </SelectContent>
              </Select>
            </div>
          ))}
          {availableTools.length === 0 && unavailableSelectedTools.length === 0 && !availableToolsLoading ? (
            <div className="rounded-md border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
              {strings.noGatewayTools}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function GatewaySwitchField({
  label,
  checked,
  onCheckedChange,
}: {
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-3 rounded-md border border-border bg-muted/10 px-3 py-2">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
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
        <ModelOptionsListScroll>
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
        </ModelOptionsListScroll>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ModelOptionsDropdown({
  options,
  selectedValues,
  strings,
  onSelect,
}: {
  options: string[];
  selectedValues: string[];
  strings: AppStrings;
  onSelect: (model: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const selected = useMemo(() => new Set(selectedValues.map((item) => item.trim()).filter(Boolean)), [selectedValues]);
  const modelOptions = useMemo(() => normalizeModelOptions(options), [options]);
  const filteredOptions = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return modelOptions;
    }
    return modelOptions.filter((option) => option.toLowerCase().includes(needle));
  }, [modelOptions, query]);

  if (modelOptions.length === 0) {
    return null;
  }

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
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          title={strings.selectModel}
          aria-label={strings.selectModel}
        >
          <ChevronDown className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-[min(24rem,var(--radix-dropdown-menu-trigger-width))] min-w-64 p-0"
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
        <ModelOptionsListScroll>
          {filteredOptions.length > 0 ? (
            filteredOptions.map((option) => (
              <DropdownMenuItem
                key={option}
                className="justify-between gap-3"
                onSelect={(event) => {
                  event.preventDefault();
                  onSelect(option);
                  setOpen(false);
                }}
              >
                <span className="min-w-0 truncate">{option}</span>
                {selected.has(option) ? <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald" /> : null}
              </DropdownMenuItem>
            ))
          ) : (
            <div className="px-2 py-6 text-center text-sm text-muted-foreground">{strings.noModelsFound}</div>
          )}
        </ModelOptionsListScroll>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ModelOptionsListScroll({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="max-h-56 overflow-y-auto overscroll-contain p-1"
      onWheel={(event) => event.stopPropagation()}
      onTouchMove={(event) => event.stopPropagation()}
    >
      {children}
    </div>
  );
}

type SettingsDialogProps = {
  dialogMode: DialogMode;
  providerMode: ProviderMode;
  form: ProviderForm;
  defaultProviders: DefaultProviderProfile[];
  botConfigs: SavedBotConfig[];
  codexAppPath: string;
  settingsError: string;
  saveDisabled: boolean;
  saving: boolean;
  editingProfileName: string | null;
  existingProviderSelectRef: React.RefObject<HTMLButtonElement | null>;
  workspaceNameInputRef: React.RefObject<HTMLInputElement | null>;
  providerNameInputRef: React.RefObject<HTMLInputElement | null>;
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
  codexAppPath,
  settingsError,
  saveDisabled,
  saving,
  editingProfileName,
  existingProviderSelectRef,
  workspaceNameInputRef,
  providerNameInputRef,
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
  const selectableDefaultProviders = useMemo(
    () => workspaceSelectableDefaultProviders(defaultProviders, gatewayEnabled),
    [defaultProviders, gatewayEnabled],
  );
  const selectedBotConfigId = availableBotConfigs.some((item) => item.id === form.botConfigId)
    ? form.botConfigId
    : BOT_CONFIG_CUSTOM_VALUE;
  const claudeCodeMode = form.remoteFrontendMode === "claude-code";
  const newProviderActive = !claudeCodeMode && (providerMode === "new" || providerMode === "gateway");
  const isEditingDefaultWorkspace = dialogMode === "edit" && editingProfileName === "Default";
  const canChangeProviderMode = dialogMode === "add" || dialogMode === "edit";
  const showProviderModeSelector = canChangeProviderMode && !claudeCodeMode;
  const providerSourceOptions = [
    {
      mode: "none" as const,
      title: strings.providerSourceNone,
      description: strings.providerSourceNoneDescription,
      disabled: false,
      disabledReason: "",
    },
    {
      mode: "existing" as const,
      title: strings.providerSourceDefault,
      description: strings.providerSourceDefaultDescription,
      disabled: selectableDefaultProviders.length === 0,
      disabledReason: strings.providerSourceDefaultUnavailable,
    },
    ...(gatewayEnabled
      ? [
          {
            mode: "gateway" as const,
            title: strings.providerSourceGateway,
            description: strings.providerSourceGatewayDescription,
            disabled: isEditingDefaultWorkspace,
            disabledReason: strings.providerSourceGatewayDefaultUnavailable,
          },
        ]
      : []),
  ];
  const selectedProviderSource =
    providerSourceOptions.find((option) => option.mode === providerMode) || providerSourceOptions[0];
  const providerSourceDescription = (option: (typeof providerSourceOptions)[number]) =>
    option.disabled && option.disabledReason
      ? `${option.description} ${option.disabledReason}`
      : option.description;
  const [wifiScan, setWifiScan] = useState<BotHandoffScanState>(emptyHandoffScanState);
  const [bluetoothScan, setBluetoothScan] = useState<BotHandoffScanState>(emptyHandoffScanState);
  const [discardConfirmOpen, setDiscardConfirmOpen] = useState(false);
  const initialFormSignatureRef = useRef(workspaceDialogDraftSignature(form, providerMode));
  const autoHandoffScanRef = useRef(false);
  const dirty = workspaceDialogDraftSignature(form, providerMode) !== initialFormSignatureRef.current;

  const requestClose = () => {
    if (saving) {
      return;
    }
    if (dirty) {
      setDiscardConfirmOpen(true);
      return;
    }
    onClose();
  };

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

  useEffect(() => {
    if (!remoteFrontendModeUsesCli(form.remoteFrontendMode)) {
      return;
    }
    const registryUrl = normalizeRegistryUrl(form.remoteWebAssetRegistryUrl);
    if (!registryUrl) {
      onSetForm((current) => ({
        ...current,
        remoteWebAssetVersions: [],
        remoteWebAssetVersionsLoading: false,
        remoteWebAssetRegistryError: "",
      }));
      return;
    }

    let cancelled = false;
    onSetForm((current) => ({
      ...current,
      remoteWebAssetVersionsLoading: true,
      remoteWebAssetRegistryError: "",
    }));

    loadCodexWebAssetVersions(registryUrl)
      .then((result) => {
        if (cancelled) return;
        const versions = result.versions.map((version) => version.trim()).filter(Boolean);
        onSetForm((current) => {
          return {
            ...current,
            remoteWebAssetVersions: codexWebAssetVersionOptions(
              versions,
              current.remoteWebAssetVersion,
            ),
            remoteWebAssetVersionsLoading: false,
            remoteWebAssetRegistryError: "",
          };
        });
      })
      .catch((error) => {
        if (cancelled) return;
        onSetForm((current) => ({
          ...current,
          remoteWebAssetVersions: [],
          remoteWebAssetVersionsLoading: false,
          remoteWebAssetRegistryError: errorMessage(error),
        }));
      });

    return () => {
      cancelled = true;
    };
  }, [form.remoteFrontendMode, form.remoteWebAssetRegistryUrl, onSetForm]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          requestClose();
        }
      }}
    >
      <DialogContent
        className="max-h-[90vh] max-w-2xl overflow-y-auto"
        closeLabel={strings.close}
        showCloseButton={!saving}
      >
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

        <div className="flex flex-col gap-3.5">
          <div className="flex flex-col gap-1.5">
            <Label>{strings.remoteFrontendMode}</Label>
            <div className="bg-background border border-border rounded-md grid grid-cols-3 p-0.5">
              <Button
                type="button"
                variant={form.remoteFrontendMode === "app" ? "secondary" : "ghost"}
                size="sm"
                className={cn(
                  "shadow-none",
                  form.remoteFrontendMode !== "app" && "text-muted-foreground hover:bg-transparent",
                )}
                disabled={!codexAppPath}
                onClick={() =>
                  onSetForm((current) => ({ ...current, remoteFrontendMode: "app" }))
                }
              >
                <Monitor className="h-3.5 w-3.5" />
                {strings.remoteFrontendApp}
              </Button>
              <Button
                type="button"
                variant={form.remoteFrontendMode === "cli" ? "secondary" : "ghost"}
                size="sm"
                className={cn(
                  "shadow-none",
                  form.remoteFrontendMode !== "cli" && "text-muted-foreground hover:bg-transparent",
                )}
                onClick={() =>
                  onSetForm((current) => ({ ...current, remoteFrontendMode: "cli" }))
                }
              >
                <Terminal className="h-3.5 w-3.5" />
                {strings.remoteFrontendCli}
              </Button>
              <Button
                type="button"
                variant={form.remoteFrontendMode === "claude-code" ? "secondary" : "ghost"}
                size="sm"
                className={cn(
                  "shadow-none",
                  form.remoteFrontendMode !== "claude-code" && "text-muted-foreground hover:bg-transparent",
                )}
                onClick={() => {
                  onSetForm((current) => ({ ...current, remoteFrontendMode: "claude-code" }));
                  onSelectProviderMode("none");
                }}
              >
                <Cpu className="h-3.5 w-3.5" />
                {strings.remoteFrontendClaudeCode}
              </Button>
            </div>
            {form.remoteFrontendMode === "app" ? (
              <p
                className="text-xs text-muted-foreground truncate"
                title={codexAppPath || strings.codexAppNotFound}
              >
                {codexAppPath ? strings.codexAppDetected(codexAppPath) : strings.codexAppNotFound}
              </p>
            ) : null}
          </div>

          {showProviderModeSelector ? (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="providerSourceSelect">{strings.providerSource}</Label>
              <Select
                value={providerMode}
                onValueChange={(value) => onSelectProviderMode(value as ProviderMode)}
              >
                <SelectTrigger id="providerSourceSelect">
                  <span className="truncate">{selectedProviderSource?.title}</span>
                </SelectTrigger>
                <SelectContent>
                  {providerSourceOptions.map((option) => (
                    <SelectItem
                      key={option.mode}
                      value={option.mode}
                      disabled={option.disabled}
                      textValue={option.title}
                      className="items-start py-2"
                    >
                      <span className="flex flex-col gap-1">
                        <span className="font-medium">{option.title}</span>
                        <span className="whitespace-normal text-xs leading-relaxed text-muted-foreground">
                          {providerSourceDescription(option)}
                        </span>
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {selectedProviderSource ? (
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {providerSourceDescription(selectedProviderSource)}
                </p>
              ) : null}
            </div>
          ) : null}

          {remoteFrontendModeUsesCli(form.remoteFrontendMode) ? (
            <div className="grid gap-3.5 sm:grid-cols-[minmax(0,1fr)_180px]">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="remoteWebAssetRegistryUrlInput">{strings.registryUrl}</Label>
                <Input
                  id="remoteWebAssetRegistryUrlInput"
                  type="url"
                  placeholder={DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL}
                  value={form.remoteWebAssetRegistryUrl}
                  onChange={(event) =>
                    onSetForm((current) => ({
                      ...current,
                      remoteWebAssetRegistryUrl: event.target.value,
                    }))
                  }
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="remoteWebAssetVersionSelect">{strings.registryVersion}</Label>
                <Select
                  value={form.remoteWebAssetVersion || DEFAULT_CODEX_WEB_ASSET_VERSION}
                  disabled={
                    form.remoteWebAssetVersionsLoading ||
                    form.remoteWebAssetVersions.length === 0
                  }
                  onValueChange={(value) =>
                    onSetForm((current) => ({ ...current, remoteWebAssetVersion: value }))
                  }
                >
                  <SelectTrigger id="remoteWebAssetVersionSelect">
                    <SelectValue
                      placeholder={
                        form.remoteWebAssetVersionsLoading
                          ? strings.loadingVersions
                          : strings.registryVersion
                      }
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {form.remoteWebAssetVersions.map((version) => (
                      <SelectItem key={version} value={version}>
                        {version}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              {form.remoteWebAssetRegistryError ? (
                <p className="text-xs text-destructive sm:col-span-2">
                  {form.remoteWebAssetRegistryError}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>

        {!claudeCodeMode && providerMode === "existing" ? (
          <div className="flex flex-col gap-3.5">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="existingProviderSelect">{strings.provider}</Label>
              <Select
                value={form.existingProfileName}
                onValueChange={onSyncExistingProvider}
              >
                <SelectTrigger id="existingProviderSelect" ref={existingProviderSelectRef}>
                  <SelectValue placeholder={strings.selectProvider} />
                </SelectTrigger>
                <SelectContent>
                  {selectableDefaultProviders.map((profile) => (
                    <SelectItem key={profile.name} value={profile.name}>
                      {profile.name} ({profile.provider_name} / {profile.model})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
        ) : null}

        {newProviderActive ? (
          <div className="flex flex-col gap-3.5">
            {providerMode === "new" ? (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="providerNameInput">{strings.providerProfileName}</Label>
                <Input
                  id="providerNameInput"
                  ref={providerNameInputRef}
                  type="text"
                  placeholder="nextai"
                  value={form.providerName}
                  onChange={(event) =>
                    onSetForm((current) => ({ ...current, providerName: event.target.value }))
                  }
                />
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
          <div className="rounded-md border border-border bg-muted/10">
            <div className="flex items-center justify-between gap-4 px-3 py-3">
              <span className="min-w-0">
                <span className="block text-sm font-medium text-foreground">{strings.bot}</span>
                <span className="mt-1 block text-xs leading-relaxed text-muted-foreground">
                  {strings.botOptionsDescription}
                </span>
              </span>
              <Switch
                checked={form.botEnabled}
                aria-label={strings.enableBotIntegration}
                onCheckedChange={(checked) =>
                  onSetForm((current) => {
                    const enabled = checked === true;
                    if (!enabled) {
                      return {
                        ...current,
                        botEnabled: false,
                      };
                    }
                    const nextPlatform = current.botPlatform === "none" ? "weixin-ilink" : current.botPlatform;
                    const nextAuthType = normalizeBotAuthType(nextPlatform, current.botAuthType);
                    return {
                      ...current,
                      botEnabled: true,
                      botPlatform: nextPlatform,
                      botAuthType: nextAuthType,
                      botAuthFields: pickBotAuthFields(current.botAuthFields, nextPlatform, nextAuthType),
                    };
                  })
                }
              />
            </div>
            {form.botEnabled ? (
              <div className="border-t border-border px-3 py-3 flex flex-col gap-3.5">
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
            disabled={saving}
            onClick={requestClose}
          >
            {strings.cancel}
          </Button>
          <Button
            type="button"
            disabled={saveDisabled || saving}
            onClick={() => onSave().catch(console.error)}
          >
            {saving ? <RefreshCw className="h-4 w-4 animate-spin" /> : null}
            {saving ? strings.saving : dialogMode === "edit" ? strings.save : strings.createProfile}
          </Button>
        </DialogFooter>
        <AlertDialog open={discardConfirmOpen} onOpenChange={setDiscardConfirmOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{strings.discardSettingsChangesTitle}</AlertDialogTitle>
              <AlertDialogDescription>{strings.discardSettingsChangesDescription}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{strings.cancel}</AlertDialogCancel>
              <Button
                type="button"
                variant="destructive"
                onClick={() => {
                  setDiscardConfirmOpen(false);
                  onClose();
                }}
              >
                {strings.discardChanges}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
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
  busy: boolean;
  onRemoveCodexHomeChange: (remove: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
};

function DeleteDialog({
  profile,
  removeCodexHome,
  busy,
  onRemoveCodexHomeChange,
  onCancel,
  onConfirm,
}: DeleteDialogProps) {
  const strings = useAppStrings();
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) {
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
                disabled={busy}
                onCheckedChange={(checked) => onRemoveCodexHomeChange(checked === true)}
              />
              <span className="text-sm">{strings.alsoDeleteCodexHome}</span>
            </Label>
            {removeCodexHome ? (
              <div className="grid gap-3">
                <p className="text-xs text-muted-foreground font-mono bg-muted/50 rounded-md px-3 py-2">
                  {profile.codex_home}
                </p>
              </div>
            ) : null}
          </>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy} onClick={onCancel}>
            {strings.cancel}
          </AlertDialogCancel>
          <Button
            type="button"
            variant="destructive"
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : null}
            {strings.delete}
          </Button>
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
      <DialogContent className="max-w-sm" closeLabel={strings.close}>
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
  const urlOptions = useMemo(() => remoteQrUrlOptions(remoteQr.remote), [remoteQr.remote]);
  const [selectedUrlKind, setSelectedUrlKind] = useState<RemoteQrUrlKind>(remoteQr.defaultUrlKind);
  const selectedOption =
    urlOptions.find((option) => option.kind === selectedUrlKind) ?? urlOptions[0] ?? null;
  const selectedUrl = selectedOption?.url ?? "";
  const selectedUrlLabel = selectedOption ? remoteQrUrlLabel(selectedOption.kind, strings) : strings.remoteUrl;
  const qrUrl = useMemo(() => compactRemoteQrUrl(selectedUrl), [selectedUrl]);
  const qrMarkup = useMemo(() => createQrSvg(qrUrl, { moduleSize: 5, quietZone: 4 }), [qrUrl]);

  useEffect(() => {
    if (!selectedOption && urlOptions[0]) {
      setSelectedUrlKind(urlOptions[0].kind);
    }
  }, [selectedOption, urlOptions]);

  useEffect(() => {
    setCopySucceeded(false);
    if (copyResetTimerRef.current !== null) {
      window.clearTimeout(copyResetTimerRef.current);
      copyResetTimerRef.current = null;
    }
  }, [selectedUrl]);

  useEffect(() => {
    return () => {
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    };
  }, []);

  const handleCopyUrl = useCallback(async () => {
    try {
      await copyText(selectedUrl, strings.clipboardUnavailable);
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
  }, [onError, selectedUrl, strings.clipboardUnavailable]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent className="max-w-md overflow-hidden p-0" closeLabel={strings.close}>
        <DialogHeader className="border-b border-border px-5 py-4">
          <DialogTitle className="text-base">{strings.remoteQr}</DialogTitle>
          <DialogDescription>{remoteQr.profile.name}</DialogDescription>
        </DialogHeader>
        <div className="px-5 py-5 flex flex-col gap-4">
          <div
            className="mx-auto rounded-lg bg-white p-3 shadow-sm"
            dangerouslySetInnerHTML={{ __html: qrMarkup }}
          />
          <div className="space-y-2">
            {urlOptions.length > 1 ? (
              <div className="grid grid-cols-2 gap-1 rounded-md border border-border bg-muted/30 p-1">
                {urlOptions.map((option) => {
                  const selected = option.kind === selectedOption?.kind;
                  return (
                    <button
                      key={option.kind}
                      type="button"
                      aria-pressed={selected}
                      className={cn(
                        "h-8 rounded-[5px] px-2 text-xs font-medium text-muted-foreground transition-colors",
                        selected ? "bg-background text-foreground shadow-sm" : "hover:bg-background/70 hover:text-foreground",
                      )}
                      onClick={() => setSelectedUrlKind(option.kind)}
                    >
                      {remoteQrUrlLabel(option.kind, strings)}
                    </button>
                  );
                })}
              </div>
            ) : null}
            <div className="text-[11px] font-semibold uppercase text-muted-foreground">{selectedUrlLabel}</div>
            <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
              <div className="min-w-0 break-all font-mono text-xs">
                {selectedUrl}
              </div>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              type="button"
              disabled={!selectedUrl}
              onClick={handleCopyUrl}
            >
              {copySucceeded ? <CheckCircle2 className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
              {copySucceeded ? strings.copied : strings.copyUrl}
            </Button>
            <Button
              type="button"
              disabled={!selectedUrl}
              onClick={() => openUrl(selectedUrl).catch(onError)}
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
      <DialogContent className="max-w-md overflow-hidden p-0" closeLabel={strings.close}>
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
  return (import.meta.env.VITE_CODEXL_SERVER_URL || DEFAULT_CODEXL_SERVER_URL).replace(/\/+$/, "");
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

class DesktopAuthHttpError extends Error {
  status: number;

  constructor(action: string, status: number) {
    super(`${action} failed: ${status}`);
    this.name = "DesktopAuthHttpError";
    this.status = status;
  }
}

async function refreshDesktopAuth(
  refreshToken: string,
): Promise<Extract<DesktopAuthRefreshResponse, { status: "refreshed" }>> {
  const response = await fetch(codexServerUrl("/api/desktop-auth/refresh"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    cache: "no-store",
    body: JSON.stringify({ refreshToken }),
  });

  if (!response.ok) {
    throw new DesktopAuthHttpError("desktop auth refresh", response.status);
  }

  const result = (await response.json()) as DesktopAuthRefreshResponse;
  if (result.status !== "refreshed") {
    throw new DesktopAuthHttpError(
      "desktop auth refresh",
      result.status === "invalid" ? 401 : 503,
    );
  }

  return result;
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
      subscription_expires_at: subscriptionExpiresAtFromDesktopLogin(result),
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
    subscription_expires_at: subscriptionExpiresAtFromDesktopLogin(result),
    access_token: "",
    refresh_token: "",
    expires_at: 0,
  };
}

function remoteCloudAuthFromDesktopRefresh(
  result: Extract<DesktopAuthRefreshResponse, { status: "refreshed" }>,
): RemoteCloudAuthConfig {
  return {
    user_id: result.cloudAuth.userId,
    display_name: result.cloudAuth.displayName,
    email: result.cloudAuth.email,
    avatar_url: result.cloudAuth.avatarUrl ?? "",
    is_pro: result.user.hasSubscription,
    subscription_expires_at: subscriptionExpiresAtFromDesktopAuthPayload(result),
    access_token: result.cloudAuth.accessToken,
    refresh_token: result.cloudAuth.refreshToken,
    expires_at: result.cloudAuth.expiresAt,
  };
}

function subscriptionExpiresAtFromDesktopLogin(
  result: Extract<DesktopAuthPollResponse, { status: "authenticated" }>,
) {
  return subscriptionExpiresAtFromDesktopAuthPayload(result);
}

function subscriptionExpiresAtFromDesktopAuthPayload(result: {
  user: DesktopAuthUser;
  cloudAuth?: DesktopCloudAuth | null;
}) {
  const user = objectValue(result.user);
  const cloudAuth = objectValue(result.cloudAuth);
  const userSubscription = objectValue(user.subscription);
  const cloudSubscription = objectValue(cloudAuth.subscription);

  return firstUnixSeconds(
    cloudAuth.subscription_expires_at,
    cloudAuth.subscriptionExpiresAt,
    cloudAuth.subscription_ends_at,
    cloudAuth.subscriptionEndsAt,
    cloudAuth.pro_expires_at,
    cloudAuth.proExpiresAt,
    cloudAuth.current_period_end,
    cloudAuth.currentPeriodEnd,
    cloudSubscription.expires_at,
    cloudSubscription.expiresAt,
    cloudSubscription.ends_at,
    cloudSubscription.endsAt,
    cloudSubscription.current_period_end,
    cloudSubscription.currentPeriodEnd,
    user.subscription_expires_at,
    user.subscriptionExpiresAt,
    user.subscription_ends_at,
    user.subscriptionEndsAt,
    user.pro_expires_at,
    user.proExpiresAt,
    user.current_period_end,
    user.currentPeriodEnd,
    userSubscription.expires_at,
    userSubscription.expiresAt,
    userSubscription.ends_at,
    userSubscription.endsAt,
    userSubscription.current_period_end,
    userSubscription.currentPeriodEnd,
  );
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

function remoteRelayUrlFromDesktopRefresh(
  result: Extract<DesktopAuthRefreshResponse, { status: "refreshed" }>,
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
    subscription_expires_at: 0,
    access_token: "",
    refresh_token: "",
    expires_at: 0,
  };
}

function hasRemoteCloudIdentity(auth: RemoteCloudAuthConfig | null | undefined) {
  if (!auth?.user_id.trim()) {
    return false;
  }

  if (!auth.access_token.trim()) {
    return false;
  }

  return auth.expires_at === 0 || auth.expires_at > Math.floor(Date.now() / 1000) + 60;
}

function remoteCloudAuthStatusRefreshDelay(
  auth: RemoteCloudAuthConfig | null | undefined,
  retryAtMs: number | null = null,
) {
  if (!auth?.user_id.trim() || !auth.access_token.trim() || auth.expires_at === 0) {
    return null;
  }

  const now = Date.now();
  const refreshToken = auth.refresh_token.trim();
  if (refreshToken && retryAtMs && retryAtMs > now) {
    return Math.min(retryAtMs - now, MAX_AUTH_STATUS_REFRESH_DELAY_MS);
  }

  const refreshAtMs = auth.expires_at * 1000 - (refreshToken ? AUTH_REFRESH_SKEW_MS : 60_000);
  const delayMs = refreshAtMs - Date.now();
  if (delayMs <= 0 && !refreshToken) {
    return null;
  }

  return Math.min(Math.max(0, delayMs), MAX_AUTH_STATUS_REFRESH_DELAY_MS);
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

function remoteCloudSubscriptionExpiresAt(auth: RemoteCloudAuthConfig) {
  const claims = remoteCloudJwtClaims(auth);
  return firstUnixSeconds(
    auth.subscription_expires_at,
    claims?.subscription_expires_at,
    claims?.subscriptionExpiresAt,
    claims?.subscription_ends_at,
    claims?.subscriptionEndsAt,
    claims?.pro_expires_at,
    claims?.proExpiresAt,
    claims?.current_period_end,
    claims?.currentPeriodEnd,
  );
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

function firstUnixSeconds(...values: unknown[]) {
  for (const value of values) {
    const seconds = unixSecondsValue(value);
    if (seconds > 0) {
      return seconds;
    }
  }
  return 0;
}

function unixSecondsValue(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.trunc(value > 1_000_000_000_000 ? value / 1000 : value);
  }

  if (typeof value !== "string") {
    return 0;
  }

  const trimmed = value.trim();
  if (!trimmed) {
    return 0;
  }

  if (/^\d+$/.test(trimmed)) {
    const parsed = Number.parseInt(trimmed, 10);
    return Number.isFinite(parsed) ? Math.trunc(parsed > 1_000_000_000_000 ? parsed / 1000 : parsed) : 0;
  }

  const parsed = Date.parse(trimmed);
  return Number.isFinite(parsed) ? Math.trunc(parsed / 1000) : 0;
}

function formatAccountDate(unixSeconds: number, language: Language) {
  const date = new Date(unixSeconds * 1000);
  if (!Number.isFinite(date.getTime())) {
    return "";
  }

  return new Intl.DateTimeFormat(language === "zh" ? "zh-CN" : "en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
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

async function copyText(text: string, unavailableMessage = "clipboard is not available") {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall back to execCommand below for embedded WebViews and non-secure contexts.
    }
  }

  if (copyTextWithHiddenSelection(text)) {
    return;
  }

  throw new Error(unavailableMessage);
}

function copyTextWithHiddenSelection(text: string) {
  if (!document?.body || typeof document.execCommand !== "function") {
    return false;
  }

  const activeElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.readOnly = true;
  textarea.setAttribute("aria-hidden", "true");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  textarea.style.top = "0";
  textarea.style.opacity = "0";

  document.body.appendChild(textarea);
  textarea.select();
  textarea.setSelectionRange(0, textarea.value.length);
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  } finally {
    document.body.removeChild(textarea);
    activeElement?.focus();
  }
  return copied;
}

function normalizeRemoteFrontendMode(value: unknown): RemoteFrontendMode {
  if (typeof value !== "string") {
    return "app";
  }
  switch (value.trim().toLowerCase()) {
    case "cli":
      return "cli";
    case "claude-code":
    case "claude_code":
    case "claude code":
      return "claude-code";
    default:
      return "app";
  }
}

function remoteFrontendModeUsesCli(mode: RemoteFrontendMode | string) {
  const normalized = normalizeRemoteFrontendMode(mode);
  return normalized === "cli" || normalized === "claude-code";
}

function remoteControlReadyForQr(remote: RemoteControlInfo | null) {
  if (!remote?.running || (!remote.url && !remote.lan_url)) {
    return false;
  }
  return remote.cdp_ready === true;
}

function remoteQrUrlOptions(remote: RemoteControlInfo): RemoteQrUrlOption[] {
  const remoteUrl = typeof remote.url === "string" ? remote.url.trim() : "";
  const lanUrl = typeof remote.lan_url === "string" ? remote.lan_url.trim() : "";
  const preferredKind: RemoteQrUrlKind =
    remote.connection_mode === "cloud" && remoteUrl ? "remote" : "lan";
  const preferredFirst: RemoteQrUrlOption[] =
    preferredKind === "remote"
      ? [
          { kind: "remote", url: remoteUrl },
          { kind: "lan", url: lanUrl },
        ]
      : [
          { kind: "lan", url: lanUrl },
          { kind: "remote", url: remoteUrl },
        ];
  const seenUrls = new Set<string>();
  const options: RemoteQrUrlOption[] = [];

  for (const option of preferredFirst) {
    if (!option.url || seenUrls.has(option.url)) {
      continue;
    }
    seenUrls.add(option.url);
    options.push(option);
  }

  return options;
}

function remoteQrUrlLabel(kind: RemoteQrUrlKind, strings: AppStrings) {
  return kind === "remote" ? strings.remoteUrl : strings.lanUrl;
}

function compactRemoteQrUrl(value: string) {
  try {
    const url = new URL(value);
    for (const key of [
      "webAssetMode",
      "webAssetBaseUrl",
      "webAssetVersion",
      "web_asset_mode",
      "web_asset_base_url",
      "web_asset_version",
    ]) {
      url.searchParams.delete(key);
    }
    return url.toString();
  } catch {
    return value;
  }
}

function normalizeRegistryUrl(value: string) {
  return value.trim().replace(/\/+$/, "");
}

function codexWebAssetVersionOptions(versions: string[], currentVersion: string) {
  const seen = new Set<string>();
  const options: string[] = [];
  for (const version of [
    currentVersion.trim(),
    DEFAULT_CODEX_WEB_ASSET_VERSION,
    ...versions.map((item) => item.trim()),
  ]) {
    if (!version || seen.has(version)) {
      continue;
    }
    seen.add(version);
    options.push(version);
  }
  return options;
}

function defaultRemoteFrontendFormFields(codexAppPath: string): Partial<ProviderForm> {
  return {
    remoteFrontendMode: codexAppPath.trim() ? "app" : "cli",
    remoteWebAssetRegistryUrl: DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL,
    remoteWebAssetVersion: DEFAULT_CODEX_WEB_ASSET_VERSION,
    remoteWebAssetVersions: [],
    remoteWebAssetVersionsLoading: false,
    remoteWebAssetRegistryError: "",
  };
}

function profileRemoteFrontendFormFields(
  profile: ProviderProfile,
  codexAppPath = "",
): Partial<ProviderForm> {
  const remoteFrontendMode = normalizeRemoteFrontendMode(profile.remote_frontend_mode);
  return {
    remoteFrontendMode:
      remoteFrontendMode === "app" && !codexAppPath.trim() ? "cli" : remoteFrontendMode,
    remoteWebAssetRegistryUrl:
      normalizeRegistryUrl(profile.remote_web_asset_registry_url || "") ||
      DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL,
    remoteWebAssetVersion:
      (profile.remote_web_asset_version || "").trim() || DEFAULT_CODEX_WEB_ASSET_VERSION,
    remoteWebAssetVersions: [],
    remoteWebAssetVersionsLoading: false,
    remoteWebAssetRegistryError: "",
  };
}

function readRemoteFrontendConfig(
  form: ProviderForm,
  strings: AppStrings,
  showError: (error: unknown) => void,
): {
  remote_frontend_mode: RemoteFrontendMode;
  remote_web_asset_registry_url: string;
  remote_web_asset_version: string;
} | null {
  const mode = normalizeRemoteFrontendMode(form.remoteFrontendMode);
  if (mode === "app" || mode === "claude-code") {
    return {
      remote_frontend_mode: mode,
      remote_web_asset_registry_url: normalizeRegistryUrl(form.remoteWebAssetRegistryUrl),
      remote_web_asset_version: form.remoteWebAssetVersion.trim() || DEFAULT_CODEX_WEB_ASSET_VERSION,
    };
  }

  const registryUrl = normalizeRegistryUrl(form.remoteWebAssetRegistryUrl);
  const version = form.remoteWebAssetVersion.trim();
  if (!registryUrl) {
    showError(strings.registryUrlRequired);
    return null;
  }
  try {
    const parsed = new URL(registryUrl);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new Error("invalid protocol");
    }
  } catch {
    showError(strings.registryUrlRequired);
    return null;
  }
  if (!version) {
    showError(strings.registryVersionRequired);
    return null;
  }
  if (form.remoteWebAssetRegistryError) {
    showError(form.remoteWebAssetRegistryError);
    return null;
  }
  if (
    form.remoteWebAssetVersions.length > 0 &&
    !form.remoteWebAssetVersions.includes(version)
  ) {
    showError(strings.registryVersionRequired);
    return null;
  }
  if (!form.remoteWebAssetVersionsLoading && form.remoteWebAssetVersions.length === 0) {
    showError(strings.registryVersionsUnavailable);
    return null;
  }

  return {
    remote_frontend_mode: mode,
    remote_web_asset_registry_url: registryUrl,
    remote_web_asset_version: version,
  };
}

async function loadCodexWebAssetVersions(registryUrl: string): Promise<CodexWebAssetVersions> {
  return invoke<CodexWebAssetVersions>("list_codex_web_asset_versions", {
    registryUrl: normalizeRegistryUrl(registryUrl),
  });
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
  const remoteFrontend = readRemoteFrontendConfig(form, strings, showError);
  if (!remoteFrontend) return null;
  return { ...provider, ...remoteFrontend };
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
  const remoteFrontend = readRemoteFrontendConfig(form, strings, showError);
  if (!remoteFrontend) return null;
  return { ...provider, ...remoteFrontend };
}

function nextAiGatewayProfileName(form: ProviderForm) {
  const explicitName = form.providerName.trim();
  if (isProviderProfileName(explicitName)) {
    return explicitName;
  }

  const source = form.workspaceName.trim() || form.gatewayModel.trim() || NEXT_AI_GATEWAY_PROVIDER_NAME;
  const slug = providerProfileNameSlug(source);
  return `${NEXT_AI_GATEWAY_PROVIDER_NAME}-${slug}-${shortStringHash(source)}`;
}

function isProviderProfileName(value: string) {
  return /^[A-Za-z0-9_-]+$/.test(value) && value.toLowerCase() !== "default";
}

function providerProfileNameSlug(value: string) {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/^-+|-+$/g, "");
  return slug || "workspace";
}

function shortStringHash(value: string) {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

function readNextAiGatewayProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  modelRef: React.RefObject<HTMLButtonElement | null>,
  strings: AppStrings,
  showError: (error: unknown) => void,
  extensionsEnabled: boolean,
): NextAiGatewayProvider | null {
  const provider = {
    workspace_name: form.workspaceName.trim(),
    name: nextAiGatewayProfileName(form),
    model: form.gatewayModel.trim(),
    proxy_url: form.proxyUrl.trim(),
    bot: readBotConfig(form, form.workspaceName),
  };

  if (!provider.workspace_name) {
    showError(strings.nameRequired);
    workspaceNameRef.current?.focus();
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
  const remoteFrontend = readRemoteFrontendConfig(form, strings, showError);
  if (!remoteFrontend) return null;
  return { ...provider, ...remoteFrontend };
}

function readExistingProviderForm(
  form: ProviderForm,
  workspaceNameRef: React.RefObject<HTMLInputElement | null>,
  providerRef: React.RefObject<HTMLButtonElement | null>,
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
  if (extensionsEnabled && !validateBotAuth(form, strings, showError)) {
    return null;
  }
  const remoteFrontend = readRemoteFrontendConfig(form, strings, showError);
  if (!remoteFrontend) return null;
  return { ...provider, ...remoteFrontend };
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

function botConfigDraftSignature(form: ProviderForm): string {
  const platform = normalizeBotPlatform(form.botPlatform);
  const authType = normalizeBotAuthType(platform, form.botAuthType);
  return jsonSignature({
    name: form.workspaceName.trim(),
    platform,
    authType,
    authFields: pickBotAuthFields(form.botAuthFields, platform, authType),
  });
}

function workspaceDialogDraftSignature(form: ProviderForm, providerMode: ProviderMode): string {
  const remoteFrontendMode = normalizeRemoteFrontendMode(form.remoteFrontendMode);
  const botPlatform = form.botEnabled ? normalizeBotPlatform(form.botPlatform) : "none";
  const botAuthType = normalizeBotAuthType(botPlatform, form.botAuthType);
  return jsonSignature({
    providerMode: remoteFrontendMode === "claude-code" ? "none" : providerMode,
    workspaceName: form.workspaceName.trim(),
    existingProfileName: form.existingProfileName.trim(),
    existingBaseUrl: form.existingBaseUrl.trim(),
    existingApiKey: form.existingApiKey.trim(),
    existingModel: form.existingModel.trim(),
    providerName: form.providerName.trim(),
    providerBaseUrl: form.providerBaseUrl.trim(),
    providerApiKey: form.providerApiKey.trim(),
    providerModel: form.providerModel.trim(),
    gatewayModel: form.gatewayModel.trim(),
    proxyUrl: form.proxyUrl.trim(),
    remoteFrontendMode,
    remoteWebAssetRegistryUrl: normalizeRegistryUrl(form.remoteWebAssetRegistryUrl),
    remoteWebAssetVersion: form.remoteWebAssetVersion.trim() || DEFAULT_CODEX_WEB_ASSET_VERSION,
    botEnabled: form.botEnabled && botPlatform !== "none",
    botPlatform,
    botAuthType,
    botAuthFields: pickBotAuthFields(form.botAuthFields, botPlatform, botAuthType),
    botConfigId: form.botEnabled ? form.botConfigId.trim() : "",
    botTenantId: form.botEnabled ? form.botTenantId.trim() : "",
    botIntegrationId: form.botEnabled ? form.botIntegrationId.trim() : "",
    botStateDir: form.botEnabled ? form.botStateDir.trim() : "",
    botForwardAllCodexMessages: form.botEnabled && botPlatform !== "none" && form.botForwardAllCodexMessages,
    botHandoffEnabled: form.botEnabled && botPlatform !== "none" && form.botHandoffEnabled,
    botHandoffIdleSeconds:
      form.botEnabled && botPlatform !== "none" && form.botHandoffEnabled
        ? form.botHandoffIdleSeconds.trim()
        : "",
    botHandoffPhoneWifiTargets:
      form.botEnabled && botPlatform !== "none" && form.botHandoffEnabled
        ? firstTarget(form.botHandoffPhoneWifiTargets)
        : "",
    botHandoffPhoneBluetoothTargets:
      form.botEnabled && botPlatform !== "none" && form.botHandoffEnabled
        ? firstTarget(form.botHandoffPhoneBluetoothTargets)
        : "",
  });
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
  const agent = objectValue(config.agent);
  const providerPlugins = arrayValue(config.providerPlugins).map(objectValue);
  return {
    host: stringValue(config.host, "127.0.0.1"),
    port: numberString(config.port, "14589"),
    usageCaptureEnabled: gatewayUsageCaptureEnabledFromConfig(config),
    requestLoggingEnabled: gatewayRequestLoggingEnabledFromConfig(config),
    providers: arrayValue(config.Providers ?? config.providers).map((item) =>
      gatewayProviderFormFromRaw(objectValue(item), providerPlugins),
    ),
    mcpServers: arrayValue(agent.mcpServers).map((item) =>
      gatewayMcpServerFormFromRaw(objectValue(item)),
    ),
    virtualModelProfiles: arrayValue(config.virtualModelProfiles).map((item) =>
      gatewayVirtualProfileFormFromRaw(objectValue(item)),
    ),
    rawConfig: config,
  };
}

function gatewayUsageCaptureEnabledFromConfig(config: JsonObject): boolean {
  const codexlUsageCapture = objectValue(config.codexlUsageCapture);
  return booleanValue(codexlUsageCapture.enabled, false);
}

function gatewayRequestLoggingEnabledFromConfig(config: JsonObject): boolean {
  const rawTrace = objectValue(config.rawTrace);
  const mode = stringValue(rawTrace.mode, "").trim().toLowerCase();
  return booleanValue(rawTrace.enabled, false) && mode !== "disabled";
}

function gatewayModelsFromConfig(config: JsonObject): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  const baseModels: string[] = [];
  for (const item of arrayValue(config.Providers ?? config.providers)) {
    const provider = objectValue(item);
    const providerName = stringValue(provider.name, "").trim();
    for (const model of gatewayProviderModels(provider)) {
      const option = gatewayModelOption(providerName, model);
      if (option && !seen.has(option)) {
        seen.add(option);
        models.push(option);
      }
      if (option) {
        baseModels.push(option);
      }
    }
  }
  for (const model of materializedGatewayVirtualModels(config, baseModels)) {
    if (model && !seen.has(model)) {
      seen.add(model);
      models.push(model);
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

const CODEXL_DEEPSEEK_THINKING_PLUGIN_KEY_PREFIX = "codexl-deepseek-thinking";

function gatewayProviderFormFromRaw(raw: JsonObject, providerPlugins: JsonObject[] = []): GatewayProviderForm {
  const name = stringValue(raw.name, "");
  const models = gatewayProviderModels(raw).join(", ");
  return {
    id: newLocalId(),
    name,
    type: stringValue(raw.type ?? raw.provider, "openai_responses"),
    apiKey: stringValue(raw.apikey ?? raw.apiKey, ""),
    baseUrl: stringValue(raw.baseurl ?? raw.baseUrl, ""),
    models,
    thinkingEffortModels: gatewayProviderDeepSeekThinkingModels(name, models, providerPlugins),
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
    thinkingEffortModels: [],
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
  const codexlUsageCapture = objectValue(config.codexlUsageCapture);
  codexlUsageCapture.enabled = form.usageCaptureEnabled;
  config.codexlUsageCapture = codexlUsageCapture;
  const rawTrace = objectValue(config.rawTrace);
  rawTrace.enabled = form.requestLoggingEnabled;
  if (form.requestLoggingEnabled) {
    const mode = stringValue(rawTrace.mode, "").trim().toLowerCase();
    if (!mode || mode === "disabled") {
      rawTrace.mode = "body_redacted";
    }
    if (rawTrace.deleteLocalAfterUpload === undefined) {
      rawTrace.deleteLocalAfterUpload = false;
    }
  }
  config.rawTrace = rawTrace;
  config.Providers = form.providers.map(gatewayProviderConfigFromForm);
  const providerPlugins = gatewayProviderPluginsFromForm(form);
  if (providerPlugins.length > 0) {
    config.providerPlugins = providerPlugins;
  } else {
    delete config.providerPlugins;
  }
  const agent = objectValue(config.agent);
  const storage = objectValue(agent.storage);
  agent.storage = Object.keys(storage).length > 0 ? storage : { type: "filesystem" };
  agent.mcpServers = form.mcpServers.map(gatewayMcpServerConfigFromForm);
  config.agent = agent;
  config.virtualModelProfiles = form.virtualModelProfiles.map(gatewayVirtualProfileConfigFromForm);

  return config;
}

function gatewayConfigFormSignature(form: GatewayConfigForm): string {
  return jsonSignature(gatewayConfigFromForm(form));
}

function gatewayConfigFormSignatureOrNull(form: GatewayConfigForm): string | null {
  try {
    return gatewayConfigFormSignature(form);
  } catch {
    return null;
  }
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

function gatewayProviderPluginsFromForm(form: GatewayConfigForm): JsonObject[] {
  const providerNames = new Set(
    form.providers.map((provider) => provider.name.trim()).filter(Boolean),
  );
  const existingPlugins = arrayValue(form.rawConfig.providerPlugins)
    .map(objectValue)
    .filter((plugin) => {
      if (isCodexLDeepSeekThinkingPlugin(plugin)) {
        return false;
      }
      const providerName = stringValue(plugin.providerName, "").trim();
      return !(
        providerName &&
        providerNames.has(providerName) &&
        providerPluginHasDeepSeekThinking(plugin)
      );
    });
  const generatedPlugins = form.providers
    .filter((provider) => gatewayProviderUsesDeepSeekThinking(provider))
    .map(gatewayDeepSeekThinkingPluginFromProvider);
  return [...existingPlugins, ...generatedPlugins];
}

function gatewayDeepSeekThinkingPluginFromProvider(provider: GatewayProviderForm): JsonObject {
  const providerName = provider.name.trim();
  const models = normalizeThinkingEffortModels(provider.thinkingEffortModels, provider.models);
  return {
    key: `${CODEXL_DEEPSEEK_THINKING_PLUGIN_KEY_PREFIX}-${slugForGatewayPluginKey(providerName || provider.id)}`,
    enabled: true,
    providerName,
    deepseekThinking: {
      enabled: true,
      models,
    },
  };
}

function gatewayProviderUsesDeepSeekThinking(provider: GatewayProviderForm): boolean {
  return (
    provider.type === "openai_chat_completions" &&
    normalizeThinkingEffortModels(provider.thinkingEffortModels, provider.models).length > 0 &&
    provider.name.trim().length > 0
  );
}

function deepSeekV4ModelsFromProvider(provider: GatewayProviderForm): string[] {
  return deepSeekV4ModelsFromText(provider.models);
}

function deepSeekV4ModelsFromText(models: string): string[] {
  const result: string[] = [];
  const seen = new Set<string>();
  for (const model of commaList(models)) {
    const normalized = normalizeGatewayModelName(model);
    const key = gatewayModelComparisonKey(normalized);
    if (!isDeepSeekV4Model(normalized) || seen.has(key)) {
      continue;
    }
    seen.add(key);
    result.push(normalized);
  }
  return result;
}

function normalizeThinkingEffortModels(selectedModels: string[], models: string): string[] {
  const availableModels = deepSeekV4ModelsFromText(models);
  const selected = new Set(selectedModels.map(gatewayModelComparisonKey).filter(Boolean));
  return availableModels.filter((model) => selected.has(gatewayModelComparisonKey(model)));
}

function normalizeGatewayModelName(model: string): string {
  return model.trim().replace(/^\/+/, "");
}

function isDeepSeekV4Model(model: string): boolean {
  const modelName = normalizeGatewayModelName(model).split("/").pop()?.toLowerCase() || "";
  return modelName.startsWith("deepseek-v4-");
}

function gatewayModelComparisonKey(model: string): string {
  const normalized = normalizeGatewayModelName(model).toLowerCase();
  return normalized.split("/").pop() || normalized;
}

function gatewayProviderDeepSeekThinkingModels(
  providerName: string,
  models: string,
  providerPlugins: JsonObject[],
): string[] {
  const name = providerName.trim();
  if (!name) {
    return [];
  }
  const availableModels = deepSeekV4ModelsFromText(models);
  if (availableModels.length === 0) {
    return [];
  }
  for (const plugin of providerPlugins) {
    if (booleanValue(plugin.enabled, true) === false) {
      continue;
    }
    if (stringValue(plugin.providerName, "").trim() !== name) {
      continue;
    }
    if (!providerPluginHasDeepSeekThinking(plugin)) {
      continue;
    }
    const pluginModels = providerPluginDeepSeekThinkingModels(plugin);
    if (pluginModels === undefined) {
      return availableModels;
    }
    const selected = new Set(pluginModels.map(gatewayModelComparisonKey).filter(Boolean));
    return availableModels.filter((model) => selected.has(gatewayModelComparisonKey(model)));
  }
  return [];
}

function providerPluginDeepSeekThinkingModels(plugin: JsonObject): string[] | undefined {
  const raw = plugin.deepseekThinking ?? plugin.deepSeekThinking;
  if (raw === true) {
    return undefined;
  }
  const deepseekThinking = objectValue(raw);
  const hasModels = Object.prototype.hasOwnProperty.call(deepseekThinking, "models");
  const hasModel = Object.prototype.hasOwnProperty.call(deepseekThinking, "model");
  if (!hasModels && !hasModel) {
    return undefined;
  }
  return stringListFromUnknown(deepseekThinking.models ?? deepseekThinking.model);
}

function providerPluginHasDeepSeekThinking(plugin: JsonObject): boolean {
  const raw = plugin.deepseekThinking ?? plugin.deepSeekThinking;
  if (raw === true) {
    return true;
  }
  const deepseekThinking = objectValue(raw);
  return Object.keys(deepseekThinking).length > 0 && booleanValue(deepseekThinking.enabled, true);
}

function isCodexLDeepSeekThinkingPlugin(plugin: JsonObject): boolean {
  return stringValue(plugin.key, "").startsWith(CODEXL_DEEPSEEK_THINKING_PLUGIN_KEY_PREFIX);
}

function slugForGatewayPluginKey(value: string): string {
  return (
    value
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "provider"
  );
}

function gatewayMcpServerFormFromRaw(raw: JsonObject): GatewayMcpServerForm {
  const transport = stringValue(raw.transport, "stdio") === "websocket" ? "websocket" : "stdio";
  return {
    id: newLocalId(),
    name: stringValue(raw.name, ""),
    enabled: booleanValue(raw.enabled, true),
    transport,
    stdioMessageMode: gatewayMcpServerStdioMessageModeFromRaw(raw.stdioMessageMode, "newline-json"),
    command: stringValue(raw.command, ""),
    args: stringListText(raw.args),
    cwd: stringValue(raw.cwd, ""),
    url: stringValue(raw.url, ""),
    headersJson: jsonText(objectValue(raw.headers)),
    envJson: jsonText(objectValue(raw.env)),
    apiKey: stringValue(raw.apiKey, ""),
    apiKeyEnv: stringValue(raw.apiKeyEnv, ""),
    protocolVersion: stringValue(raw.protocolVersion, "2024-11-05"),
    startupTimeoutMs: numberString(raw.startupTimeoutMs, "10000"),
    requestTimeoutMs: numberString(raw.requestTimeoutMs, "30000"),
    raw,
  };
}

function createGatewayMcpServerForm(): GatewayMcpServerForm {
  return {
    id: newLocalId(),
    name: "",
    enabled: true,
    transport: "stdio",
    stdioMessageMode: "newline-json",
    command: "npx",
    args: "-y, @modelcontextprotocol/server-filesystem, .",
    cwd: "",
    url: "",
    headersJson: "{}",
    envJson: "{}",
    apiKey: "",
    apiKeyEnv: "",
    protocolVersion: "2024-11-05",
    startupTimeoutMs: "10000",
    requestTimeoutMs: "30000",
    raw: {},
  };
}

function cloneGatewayMcpServerForm(server: GatewayMcpServerForm): GatewayMcpServerForm {
  return {
    ...server,
    raw: cloneJsonObject(server.raw),
  };
}

function gatewayMcpServerConfigFromForm(server: GatewayMcpServerForm): JsonObject {
  const raw = cloneJsonObject(server.raw);
  raw.name = server.name.trim();
  raw.enabled = server.enabled;
  raw.transport = server.transport;
  raw.protocolVersion = server.protocolVersion.trim() || "2024-11-05";
  raw.startupTimeoutMs = integerValue(server.startupTimeoutMs, 10000);
  raw.requestTimeoutMs = integerValue(server.requestTimeoutMs, 30000);

  if (server.transport === "websocket") {
    raw.url = server.url.trim();
    raw.headers = jsonObjectFromText(server.headersJson, "Headers JSON");
    raw.apiKey = server.apiKey.trim();
    raw.apiKeyEnv = server.apiKeyEnv.trim();
    delete raw.command;
    delete raw.args;
    delete raw.env;
    delete raw.cwd;
    delete raw.stdioMessageMode;
    return raw;
  }

  raw.command = server.command.trim();
  raw.stdioMessageMode = server.stdioMessageMode;
  raw.args = stringListFromText(server.args, "Args");
  raw.env = jsonObjectFromText(server.envJson, "Env JSON");
  if (server.cwd.trim()) {
    raw.cwd = server.cwd.trim();
  } else {
    delete raw.cwd;
  }
  delete raw.url;
  delete raw.headers;
  delete raw.apiKey;
  delete raw.apiKeyEnv;
  return raw;
}

function gatewayMcpServerStdioMessageModeFromRaw(
  value: unknown,
  fallback: GatewayMcpServerStdioMessageMode,
): GatewayMcpServerStdioMessageMode {
  const normalized = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (normalized === "content-length" || normalized === "content_length" || normalized === "contentlength") {
    return "content-length";
  }
  if (normalized === "newline-json" || normalized === "newline_json" || normalized === "jsonl") {
    return "newline-json";
  }
  return fallback;
}

function gatewayMcpServerTarget(server: GatewayMcpServerForm): string {
  return server.transport === "websocket" ? server.url : [server.command, server.args].filter(Boolean).join(" ");
}

function gatewayVirtualProfileFormFromRaw(raw: JsonObject): GatewayVirtualProfileForm {
  const match = objectValue(raw.match);
  const baseModel = objectValue(raw.baseModel);
  const execution = objectValue(raw.execution);
  const materialization = objectValue(raw.materialization);
  const key = stringValue(raw.key, "");
  return {
    id: newLocalId(),
    profileId: stringValue(raw.id, key),
    key,
    displayName: stringValue(raw.displayName, key),
    description: stringValue(raw.description, ""),
    enabled: booleanValue(raw.enabled, true),
    exactAliases: stringListText(match.exactAliases),
    prefixes: stringListText(match.prefixes),
    suffixes: stringListText(match.suffixes),
    baseModelMode: normalizeGatewayVirtualBaseModelMode(stringValue(baseModel.mode, "request")),
    fixedModel: stringValue(baseModel.fixedModel, ""),
    matchMultimodal: booleanValue(execution.matchMultimodal ?? execution.match_multimodal, false),
    matchWebSearch: booleanValue(
      execution.matchWebSearch ?? execution.match_web_search ?? execution.matchWebsearch,
      false,
    ),
    maxTurns: numberString(execution.maxTurns, "6"),
    maxToolCalls: numberString(execution.maxToolCalls, "8"),
    clientToolsPolicy: stringValue(execution.clientToolsPolicy, "allow") === "deny" ? "deny" : "allow",
    includeInGatewayModels: booleanValue(materialization.includeInGatewayModels, true),
    tools: arrayValue(raw.tools).map((item) => gatewayVirtualToolFormFromRaw(objectValue(item))),
    raw,
  };
}

function createGatewayVirtualProfileForm(availableTools: GatewayAvailableTool[] = []): GatewayVirtualProfileForm {
  return {
    id: newLocalId(),
    profileId: "mcp-tools",
    key: "mcp-tools",
    displayName: "MCP Tools",
    description: "Inject enabled MCP tools through Gateway virtual models.",
    enabled: true,
    exactAliases: "",
    prefixes: "",
    suffixes: ":mcp-tools",
    baseModelMode: "strip_suffix",
    fixedModel: "",
    matchMultimodal: true,
    matchWebSearch: true,
    maxTurns: "6",
    maxToolCalls: "8",
    clientToolsPolicy: "allow",
    includeInGatewayModels: true,
    tools: availableTools.map(gatewayVirtualToolFormFromAvailableTool),
    raw: {},
  };
}

function cloneGatewayVirtualProfileForm(profile: GatewayVirtualProfileForm): GatewayVirtualProfileForm {
  return {
    ...profile,
    tools: profile.tools.map(cloneGatewayVirtualToolForm),
    raw: cloneJsonObject(profile.raw),
  };
}

function gatewayVirtualProfileConfigFromForm(profile: GatewayVirtualProfileForm): JsonObject {
  const raw = cloneJsonObject(profile.raw);
  const key = profile.key.trim() || profile.profileId.trim() || "vision-search";
  raw.id = key;
  raw.key = key;
  raw.displayName = profile.displayName.trim() || key;
  raw.description = profile.description.trim();
  raw.enabled = profile.enabled;
  raw.match = {
    exactAliases: stringListFromText(profile.exactAliases, "Exact Aliases"),
    prefixes: stringListFromText(profile.prefixes, "Prefixes"),
    suffixes: stringListFromText(profile.suffixes, "Suffixes"),
  };
  const baseModel = objectValue(raw.baseModel);
  baseModel.mode = profile.baseModelMode;
  if (profile.fixedModel.trim()) {
    baseModel.fixedModel = profile.fixedModel.trim();
  } else {
    delete baseModel.fixedModel;
  }
  raw.baseModel = baseModel;
  const execution = objectValue(raw.execution);
  execution.mode = "tool_loop";
  execution.maxTurns = integerValue(profile.maxTurns, 6);
  execution.maxToolCalls = integerValue(profile.maxToolCalls, 8);
  execution.clientToolsPolicy = profile.clientToolsPolicy;
  execution.matchMultimodal = profile.matchMultimodal;
  execution.matchWebSearch = profile.matchWebSearch;
  raw.execution = execution;
  const materialization = objectValue(raw.materialization);
  materialization.enabled = true;
  materialization.includeInGatewayModels = profile.includeInGatewayModels;
  raw.materialization = materialization;
  raw.tools = profile.tools.map(gatewayVirtualToolConfigFromForm).filter((tool) => stringValue(tool.name, ""));
  return raw;
}

function gatewayVirtualToolFormFromRaw(raw: JsonObject): GatewayVirtualToolForm {
  return {
    id: newLocalId(),
    name: stringValue(raw.name, ""),
    description: stringValue(raw.description, ""),
    visibility: stringValue(raw.visibility, "internal") === "client" ? "client" : "internal",
    inputSchemaJson: jsonText(objectValue(raw.inputSchema ?? raw.input_schema ?? raw.parameters)),
    raw,
  };
}

function gatewayVirtualToolFormFromAvailableTool(tool: GatewayAvailableTool): GatewayVirtualToolForm {
  const inputSchema = tool.inputSchema && Object.keys(tool.inputSchema).length > 0 ? tool.inputSchema : {};
  return {
    id: newLocalId(),
    name: tool.name,
    description: tool.description,
    visibility: "internal",
    inputSchemaJson: jsonText(inputSchema),
    raw: {},
  };
}

function cloneGatewayVirtualToolForm(tool: GatewayVirtualToolForm): GatewayVirtualToolForm {
  return {
    ...tool,
    raw: cloneJsonObject(tool.raw),
  };
}

function gatewayVirtualToolConfigFromForm(tool: GatewayVirtualToolForm): JsonObject {
  const raw = cloneJsonObject(tool.raw);
  raw.name = tool.name.trim();
  raw.description = tool.description.trim();
  raw.visibility = tool.visibility;
  const inputSchema = jsonObjectFromText(tool.inputSchemaJson, "Input Schema JSON");
  if (Object.keys(inputSchema).length > 0) {
    raw.inputSchema = inputSchema;
  } else {
    delete raw.inputSchema;
  }
  delete raw.input_schema;
  delete raw.parameters;
  return raw;
}

function normalizeGatewayVirtualBaseModelMode(value: string): GatewayVirtualBaseModelMode {
  if (value === "fixed" || value === "strip_prefix" || value === "strip_suffix") {
    return value;
  }
  return "request";
}

function materializedGatewayVirtualModels(config: JsonObject, baseModels: string[]): string[] {
  const result: string[] = [];
  for (const item of arrayValue(config.virtualModelProfiles)) {
    const profile = objectValue(item);
    if (!booleanValue(profile.enabled, true)) {
      continue;
    }
    const materialization = objectValue(profile.materialization);
    if (
      !booleanValue(materialization.enabled, true) ||
      !booleanValue(materialization.includeInGatewayModels, true)
    ) {
      continue;
    }
    const match = objectValue(profile.match);
    const prefixes = stringListFromUnknown(match.prefixes);
    const suffixes = stringListFromUnknown(match.suffixes);
    for (const baseModel of baseModels) {
      const slashIndex = baseModel.indexOf("/");
      if (slashIndex < 0) {
        continue;
      }
      const provider = baseModel.slice(0, slashIndex);
      const model = baseModel.slice(slashIndex + 1);
      for (const prefix of prefixes) {
        result.push(`${provider}/${prefix}${model}`);
      }
      for (const suffix of suffixes) {
        result.push(`${provider}/${model}${suffix}`);
      }
    }
    const baseModel = objectValue(profile.baseModel);
    const fixedModel = stringValue(baseModel.fixedModel, "").trim();
    if (!fixedModel) {
      continue;
    }
    for (const alias of stringListFromUnknown(match.exactAliases)) {
      result.push(alias.includes("/") ? alias : gatewayModelOption(fixedModel.split("/")[0] || "", alias));
    }
  }
  return result;
}

function gatewayAvailableToolsFromResponse(response: GatewayToolsResponse): GatewayAvailableTool[] {
  const tools: GatewayAvailableTool[] = [];
  const seen = new Set<string>();
  for (const item of arrayValue(response.tools)) {
    const tool = gatewayAvailableToolFromRaw(objectValue(item));
    if (!tool || seen.has(tool.name)) continue;
    seen.add(tool.name);
    tools.push(tool);
  }
  return tools.sort((left, right) => left.name.localeCompare(right.name));
}

function gatewayAvailableToolFromRaw(raw: JsonObject): GatewayAvailableTool | null {
  const name = stringValue(raw.name, "").trim();
  if (!name) return null;
  const inputSchema = objectValue(raw.inputSchema ?? raw.input_schema ?? raw.parameters);
  return {
    name,
    description: stringValue(raw.description, ""),
    inputSchema: Object.keys(inputSchema).length > 0 ? inputSchema : undefined,
  };
}

function objectValue(value: unknown): JsonObject {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonObject) : {};
}

function arrayValue(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function booleanValue(value: unknown, fallback: boolean): boolean {
  if (typeof value === "boolean") {
    return value;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "true") return true;
    if (normalized === "false") return false;
  }
  return fallback;
}

function stringValue(value: unknown, fallback: string): string {
  return typeof value === "string" ? value : fallback;
}

function numberString(value: unknown, fallback: string): string {
  return typeof value === "number" && Number.isFinite(value) ? String(value) : fallback;
}

function numberValue(value: unknown, fallback: number): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  const parsed = Number.parseFloat(String(value ?? ""));
  return Number.isFinite(parsed) ? parsed : fallback;
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

function normalizeModelOptions(values: string[]) {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const model = String(value || "").trim().replace(/^\/+/, "");
    if (!model || seen.has(model)) {
      continue;
    }
    seen.add(model);
    result.push(model);
  }
  return result;
}

function stringListText(value: unknown): string {
  return stringListFromUnknown(value).join(", ");
}

function stringListFromUnknown(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map((item) => stringValue(item, "").trim()).filter(Boolean);
  }
  if (typeof value === "string") {
    return commaList(value);
  }
  return [];
}

function stringListFromText(value: string, label: string): string[] {
  const trimmed = value.trim();
  if (!trimmed) {
    return [];
  }
  if (trimmed.startsWith("[")) {
    const parsed = parseJsonText(trimmed, label);
    if (!Array.isArray(parsed) || parsed.some((item) => typeof item !== "string")) {
      throw new Error(`${label} must be a JSON string array.`);
    }
    return parsed.map((item) => item.trim()).filter(Boolean);
  }
  return commaList(value);
}

function jsonText(value: unknown): string {
  return JSON.stringify(objectValue(value), null, 2);
}

function jsonObjectFromText(value: string, label: string): JsonObject {
  const trimmed = value.trim();
  if (!trimmed) {
    return {};
  }
  const parsed = parseJsonText(trimmed, label);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${label} must be a JSON object.`);
  }
  return parsed as JsonObject;
}

function parseJsonText(value: string, label: string): unknown {
  try {
    return JSON.parse(value);
  } catch (error) {
    throw new Error(`${label} has invalid JSON: ${errorMessage(error)}`);
  }
}

function cloneJsonObject(value: JsonObject): JsonObject {
  return JSON.parse(JSON.stringify(value || {})) as JsonObject;
}

function jsonSignature(value: unknown): string {
  return JSON.stringify(value);
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
      provider_config_format:
        (profile.provider_config_format || "").trim() === "top_level"
          ? "top_level"
          : "profile",
      base_url: profile.base_url.trim(),
      model: profile.model.trim(),
      proxy_url: (profile.proxy_url || "").trim(),
      remote_frontend_mode: normalizeRemoteFrontendMode(profile.remote_frontend_mode),
      remote_web_asset_registry_url:
        normalizeRegistryUrl(profile.remote_web_asset_registry_url || "") ||
        DEFAULT_CODEX_WEB_ASSET_REGISTRY_URL,
      remote_web_asset_version:
        (profile.remote_web_asset_version || "").trim() || DEFAULT_CODEX_WEB_ASSET_VERSION,
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

function profileKey(profile: ProviderProfile) {
  return profile.id.trim() || profile.name;
}

function createDefaultProviderProfileForm(): DefaultProviderProfile {
  return {
    name: "",
    provider_name: "",
    base_url: "",
    api_key: "",
    model: "",
    config_format: "profile",
  };
}

function cloneDefaultProviderProfile(profile: DefaultProviderProfile): DefaultProviderProfile {
  return {
    name: profile.name,
    provider_name: profile.provider_name,
    base_url: profile.base_url,
    api_key: profile.api_key,
    model: profile.model,
    config_format: profile.config_format || "profile",
  };
}

function normalizeDefaultProviderProfileForm(profile: DefaultProviderProfile): DefaultProviderProfile {
  const name = profile.name.trim();
  return {
    name,
    provider_name: profile.provider_name.trim() || name,
    base_url: profile.base_url.trim(),
    api_key: profile.api_key.trim(),
    model: profile.model.trim(),
    config_format: profile.config_format || "profile",
  };
}

function defaultProviderProfileDraftSignature(profile: DefaultProviderProfile): string {
  const normalized = normalizeDefaultProviderProfileForm(profile);
  return jsonSignature({
    name: normalized.name,
    provider_name: normalized.provider_name,
    base_url: normalized.base_url,
    api_key: normalized.api_key,
    model: normalized.model,
    config_format: normalized.config_format || "profile",
  });
}

function profileManagementDefaultProviders(providers: DefaultProviderProfile[]) {
  return providers.filter((profile) => {
    const name = profile.name.trim();
    return name && name !== "Default" && !isNextAiGatewayProvider(profile);
  });
}

function workspaceProfilesUsingDefaultProvider(
  provider: DefaultProviderProfile,
  profiles: ProviderProfile[],
) {
  const codexProfileName = provider.name.trim();
  return profiles.filter((profile) => profile.codex_profile_name.trim() === codexProfileName);
}

function isNextAiGatewayProvider(profile: { provider_name: string }) {
  return profile.provider_name.trim().toLowerCase() === NEXT_AI_GATEWAY_PROVIDER_NAME;
}

function workspaceSelectableDefaultProviders(
  providers: DefaultProviderProfile[],
  gatewayEnabled: boolean,
) {
  if (!gatewayEnabled) {
    return providers;
  }
  return providers.filter((profile) => !isNextAiGatewayProvider(profile));
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

function userFacingErrorMessage(error: unknown, strings: AppStrings) {
  if (isMissingTauriRuntimeError(error)) {
    return strings.desktopRuntimeUnavailableDescription;
  }
  return errorMessage(error).replace(/^Error:\s*/, "");
}

function isMissingTauriRuntimeError(error: unknown) {
  const message = errorMessage(error);
  return (
    message.includes("reading 'invoke'") ||
    message.includes('reading "invoke"') ||
    message.includes("__TAURI_INTERNALS__") ||
    message.includes("window.__TAURI__")
  );
}

function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function gatewayUsageOverviewMetrics(summary: GatewayUsageSummary | null) {
  const days = gatewayUsageDailyRange(summary);
  const lifetimeTokens = gatewayUsageTotalTokens(summary?.totals);
  const peakTokens = Math.max(0, ...days.map((item) => item.totalTokens));
  const longestTaskSeconds = Math.max(
    0,
    ...(summary?.bySession ?? []).map((session) => {
      if (!session.firstReceivedAtUnix || !session.lastReceivedAtUnix) {
        return 0;
      }
      return Math.max(0, session.lastReceivedAtUnix - session.firstReceivedAtUnix);
    }),
  );
  let currentStreakDays = 0;
  for (let index = days.length - 1; index >= 0; index -= 1) {
    if (days[index].totalTokens <= 0) {
      break;
    }
    currentStreakDays += 1;
  }

  let longestStreakDays = 0;
  let activeStreakDays = 0;
  for (const day of days) {
    if (day.totalTokens > 0) {
      activeStreakDays += 1;
      longestStreakDays = Math.max(longestStreakDays, activeStreakDays);
    } else {
      activeStreakDays = 0;
    }
  }

  return {
    lifetimeTokens,
    peakTokens,
    longestTaskSeconds,
    currentStreakDays,
    longestStreakDays,
  };
}

function gatewayUsageDailyRange(summary: GatewayUsageSummary | null) {
  const fallbackEnd = new Date();
  const endDate = parseDateOnly(summary?.endDate) ?? fallbackEnd;
  const parsedStartDate = parseDateOnly(summary?.startDate);
  const startDate =
    parsedStartDate && localDateDayNumber(parsedStartDate) <= localDateDayNumber(endDate)
      ? parsedStartDate
      : addCalendarDays(endDate, -364);
  const totalsByDay = new Map(
    (summary?.daily ?? []).map((item) => [item.day, gatewayUsageTotalTokens(item)]),
  );
  const lifetimeTokens = gatewayUsageTotalTokens(summary?.totals);
  if (lifetimeTokens > 0 && !Array.from(totalsByDay.values()).some((value) => value > 0)) {
    const lastReceivedAtUnix = gatewayUsageLastReceivedAtUnix(summary?.totals);
    if (lastReceivedAtUnix > 0) {
      totalsByDay.set(dateInputValue(new Date(lastReceivedAtUnix * 1000)), lifetimeTokens);
    }
  }
  const days = [];
  const spanDays = Math.max(0, daysBetweenDates(startDate, endDate));
  for (let offset = 0; offset <= spanDays; offset += 1) {
    const date = addCalendarDays(startDate, offset);
    const key = dateInputValue(date);
    days.push({
      date,
      key,
      totalTokens: totalsByDay.get(key) ?? 0,
    });
  }
  return days;
}

function buildGatewayUsageHeatmap(
  summary: GatewayUsageSummary | null,
  mode: GatewayUsageOverviewMode,
  language: Language,
) {
  const rangeDays = gatewayUsageDailyRange(summary);
  const rangeStart = rangeDays[0]?.date ?? addCalendarDays(new Date(), -364);
  const rangeEnd = rangeDays[rangeDays.length - 1]?.date ?? new Date();
  const gridStart = startOfUsageWeek(rangeStart);
  const gridEnd = endOfUsageWeek(rangeEnd);
  const weekCount = Math.max(1, Math.floor(daysBetweenDates(gridStart, gridEnd) / 7) + 1);
  const rangeStartDay = localDateDayNumber(rangeStart);
  const rangeEndDay = localDateDayNumber(rangeEnd);
  const dailyByDate = new Map(rangeDays.map((day) => [day.key, day.totalTokens]));
  const weeklyTotals = new Map<number, number>();
  const weeklyRanges = new Map<number, { startDate: Date; endDate: Date }>();
  const cumulativeByDate = new Map<string, number>();
  let cumulativeTokens = 0;

  for (const day of rangeDays) {
    const weekIndex = Math.floor(daysBetweenDates(gridStart, day.date) / 7);
    weeklyTotals.set(weekIndex, (weeklyTotals.get(weekIndex) ?? 0) + day.totalTokens);
    const range = weeklyRanges.get(weekIndex);
    if (range) {
      range.endDate = day.date;
    } else {
      weeklyRanges.set(weekIndex, { startDate: day.date, endDate: day.date });
    }
    cumulativeTokens += day.totalTokens;
    if (day.totalTokens > 0) {
      cumulativeByDate.set(day.key, cumulativeTokens);
    }
  }

  const cells = [];
  if (mode === "weekly") {
    for (let weekIndex = 0; weekIndex < weekCount; weekIndex += 1) {
      const range = weeklyRanges.get(weekIndex);
      const date = range?.endDate ?? addCalendarDays(gridStart, weekIndex * 7 + 6);
      const key = `week-${weekIndex}`;
      const value = weeklyTotals.get(weekIndex) ?? 0;
      cells.push({
        key,
        date,
        inRange: Boolean(range),
        weekIndex,
        dayIndex: 0,
        value,
        tooltip: gatewayUsageWeeklyHeatmapTooltip(range?.startDate ?? date, date, value, language),
      });
    }

    return {
      cells,
      maxValue: Math.max(0, ...cells.filter((cell) => cell.inRange).map((cell) => cell.value)),
      rowCount: 1,
      weekCount,
      monthLabels: gatewayUsageHeatmapMonthLabels(rangeStart, rangeEnd, gridStart, language),
    };
  }

  const gridSpanDays = Math.max(0, daysBetweenDates(gridStart, gridEnd));
  for (let offset = 0; offset <= gridSpanDays; offset += 1) {
    const date = addCalendarDays(gridStart, offset);
    const key = dateInputValue(date);
    const dateDay = localDateDayNumber(date);
    const inRange = dateDay >= rangeStartDay && dateDay <= rangeEndDay;
    const weekIndex = Math.floor(offset / 7);
    const dayIndex = date.getDay();
    const dailyTokens = inRange ? dailyByDate.get(key) ?? 0 : 0;
    const value =
      mode === "cumulative"
        ? inRange && dailyTokens > 0
          ? cumulativeByDate.get(key) ?? 0
          : 0
        : dailyTokens;
    cells.push({
      key,
      date,
      inRange,
      weekIndex,
      dayIndex,
      value,
      tooltip: gatewayUsageHeatmapTooltip(date, value, language),
    });
  }

  const maxValue = Math.max(0, ...cells.filter((cell) => cell.inRange).map((cell) => cell.value));

  return {
    cells,
    maxValue,
    rowCount: 7,
    weekCount,
    monthLabels: gatewayUsageHeatmapMonthLabels(rangeStart, rangeEnd, gridStart, language),
  };
}

function gatewayUsageHeatmapTooltip(
  date: Date,
  value: number,
  language: Language,
) {
  return `${formatTokenCount(value)} tokens on ${formatUsageHeatmapDate(date, language)}`;
}

function gatewayUsageWeeklyHeatmapTooltip(
  startDate: Date,
  endDate: Date,
  value: number,
  language: Language,
) {
  const startLabel = formatUsageHeatmapDate(startDate, language);
  const endLabel = formatUsageHeatmapDate(endDate, language);
  const dateLabel = startLabel === endLabel ? endLabel : `${startLabel} - ${endLabel}`;
  return `${formatTokenCount(value)} tokens on ${dateLabel}`;
}

function positionGatewayUsageHeatmapTooltip(
  text: string,
  clientX: number,
  clientY: number,
): GatewayUsageHeatmapTooltipState {
  const margin = 8;
  const offset = 12;
  const maxTooltipWidth = 256;
  const estimatedTooltipHeight = 34;
  const viewportWidth = typeof window === "undefined" ? maxTooltipWidth + margin * 2 : window.innerWidth;
  const viewportHeight = typeof window === "undefined" ? 800 : window.innerHeight;
  const tooltipWidth = Math.min(maxTooltipWidth, Math.max(0, viewportWidth - margin * 2));
  const halfTooltipWidth = tooltipWidth / 2;
  const minX = margin + halfTooltipWidth;
  const maxX = Math.max(minX, viewportWidth - margin - halfTooltipWidth);
  const x = Math.min(Math.max(clientX, minX), maxX);
  const hasRoomAbove = clientY - offset - estimatedTooltipHeight >= margin;
  const hasRoomBelow = clientY + offset + estimatedTooltipHeight <= viewportHeight - margin;
  const placement = hasRoomAbove || !hasRoomBelow ? "above" : "below";
  const rawY = placement === "above" ? clientY - offset : clientY + offset;
  const y = placement === "above" ? Math.max(margin, rawY) : Math.min(viewportHeight - margin, rawY);

  return { placement, text, x, y };
}

function gatewayUsageHeatmapMonthLabels(startDate: Date, endDate: Date, gridStart: Date, language: Language) {
  const labels: Array<{ label: string; weekIndex: number }> = [];
  const pushLabel = (date: Date) => {
    const weekIndex = Math.floor(daysBetweenDates(gridStart, date) / 7);
    const label = new Intl.DateTimeFormat(language === "zh" ? "zh-CN" : "en-US", {
      month: "short",
    }).format(date);
    if (labels[labels.length - 1]?.label !== label) {
      labels.push({ label, weekIndex });
    }
  };

  pushLabel(startDate);
  for (
    let cursor = new Date(startDate.getFullYear(), startDate.getMonth() + 1, 1);
    localDateDayNumber(cursor) <= localDateDayNumber(endDate);
    cursor = new Date(cursor.getFullYear(), cursor.getMonth() + 1, 1)
  ) {
    pushLabel(cursor);
  }
  return labels;
}

function gatewayUsageHeatmapColor(value: number, maxValue: number) {
  if (!Number.isFinite(value) || value <= 0 || maxValue <= 0) {
    return "color-mix(in oklch, var(--app-muted) 48%, transparent)";
  }

  const ratio = value / maxValue;
  if (ratio >= 0.76) return "oklch(0.74 0.15 248)";
  if (ratio >= 0.48) return "oklch(0.62 0.12 248)";
  if (ratio >= 0.24) return "oklch(0.5 0.09 248)";
  return "oklch(0.38 0.06 248)";
}

function gatewayUsageTotalTokens(value: unknown) {
  const record = objectValue(value);
  return Math.max(0, numberValue(record.totalTokens ?? record.total_tokens, 0));
}

function gatewayUsageLastReceivedAtUnix(value: unknown) {
  const record = objectValue(value);
  return Math.max(0, numberValue(record.lastReceivedAtUnix ?? record.last_received_at_unix, 0));
}

function parseDateOnly(value: string | null | undefined) {
  if (!value) {
    return null;
  }
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!match) {
    return null;
  }
  const year = Number.parseInt(match[1], 10);
  const month = Number.parseInt(match[2], 10);
  const day = Number.parseInt(match[3], 10);
  const date = new Date(year, month - 1, day);
  return Number.isFinite(date.getTime()) ? date : null;
}

function addCalendarDays(date: Date, days: number) {
  const next = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  next.setDate(next.getDate() + days);
  return next;
}

function startOfUsageWeek(date: Date) {
  return addCalendarDays(date, -date.getDay());
}

function endOfUsageWeek(date: Date) {
  return addCalendarDays(date, 6 - date.getDay());
}

function localDateDayNumber(date: Date) {
  return Math.floor(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000);
}

function daysBetweenDates(start: Date, end: Date) {
  return localDateDayNumber(end) - localDateDayNumber(start);
}

function formatUsageHeatmapDate(date: Date, language: Language) {
  return new Intl.DateTimeFormat(language === "zh" ? "zh-CN" : "en-US", {
    month: "short",
    day: "numeric",
  }).format(date);
}

function formatUsageDays(value: number, strings: AppStrings) {
  return `${formatCompactNumber(Math.max(0, value))} ${strings.gatewayUsageDayUnit}`;
}

function formatDurationCompact(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "-";
  }
  const totalSeconds = Math.max(0, Math.round(seconds));
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  if (days > 0) {
    return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  }
  if (hours > 0) {
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  if (minutes > 0) {
    return `${minutes}m`;
  }
  return `${totalSeconds}s`;
}

function formatCompactNumber(value: number) {
  if (!Number.isFinite(value)) {
    return "0";
  }
  return new Intl.NumberFormat(undefined, {
    notation: Math.abs(value) >= 10000 ? "compact" : "standard",
    maximumFractionDigits: Math.abs(value) >= 10000 ? 1 : 0,
  }).format(value);
}

function formatTokenCount(value: number) {
  return formatCompactNumber(Math.max(0, value || 0));
}

function gatewayUsageSeriesLabel(value: string, strings: AppStrings) {
  if (value === "input") return strings.gatewayUsageInput;
  if (value === "output") return strings.gatewayUsageOutput;
  if (value === "cache") return strings.gatewayUsageCache;
  if (value === "total") return strings.gatewayUsageTotal;
  return value;
}

function gatewayUsageCacheTokens(
  value?: { cacheReadTokens?: number; cacheWriteTokens?: number } | null,
) {
  return (value?.cacheReadTokens ?? 0) + (value?.cacheWriteTokens ?? 0);
}

function gatewayUsageCacheRate(value?: { inputTokens?: number; cacheReadTokens?: number } | null) {
  const base = gatewayUsageCacheRateBase(value);
  return base > 0 ? (value?.cacheReadTokens ?? 0) / base : 0;
}

function gatewayUsageCacheRateBase(value?: { inputTokens?: number; cacheReadTokens?: number } | null) {
  return (value?.inputTokens ?? 0) + (value?.cacheReadTokens ?? 0);
}

function gatewayUsageSessionLabel(value: string | null | undefined, strings: AppStrings) {
  const sessionId = (value || "").trim();
  return sessionId || strings.gatewayUsageUnknownSession;
}

function compactGatewayUsageLabel(value: string) {
  const label = value.trim();
  if (label.length <= 32) {
    return label;
  }
  return `${label.slice(0, 18)}...${label.slice(-10)}`;
}

function gatewayUsageWindowLabel(summary: Pick<GatewayUsageSummary, "windowDays" | "windowHours">) {
  if (summary.windowHours > 0 && (summary.windowHours < 48 || summary.windowHours % 24 !== 0)) {
    return `${summary.windowHours}h`;
  }
  return `${summary.windowDays}d`;
}

function gatewayUsageDateRangeForHours(hours: number): GatewayUsageDateRange {
  const end = new Date();
  const start = new Date(end.getTime() - Math.max(1, hours) * 60 * 60 * 1000);
  return {
    startDate: dateInputValue(start),
    endDate: dateInputValue(end),
    hours,
  };
}

function gatewayUsageDateRangeForDays(days: number): GatewayUsageDateRange {
  const end = new Date();
  const start = new Date(end);
  start.setDate(end.getDate() - Math.max(1, days) + 1);
  return {
    startDate: dateInputValue(start),
    endDate: dateInputValue(end),
  };
}

function usageDateRangeMatchesHours(range: GatewayUsageDateRange, hours: number) {
  return range.hours === hours;
}

function usageDateRangeMatchesDays(range: GatewayUsageDateRange, days: number) {
  if (range.hours) {
    return false;
  }
  const preset = gatewayUsageDateRangeForDays(days);
  return range.startDate === preset.startDate && range.endDate === preset.endDate;
}

function dateInputValue(date: Date) {
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

function formatPercent(value: number) {
  const percent = Number.isFinite(value) ? value : 0;
  return new Intl.NumberFormat(undefined, {
    style: "percent",
    maximumFractionDigits: 1,
  }).format(percent);
}

function formatLatency(value: number) {
  if (!Number.isFinite(value) || value <= 0) {
    return "0 ms";
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(value >= 10000 ? 0 : 1)} s`;
  }
  return `${Math.round(value)} ms`;
}

function formatUnixDateTime(value?: number | null) {
  if (!value || !Number.isFinite(value)) {
    return "-";
  }
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value * 1000));
}

function gatewayUsageStatusClass(status: string) {
  const normalized = status.trim().toLowerCase();
  if (normalized === "success") {
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-300";
  }
  if (normalized === "timeout" || normalized === "rate-limited") {
    return "border-amber-500/30 bg-amber-500/10 text-amber-300";
  }
  return "border-destructive/30 bg-destructive/10 text-red-300";
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

function normalizeTranscribeSettings(settings: {
  transcribeBaseUrl?: string;
  transcribeApiUrl?: string;
  transcribeApiKey?: string;
  transcribeModel?: string;
}) {
  const rawBaseUrl = settings.transcribeBaseUrl ?? settings.transcribeApiUrl ?? "";
  const transcribeBaseUrl = normalizeTranscribeBaseUrl(rawBaseUrl);
  return {
    transcribeBaseUrl,
    transcribeApiUrl: transcribeBaseUrl,
    transcribeApiKey: String(settings.transcribeApiKey || "").trim(),
    transcribeModel: String(settings.transcribeModel || "").trim() || DEFAULT_TRANSCRIBE_MODEL,
  };
}

function normalizeTranscribeBaseUrl(value: unknown) {
  const suffix = "/audio/transcriptions";
  let baseUrl = String(value || "").trim().replace(/\/+$/, "");
  if (baseUrl.toLowerCase().endsWith(suffix)) {
    baseUrl = baseUrl.slice(0, -suffix.length).replace(/\/+$/, "");
  }
  return baseUrl;
}

function isHttpUrl(value: string) {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

function normalizeExtensionSettings(value: Partial<ExtensionSettings> | undefined | null): ExtensionSettings {
  const raw = value as
    | (Partial<ExtensionSettings> & {
        botGatewayEnabled?: boolean;
        nextAiGatewayEnabled?: boolean;
      })
    | undefined
    | null;
  return {
    enabled: Boolean(value?.enabled),
    bot_gateway_enabled: raw?.bot_gateway_enabled ?? raw?.botGatewayEnabled ?? false,
    next_ai_gateway_enabled: raw?.next_ai_gateway_enabled ?? raw?.nextAiGatewayEnabled ?? false,
  };
}

function extensionSettingsEqual(
  left: Partial<ExtensionSettings> | undefined | null,
  right: Partial<ExtensionSettings> | undefined | null,
): boolean {
  const normalizedLeft = normalizeExtensionSettings(left);
  const normalizedRight = normalizeExtensionSettings(right);
  return (
    normalizedLeft.enabled === normalizedRight.enabled &&
    normalizedLeft.bot_gateway_enabled === normalizedRight.bot_gateway_enabled &&
    normalizedLeft.next_ai_gateway_enabled === normalizedRight.next_ai_gateway_enabled
  );
}

function transcribeSettingsEqual(
  left: ReturnType<typeof normalizeTranscribeSettings>,
  right: ReturnType<typeof normalizeTranscribeSettings>,
): boolean {
  return (
    left.transcribeBaseUrl === right.transcribeBaseUrl &&
    left.transcribeApiKey === right.transcribeApiKey &&
    left.transcribeModel === right.transcribeModel
  );
}

function savedBotConfigsEqual(left: SavedBotConfig[], right: SavedBotConfig[]): boolean {
  return jsonSignature(normalizeSavedBotConfigs(left)) === jsonSignature(normalizeSavedBotConfigs(right));
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
