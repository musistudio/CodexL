import { TRANSPORT_OPEN, openRealtimeSession } from "./realtimeTransport.js?v=20260604-pwa-friendly-status-v14";
import { decodeCodexQrFromVideo } from "./qrDecoder.js?v=20260604-pwa-friendly-status-v14";

const MAX_SCREEN_ZOOM = 4;
const MIN_SCREEN_ZOOM = 1;
const POINTER_MOVE_SEND_INTERVAL_MS = 33;
const RECONNECT_MS = 1000;
const SCROLL_SEND_INTERVAL_MS = 45;
const TRANSPORT_CONNECT_TIMEOUT_MS = 2500;
const WEB_BRIDGE_PARENT_RELOAD_MS = 45000;
const WEB_BRIDGE_STATUS_MESSAGE = "codex-web-bridge-status";
const WEB_RECONNECT_MAX_MS = 8000;
const WEB_RECONNECT_MIN_MS = 1000;
const WEB_CACHE_PREPARE_TIMEOUT_MS = 120000;
const REMOTE_INFO_AUTH_ERROR = "remote-info-auth";
const REMOTE_INFO_UNAVAILABLE_ERROR = "remote-info-unavailable";
const DEFAULT_WEB_ASSET_REGISTRY_URL = "https://web.codexl.io";
const VIEWPORT_SEND_DEBOUNCE_MS = 120;
const PAGE_ZOOM_SEND_DEBOUNCE_MS = 120;
const PAGE_ZOOM_STORAGE_KEY = "codex-app-remotely.pageZoomScale";
const INSTANCE_STORAGE_KEY = "codexl-remote.instances";
const CONTROL_CONNECTION_STORAGE_PREFIX = "codexl-remote.control-connection.";
const WEB_ASSET_RUNTIME_CONFIG_STORAGE_PREFIX = "codexl-remote.web-asset-runtime.";
const WEB_ASSET_RUNTIME_CONFIG_LATEST_MAX_AGE_MS = 30 * 60 * 1000;
const WEB_ASSET_RUNTIME_CONFIG_VERSIONED_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const WEB_ASSET_VERSION_REFRESH_DELAY_MS = 1500;
const REMOTE_MODE_SCREENCAST = "screencast";
const REMOTE_MODE_WEB = "web";
const MIN_PAGE_ZOOM_SCALE = 1;
const MAX_PAGE_ZOOM_SCALE = 3;
const SIDEBAR_SWIPE_EDGE_MAX_PX = 80;
const SIDEBAR_SWIPE_EDGE_MIN_PX = 44;
const SIDEBAR_SWIPE_EDGE_RATIO = 0.08;
const SIDEBAR_SWIPE_CLOSE_REGION_MAX_PX = 360;
const SIDEBAR_SWIPE_CLOSE_REGION_MIN_PX = 96;
const SIDEBAR_SWIPE_CLOSE_REGION_RATIO = 0.36;
const SIDEBAR_SWIPE_RIGHT_OPEN_REGION_MAX_PX = 220;
const SIDEBAR_SWIPE_RIGHT_OPEN_REGION_MIN_PX = 76;
const SIDEBAR_SWIPE_RIGHT_OPEN_REGION_RATIO = 0.22;
const SIDEBAR_SWIPE_MIN_DISTANCE_PX = 72;
const SIDEBAR_SWIPE_MAX_VERTICAL_PX = 52;
const SIDEBAR_SWIPE_DIRECTION_RATIO = 1.35;
const PWA_BUILD = "20260604-pwa-friendly-status-v14";
const WEB_BRIDGE_URL_PARAM = "codexBridgeUrl";
const PARENT_BRIDGE_OPEN_MESSAGE = "codex-web-parent-bridge-open";
const PARENT_BRIDGE_OPENED_MESSAGE = "codex-web-parent-bridge-opened";
const PARENT_BRIDGE_SEND_MESSAGE = "codex-web-parent-bridge-send";
const PARENT_BRIDGE_MESSAGE = "codex-web-parent-bridge-message";
const PARENT_BRIDGE_CLOSE_MESSAGE = "codex-web-parent-bridge-close";
const PARENT_BRIDGE_CLOSED_MESSAGE = "codex-web-parent-bridge-closed";
const PARENT_BRIDGE_ERROR_MESSAGE = "codex-web-parent-bridge-error";
const SERVICE_WORKER_URL = `service-worker.js?v=${PWA_BUILD}`;
const E2EE_VERSION = "v1";
const E2EE_SALT_PREFIX = "codexl-remote-e2ee-v1";
const E2EE_STORAGE_PREFIX = "codexl.remote.e2ee.v1.";
const E2EE_AAD = new TextEncoder().encode("codexl-remote-e2ee-v1");
const E2EE_BINARY_MAGIC = new Uint8Array([0x43, 0x58, 0x45, 0x31]);
const E2EE_PBKDF2_ITERATIONS = 150000;
const urlParams = new URLSearchParams(location.search);
const CONTROL_LANGUAGE = detectControlLanguage();
const CONTROL_STRINGS = {
  en: {
    appName: "CodexL",
    backToInstances: "Back to instances",
    connect: "Connect",
    connectionIssueBody: "Check that the desktop workspace is still running remote control, then try again.",
    connectionIssueTitle: "Connection issue",
    keyboardProxy: "Keyboard input proxy",
    latest: "Latest",
    loadingTitle: "Connecting",
    locale: "en",
    pageZoom: "Page zoom",
    passwordDescription: "This workspace requires the password set on the desktop client.",
    passwordPlaceholder: "Password",
    passwordTitle: "Remote password",
    quality: "Quality",
    qualityAuto: "Auto",
    qualityBad: "Saver",
    qualityGood: "Quality",
    qualityMedium: "Smooth",
    remoteMode: "Remote mode",
    retry: "Retry",
    screenLabel: "Codex app screen",
    secureWeb: "Use HTTPS",
    webAssetVersion: "Codex app version",
    webFrameTitle: "Remote workspace view",
    webMode: "Workspace",
    status: {
      cdpConnected: "Desktop screen connected",
      connected: "Connected",
      connecting: "Connecting",
      connectingWebBridge: "Connecting to your workspace",
      controlDisconnectedRetrying: "Connection lost, retrying",
      enterPassword: "Enter the remote password.",
      frameDecodeFailed: "Unable to show the remote screen",
      httpsBundleUnavailable: "Secure workspace view unavailable",
      invalidUrl: "Invalid URL",
      loadingRemoteScreen: "Loading remote screen",
      loadingWebBridge: "Opening your workspace",
      missingToken: "Missing token",
      notConnected: "Not connected",
      offlineWaiting: "Offline, waiting to reconnect",
      openingLanRemote: "Opening local network connection",
      preparingWebCache: "Preparing your workspace",
      remoteConnectionUnavailable: "Remote connection unavailable.",
      remoteMetadataTimedOut: "Remote metadata timed out.",
      remotePasswordRequired: "Remote password required",
      remoteServiceUnavailable: "Remote service is unavailable.",
      remoteTokenExpired: "Connection token expired. Scan the latest remote QR code.",
      restoringConnection: "Restoring connection",
      unableUnlock: "Unable to unlock remote control.",
      unlocking: "Unlocking",
      waitingForCdp: "Waiting for the desktop screen",
      webBridgeDisconnectedRetrying: "Workspace connection lost, retrying",
      webBridgeUnavailable: "Workspace view unavailable",
      webCacheFailed: "Unable to prepare workspace",
      webConnected: "Workspace ready",
    },
    loadingAppBundle: (version) => `Loading workspace (${version})`,
    openingHttpsAppBundle: (version) => `Opening secure workspace (${version})`,
    webCacheUpdated: (count) => `Workspace files updated (${count})`,
  },
  zh: {
    appName: "CodexL",
    backToInstances: "返回实例列表",
    connect: "连接",
    connectionIssueBody: "请确认桌面端工作区仍在运行远程控制，然后重试。",
    connectionIssueTitle: "连接遇到问题",
    keyboardProxy: "键盘输入代理",
    latest: "最新",
    loadingTitle: "正在连接",
    locale: "zh-CN",
    pageZoom: "页面缩放",
    passwordDescription: "该工作区需要输入桌面端设置的远程密码。",
    passwordPlaceholder: "密码",
    passwordTitle: "远程密码",
    quality: "画质",
    qualityAuto: "自动",
    qualityBad: "省流",
    qualityGood: "高画质",
    qualityMedium: "流畅",
    remoteMode: "远程模式",
    retry: "重试",
    screenLabel: "Codex 应用画面",
    secureWeb: "使用 HTTPS",
    webAssetVersion: "Codex App 版本",
    webFrameTitle: "远程工作区视图",
    webMode: "工作区",
    status: {
      cdpConnected: "桌面画面已连接",
      connected: "已连接",
      connecting: "连接中",
      connectingWebBridge: "正在连接远程工作区",
      controlDisconnectedRetrying: "连接已断开，正在重试",
      enterPassword: "请输入远程密码。",
      frameDecodeFailed: "无法显示远程画面",
      httpsBundleUnavailable: "安全工作区视图不可用",
      invalidUrl: "URL 无效",
      loadingRemoteScreen: "正在加载远程画面",
      loadingWebBridge: "正在打开远程工作区",
      missingToken: "连接 token 缺失",
      notConnected: "未连接",
      offlineWaiting: "网络离线，等待重连",
      openingLanRemote: "正在打开局域网连接",
      preparingWebCache: "正在准备远程工作区",
      remoteConnectionUnavailable: "远程连接暂不可用。",
      remoteMetadataTimedOut: "远程信息请求超时。",
      remotePasswordRequired: "需要远程密码",
      remoteServiceUnavailable: "远程服务不可用。",
      remoteTokenExpired: "连接 token 已过期。请扫描最新的远程二维码。",
      restoringConnection: "正在恢复连接",
      unableUnlock: "无法解锁远程控制。",
      unlocking: "正在解锁",
      waitingForCdp: "正在等待桌面画面",
      webBridgeDisconnectedRetrying: "远程工作区连接已断开，正在重试",
      webBridgeUnavailable: "远程工作区视图不可用",
      webCacheFailed: "远程工作区准备失败",
      webConnected: "远程工作区已就绪",
    },
    loadingAppBundle: (version) => `正在加载远程工作区（${version}）`,
    openingHttpsAppBundle: (version) => `正在打开安全工作区（${version}）`,
    webCacheUpdated: (count) => `远程工作区文件已更新（${count}）`,
  },
};
const controlStrings = CONTROL_STRINGS[CONTROL_LANGUAGE];
const initialConnection = connectionFromUrlParams(urlParams);

let token = "";
let remoteAuthMode = "";
let remoteCloudUser = "";
let remoteJwt = "";
let remoteCrypto = null;
let remoteRequiresPassword = false;
let remoteOrigin = location.origin;
let remotePathPrefix = websocketBasePath(location.pathname);
let webAssetBaseUrl = "";
let webAssetVersion = "latest";
let webAssetRuntimeConfig = null;
let webAssetRuntimeConfigKey = "";
let webAssetRuntimeConfigInflight = null;
let webAssetRuntimeConfigInflightKey = "";
let transportPreference = (urlParams.get("transport") || "auto").toLowerCase();
let webTransportFallbackLogged = false;
let pointerMoveTimer = null;
let renderLoopStarted = false;
let scanStream = null;
let scanTimer = null;
let activeInstanceId = "";
let activeControlConnection = null;
let addDialogLocked = false;
let editingInstanceId = "";
let pendingDeleteInstanceId = "";
let pendingPasswordConnection = null;
let webAssetVersionRefreshTimer = null;
let instances = readStoredInstances();
let parentBridgeClients = new Map();
let preconnectedWebAssetOrigins = new Set();

const state = {
  connected: false,
  controlSocket: null,
  composing: false,
  ignoreNextClick: false,
  ignoreNextInput: false,
  lastCompositionText: "",
  lastCompositionTs: 0,
  lastSentText: "",
  lastSentTextAt: 0,
  provisionalText: "",
  provisionalTextAt: 0,
  reconnectTimer: null,
  editableRects: [],
  frameConnected: false,
  frameFormat: "jpeg",
  frameProfile: null,
  lastTouch: null,
  lastSentViewportKey: "",
  latestFrame: null,
  latestFrameMeta: null,
  latestPointer: null,
  pendingScroll: null,
  hasStoredPageZoom: false,
  pageZoomMenuOpen: false,
  pageZoomScale: MIN_PAGE_ZOOM_SCALE,
  pageZoomTimer: null,
  pinch: null,
  scrollTimer: null,
  frameSocket: null,
  lastSentPageZoomKey: "",
  screenBaseSize: null,
  screenPanX: 0,
  screenPanY: 0,
  screenSize: null,
  sidebarSwipe: null,
  statusText: "",
  touchMoved: false,
  transportConnectAttempt: 0,
  transportSession: null,
  viewportTimer: null,
  webBridgeLastConnectedAt: 0,
  webBridgeStaleTimer: null,
  webConnectAttempt: 0,
  remoteMode: REMOTE_MODE_WEB,
  webReconnectDelayMs: WEB_RECONNECT_MIN_MS,
  webFrameLoaded: false,
  zoom: 1,
};

const connectView = document.querySelector("#connectView");
const controlView = document.querySelector("#controlView");
const addDialogBackdrop = document.querySelector("#addDialogBackdrop");
const addInstanceButton = document.querySelector("#addInstanceButton");
const backButton = document.querySelector("#backButton");
const closeAddDialogButton = document.querySelector("#closeAddDialogButton");
const closeEditDialogButton = document.querySelector("#closeEditDialogButton");
const cancelDeleteButton = document.querySelector("#cancelDeleteButton");
const screenWrap = document.querySelector("#screenWrap");
const screen = document.querySelector("#screen");
const screenContext = screen?.getContext("2d", { alpha: false }) || null;
const confirmDeleteButton = document.querySelector("#confirmDeleteButton");
const deleteConfirmBackdrop = document.querySelector("#deleteConfirmBackdrop");
const deleteConfirmMessage = document.querySelector("#deleteConfirmMessage");
const deleteInstanceButton = document.querySelector("#deleteInstanceButton");
const editConnectionInput = document.querySelector("#editConnectionInput");
const editDialogBackdrop = document.querySelector("#editDialogBackdrop");
const editNameInput = document.querySelector("#editNameInput");
const editStatus = document.querySelector("#editStatus");
const emptyState = document.querySelector("#emptyState");
const emptyStateActions = document.querySelector("#emptyStateActions");
const emptyStateBackButton = document.querySelector("#emptyBackButton");
const emptyStateMessage = document.querySelector("#emptyStateMessage");
const emptyStateRetryButton = document.querySelector("#emptyRetryButton");
const emptyStateTitle = document.querySelector("#emptyStateTitle");
const instanceHeader = document.querySelector(".instance-header");
const instanceList = document.querySelector("#instanceList");
const instanceNameInput = document.querySelector("#instanceNameInput");
const instanceSearchInput = document.querySelector("#instanceSearchInput");
const keyboardProxy = document.querySelector("#keyboardProxy");
const pageZoomButton = document.querySelector("#pageZoomButton");
const pageZoomControl = document.querySelector("#pageZoomControl");
const pageZoomLabel = document.querySelector("#pageZoomLabel");
const pageZoomMenu = document.querySelector("#pageZoomMenu");
const pageZoomSlider = document.querySelector("#pageZoomSlider");
const passwordForm = document.querySelector("#passwordForm");
const passwordGate = document.querySelector("#passwordGate");
const passwordInput = document.querySelector("#passwordInput");
const passwordStatus = document.querySelector("#passwordStatus");
const qualitySelect = document.querySelector("#qualitySelect");
const remoteModeSelect = document.querySelector("#remoteModeSelect");
const secureWebButton = document.querySelector("#secureWebButton");
const scanButton = document.querySelector("#scanButton");
const scanStatus = document.querySelector("#scanStatus");
const scanVideo = document.querySelector("#scanVideo");
const stopScanButton = document.querySelector("#stopScanButton");
const toggleSearchButton = document.querySelector("#toggleSearchButton");
const toggleSearchButtonLabel = toggleSearchButton?.querySelector(".button-label") || null;
const connectionInput = document.querySelector("#connectionInput");
const connectButton = document.querySelector("#connectButton");
const saveEditButton = document.querySelector("#saveEditButton");
const subtitle = document.querySelector("#subtitle");
const webFrame = document.querySelector("#webFrame");
const webAssetVersionSelect = document.querySelector("#webAssetVersionSelect");
const isListPage = Boolean(connectView);
const isControlPage = Boolean(controlView);

setupViewportZoomGuards();
registerServiceWorker();
if (isListPage) {
  initListPage();
}
if (isControlPage) {
  initControlPage();
}

function initListPage() {
  setupListEventHandlers();
  resetTransientInstanceStatuses();
  renderInstances();
  if (initialConnection) {
    const savedInitialInstance = upsertInstanceFromConnection(initialConnection, { status: controlStrings.status.notConnected });
    if (savedInitialInstance) {
      navigateToControl(savedInitialInstance);
      return;
    }
  }
  showConnectView();
}

function initControlPage() {
  applyControlTranslations();
  setupControlEventHandlers();
  initializePageZoomControl();
  const connection = connectionForControlPage();
  if (!connection) {
    void restoreControlConnectionFromCookie();
    return;
  }

  void startConnection(connection, { instanceId: connection.id || urlParams.get("id") || "" });
}

async function restoreControlConnectionFromCookie() {
  const instanceId = urlParams.get("id") || "";
  setStatus(controlStrings.status.restoringConnection);
  setEmptyStateText(controlStrings.status.restoringConnection);
  const connection = await connectionFromRemoteInfoCookie();
  if (connection) {
    void startConnection(connection, { instanceId });
    return;
  }

  navigateToList();
}

function applyControlTranslations() {
  document.documentElement.lang = controlStrings.locale;
  document.title = controlStrings.appName;

  setEmptyStateLoading(controlStrings.status.loadingWebBridge);
  setControlLabel(backButton, controlStrings.backToInstances);
  setControlLabel(remoteModeSelect, controlStrings.remoteMode);
  setControlLabel(secureWebButton, controlStrings.secureWeb);
  setControlLabel(webAssetVersionSelect, controlStrings.webAssetVersion);
  setControlLabel(pageZoomButton, controlStrings.pageZoom);
  setControlLabel(pageZoomSlider, controlStrings.pageZoom);
  setControlLabel(qualitySelect, controlStrings.quality);
  setControlLabel(screen, controlStrings.screenLabel);
  setControlLabel(keyboardProxy, controlStrings.keyboardProxy);

  if (secureWebButton) {
    secureWebButton.textContent = controlStrings.secureWeb;
  }
  if (webFrame) {
    webFrame.title = controlStrings.webFrameTitle;
  }
  if (remoteModeSelect?.options?.length) {
    for (const option of remoteModeSelect.options) {
      if (option.value === REMOTE_MODE_WEB) {
        option.textContent = controlStrings.webMode;
      }
    }
  }
  if (qualitySelect?.options?.length) {
    const labels = {
      auto: controlStrings.qualityAuto,
      good: controlStrings.qualityGood,
      medium: controlStrings.qualityMedium,
      bad: controlStrings.qualityBad,
    };
    for (const option of qualitySelect.options) {
      option.textContent = labels[option.value] || option.textContent;
    }
  }
  if (webAssetVersionSelect?.options?.length) {
    for (const option of webAssetVersionSelect.options) {
      if (option.value === "latest") {
        option.textContent = controlStrings.latest;
      }
    }
  }
  if (passwordForm) {
    setElementText(passwordForm.querySelector("h1"), controlStrings.passwordTitle);
    setElementText(passwordForm.querySelector("p"), controlStrings.passwordDescription);
    setElementText(passwordForm.querySelector("button[type='submit']"), controlStrings.connect);
  }
  if (passwordInput) {
    passwordInput.placeholder = controlStrings.passwordPlaceholder;
  }
  if (emptyStateRetryButton) {
    emptyStateRetryButton.textContent = controlStrings.retry;
  }
  if (emptyStateBackButton) {
    emptyStateBackButton.textContent = controlStrings.backToInstances;
  }
}

function setControlLabel(element, label) {
  if (!element || !label) {
    return;
  }
  element.setAttribute("aria-label", label);
  if ("title" in element) {
    element.title = label;
  }
}

function setElementText(element, text) {
  if (element) {
    element.textContent = text;
  }
}

function setEmptyStateText(text) {
  setEmptyStateLoading(text);
}

function setEmptyStateLoading(message) {
  setEmptyStateContent({
    actions: false,
    message,
    title: controlStrings.loadingTitle,
  });
}

function setEmptyStateIssue(message) {
  setEmptyStateContent({
    actions: true,
    message: message || controlStrings.connectionIssueBody,
    title: controlStrings.connectionIssueTitle,
  });
}

function setEmptyStateContent({ actions = false, message = "", title = "" } = {}) {
  if (!emptyState) {
    return;
  }

  emptyState.classList.toggle("has-actions", actions);
  if (emptyStateTitle || emptyStateMessage) {
    setElementText(emptyStateTitle, title || message);
    setElementText(emptyStateMessage, message && title !== message ? message : "");
  } else {
    emptyState.textContent = [title, message].filter(Boolean).join(" ");
  }
  if (emptyStateActions) {
    emptyStateActions.hidden = !actions;
  }
}

function detectControlLanguage() {
  const requestedLanguage = new URLSearchParams(location.search).get("lang") ||
    new URLSearchParams(location.search).get("language");
  const normalized = normalizeControlLanguage(requestedLanguage);
  if (normalized) {
    return normalized;
  }

  const candidates = [navigator.language, ...(navigator.languages || [])]
    .filter(Boolean)
    .map((language) => language.toLowerCase());
  return candidates.some((language) => language.startsWith("zh")) ? "zh" : "en";
}

function normalizeControlLanguage(value) {
  const language = String(value || "").trim().toLowerCase();
  if (!language) {
    return null;
  }
  if (language.startsWith("zh")) {
    return "zh";
  }
  if (language.startsWith("en")) {
    return "en";
  }
  return null;
}

async function connectionFromRemoteInfoCookie() {
  try {
    const infoUrl = new URL(`${websocketBasePath(location.pathname)}/api/remote-info`, location.origin);
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 1800);
    const response = await fetch(infoUrl, {
      cache: "no-store",
      credentials: "same-origin",
      signal: controller.signal,
    }).finally(() => {
      window.clearTimeout(timeout);
    });
    if (!response.ok) {
      return null;
    }
    const info = await response.json();
    if (!info || typeof info !== "object") {
      return null;
    }
    const recovered = {
      deviceName: remoteInfoField(info, "deviceName", "device_name") || "",
      name: remoteInfoField(info, "name", "itemName", "item_name") || "",
      remoteMode: remoteInfoField(info, "remoteMode", "remote_mode", "mode") || "",
      token: remoteInfoField(info, "token") || "",
      url: remoteInfoField(info, "url", "lanUrl", "lan_url") || location.href,
      webAssetBaseUrl: remoteInfoField(info, "webAssetBaseUrl", "web_asset_base_url") || "",
      webAssetVersion: remoteInfoField(info, "webAssetVersion", "web_asset_version") || "",
      workspaceName: remoteInfoField(info, "workspaceName", "workspace_name") || "",
    };
    return buildInstanceFromConnection(recovered, {
      existing: { id: urlParams.get("id") || createInstanceId(), status: controlStrings.status.connecting },
      status: controlStrings.status.connecting,
    });
  } catch {
    return null;
  }
}

function setupViewportZoomGuards() {
  const preventZoom = (event) => {
    event.preventDefault();
  };

  document.addEventListener("gesturestart", preventZoom, { passive: false });
  document.addEventListener("gesturechange", preventZoom, { passive: false });
  document.addEventListener("gestureend", preventZoom, { passive: false });
  document.addEventListener(
    "wheel",
    (event) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
      }
    },
    { passive: false },
  );
  document.addEventListener(
    "touchmove",
    (event) => {
      const target = event.target;
      if (event.touches.length < 2 || (screenWrap && target instanceof Node && screenWrap.contains(target))) {
        return;
      }
      event.preventDefault();
    },
    { passive: false },
  );
}

function setupListEventHandlers() {
  addInstanceButton?.addEventListener("click", () => {
    openAddDialog();
  });

  instanceSearchInput?.addEventListener("input", () => {
    renderInstances();
  });

  toggleSearchButton?.addEventListener("click", () => {
    toggleMobileSearch();
  });

  closeAddDialogButton?.addEventListener("click", () => {
    closeAddDialog();
  });

  addDialogBackdrop?.addEventListener("click", (event) => {
    if (event.target === addDialogBackdrop) {
      closeAddDialog();
    }
  });

  closeEditDialogButton?.addEventListener("click", () => {
    closeEditDialog();
  });

  editDialogBackdrop?.addEventListener("click", (event) => {
    if (event.target === editDialogBackdrop) {
      closeEditDialog();
    }
  });

  instanceList?.addEventListener("click", (event) => {
    if (!(event.target instanceof Element)) {
      return;
    }

    const button = event.target.closest("[data-instance-action]");
    if (!button) {
      return;
    }

    const instance = findInstance(button.dataset.instanceId);
    if (!instance) {
      return;
    }

    if (button.dataset.instanceAction === "edit") {
      openEditDialog(instance);
      return;
    }

    if (button.dataset.instanceAction === "delete") {
      openDeleteConfirmDialog(instance);
      return;
    }

    if (button.dataset.instanceAction === "connect") {
      navigateToControl(instance);
    }
  });

  saveEditButton?.addEventListener("click", () => {
    saveEditedInstance();
  });

  deleteInstanceButton?.addEventListener("click", () => {
    requestDeleteEditedInstance();
  });

  cancelDeleteButton?.addEventListener("click", () => {
    closeDeleteConfirmDialog();
  });

  confirmDeleteButton?.addEventListener("click", () => {
    confirmPendingDelete();
  });

  deleteConfirmBackdrop?.addEventListener("click", (event) => {
    if (event.target === deleteConfirmBackdrop) {
      closeDeleteConfirmDialog();
    }
  });

  scanButton?.addEventListener("click", () => {
    startQrScan().catch((error) => {
      setScanStatus(error?.message || "Unable to start the camera scanner.");
      stopQrScan();
    });
  });

  stopScanButton?.addEventListener("click", () => {
    stopQrScan();
    setScanStatus("Scanner stopped.");
  });

  connectButton?.addEventListener("click", () => {
    const connection = parseConnection(connectionInput?.value);
    if (!connection) {
      setScanStatus("Paste a valid connection URL or QR payload.");
      return;
    }
    addInstanceFromConnection(connection, { name: instanceNameInput?.value });
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") {
      return;
    }
    if (deleteConfirmBackdrop && !deleteConfirmBackdrop.hidden) {
      closeDeleteConfirmDialog();
      return;
    }
    if (editDialogBackdrop && !editDialogBackdrop.hidden) {
      closeEditDialog();
      return;
    }
    if (addDialogBackdrop && !addDialogBackdrop.hidden) {
      closeAddDialog();
      return;
    }
    if (instanceHeader?.classList.contains("search-open")) {
      setMobileSearchOpen(false, { clear: true });
    }
  });
}

function toggleMobileSearch() {
  const nextOpen = !instanceHeader?.classList.contains("search-open");
  setMobileSearchOpen(nextOpen, { clear: !nextOpen, focus: nextOpen });
}

function setMobileSearchOpen(open, { clear = false, focus = false } = {}) {
  if (!instanceHeader) {
    return;
  }

  instanceHeader.classList.toggle("search-open", open);
  if (toggleSearchButton) {
    toggleSearchButton.setAttribute("aria-expanded", open ? "true" : "false");
    toggleSearchButton.setAttribute("aria-label", open ? "Close search" : "Search instances");
    toggleSearchButton.title = open ? "Close search" : "Search instances";
  }
  if (toggleSearchButtonLabel) {
    toggleSearchButtonLabel.textContent = open ? "Close" : "Search";
  }
  if (clear && instanceSearchInput?.value) {
    instanceSearchInput.value = "";
    renderInstances();
  }
  if (focus && instanceSearchInput) {
    requestAnimationFrame(() => {
      if (instanceHeader.classList.contains("search-open")) {
        instanceSearchInput.focus();
      }
    });
  }
}

function setupControlEventHandlers() {
  backButton?.addEventListener("click", () => {
    navigateToList();
  });
  emptyStateBackButton?.addEventListener("click", () => {
    navigateToList();
  });
  emptyStateRetryButton?.addEventListener("click", () => {
    retryActiveControlConnection();
  });

  passwordForm?.addEventListener("submit", (event) => {
    event.preventDefault();
    unlockPendingPasswordConnection();
  });

  qualitySelect?.addEventListener("change", () => {
    send({ type: "profileMode", mode: qualitySelect.value });
  });

  remoteModeSelect?.addEventListener("change", () => {
    switchRemoteMode(remoteModeSelect.value, { remember: true });
  });

  secureWebButton?.addEventListener("click", () => {
    openSecureRegistryWeb();
  });

  webAssetVersionSelect?.addEventListener("change", () => {
    const nextVersion = normalizeWebAssetVersion(webAssetVersionSelect.value);
    if (!webAssetRegistryEnabled() || nextVersion === webAssetVersion) {
      return;
    }
    webAssetVersion = nextVersion;
    webAssetRuntimeConfig = null;
    webAssetRuntimeConfigKey = "";
    webAssetRuntimeConfigInflight = null;
    webAssetRuntimeConfigInflightKey = "";
    primeWebAssetRuntimeConfig();
    persistActiveWebAssetSelection();
    updateSecureWebButton();
    void connectWebBridgeMode();
  });

  webFrame?.addEventListener("load", () => {
    if (state.remoteMode !== REMOTE_MODE_WEB) {
      return;
    }
    state.webFrameLoaded = true;
    applyRemoteModeLayout();
    const nextStatus = state.connected ? controlStrings.status.webConnected : controlStrings.status.connectingWebBridge;
    setStatus(nextStatus);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, nextStatus, state.connected ? { lastConnectedAt: Date.now() } : {});
    }
    if (!state.connected) {
      scheduleWebBridgeStaleReload(controlStrings.status.webBridgeDisconnectedRetrying);
    }
  });
  webFrame?.addEventListener("error", () => {
    handleWebBridgeDisconnect(controlStrings.status.webBridgeDisconnectedRetrying);
  });

  window.addEventListener("message", handleWebBridgeStatusMessage);
  window.addEventListener("online", () => {
    if (state.remoteMode === REMOTE_MODE_WEB && !state.webFrameLoaded) {
      scheduleWebReconnect(controlStrings.status.webBridgeDisconnectedRetrying, { immediate: true });
    } else if (state.remoteMode === REMOTE_MODE_SCREENCAST && !state.connected) {
      scheduleRealtimeReconnect({ immediate: true });
    }
  });
  window.addEventListener("offline", () => {
    if (state.remoteMode !== REMOTE_MODE_WEB && state.remoteMode !== REMOTE_MODE_SCREENCAST) {
      return;
    }
    setStatus(controlStrings.status.offlineWaiting);
    if (emptyState && (state.remoteMode === REMOTE_MODE_WEB ? !state.webFrameLoaded : !state.frameConnected)) {
      emptyState.hidden = false;
      setEmptyStateIssue(controlStrings.status.offlineWaiting);
    }
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, controlStrings.status.offlineWaiting);
    }
  });

  pageZoomButton?.addEventListener("click", () => {
    togglePageZoomMenu();
  });

  pageZoomSlider?.addEventListener("input", () => {
    setPageZoomScale(pageZoomSlider.value, { remember: true });
    queuePageZoomUpdate();
  });

  pageZoomSlider?.addEventListener("change", () => {
    setPageZoomScale(pageZoomSlider.value, { remember: true });
    sendPageZoom({ force: true });
  });

  document.addEventListener("pointerdown", (event) => {
    if (state.pageZoomMenuOpen && pageZoomControl && !pageZoomControl.contains(event.target)) {
      closePageZoomMenu();
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && state.pageZoomMenuOpen) {
      closePageZoomMenu();
    }
  });

  if (window.ResizeObserver && screenWrap) {
    const screenResizeObserver = new ResizeObserver(() => {
      fitScreenToWrap();
      queueViewportUpdate();
    });
    screenResizeObserver.observe(screenWrap);
  }

  window.addEventListener("resize", () => {
    fitScreenToWrap();
    queueViewportUpdate();
  });
  window.visualViewport?.addEventListener("resize", () => {
    fitScreenToWrap();
    queueViewportUpdate();
  });
  window.addEventListener("orientationchange", () => {
    fitScreenToWrap();
    queueViewportUpdate();
  });

  screenWrap?.addEventListener("click", (event) => {
    if (state.ignoreNextClick) {
      state.ignoreNextClick = false;
      return;
    }

    const point = normalizedPoint(event);
    if (!point) {
      return;
    }

    if (isEditablePoint(point)) {
      focusKeyboardProxy();
    } else {
      blurKeyboardProxy();
    }
    send({ type: "click", ...point });
  });

  screenWrap?.addEventListener(
    "mousemove",
    (event) => {
      const point = normalizedPoint(event);
      if (point) {
        state.latestPointer = point;
      }
    },
    { passive: true },
  );

  keyboardProxy?.addEventListener("keydown", (event) => {
    if (state.composing) {
      return;
    }

    if (["Backspace", "Delete", "Enter", "Escape", "Tab"].includes(event.key)) {
      event.preventDefault();
      send({ type: "key", key: event.key });
      updateProvisionalForKey(event.key);
      ignoreNextInputBriefly();
      resetKeyboardProxy();
    }
  });

  keyboardProxy?.addEventListener("beforeinput", (event) => {
    if (state.composing || event.isComposing || event.inputType === "insertCompositionText") {
      return;
    }

    const committedText = committedTextFromBeforeInput(event);
    if (committedText) {
      event.preventDefault();
      sendCommittedText(committedText);
      ignoreNextInputBriefly();
      resetKeyboardProxy();
      return;
    }

    const text = textFromBeforeInput(event);
    if (text) {
      event.preventDefault();
      if (!isRecentCompositionText(text)) {
        if (event.inputType === "insertText" && text.length > 1 && currentProvisionalText()) {
          sendCommittedText(text);
        } else {
          sendProvisionalText(text);
        }
      }
      ignoreNextInputBriefly();
      resetKeyboardProxy();
      return;
    }

    const key = keyFromInputType(event.inputType);
    if (key) {
      event.preventDefault();
      send({ type: "key", key });
      updateProvisionalForKey(key);
      ignoreNextInputBriefly();
      resetKeyboardProxy();
    }
  });

  keyboardProxy?.addEventListener("input", () => {
    if (state.ignoreNextInput) {
      state.ignoreNextInput = false;
      resetKeyboardProxy();
      return;
    }

    if (state.composing) {
      return;
    }

    flushKeyboardProxyValue();
  });

  keyboardProxy?.addEventListener("compositionstart", () => {
    state.composing = true;
  });

  keyboardProxy?.addEventListener("compositionend", (event) => {
    state.composing = false;
    const text = event.data || keyboardProxy.value;
    if (text) {
      state.lastCompositionText = text;
      state.lastCompositionTs = Date.now();
      sendCommittedText(text);
    }
    ignoreNextInputBriefly();
    resetKeyboardProxy();
  });

  screenWrap?.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const point = normalizedPoint(event) || { x: 0.5, y: 0.5 };
      queueScroll(point, event.deltaX, event.deltaY);
    },
    { passive: false },
  );

  screenWrap?.addEventListener(
    "touchstart",
    (event) => {
      if (event.touches.length >= 2) {
        event.preventDefault();
        startPinch(event);
        return;
      }

      const touch = event.touches[0];
      state.lastTouch = touch ? { x: touch.clientX, y: touch.clientY, ts: Date.now() } : null;
      beginSidebarSwipe(touch);
      state.touchMoved = false;
    },
    { passive: false },
  );

  screenWrap?.addEventListener(
    "touchmove",
    (event) => {
      if (event.touches.length >= 2) {
        event.preventDefault();
        updatePinch(event);
        return;
      }

      if (state.pinch) {
        event.preventDefault();
        return;
      }

      const touch = event.touches[0];
      if (!touch || !state.lastTouch) {
        return;
      }

      const sidebarSwipeState = updateSidebarSwipe(touch);
      if (sidebarSwipeState === "handled" || sidebarSwipeState === "tracking") {
        event.preventDefault();
        return;
      }

      const dy = state.lastTouch.y - touch.clientY;
      const dx = state.lastTouch.x - touch.clientX;
      if (Math.abs(dx) < 4 && Math.abs(dy) < 4) {
        return;
      }

      event.preventDefault();
      state.touchMoved = true;
      const point = normalizedPointFromClient(touch.clientX, touch.clientY) || { x: 0.5, y: 0.5 };
      queueScroll(point, dx * 2, dy * 2);
      state.lastTouch = { x: touch.clientX, y: touch.clientY, ts: Date.now() };
    },
    { passive: false },
  );

  screenWrap?.addEventListener(
    "touchend",
    (event) => {
      if (state.pinch) {
        if (event.touches.length < 2) {
          endPinch();
        }
        event.preventDefault();
        return;
      }

      if (state.touchMoved) {
        state.ignoreNextClick = true;
        state.lastTouch = null;
        state.sidebarSwipe = null;
        event.preventDefault();
        return;
      }

      state.lastTouch = null;
      state.sidebarSwipe = null;
    },
    { passive: false },
  );

  screenWrap?.addEventListener(
    "touchcancel",
    () => {
      if (state.pinch) {
        endPinch();
      }
      state.lastTouch = null;
      state.sidebarSwipe = null;
    },
    { passive: true },
  );
}

function connectionForControlPage() {
  const instanceId = urlParams.get("id") || "";
  if (instanceId) {
    const stored = findInstance(instanceId);
    if (initialConnection) {
      return refreshInstanceFromControlUrl(instanceId, stored, initialConnection) || stored;
    }
    const sessionConnection = readControlConnection(instanceId);
    if (sessionConnection) {
      return refreshInstanceFromControlUrl(instanceId, stored, sessionConnection) || stored || sessionConnection;
    }
    return stored;
  }

  if (initialConnection) {
    return upsertInstanceFromConnection(initialConnection, { status: controlStrings.status.connecting }) || initialConnection;
  }

  return null;
}

function navigateToControl(instance) {
  const url = new URL("control.html", location.href);
  const connectionUrl = connectionUrlForNavigation(instance);
  url.searchParams.set("id", instance.id);
  url.searchParams.set("url", connectionUrl);
  applyControlConnectionParams(url.searchParams, instance, connectionUrl);
  rememberControlConnection(instance);
  location.href = url.toString();
}

function connectionUrlForNavigation(instance) {
  try {
    const url = normalizeConnectionUrl(instance.url);
    if (instance.token) {
      url.searchParams.set("token", instance.token);
    }
    if (!connectionUsesCloudRemote(instance, url)) {
      url.searchParams.delete("requirePassword");
      url.searchParams.delete("e2ee");
    }
    return url.toString();
  } catch {
    return instance.url || "";
  }
}

function applyControlConnectionParams(params, instance, connectionUrl) {
  if (instance.token) {
    params.set("token", instance.token);
  }
  if (instance.remoteMode) {
    params.set("remoteMode", instance.remoteMode);
  }
  if (instance.webAssetBaseUrl) {
    params.set("webAssetBaseUrl", instance.webAssetBaseUrl);
  }
  if (instance.webAssetVersion) {
    params.set("webAssetVersion", instance.webAssetVersion);
  }
  try {
    const url = normalizeConnectionUrl(connectionUrl);
    for (const key of ["cloudUser", "jwt", "transport"]) {
      const value = url.searchParams.get(key);
      if (value) {
        params.set(key, value);
      }
    }
    if (connectionUsesCloudRemote(instance, url)) {
      for (const key of ["requirePassword", "e2ee"]) {
        const value = url.searchParams.get(key);
        if (value) {
          params.set(key, value);
        }
      }
    }
  } catch {
    // The nested connection URL remains the primary source when it is parseable.
  }
}

function rememberControlConnection(instance) {
  try {
    sessionStorage.setItem(controlConnectionStorageKey(instance.id), JSON.stringify(instance));
  } catch {
    // URL parameters still carry the connection for reloads when session storage is unavailable.
  }
}

function readControlConnection(instanceId) {
  try {
    const raw = sessionStorage.getItem(controlConnectionStorageKey(instanceId));
    return raw ? normalizeStoredInstance(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

function controlConnectionStorageKey(instanceId) {
  return `${CONTROL_CONNECTION_STORAGE_PREFIX}${instanceId}`;
}

function navigateToList() {
  closeRealtimeSession();
  resetWebFrame();

  const url = new URL("index.html", location.href);
  location.href = url.toString();
}

function retryActiveControlConnection() {
  const remembered = pendingPasswordConnection || activeControlConnection;
  const connection = remembered?.connection || connectionForControlPage();
  const instanceId = remembered?.instanceId || activeInstanceId || urlParams.get("id") || connection?.id || "";
  if (!connection) {
    navigateToList();
    return;
  }

  hidePasswordGate();
  setStatus(controlStrings.status.connecting);
  setEmptyStateLoading(controlStrings.status.connecting);
  if (emptyState) {
    emptyState.hidden = false;
  }
  void startConnection(connection, { instanceId });
}

function showControlConnectionIssue(message, { instanceId = activeInstanceId } = {}) {
  const nextMessage = message || controlStrings.connectionIssueBody;
  setScanStatus(nextMessage);
  setStatus(nextMessage);
  if (emptyState) {
    emptyState.hidden = false;
    setEmptyStateIssue(nextMessage);
  }
  if (instanceId) {
    updateInstanceStatus(instanceId, nextMessage);
  }
}

function showConnectView() {
  if (controlView) {
    controlView.hidden = true;
  }
  if (connectView) {
    connectView.hidden = false;
  }
  stopQrScan();
  renderInstances();
  if (instances.length === 0) {
    openAddDialog({ locked: true });
  } else {
    closeAddDialog();
  }
}

async function startConnection(connection, { instanceId = connection.id || "" } = {}) {
  let connectionUrl;
  try {
    connectionUrl = normalizeConnectionUrl(connection.url);
  } catch {
    showControlConnectionIssue(controlStrings.status.invalidUrl, { instanceId });
    return;
  }

  const nextToken = connection.token || connectionUrl.searchParams.get("token") || "";
  if (!nextToken) {
    showControlConnectionIssue(controlStrings.status.missingToken, { instanceId });
    return;
  }

  activeControlConnection = { connection, instanceId };
  token = nextToken;
  remoteAuthMode = connectionUrl.searchParams.get("auth") || "";
  remoteCloudUser = connection.cloudUser || connectionUrl.searchParams.get("cloudUser") || "";
  remoteJwt = connection.jwt || connectionUrl.searchParams.get("jwt") || "";
  remoteOrigin = connectionUrl.origin;
  remotePathPrefix = websocketBasePath(connectionUrl.pathname);

  let remoteInfo = null;
  try {
    remoteInfo = await fetchRemoteInfo();
  } catch (error) {
    const message = error?.message || controlStrings.status.remoteConnectionUnavailable;
    showControlConnectionIssue(message, { instanceId });
    return;
  }
  const requiresPassword = connectionRequiresPassword(connection, connectionUrl, remoteInfo);
  if (requiresPassword && (!remoteCrypto || remoteCrypto.token !== nextToken)) {
    remoteCrypto = null;
    pendingPasswordConnection = { connection, instanceId };
    showPasswordGate();
    return;
  }

  hidePasswordGate();
  stopQrScan();
  if (isListPage) {
    closeAddDialog({ force: true });
    closeEditDialog();
  }
  activeInstanceId = instanceId;
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.connecting);
  }
  remoteRequiresPassword = requiresPassword;
  webAssetBaseUrl = connectionWebAssetBaseUrl(connection, connectionUrl, remoteInfo);
  webAssetVersion = connectionWebAssetVersion(connection, connectionUrl, remoteInfo);
  webAssetRuntimeConfig = null;
  webAssetRuntimeConfigKey = "";
  webAssetRuntimeConfigInflight = null;
  webAssetRuntimeConfigInflightKey = "";
  configureWebAssetVersionSelect();
  updateSecureWebButton();
  if (webAssetRegistryEnabled()) {
    primeWebAssetRuntimeConfig();
    queueWebAssetVersionsRefresh();
  }
  transportPreference = (connectionUrl.searchParams.get("transport") || urlParams.get("transport") || "auto").toLowerCase();
  webTransportFallbackLogged = false;
  state.remoteMode = normalizeRemoteMode(
    connection.remoteMode ||
      connection.mode ||
      connectionUrl.searchParams.get("remoteMode") ||
      connectionUrl.searchParams.get("mode") ||
      remoteInfoField(remoteInfo, "remoteMode", "remote_mode", "mode"),
  );
  if (remoteModeSelect) {
    remoteModeSelect.value = state.remoteMode;
  }
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.connecting, {
      remoteMode: state.remoteMode,
      webAssetBaseUrl,
      webAssetVersion,
    });
  }

  closeRealtimeSession();
  resetWebFrame();

  state.connected = false;
  state.frameConnected = false;
  state.latestFrame = null;
  state.latestFrameMeta = null;
  state.screenSize = null;
  state.webBridgeLastConnectedAt = 0;
  state.webReconnectDelayMs = WEB_RECONNECT_MIN_MS;
  state.webFrameLoaded = false;
  if (screen) {
    screen.hidden = true;
  }
  if (webFrame) {
    webFrame.hidden = true;
  }
  if (emptyState) {
    emptyState.hidden = false;
  }

  if (connectView) {
    connectView.hidden = true;
  }
  if (controlView) {
    controlView.hidden = false;
  }
  setStatus(controlStrings.status.connecting);
  applyRemoteModeLayout();
  if (state.remoteMode === REMOTE_MODE_WEB) {
    if (shouldOpenRemoteControlPageDirectly()) {
      openRemoteControlPageDirectly();
      return;
    }
    void connectWebBridgeMode();
  } else {
    connectRealtime();
  }
  startControlLoops();
}

function switchRemoteMode(mode, { remember = false } = {}) {
  const nextMode = normalizeRemoteMode(mode);
  if (nextMode === state.remoteMode) {
    return;
  }

  state.remoteMode = nextMode;
  if (remoteModeSelect) {
    remoteModeSelect.value = nextMode;
  }
  if (remember && activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.connecting, { remoteMode: nextMode });
  }

  closeRealtimeSession();
  resetWebFrame();
  state.connected = false;
  state.frameConnected = false;
  state.latestFrame = null;
  state.latestFrameMeta = null;
  state.screenSize = null;
  state.webBridgeLastConnectedAt = 0;
  state.webReconnectDelayMs = WEB_RECONNECT_MIN_MS;
  state.webFrameLoaded = false;
  applyRemoteModeLayout();

  if (nextMode === REMOTE_MODE_WEB) {
    if (shouldOpenRemoteControlPageDirectly()) {
      openRemoteControlPageDirectly();
      return;
    }
    void connectWebBridgeMode();
  } else {
    connectRealtime();
  }
}

function closeRealtimeSession() {
  clearReconnectTimer();
  clearWebBridgeStaleTimer();
  state.transportConnectAttempt += 1;
  state.webConnectAttempt += 1;
  const previousTransportSession = state.transportSession;
  const previousControlSocket = state.controlSocket;
  const previousFrameSocket = state.frameSocket;
  state.transportSession = null;
  state.controlSocket = null;
  state.frameSocket = null;
  previousTransportSession?.close();
  if (!previousTransportSession) {
    previousControlSocket?.close();
    previousFrameSocket?.close();
  }
}

function resetWebFrame() {
  state.webFrameLoaded = false;
  closeParentBridgeClients();
  if (webFrame) {
    webFrame.hidden = true;
    webFrame.removeAttribute("src");
  }
}

function applyRemoteModeLayout() {
  const webMode = state.remoteMode === REMOTE_MODE_WEB;
  if (qualitySelect) {
    qualitySelect.hidden = webMode;
  }
  if (pageZoomControl) {
    pageZoomControl.hidden = webMode;
  }
  screenWrap?.classList.toggle("web-mode", webMode);
  if (screen) {
    screen.hidden = webMode || !state.screenSize;
  }
  if (webFrame) {
    webFrame.hidden = !webMode;
  }
  if (emptyState) {
    emptyState.hidden = webMode ? state.webFrameLoaded : state.frameConnected;
    setEmptyStateLoading(webMode
      ? controlStrings.status.loadingWebBridge
      : controlStrings.status.loadingRemoteScreen);
  }
  updateSecureWebButton();
}

async function connectWebBridgeMode() {
  if (state.remoteMode !== REMOTE_MODE_WEB) {
    return;
  }
  if (!token) {
    showControlConnectionIssue(controlStrings.status.missingToken);
    return;
  }

  const attempt = ++state.webConnectAttempt;
  clearReconnectTimer();
  const registryMode = webAssetRegistryEnabled();
  if (registryMode) {
    primeWebAssetRuntimeConfig();
  }
  const remoteFrameUrl = webBridgeUrl();
  if (!webFrame || !remoteFrameUrl) {
    showControlConnectionIssue(controlStrings.status.webBridgeUnavailable);
    return;
  }

  state.connected = false;
  state.frameConnected = false;
  state.webFrameLoaded = false;
  const preparingStatus = registryMode
    ? controlStrings.loadingAppBundle(webAssetVersion)
    : controlStrings.status.preparingWebCache;
  setStatus(preparingStatus);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, preparingStatus);
  }
  applyRemoteModeLayout();

  if (registryMode || canLoadWebFrameDirectly()) {
    const loadingStatus = registryMode
      ? controlStrings.loadingAppBundle(webAssetVersion)
      : controlStrings.status.loadingWebBridge;
    setStatus(loadingStatus);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, loadingStatus);
    }
    loadWebFrame(remoteFrameUrl, attempt);
    return;
  }

  let frameUrl = remoteFrameUrl;
  try {
    const result = await prepareWebCache(remoteFrameUrl);
    if (state.remoteMode !== REMOTE_MODE_WEB || state.webConnectAttempt !== attempt) {
      return;
    }
    frameUrl = result?.frameUrl || frameUrl;
    if (result?.updated) {
      setStatus(controlStrings.webCacheUpdated(result.cached || 0));
    }
  } catch (error) {
    console.warn("[web-cache] preparation failed", error);
    if (state.remoteMode !== REMOTE_MODE_WEB || state.webConnectAttempt !== attempt) {
      return;
    }
    const message = error?.message || controlStrings.status.webCacheFailed;
    handleWebBridgeDisconnect(message);
    return;
  }
  if (state.remoteMode !== REMOTE_MODE_WEB || state.webConnectAttempt !== attempt) {
    return;
  }
  setStatus(controlStrings.status.loadingWebBridge);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.loadingWebBridge);
  }
  loadWebFrame(frameUrl, attempt);
}

function loadWebFrame(frameUrl, attempt) {
  if (!webFrame || state.remoteMode !== REMOTE_MODE_WEB || state.webConnectAttempt !== attempt) {
    return;
  }

  const applyFrameUrl = () => {
    if (state.remoteMode === REMOTE_MODE_WEB && state.webConnectAttempt === attempt && webFrame.src !== frameUrl) {
      webFrame.src = frameUrl;
    }
  };

  if (webFrame.src === frameUrl) {
    webFrame.removeAttribute("src");
    requestAnimationFrame(applyFrameUrl);
    return;
  }

  webFrame.src = frameUrl;
}

function startControlLoops() {
  if (!pointerMoveTimer) {
    pointerMoveTimer = setInterval(flushPointerMove, POINTER_MOVE_SEND_INTERVAL_MS);
  }
  if (!renderLoopStarted) {
    renderLoopStarted = true;
    requestAnimationFrame(renderFrameLoop);
  }
}

function connectionRequiresPassword(connection, connectionUrl, remoteInfo = null) {
  if (!connectionUsesCloudRemote(connection, connectionUrl)) {
    return false;
  }
  return (
    booleanFlag(connection.requirePassword) ||
    booleanFlag(connection.require_password) ||
    booleanFlag(connectionUrl.searchParams.get("requirePassword")) ||
    connectionUrl.searchParams.get("e2ee") === E2EE_VERSION ||
    booleanFlag(remoteInfoField(remoteInfo, "requirePassword", "require_password"))
  );
}

function connectionUsesCloudRemote(connection, connectionUrl) {
  const authMode = String(
    connection.auth ||
      connection.authMode ||
      connection.auth_mode ||
      connectionUrl.searchParams.get("auth") ||
      "",
  )
    .trim()
    .toLowerCase();
  return (
    authMode === "cloud" ||
    Boolean(
      connection.cloudUser ||
        connection.cloud_user ||
        connection.jwt ||
        connectionUrl.searchParams.get("cloudUser") ||
        connectionUrl.searchParams.get("jwt"),
    )
  );
}

function booleanFlag(value) {
  if (value === true) {
    return true;
  }
  const normalized = String(value || "").trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes" || normalized === "on";
}

function showPasswordGate() {
  if (!passwordGate || !passwordInput) {
    setStatus(controlStrings.status.remotePasswordRequired);
    return;
  }
  passwordGate.hidden = false;
  passwordInput.value = "";
  if (passwordStatus) {
    passwordStatus.textContent = "";
  }
  setStatus(controlStrings.status.remotePasswordRequired);
  requestAnimationFrame(() => {
    if (!passwordGate.hidden) {
      passwordInput.focus();
    }
  });
}

function hidePasswordGate() {
  if (passwordGate) {
    passwordGate.hidden = true;
  }
  if (passwordStatus) {
    passwordStatus.textContent = "";
  }
}

async function unlockPendingPasswordConnection() {
  if (!pendingPasswordConnection || !passwordInput) {
    return;
  }
  const password = passwordInput.value;
  if (!password) {
    if (passwordStatus) {
      passwordStatus.textContent = controlStrings.status.enterPassword;
    }
    return;
  }
  const { connection, instanceId } = pendingPasswordConnection;
  let connectionUrl;
  try {
    connectionUrl = normalizeConnectionUrl(connection.url);
  } catch {
    return;
  }
  const nextToken = connection.token || connectionUrl.searchParams.get("token") || "";
  try {
    if (passwordStatus) {
      passwordStatus.textContent = controlStrings.status.unlocking;
    }
    remoteCrypto = await createRemoteCrypto(password, nextToken);
    pendingPasswordConnection = null;
    hidePasswordGate();
    await startConnection(connection, { instanceId });
  } catch (error) {
    remoteCrypto = null;
    if (passwordStatus) {
      passwordStatus.textContent = error?.message || controlStrings.status.unableUnlock;
    }
  }
}

function renderInstances() {
  if (!instanceList || !addInstanceButton) {
    return;
  }

  const visibleInstances = filteredInstances();
  addInstanceButton.hidden = instances.length === 0;
  instanceList.replaceChildren();

  if (instances.length > 0 && visibleInstances.length === 0) {
    instanceList.appendChild(createSearchEmptyState());
    return;
  }

  for (const instance of visibleInstances) {
    instanceList.appendChild(createInstanceCard(instance));
  }
}

function filteredInstances() {
  const query = normalizeSearchQuery(instanceSearchInput?.value);
  if (!query) {
    return instances;
  }

  return instances.filter((instance) => instanceSearchText(instance).includes(query));
}

function instanceSearchText(instance) {
  return normalizeSearchQuery(
    [
      instance.name,
      instance.host,
      hostFromConnectionUrl(instance.url),
      remoteModeLabel(instance.remoteMode),
      instance.webAssetVersion || "",
      instance.status || controlStrings.status.notConnected,
    ].join(" "),
  );
}

function normalizeSearchQuery(value) {
  return String(value || "").trim().toLowerCase();
}

function createSearchEmptyState() {
  const empty = document.createElement("div");
  empty.className = "instance-list-empty";
  empty.textContent = "No instances match your search.";
  return empty;
}

function createInstanceCard(instance) {
  const card = document.createElement("article");
  card.className = "instance-card";

  const header = document.createElement("div");
  header.className = "instance-card-header";

  const titleWrap = document.createElement("div");
  titleWrap.className = "instance-title-wrap";
  const titleRow = document.createElement("div");
  titleRow.className = "instance-title-row";
  const name = document.createElement("h2");
  name.className = "instance-name";
  name.textContent = instance.name || "Untitled instance";

  const status = document.createElement("span");
  status.className = "status-badge";
  status.dataset.status = statusKind(instance.status);
  status.textContent = instance.status || controlStrings.status.notConnected;

  const host = document.createElement("p");
  host.className = "instance-host";
  host.textContent = instance.host || hostFromConnectionUrl(instance.url) || "Unknown host";
  titleRow.append(name, status);
  titleWrap.append(titleRow, host);

  header.append(titleWrap);

  const meta = document.createElement("div");
  meta.className = "instance-meta";
  const metaLines = [
    createMetaLine("Mode", remoteModeLabel(instance.remoteMode)),
    createMetaLine("Last connected", formatTime(instance.lastConnectedAt)),
  ];
  if (instance.webAssetBaseUrl) {
    metaLines.splice(1, 0, createMetaLine("Version", instance.webAssetVersion || "latest"));
  }
  meta.append(...metaLines);

  const actions = document.createElement("div");
  actions.className = "instance-actions";

  const connect = document.createElement("button");
  connect.className = "primary-button";
  connect.type = "button";
  connect.dataset.instanceAction = "connect";
  connect.dataset.instanceId = instance.id;
  connect.textContent = "Connect";

  const edit = document.createElement("button");
  edit.type = "button";
  edit.dataset.instanceAction = "edit";
  edit.dataset.instanceId = instance.id;
  edit.textContent = "Edit";

  const remove = document.createElement("button");
  remove.className = "danger-button";
  remove.type = "button";
  remove.dataset.instanceAction = "delete";
  remove.dataset.instanceId = instance.id;
  remove.setAttribute("aria-label", `Delete ${instance.name || "instance"}`);
  remove.textContent = "Delete";

  actions.append(connect, edit, remove);
  card.append(header, meta, actions);
  return card;
}

function createMetaLine(label, value) {
  const line = document.createElement("div");
  line.textContent = `${label}: ${value}`;
  return line;
}

function openAddDialog({ locked = false } = {}) {
  if (!addDialogBackdrop || !closeAddDialogButton || !connectionInput || !instanceNameInput) {
    return;
  }

  addDialogLocked = locked;
  addDialogBackdrop.hidden = false;
  closeAddDialogButton.hidden = locked;
  connectionInput.value = "";
  instanceNameInput.value = "";
  stopQrScan();
  setScanStatus(`Scan the QR code from CodexL, or paste the connection URL. Build ${PWA_BUILD}.`);
  requestAnimationFrame(() => {
    if (!addDialogBackdrop.hidden) {
      instanceNameInput.focus();
    }
  });
}

function closeAddDialog({ force = false } = {}) {
  if (addDialogLocked && !force) {
    return;
  }

  stopQrScan();
  addDialogLocked = false;
  if (addDialogBackdrop) {
    addDialogBackdrop.hidden = true;
  }
}

function openEditDialog(instance) {
  if (!editDialogBackdrop || !editNameInput || !editConnectionInput) {
    return;
  }

  editingInstanceId = instance.id;
  editNameInput.value = instance.name || "";
  editConnectionInput.value = instance.url || "";
  setEditStatus("");
  editDialogBackdrop.hidden = false;
  requestAnimationFrame(() => {
    if (!editDialogBackdrop.hidden) {
      editNameInput.focus();
    }
  });
}

function closeEditDialog() {
  editingInstanceId = "";
  if (editDialogBackdrop) {
    editDialogBackdrop.hidden = true;
  }
  setEditStatus("");
}

function addInstanceFromConnection(connection, { connect = false, name = "" } = {}) {
  const instance = upsertInstanceFromConnection(connection, {
    name,
    status: connect ? controlStrings.status.connecting : controlStrings.status.notConnected,
  });
  if (!instance) {
    setScanStatus("Paste a valid connection URL or QR payload.");
    return;
  }

  renderInstances();
  if (connect) {
    navigateToControl(instance);
    return;
  }

  closeAddDialog({ force: true });
}

function saveEditedInstance() {
  const current = findInstance(editingInstanceId);
  if (!current) {
    closeEditDialog();
    return;
  }

  const connection = parseConnection(editConnectionInput.value);
  if (!connection) {
    setEditStatus("Connection URL is invalid.");
    return;
  }

  const next = buildInstanceFromConnection(connection, {
    existing: current,
    name: editNameInput.value,
    status: current.status,
  });
  if (!next) {
    setEditStatus("Connection token is missing.");
    return;
  }

  const nextIdentity = instanceIdentity(next);
  const duplicate = instances.find((instance) => instance.id !== current.id && instanceIdentity(instance) === nextIdentity);
  if (duplicate) {
    setEditStatus("Another instance already uses this connection.");
    return;
  }

  instances = instances.map((instance) => (instance.id === current.id ? next : instance));
  saveStoredInstances();
  renderInstances();
  closeEditDialog();
}

function requestDeleteEditedInstance() {
  const instance = findInstance(editingInstanceId);
  if (instance) {
    openDeleteConfirmDialog(instance);
  }
}

function openDeleteConfirmDialog(instance) {
  if (!deleteConfirmBackdrop || !confirmDeleteButton) {
    deleteInstance(instance.id);
    return;
  }

  pendingDeleteInstanceId = instance.id;
  if (deleteConfirmMessage) {
    deleteConfirmMessage.textContent = `Delete "${instance.name || "Untitled instance"}"? This instance will be removed from the list.`;
  }
  deleteConfirmBackdrop.hidden = false;
  requestAnimationFrame(() => {
    if (!deleteConfirmBackdrop.hidden) {
      confirmDeleteButton.focus();
    }
  });
}

function closeDeleteConfirmDialog() {
  pendingDeleteInstanceId = "";
  if (deleteConfirmBackdrop) {
    deleteConfirmBackdrop.hidden = true;
  }
}

function confirmPendingDelete() {
  const instanceId = pendingDeleteInstanceId;
  closeDeleteConfirmDialog();
  deleteInstance(instanceId);
}

function deleteInstance(instanceId) {
  if (!instanceId) {
    return;
  }

  instances = instances.filter((instance) => instance.id !== instanceId);
  saveStoredInstances();
  renderInstances();
  if (editingInstanceId === instanceId) {
    closeEditDialog();
  }
  if (instances.length === 0 && connectView && !connectView.hidden) {
    openAddDialog({ locked: true });
  }
}

function setEditStatus(text) {
  if (!editStatus) {
    return;
  }

  editStatus.textContent = text;
  editStatus.hidden = !text;
}

function upsertInstanceFromConnection(connection, { name = "", status = "" } = {}) {
  const candidate = buildInstanceFromConnection(connection, { name, status });
  if (!candidate) {
    return null;
  }

  const identity = instanceIdentity(candidate);
  const existing = instances.find((instance) => instanceIdentity(instance) === identity);
  if (existing) {
    const suggestedName = suggestedInstanceName(connection);
    const updated = {
      ...existing,
      host: candidate.host,
      name: normalizeInstanceName(name) || suggestedName || existing.name || candidate.name,
      remoteMode: candidate.remoteMode,
      requirePassword: candidate.requirePassword,
      status: status || existing.status,
      token: candidate.token,
      updatedAt: Date.now(),
      url: candidate.url,
      webAssetBaseUrl: candidate.webAssetBaseUrl,
      webAssetVersion: candidate.webAssetVersion,
    };
    instances = instances.map((instance) => (instance.id === existing.id ? updated : instance));
    saveStoredInstances();
    return updated;
  }

  instances = [candidate, ...instances];
  saveStoredInstances();
  return candidate;
}

function refreshInstanceFromControlUrl(instanceId, stored, connection) {
  const existing = stored || {
    createdAt: Date.now(),
    id: instanceId,
    lastConnectedAt: 0,
    name: "",
    remoteMode: "",
    status: controlStrings.status.connecting,
  };
  const refreshed = buildInstanceFromConnection(connection, { existing, status: controlStrings.status.connecting });
  if (!refreshed) {
    return null;
  }

  const updated = {
    ...refreshed,
    createdAt: stored?.createdAt || refreshed.createdAt,
    id: instanceId,
    lastConnectedAt: stored?.lastConnectedAt || refreshed.lastConnectedAt,
    name: stored?.name || refreshed.name,
    status: controlStrings.status.connecting,
  };
  const existingIndex = instances.findIndex((instance) => instance.id === instanceId);
  instances =
    existingIndex >= 0
      ? instances.map((instance) => (instance.id === instanceId ? updated : instance))
      : [updated, ...instances];
  saveStoredInstances();
  return updated;
}

function buildInstanceFromConnection(connection, { existing = null, name = "", status = "" } = {}) {
  let connectionUrl;
  try {
    connectionUrl = normalizeConnectionUrl(connection.url);
  } catch {
    return null;
  }

  const nextToken = connection.token || connectionUrl.searchParams.get("token") || "";
  if (!nextToken) {
    return null;
  }
  if (connection.cloudUser) {
    connectionUrl.searchParams.set("cloudUser", connection.cloudUser);
  }
  if (connection.jwt) {
    connectionUrl.searchParams.set("jwt", connection.jwt);
  }
  const nextWebAssetBaseUrl = connectionWebAssetBaseUrl(connection, connectionUrl);
  const nextWebAssetVersion = connectionWebAssetVersion(connection, connectionUrl);
  if (nextWebAssetBaseUrl) {
    connectionUrl.searchParams.set("webAssetBaseUrl", nextWebAssetBaseUrl);
    connectionUrl.searchParams.set("webAssetVersion", nextWebAssetVersion);
  }
  const requirePassword = connectionRequiresPassword(connection, connectionUrl);
  if (requirePassword) {
    connectionUrl.searchParams.set("requirePassword", "1");
    connectionUrl.searchParams.set("e2ee", E2EE_VERSION);
  } else {
    connectionUrl.searchParams.delete("requirePassword");
    connectionUrl.searchParams.delete("e2ee");
  }

  const now = Date.now();
  const remoteMode = normalizeRemoteMode(
    connection.remoteMode ||
      connection.mode ||
      connectionUrl.searchParams.get("remoteMode") ||
      connectionUrl.searchParams.get("mode") ||
      existing?.remoteMode,
  );
  const suggestedName = suggestedInstanceName(connection, connectionUrl);
  if (suggestedName) {
    connectionUrl.searchParams.set("name", suggestedName);
  }
  const displayName = normalizeInstanceName(name) || existing?.name || suggestedName || defaultInstanceName(connectionUrl);
  return {
    createdAt: existing?.createdAt || now,
    host: connectionUrl.host,
    id: existing?.id || createInstanceId(),
    lastConnectedAt: existing?.lastConnectedAt || 0,
    name: displayName,
    remoteMode,
    requirePassword,
    status: status || existing?.status || controlStrings.status.notConnected,
    token: nextToken,
    updatedAt: now,
    url: connectionUrl.toString(),
    webAssetBaseUrl: nextWebAssetBaseUrl,
    webAssetVersion: nextWebAssetVersion,
  };
}

function updateInstanceStatus(instanceId, status, extras = {}) {
  let changed = false;
  instances = instances.map((instance) => {
    if (instance.id !== instanceId) {
      return instance;
    }

    changed = true;
    return {
      ...instance,
      ...extras,
      status,
      updatedAt: Date.now(),
    };
  });

  if (!changed) {
    return;
  }

  saveStoredInstances();
  if (connectView && !connectView.hidden) {
    renderInstances();
  }
}

function resetTransientInstanceStatuses() {
  let changed = false;
  instances = instances.map((instance) => {
    if (statusKind(instance.status) === "idle") {
      return instance;
    }

    changed = true;
    return {
      ...instance,
      status: controlStrings.status.notConnected,
      updatedAt: Date.now(),
    };
  });

  if (changed) {
    saveStoredInstances();
  }
}

function findInstance(instanceId) {
  return instances.find((instance) => instance.id === instanceId) || null;
}

function readStoredInstances() {
  try {
    const raw = localStorage.getItem(INSTANCE_STORAGE_KEY);
    const stored = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(stored)) {
      return [];
    }

    return stored.map(normalizeStoredInstance).filter(Boolean);
  } catch {
    return [];
  }
}

function normalizeStoredInstance(instance) {
  if (!instance || typeof instance !== "object") {
    return null;
  }

  return buildInstanceFromConnection(
    {
      remoteMode: typeof instance.remoteMode === "string" ? instance.remoteMode : "",
      requirePassword: Boolean(instance.requirePassword),
      token: typeof instance.token === "string" ? instance.token : "",
      url: typeof instance.url === "string" ? instance.url : "",
      webAssetBaseUrl: typeof instance.webAssetBaseUrl === "string" ? instance.webAssetBaseUrl : "",
      webAssetVersion: typeof instance.webAssetVersion === "string" ? instance.webAssetVersion : "",
    },
    {
      existing: {
        createdAt: Number(instance.createdAt) || Date.now(),
        id: typeof instance.id === "string" && instance.id ? instance.id : createInstanceId(),
        lastConnectedAt: Number(instance.lastConnectedAt) || 0,
        name: typeof instance.name === "string" ? instance.name : "",
        remoteMode: typeof instance.remoteMode === "string" ? instance.remoteMode : "",
        status: typeof instance.status === "string" && instance.status ? instance.status : controlStrings.status.notConnected,
      },
      name: typeof instance.name === "string" ? instance.name : "",
      status: typeof instance.status === "string" && instance.status ? instance.status : controlStrings.status.notConnected,
    },
  );
}

function saveStoredInstances() {
  try {
    localStorage.setItem(INSTANCE_STORAGE_KEY, JSON.stringify(instances));
  } catch {
    // Storage can be blocked in private browsing modes.
  }
}

function instanceIdentity(instance) {
  let url;
  try {
    url = normalizeConnectionUrl(instance.url);
  } catch {
    return `${instance.host || ""}|${instance.token || ""}`;
  }

  return `${url.origin}${websocketBasePath(url.pathname)}|${instance.token || ""}`;
}

function createInstanceId() {
  if (globalThis.crypto?.randomUUID) {
    return globalThis.crypto.randomUUID();
  }

  return `instance-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function normalizeInstanceName(name) {
  return String(name || "").trim().replace(/\s+/g, " ");
}

function suggestedInstanceName(connection, parsedUrl) {
  let connectionUrl = parsedUrl;
  if (!connectionUrl) {
    try {
      connectionUrl = normalizeConnectionUrl(connection.url);
    } catch {
      connectionUrl = null;
    }
  }

  const directName = normalizeInstanceName(
    connection.name ||
      connectionUrl?.searchParams.get("name") ||
      connectionUrl?.searchParams.get("itemName") ||
      connectionUrl?.searchParams.get("item_name") ||
      "",
  );
  if (directName) {
    return directName;
  }

  return mobileInstanceItemName(
    connection.deviceName || connectionUrl?.searchParams.get("deviceName") || connectionUrl?.searchParams.get("device_name") || "",
    connection.workspaceName ||
      connectionUrl?.searchParams.get("workspaceName") ||
      connectionUrl?.searchParams.get("workspace_name") ||
      "",
  );
}

function mobileInstanceItemName(deviceName, workspaceName) {
  const device = mobileInstanceNameSegment(deviceName);
  const workspace = mobileInstanceNameSegment(workspaceName);
  return [device, workspace].filter(Boolean).join("-");
}

function mobileInstanceNameSegment(value) {
  return String(value || "")
    .trim()
    .replace(/\.local$/i, "")
    .toLowerCase()
    .replace(/[\/\\-]+/g, "-")
    .replace(/[\u0000-\u001f\u007f]+/g, " ")
    .replace(/\s+/g, " ")
    .replace(/\s*-\s*/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-+|-+$/g, "")
    .trim();
}

function defaultInstanceName(url) {
  return url.hostname;
}

async function createRemoteCrypto(password, keyToken) {
  if (!globalThis.crypto?.subtle) {
    throw new Error("This browser cannot unlock encrypted remote control.");
  }
  const keyBytes = await deriveRemoteCryptoKeyBytes(password, keyToken);
  const key = await globalThis.crypto.subtle.importKey(
    "raw",
    keyBytes,
    { name: "AES-GCM" },
    false,
    ["decrypt", "encrypt"],
  );
  const keyBase64 = base64UrlEncode(keyBytes);
  try {
    sessionStorage.setItem(remoteCryptoStorageKey(keyToken), keyBase64);
  } catch {
    // The bridge can still use sockets owned by this page; only iframe reconnects lose the key.
  }
  return {
    key,
    keyBase64,
    token: keyToken,
    async decryptBytes(value) {
      const bytes = bytesFromBinary(value);
      if (bytes.byteLength < E2EE_BINARY_MAGIC.byteLength + 12 || !startsWithBytes(bytes, E2EE_BINARY_MAGIC)) {
        throw new Error("Encrypted remote payload is required.");
      }
      const nonce = bytes.slice(E2EE_BINARY_MAGIC.byteLength, E2EE_BINARY_MAGIC.byteLength + 12);
      const payload = bytes.slice(E2EE_BINARY_MAGIC.byteLength + 12);
      return globalThis.crypto.subtle.decrypt(
        { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
        key,
        payload,
      );
    },
    async decryptText(value) {
      const envelope = JSON.parse(String(value || ""));
      if (envelope?.type !== "e2ee" || envelope.version !== 1) {
        throw new Error("Encrypted remote payload is required.");
      }
      const nonce = base64UrlDecode(String(envelope.nonce || ""));
      const payload = base64UrlDecode(String(envelope.payload || ""));
      const decrypted = await globalThis.crypto.subtle.decrypt(
        { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
        key,
        payload,
      );
      return new TextDecoder().decode(decrypted);
    },
    async encryptBytes(value) {
      const nonce = globalThis.crypto.getRandomValues(new Uint8Array(12));
      const encrypted = new Uint8Array(
        await globalThis.crypto.subtle.encrypt(
          { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
          key,
          bytesFromBinary(value),
        ),
      );
      const packet = new Uint8Array(E2EE_BINARY_MAGIC.byteLength + nonce.byteLength + encrypted.byteLength);
      packet.set(E2EE_BINARY_MAGIC, 0);
      packet.set(nonce, E2EE_BINARY_MAGIC.byteLength);
      packet.set(encrypted, E2EE_BINARY_MAGIC.byteLength + nonce.byteLength);
      return packet.buffer;
    },
    async encryptText(value) {
      const nonce = globalThis.crypto.getRandomValues(new Uint8Array(12));
      const encrypted = new Uint8Array(
        await globalThis.crypto.subtle.encrypt(
          { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
          key,
          new TextEncoder().encode(String(value || "")),
        ),
      );
      return JSON.stringify({
        type: "e2ee",
        version: 1,
        nonce: base64UrlEncode(nonce),
        payload: base64UrlEncode(encrypted),
      });
    },
  };
}

async function deriveRemoteCryptoKeyBytes(password, keyToken) {
  const material = await globalThis.crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(password),
    "PBKDF2",
    false,
    ["deriveBits"],
  );
  const bits = await globalThis.crypto.subtle.deriveBits(
    {
      hash: "SHA-256",
      iterations: E2EE_PBKDF2_ITERATIONS,
      name: "PBKDF2",
      salt: new TextEncoder().encode(`${E2EE_SALT_PREFIX}\0${keyToken}`),
    },
    material,
    256,
  );
  return new Uint8Array(bits);
}

function applyCryptoParams(params) {
  if (!remoteRequiresPassword && !remoteCrypto) {
    return params;
  }
  params.set("requirePassword", "1");
  params.set("e2ee", E2EE_VERSION);
  return params;
}

function applyWebEndpointCryptoParams(params) {
  if (!remoteWebEndpointRequiresCrypto()) {
    return params;
  }
  return applyCryptoParams(params);
}

function remoteWebEndpointRequiresCrypto() {
  return Boolean(
    (remoteRequiresPassword || remoteCrypto) &&
      (remoteAuthMode === "cloud" || remoteCloudUser || remoteJwt),
  );
}

function remoteCryptoStorageKey(keyToken) {
  return `${E2EE_STORAGE_PREFIX}${keyToken}`;
}

function bytesFromBinary(value) {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  return new TextEncoder().encode(String(value || ""));
}

function startsWithBytes(bytes, prefix) {
  if (bytes.byteLength < prefix.byteLength) {
    return false;
  }
  for (let index = 0; index < prefix.byteLength; index += 1) {
    if (bytes[index] !== prefix[index]) {
      return false;
    }
  }
  return true;
}

function base64UrlEncode(value) {
  const bytes = bytesFromBinary(value);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function base64UrlDecode(value) {
  const normalized = String(value || "").replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function hostFromConnectionUrl(value) {
  try {
    return normalizeConnectionUrl(value).host;
  } catch {
    return "";
  }
}

function statusKind(status) {
  const normalized = String(status || "").toLowerCase();
  if (!normalized || normalized === "not connected") {
    return "idle";
  }
  if (normalized.includes("disconnect") || normalized.includes("retry")) {
    return "retrying";
  }
  if (normalized.includes("connecting") || normalized.includes("loading") || normalized.includes("preparing")) {
    return "connecting";
  }
  if (normalized.includes("cdp connected")) {
    return "cdp";
  }
  if (normalized.includes("web connected")) {
    return "connected";
  }
  if (normalized.includes("connected")) {
    return "connected";
  }
  return "idle";
}

function formatTime(value) {
  const timestamp = Number(value) || 0;
  if (!timestamp) {
    return "Never";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
}

function connectionFromUrlParams(params) {
  const encodedUrl = params.get("url") || params.get("connection") || "";
  if (encodedUrl) {
    const parsed = parseConnection(encodedUrl);
    if (parsed) {
      return parsed;
    }
  }

  const directToken = params.get("token") || "";
  if (!directToken) {
    return null;
  }

  return {
    cloudUser: params.get("cloudUser") || "",
    deviceName: params.get("deviceName") || params.get("device_name") || "",
    jwt: params.get("jwt") || "",
    remoteMode: params.get("remoteMode") || params.get("mode") || "",
    name: params.get("name") || params.get("itemName") || params.get("item_name") || "",
    requirePassword: booleanFlag(params.get("requirePassword")) || params.get("e2ee") === E2EE_VERSION,
    token: directToken,
    url: location.href,
    webAssetBaseUrl: params.get("webAssetBaseUrl") || params.get("web_asset_base_url") || "",
    webAssetVersion: params.get("webAssetVersion") || params.get("web_asset_version") || "",
    workspaceName: params.get("workspaceName") || params.get("workspace_name") || "",
  };
}

function parseConnection(raw) {
  const value = String(raw || "").trim();
  if (!value) {
    return null;
  }

  try {
    const parsed = JSON.parse(value);
    if (parsed && typeof parsed === "object") {
      if (typeof parsed.url === "string") {
        const connection = parseConnection(parsed.url);
        if (!connection) {
          return null;
        }
        return {
          cloudUser: typeof parsed.cloudUser === "string" ? parsed.cloudUser : connection.cloudUser,
          deviceName:
            typeof parsed.deviceName === "string"
              ? parsed.deviceName
              : typeof parsed.device_name === "string"
                ? parsed.device_name
                : connection.deviceName,
          jwt: typeof parsed.jwt === "string" ? parsed.jwt : connection.jwt,
          remoteMode:
            typeof parsed.remoteMode === "string"
              ? parsed.remoteMode
              : typeof parsed.mode === "string"
                ? parsed.mode
                : connection.remoteMode,
          requirePassword:
            parsed.requirePassword === true ||
            parsed.require_password === true ||
            connection.requirePassword,
          name:
            typeof parsed.name === "string"
              ? parsed.name
              : typeof parsed.itemName === "string"
                ? parsed.itemName
                : typeof parsed.item_name === "string"
                  ? parsed.item_name
                  : connection.name,
          token: typeof parsed.token === "string" ? parsed.token : connection.token,
          url: connection.url,
          webAssetBaseUrl:
            typeof parsed.webAssetBaseUrl === "string"
              ? parsed.webAssetBaseUrl
              : typeof parsed.web_asset_base_url === "string"
                ? parsed.web_asset_base_url
                : connection.webAssetBaseUrl,
          webAssetVersion:
            typeof parsed.webAssetVersion === "string"
              ? parsed.webAssetVersion
              : typeof parsed.web_asset_version === "string"
                ? parsed.web_asset_version
                : connection.webAssetVersion,
          workspaceName:
            typeof parsed.workspaceName === "string"
              ? parsed.workspaceName
              : typeof parsed.workspace_name === "string"
                ? parsed.workspace_name
                : connection.workspaceName,
        };
      }

      if (typeof parsed.host === "string" && parsed.port) {
        const protocol = typeof parsed.protocol === "string" ? parsed.protocol.replace(/:$/, "") : "http";
        const url = new URL(`${protocol}://${parsed.host}:${parsed.port}/`);
        if (typeof parsed.token === "string") {
          url.searchParams.set("token", parsed.token);
        }
        if (typeof parsed.cloudUser === "string") {
          url.searchParams.set("cloudUser", parsed.cloudUser);
        }
        if (typeof parsed.jwt === "string") {
          url.searchParams.set("jwt", parsed.jwt);
        }
        const parsedName =
          typeof parsed.name === "string"
            ? parsed.name
            : typeof parsed.itemName === "string"
              ? parsed.itemName
              : typeof parsed.item_name === "string"
                ? parsed.item_name
                : "";
        if (parsedName) {
          url.searchParams.set("name", parsedName);
        }
        const parsedDeviceName =
          typeof parsed.deviceName === "string"
            ? parsed.deviceName
            : typeof parsed.device_name === "string"
              ? parsed.device_name
              : "";
        if (parsedDeviceName) {
          url.searchParams.set("deviceName", parsedDeviceName);
        }
        const parsedWorkspaceName =
          typeof parsed.workspaceName === "string"
            ? parsed.workspaceName
            : typeof parsed.workspace_name === "string"
              ? parsed.workspace_name
              : "";
        if (parsedWorkspaceName) {
          url.searchParams.set("workspaceName", parsedWorkspaceName);
        }
        const parsedMode =
          typeof parsed.remoteMode === "string"
            ? parsed.remoteMode
            : typeof parsed.mode === "string"
              ? parsed.mode
              : "";
        if (parsedMode) {
          url.searchParams.set("remoteMode", parsedMode);
        }
        const parsedWebAssetBaseUrl =
          typeof parsed.webAssetBaseUrl === "string"
            ? parsed.webAssetBaseUrl
            : typeof parsed.web_asset_base_url === "string"
              ? parsed.web_asset_base_url
              : "";
        const parsedWebAssetVersion =
          typeof parsed.webAssetVersion === "string"
            ? parsed.webAssetVersion
            : typeof parsed.web_asset_version === "string"
              ? parsed.web_asset_version
              : "";
        if (parsedWebAssetBaseUrl) {
          url.searchParams.set("webAssetBaseUrl", parsedWebAssetBaseUrl);
          url.searchParams.set("webAssetVersion", parsedWebAssetVersion || "latest");
        }
        const requirePassword = parsed.requirePassword === true || parsed.require_password === true;
        if (requirePassword) {
          url.searchParams.set("requirePassword", "1");
          url.searchParams.set("e2ee", E2EE_VERSION);
        }
        return {
          cloudUser: typeof parsed.cloudUser === "string" ? parsed.cloudUser : "",
          deviceName: parsedDeviceName,
          jwt: typeof parsed.jwt === "string" ? parsed.jwt : "",
          remoteMode: parsedMode,
          requirePassword,
          name: parsedName,
          token: typeof parsed.token === "string" ? parsed.token : "",
          url: url.toString(),
          webAssetBaseUrl: parsedWebAssetBaseUrl,
          webAssetVersion: parsedWebAssetVersion,
          workspaceName: parsedWorkspaceName,
        };
      }
    }
  } catch {
    // Treat non-JSON payloads as URLs below.
  }

  try {
    const url = normalizeConnectionUrl(value);
    return {
      cloudUser: url.searchParams.get("cloudUser") || "",
      deviceName: url.searchParams.get("deviceName") || url.searchParams.get("device_name") || "",
      jwt: url.searchParams.get("jwt") || "",
      remoteMode: url.searchParams.get("remoteMode") || url.searchParams.get("mode") || "",
      requirePassword: booleanFlag(url.searchParams.get("requirePassword")) || url.searchParams.get("e2ee") === E2EE_VERSION,
      name: url.searchParams.get("name") || url.searchParams.get("itemName") || url.searchParams.get("item_name") || "",
      token: url.searchParams.get("token") || "",
      url: url.toString(),
      webAssetBaseUrl: url.searchParams.get("webAssetBaseUrl") || url.searchParams.get("web_asset_base_url") || "",
      webAssetVersion: url.searchParams.get("webAssetVersion") || url.searchParams.get("web_asset_version") || "",
      workspaceName: url.searchParams.get("workspaceName") || url.searchParams.get("workspace_name") || "",
    };
  } catch {
    return null;
  }
}

function normalizeConnectionUrl(value) {
  const url = new URL(String(value || "").trim());
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Unsupported connection protocol");
  }
  return url;
}

function setScanStatus(text) {
  if (!scanStatus) {
    return;
  }

  scanStatus.textContent = text;
}

async function startQrScan() {
  if (!window.isSecureContext && location.hostname !== "localhost" && location.hostname !== "127.0.0.1") {
    throw new Error("Camera scanning requires HTTPS or localhost.");
  }
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error("Camera access is not available in this browser.");
  }

  stopQrScan();
  setScanStatus("Requesting camera permission...");
  const detector = createQrDetector();
  scanStream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      facingMode: { ideal: "environment" },
      height: { ideal: 1080 },
      width: { ideal: 1080 },
    },
  });
  scanVideo.srcObject = scanStream;
  scanVideo.hidden = false;
  scanButton.hidden = true;
  stopScanButton.hidden = false;
  await scanVideo.play();
  setScanStatus("Point the camera at the CodexL QR code.");
  scanFrame(detector);
}

async function scanFrame(detector) {
  if (!scanStream) {
    return;
  }

  try {
    const rawValue = await readQrRawValue(detector);
    const connection = parseConnection(rawValue);
    if (connection) {
      setScanStatus("QR code detected. Adding instance...");
      addInstanceFromConnection(connection, { name: instanceNameInput.value });
      return;
    }
  } catch {
    // Some browsers throw while the video element is warming up.
  }

  scanTimer = window.setTimeout(() => scanFrame(detector), 180);
}

function createQrDetector() {
  if ("BarcodeDetector" in window) {
    try {
      return { detector: new BarcodeDetector({ formats: ["qr_code"] }), type: "native" };
    } catch {
      // Fall through to the local CodexL QR decoder.
    }
  }
  return { type: "codex" };
}

async function readQrRawValue(detector) {
  if (detector?.type === "native") {
    try {
      const nativeValue = await detectWithBarcodeDetector(detector.detector);
      if (nativeValue) {
        return nativeValue;
      }
    } catch {
      // Some browsers expose BarcodeDetector but fail on live video frames.
    }
  }
  return decodeCodexQrFromVideo(scanVideo) || "";
}

async function detectWithBarcodeDetector(detector) {
  const codes = await detector.detect(scanVideo);
  return codes?.[0]?.rawValue || "";
}

function stopQrScan() {
  if (scanTimer) {
    clearTimeout(scanTimer);
    scanTimer = null;
  }
  if (scanStream) {
    for (const track of scanStream.getTracks()) {
      track.stop();
    }
    scanStream = null;
  }
  if (scanVideo) {
    scanVideo.pause();
    scanVideo.srcObject = null;
    scanVideo.hidden = true;
  }
  if (scanButton) {
    scanButton.hidden = false;
  }
  if (stopScanButton) {
    stopScanButton.hidden = true;
  }
}

function registerServiceWorker() {
  if (!("serviceWorker" in navigator) || location.protocol === "file:") {
    return;
  }
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (sessionStorage.getItem("codexl-sw-reloaded") === PWA_BUILD) {
      return;
    }
    sessionStorage.setItem("codexl-sw-reloaded", PWA_BUILD);
    location.reload();
  });
  navigator.serviceWorker
    .register(SERVICE_WORKER_URL)
    .then((registration) => registration.update())
    .catch(() => {
      // Screen mode still works without offline caching; the workspace view requires the worker.
    });
}

async function connectRealtime() {
  if (state.remoteMode !== REMOTE_MODE_SCREENCAST) {
    return;
  }
  if (!token) {
    showControlConnectionIssue(controlStrings.status.missingToken);
    return;
  }

  const attempt = ++state.transportConnectAttempt;
  clearReconnectTimer();
  setStatus(controlStrings.status.connecting);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.connecting);
  }

  let session;
  try {
    session = await openRealtimeSession({
      controlWebSocketUrl: wsUrl("/ws/control"),
      crypto: remoteCrypto,
      frameWebSocketUrl: wsUrl("/ws/frame"),
      onWebTransportFallback: logWebTransportFallback,
      timeoutMs: TRANSPORT_CONNECT_TIMEOUT_MS,
      transportPreference,
      webTransportEligible: remoteOrigin.startsWith("https:"),
      webTransportUrl: webTransportUrl(),
    });
  } catch {
    if (state.transportConnectAttempt === attempt) {
      handleTransportDisconnect(controlStrings.status.controlDisconnectedRetrying);
    }
    return;
  }

  if (state.transportConnectAttempt !== attempt) {
    session.close();
    return;
  }

  state.transportSession = session;
  state.controlSocket = session.control;
  state.frameSocket = session.frame;

  session.control.addEventListener("message", (event) => {
    if (typeof event.data === "string") {
      handleControlMessage(JSON.parse(event.data));
    }
  });

  session.frame.addEventListener("message", (event) => {
    if (typeof event.data !== "string") {
      state.latestFrame = event.data;
    }
  });

  session.frame.addEventListener("metadata", (event) => {
    if (event.detail && typeof event.detail === "object") {
      handleControlMessage({ ...event.detail, type: "frameMeta" });
    }
  });

  session.addEventListener("close", () => {
    if (state.transportSession !== session) {
      return;
    }

    handleTransportDisconnect(controlStrings.status.controlDisconnectedRetrying);
  });

  session.addEventListener("error", () => {
    session.close();
  });

  if (session.control.readyState !== TRANSPORT_OPEN || session.frame.readyState !== TRANSPORT_OPEN) {
    session.close();
    return;
  }

  state.connected = true;
  state.frameConnected = true;
  clearReconnectTimer();
  state.lastSentViewportKey = "";
  state.lastSentPageZoomKey = "";
  setStatus(controlStrings.status.connected);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.connected, { lastConnectedAt: Date.now() });
  }
  sendViewport();
  sendPageZoom({ force: true });
  send({ type: "refresh" });
}

function wsUrl(pathname) {
  const protocol = remoteOrigin.startsWith("https:") ? "wss:" : "ws:";
  const params = new URLSearchParams({ token });
  applyCryptoParams(params);
  if (remoteCloudUser) {
    params.set("cloudUser", remoteCloudUser);
  }
  if (remoteJwt) {
    params.set("jwt", remoteJwt);
  }
  const host = new URL(remoteOrigin).host;
  return `${protocol}//${host}${remotePathPrefix}${pathname}?${params.toString()}`;
}

async function fetchRemoteInfo() {
  try {
    const infoUrl = new URL(`${remotePathPrefix}/api/remote-info`, remoteOrigin);
    infoUrl.searchParams.set("token", token);
    if (remoteCloudUser) {
      infoUrl.searchParams.set("cloudUser", remoteCloudUser);
    }
    if (remoteJwt) {
      infoUrl.searchParams.set("jwt", remoteJwt);
    }
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 1800);
    const response = await fetch(infoUrl, { cache: "no-store", signal: controller.signal }).finally(() => {
      window.clearTimeout(timeout);
    });
    if (response.status === 401 || response.status === 403) {
      throw remoteInfoError(REMOTE_INFO_AUTH_ERROR, controlStrings.status.remoteTokenExpired);
    }
    if (!response.ok) {
      return null;
    }
    const info = await response.json();
    return info && typeof info === "object" ? info : null;
  } catch (error) {
    if (error?.code === REMOTE_INFO_AUTH_ERROR) {
      throw error;
    }
    if (error?.name === "AbortError") {
      throw remoteInfoError(REMOTE_INFO_UNAVAILABLE_ERROR, controlStrings.status.remoteMetadataTimedOut);
    }
    throw remoteInfoError(REMOTE_INFO_UNAVAILABLE_ERROR, controlStrings.status.remoteServiceUnavailable);
  }
}

function remoteInfoError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function webAssetRegistryEnabled() {
  return Boolean(webAssetBaseUrl);
}

function connectionWebAssetBaseUrl(connection, connectionUrl, remoteInfo = null) {
  return normalizeWebAssetBaseUrl(
    connection.webAssetBaseUrl ||
      connection.web_asset_base_url ||
      connectionUrl.searchParams.get("webAssetBaseUrl") ||
      connectionUrl.searchParams.get("web_asset_base_url") ||
      remoteInfoField(remoteInfo, "webAssetBaseUrl", "web_asset_base_url") ||
      urlParams.get("webAssetBaseUrl") ||
      urlParams.get("web_asset_base_url") ||
      "",
  );
}

function connectionWebAssetVersion(connection, connectionUrl, remoteInfo = null) {
  return normalizeWebAssetVersion(
    connection.webAssetVersion ||
      connection.web_asset_version ||
      connectionUrl.searchParams.get("webAssetVersion") ||
      connectionUrl.searchParams.get("web_asset_version") ||
      remoteInfoField(remoteInfo, "webAssetVersion", "web_asset_version") ||
      urlParams.get("webAssetVersion") ||
      urlParams.get("web_asset_version") ||
      "latest",
  );
}

function remoteInfoField(remoteInfo, ...keys) {
  if (!remoteInfo || typeof remoteInfo !== "object") {
    return "";
  }
  for (const key of keys) {
    const value = remoteInfo[key];
    if (value !== undefined && value !== null) {
      return value;
    }
  }
  return "";
}

function normalizeWebAssetBaseUrl(value) {
  const raw = String(value || "").trim();
  if (!raw) {
    return "";
  }
  try {
    const url = new URL(raw, location.href);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "";
    }
    url.hash = "";
    url.search = "";
    return url.toString().replace(/\/+$/, "");
  } catch {
    return "";
  }
}

function normalizeWebAssetVersion(value) {
  const version = String(value || "").trim() || "latest";
  return /^[0-9A-Za-z._-]+$/.test(version) ? version : "latest";
}

function applyWebAssetParams(params) {
  if (!webAssetRegistryEnabled()) {
    return params;
  }
  params.set("webAssetMode", "registry");
  params.set("webAssetBaseUrl", webAssetBaseUrl);
  params.set("webAssetVersion", webAssetVersion);
  return params;
}

function applyCodexLRuntimeParams(params) {
  const config = webAssetRuntimeConfig;
  const pluginRuntimeScriptUrl = config?.pluginRuntimeScriptUrl || "";
  if (pluginRuntimeScriptUrl) {
    params.set("codexlPluginRuntimeUrl", pluginRuntimeScriptUrl);
    params.set("codexlRuntimeUrl", pluginRuntimeScriptUrl);
  } else if (config?.runtimeBaseUrl) {
    params.set("codexlRuntimeBaseUrl", config.runtimeBaseUrl);
  }
  return params;
}

function webAssetFrameUrl() {
  const cachedFrameUrl = cachedWebAssetFrameUrl();
  if (cachedFrameUrl) {
    return new URL(cachedFrameUrl);
  }
  const base = new URL(`${webAssetBaseUrl}/`);
  return new URL(`${encodeURIComponent(webAssetVersion)}/`, base);
}

function configureWebAssetVersionSelect(versions = [], latest = "") {
  if (!webAssetVersionSelect) {
    return;
  }
  if (!webAssetRegistryEnabled()) {
    webAssetVersionSelect.hidden = true;
    return;
  }

  const labels = new Map();
  const latestVersion = normalizeWebAssetVersion(latest || "");
  labels.set(
    "latest",
    latest && latestVersion !== "latest"
      ? `${controlStrings.latest} (${latestVersion})`
      : controlStrings.latest,
  );
  labels.set(webAssetVersion, webAssetVersion === "latest" ? labels.get("latest") : webAssetVersion);
  for (const item of versions) {
    const version = normalizeWebAssetVersion(typeof item === "string" ? item : item?.version);
    if (!version || version === "latest") {
      continue;
    }
    labels.set(version, typeof item?.label === "string" && item.label ? item.label : version);
  }

  webAssetVersionSelect.replaceChildren(
    ...Array.from(labels, ([value, label]) => {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      return option;
    }),
  );
  webAssetVersionSelect.value = labels.has(webAssetVersion) ? webAssetVersion : "latest";
  webAssetVersionSelect.hidden = false;
}

async function refreshWebAssetVersions() {
  const baseUrl = webAssetBaseUrl;
  if (!baseUrl) {
    return;
  }
  try {
    const response = await fetch(new URL("versions.json", `${baseUrl}/`), { cache: "no-cache" });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    const manifest = await response.json();
    if (baseUrl !== webAssetBaseUrl) {
      return;
    }
    configureWebAssetVersionSelect(manifest?.versions || [], manifest?.latest || "");
    updateSecureWebButton();
  } catch (error) {
    console.warn("[web-assets] unable to load versions.json", error);
  }
}

function queueWebAssetVersionsRefresh() {
  if (webAssetVersionRefreshTimer) {
    clearTimeout(webAssetVersionRefreshTimer);
    webAssetVersionRefreshTimer = null;
  }
  if (!webAssetRegistryEnabled()) {
    return;
  }
  webAssetVersionRefreshTimer = window.setTimeout(() => {
    webAssetVersionRefreshTimer = null;
    void refreshWebAssetVersions();
  }, WEB_ASSET_VERSION_REFRESH_DELAY_MS);
}

function webAssetRuntimeCacheKey() {
  return `${webAssetBaseUrl}|${webAssetVersion}`;
}

function primeWebAssetRuntimeConfig() {
  if (!webAssetRegistryEnabled()) {
    return;
  }
  preconnectWebAssetBaseUrl(webAssetBaseUrl);
  hydrateCachedWebAssetRuntimeConfig();
  void refreshWebAssetRuntimeConfig({ force: true });
}

async function refreshWebAssetRuntimeConfig({ force = false } = {}) {
  if (!webAssetRegistryEnabled()) {
    webAssetRuntimeConfig = null;
    webAssetRuntimeConfigKey = "";
    webAssetRuntimeConfigInflight = null;
    webAssetRuntimeConfigInflightKey = "";
    return null;
  }
  const key = webAssetRuntimeCacheKey();
  if (!force && webAssetRuntimeConfig && webAssetRuntimeConfigKey === key) {
    return webAssetRuntimeConfig;
  }
  if (webAssetRuntimeConfigInflight && webAssetRuntimeConfigInflightKey === key) {
    return webAssetRuntimeConfigInflight;
  }

  webAssetRuntimeConfigInflightKey = key;
  webAssetRuntimeConfigInflight = loadWebAssetRuntimeConfig(webAssetBaseUrl, webAssetVersion)
    .then((config) => {
      if (key !== webAssetRuntimeCacheKey()) {
        return webAssetRuntimeConfig;
      }
      webAssetRuntimeConfig = config;
      webAssetRuntimeConfigKey = key;
      writeCachedWebAssetRuntimeConfig(key, config);
      return config;
    })
    .catch((error) => {
      console.warn("[web-assets] unable to load runtime manifest", error);
      if (key === webAssetRuntimeCacheKey() && webAssetRuntimeConfigKey !== key) {
        webAssetRuntimeConfig = null;
        webAssetRuntimeConfigKey = key;
      }
      return null;
    })
    .finally(() => {
      if (webAssetRuntimeConfigInflightKey === key) {
        webAssetRuntimeConfigInflight = null;
        webAssetRuntimeConfigInflightKey = "";
      }
    });
  return webAssetRuntimeConfigInflight;
}

async function loadWebAssetRuntimeConfig(baseUrl, version) {
  const manifestUrl = await webAssetManifestUrl(baseUrl, version);
  if (!manifestUrl) {
    return null;
  }
  const response = await fetch(manifestUrl, { cache: "force-cache" });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  const manifest = await response.json();
  return webAssetRuntimeConfigFromManifest(manifest, manifestUrl);
}

function webAssetRuntimeConfigFromManifest(manifest, manifestUrl) {
  const runtimeBaseUrl = resolveWebAssetRuntimeBaseUrl(
    manifest?.runtimeBaseUrl || manifest?.runtime_base_url || "",
    manifestUrl,
  );
  const bridgeScriptUrl =
    resolveWebAssetRuntimeScriptUrl(
      manifest?.bridgeScriptUrl || manifest?.bridge_script_url || "",
      manifestUrl,
    ) ||
    runtimeScriptUrlFromBase(runtimeBaseUrl, "_codexl_bridge.js") ||
    resolveWebAssetRuntimeScriptUrl(
      manifest?.bridgeScript || manifest?.bridge_script || "",
      manifestUrl,
    );
  const pluginRuntimeScriptUrl =
    resolveWebAssetRuntimeScriptUrl(
      manifest?.pluginRuntimeScriptUrl || manifest?.plugin_runtime_script_url || "",
      manifestUrl,
    ) ||
    runtimeScriptUrlFromBase(runtimeBaseUrl, "_codexl_plugin.js") ||
    resolveWebAssetRuntimeScriptUrl(
      manifest?.pluginRuntimeScript || manifest?.plugin_runtime_script || "",
      manifestUrl,
    );
  return {
    bridgeScriptUrl,
    frameUrl: webAssetEntryFrameUrl(manifest, manifestUrl),
    manifestUrl: manifestUrl.toString(),
    pluginRuntimeScriptUrl,
    runtimeBaseUrl,
  };
}

async function webAssetManifestUrl(baseUrl, version) {
  const base = new URL(`${baseUrl}/`);
  const normalizedVersion = normalizeWebAssetVersion(version);
  if (normalizedVersion === "latest") {
    try {
      const latestUrl = new URL("latest.json", base);
      const response = await fetch(latestUrl, { cache: "no-cache" });
      if (response.ok) {
        const latest = await response.json();
        const manifestPath =
          typeof latest?.manifest === "string"
            ? latest.manifest
            : typeof latest?.manifestPath === "string"
              ? latest.manifestPath
              : "";
        if (manifestPath) {
          return new URL(manifestPath, base);
        }
      }
    } catch (error) {
      console.warn("[web-assets] unable to resolve latest manifest", error);
    }
  }
  return new URL(`${encodeURIComponent(normalizedVersion)}/manifest.json`, base);
}

function webAssetEntryFrameUrl(manifest, manifestUrl) {
  const entry = String(manifest?.entry || manifest?.entryPath || manifest?.entry_path || "index.html").trim();
  try {
    const entryUrl = new URL(entry || "index.html", manifestUrl);
    if (entryUrl.protocol !== "http:" && entryUrl.protocol !== "https:") {
      return "";
    }
    entryUrl.hash = "";
    entryUrl.search = "";
    if (entryUrl.pathname.endsWith("/index.html")) {
      entryUrl.pathname = entryUrl.pathname.slice(0, -"index.html".length);
    }
    return entryUrl.toString();
  } catch {
    return "";
  }
}

function resolveWebAssetRuntimeBaseUrl(value, baseUrl) {
  const raw = String(value || "").trim();
  if (!raw) {
    return "";
  }
  try {
    const url = new URL(raw.endsWith("/") ? raw : `${raw}/`, baseUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "";
    }
    url.hash = "";
    url.search = "";
    return url.toString().replace(/\/+$/, "");
  } catch {
    return "";
  }
}

function resolveWebAssetRuntimeScriptUrl(value, baseUrl) {
  const raw = String(value || "").trim();
  if (!raw) {
    return "";
  }
  try {
    const url = new URL(raw, baseUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "";
    }
    return url.toString();
  } catch {
    return "";
  }
}

function runtimeScriptUrlFromBase(baseUrl, fileName) {
  if (!baseUrl) {
    return "";
  }
  try {
    return new URL(fileName, `${baseUrl}/`).toString();
  } catch {
    return "";
  }
}

function hydrateCachedWebAssetRuntimeConfig() {
  const key = webAssetRuntimeCacheKey();
  if (webAssetRuntimeConfig && webAssetRuntimeConfigKey === key) {
    return webAssetRuntimeConfig;
  }
  const cached = readCachedWebAssetRuntimeConfig(key);
  if (!cached) {
    return null;
  }
  webAssetRuntimeConfig = cached;
  webAssetRuntimeConfigKey = key;
  return cached;
}

function cachedWebAssetFrameUrl() {
  const key = webAssetRuntimeCacheKey();
  let config = webAssetRuntimeConfig && webAssetRuntimeConfigKey === key ? webAssetRuntimeConfig : null;
  if (!config) {
    config = readCachedWebAssetRuntimeConfig(key);
    if (config) {
      webAssetRuntimeConfig = config;
      webAssetRuntimeConfigKey = key;
    }
  }
  const frameUrl = config?.frameUrl || "";
  if (!frameUrl) {
    return "";
  }
  try {
    const url = new URL(frameUrl);
    return url.protocol === "http:" || url.protocol === "https:" ? url.toString() : "";
  } catch {
    return "";
  }
}

function readCachedWebAssetRuntimeConfig(key) {
  try {
    const raw = localStorage.getItem(webAssetRuntimeConfigStorageKey(key));
    const cached = raw ? JSON.parse(raw) : null;
    if (!cached || typeof cached !== "object") {
      return null;
    }
    const cachedAt = Number(cached.cachedAt) || 0;
    if (!cachedAt || Date.now() - cachedAt > webAssetRuntimeConfigMaxAgeMs()) {
      return null;
    }
    const config = cached.config;
    return validWebAssetRuntimeConfig(config) ? config : null;
  } catch {
    return null;
  }
}

function writeCachedWebAssetRuntimeConfig(key, config) {
  if (!validWebAssetRuntimeConfig(config)) {
    return;
  }
  try {
    localStorage.setItem(
      webAssetRuntimeConfigStorageKey(key),
      JSON.stringify({
        cachedAt: Date.now(),
        config,
      }),
    );
  } catch {
    // Runtime metadata is an optimization only.
  }
}

function webAssetRuntimeConfigStorageKey(key) {
  return `${WEB_ASSET_RUNTIME_CONFIG_STORAGE_PREFIX}${key}`;
}

function webAssetRuntimeConfigMaxAgeMs() {
  return normalizeWebAssetVersion(webAssetVersion) === "latest"
    ? WEB_ASSET_RUNTIME_CONFIG_LATEST_MAX_AGE_MS
    : WEB_ASSET_RUNTIME_CONFIG_VERSIONED_MAX_AGE_MS;
}

function validWebAssetRuntimeConfig(config) {
  if (!config || typeof config !== "object") {
    return false;
  }
  return ["bridgeScriptUrl", "frameUrl", "manifestUrl", "pluginRuntimeScriptUrl", "runtimeBaseUrl"].every((key) => {
    const value = config[key];
    return value === undefined || value === null || typeof value === "string";
  });
}

function preconnectWebAssetBaseUrl(baseUrl) {
  const raw = String(baseUrl || "").trim();
  if (!raw) {
    return;
  }
  try {
    const origin = new URL(raw).origin;
    if (preconnectedWebAssetOrigins.has(origin)) {
      return;
    }
    preconnectedWebAssetOrigins.add(origin);
    const preconnect = document.createElement("link");
    preconnect.rel = "preconnect";
    preconnect.href = origin;
    preconnect.crossOrigin = "anonymous";
    document.head.appendChild(preconnect);

    const dnsPrefetch = document.createElement("link");
    dnsPrefetch.rel = "dns-prefetch";
    dnsPrefetch.href = origin;
    document.head.appendChild(dnsPrefetch);
  } catch {
    // Ignore malformed registry URLs; normal URL validation handles them elsewhere.
  }
}

function defaultWebAssetRegistryUrl() {
  return normalizeWebAssetBaseUrl(
    urlParams.get("defaultWebAssetBaseUrl") ||
      urlParams.get("default_web_asset_base_url") ||
      DEFAULT_WEB_ASSET_REGISTRY_URL,
  );
}

function secureRegistryWebAvailable() {
  return Boolean(webAssetBaseUrl || defaultWebAssetRegistryUrl());
}

function shouldShowSecureWebButton() {
  return state.remoteMode === REMOTE_MODE_WEB && !window.isSecureContext && secureRegistryWebAvailable();
}

function updateSecureWebButton() {
  if (!secureWebButton) {
    return;
  }
  secureWebButton.hidden = !shouldShowSecureWebButton();
}

function openSecureRegistryWeb() {
  const nextBaseUrl = webAssetBaseUrl || defaultWebAssetRegistryUrl();
  if (!nextBaseUrl) {
    setStatus(controlStrings.status.httpsBundleUnavailable);
    return;
  }

  webAssetBaseUrl = nextBaseUrl;
  webAssetVersion = normalizeWebAssetVersion(webAssetVersion || urlParams.get("webAssetVersion") || "latest");
  webAssetRuntimeConfig = null;
  webAssetRuntimeConfigKey = "";
  webAssetRuntimeConfigInflight = null;
  webAssetRuntimeConfigInflightKey = "";
  primeWebAssetRuntimeConfig();
  persistActiveWebAssetSelection();
  configureWebAssetVersionSelect();
  updateSecureWebButton();

  if (window.isSecureContext) {
    queueWebAssetVersionsRefresh();
    void connectWebBridgeMode();
    return;
  }

  void openRegistryStandalonePage();
}

function persistActiveWebAssetSelection() {
  if (!activeInstanceId || !webAssetRegistryEnabled()) {
    return;
  }
  let changed = false;
  instances = instances.map((instance) => {
    if (instance.id !== activeInstanceId) {
      return instance;
    }
    let nextUrl = instance.url;
    try {
      const url = normalizeConnectionUrl(instance.url);
      url.searchParams.set("webAssetBaseUrl", webAssetBaseUrl);
      url.searchParams.set("webAssetVersion", webAssetVersion);
      nextUrl = url.toString();
    } catch {
      // Keep the existing URL if it cannot be parsed; the instance fields still persist the selection.
    }
    changed = true;
    return {
      ...instance,
      updatedAt: Date.now(),
      url: nextUrl,
      webAssetBaseUrl,
      webAssetVersion,
    };
  });
  if (changed) {
    saveStoredInstances();
  }
}

function webBridgeUrl() {
  const url = webAssetRegistryEnabled()
    ? webAssetFrameUrl()
    : new URL(`${remotePathPrefix}/web/index.html`, remoteOrigin);
  url.searchParams.set("hostId", "local");
  url.searchParams.set("token", token);
  applyWebEndpointCryptoParams(url.searchParams);
  url.searchParams.set(WEB_BRIDGE_URL_PARAM, webBridgeSocketUrl());
  if (webAssetRegistryEnabled()) {
    url.searchParams.set("codexBridgeParent", "1");
    url.searchParams.set("codexBridgeParentOrigin", location.origin);
  }
  applyCodexLRuntimeParams(url.searchParams);
  const bridgeTransportUrl = webBridgeWebTransportUrl();
  if (bridgeTransportUrl) {
    url.searchParams.set("codexBridgeTransportUrl", bridgeTransportUrl);
  }
  url.searchParams.set("transport", transportPreference || "auto");
  return url.toString();
}

function registryStandaloneUrl() {
  const url = webAssetFrameUrl();
  url.searchParams.set("hostId", "local");
  url.searchParams.set("token", token);
  applyWebEndpointCryptoParams(url.searchParams);
  url.searchParams.set(WEB_BRIDGE_URL_PARAM, webBridgeSocketUrl());
  applyCodexLRuntimeParams(url.searchParams);
  const bridgeTransportUrl = webBridgeWebTransportUrl();
  if (bridgeTransportUrl) {
    url.searchParams.set("codexBridgeTransportUrl", bridgeTransportUrl);
  }
  url.searchParams.set("transport", transportPreference || "auto");
  return url.toString();
}

function localWebFrameUrl(scope, remoteFrameUrl) {
  const remoteUrl = new URL(remoteFrameUrl);
  const url = new URL("web/index.html", scope || location.href);
  url.search = remoteUrl.search;
  applyWebEndpointCryptoParams(url.searchParams);
  url.searchParams.set(WEB_BRIDGE_URL_PARAM, webBridgeSocketUrl());
  applyCodexLRuntimeParams(url.searchParams);
  const bridgeTransportUrl = webBridgeWebTransportUrl();
  if (bridgeTransportUrl) {
    url.searchParams.set("codexBridgeTransportUrl", bridgeTransportUrl);
  }
  url.searchParams.set("transport", transportPreference || "auto");
  return url.toString();
}

function webVersionUrl() {
  const url = new URL(`${remotePathPrefix}/web/_version`, remoteOrigin);
  url.searchParams.set("hostId", "local");
  url.searchParams.set("token", token);
  applyWebEndpointCryptoParams(url.searchParams);
  return url.toString();
}

function webResourceWebSocketUrl() {
  const protocol = remoteOrigin.startsWith("https:") ? "wss:" : "ws:";
  const host = new URL(remoteOrigin).host;
  const params = new URLSearchParams({
    hostId: "local",
    token,
  });
  applyWebEndpointCryptoParams(params);
  return `${protocol}//${host}${remotePathPrefix}/web/_resource?${params.toString()}`;
}

function webResourceWebTransportUrl() {
  return webTransportUrlWithPath("/wt/web-resource", { webEndpoint: true });
}

function webBridgeSocketUrl() {
  const protocol = remoteOrigin.startsWith("https:") ? "wss:" : "ws:";
  const host = new URL(remoteOrigin).host;
  const params = new URLSearchParams({
    hostId: "local",
    token,
  });
  applyWebEndpointCryptoParams(params);
  return `${protocol}//${host}${remotePathPrefix}/web/_bridge?${params.toString()}`;
}

function webBridgeWebTransportUrl() {
  return webTransportUrlWithPath("/wt/web-bridge", { webEndpoint: true });
}

function canLoadWebFrameDirectly() {
  return (
    !remoteOrigin.startsWith("https:") &&
    remoteOrigin === location.origin &&
    remotePathPrefix === websocketBasePath(location.pathname)
  );
}

function shouldOpenRemoteControlPageDirectly() {
  return location.protocol === "https:" && remoteOrigin.startsWith("http:");
}

function openRemoteControlPageDirectly() {
  const url = new URL(`${remotePathPrefix}/control.html`, remoteOrigin);
  url.searchParams.set("token", token);
  url.searchParams.set("remoteMode", REMOTE_MODE_WEB);
  url.searchParams.set("transport", transportPreference || "auto");
  applyCryptoParams(url.searchParams);
  applyWebAssetParams(url.searchParams);
  if (remoteCloudUser) {
    url.searchParams.set("cloudUser", remoteCloudUser);
  }
  if (remoteJwt) {
    url.searchParams.set("jwt", remoteJwt);
  }
  setStatus(controlStrings.status.openingLanRemote);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.openingLanRemote);
  }
  location.href = url.toString();
}

async function openRegistryStandalonePage() {
  const statusText = controlStrings.openingHttpsAppBundle(webAssetVersion);
  setStatus(statusText);
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, statusText, {
      webAssetBaseUrl,
      webAssetVersion,
    });
  }
  primeWebAssetRuntimeConfig();
  location.href = registryStandaloneUrl();
}

async function prepareWebCache(iframeUrl) {
  if (!("serviceWorker" in navigator)) {
    throw new Error("This browser cannot prepare the remote workspace.");
  }
  const registration = await navigator.serviceWorker.ready;
  const worker = await webCacheServiceWorker(registration);
  const cacheIframeUrl = localWebFrameUrl(registration.scope, iframeUrl);

  return new Promise((resolve, reject) => {
    const requestId = `web-cache-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const channel = typeof MessageChannel === "function" ? new MessageChannel() : null;
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error("Timed out preparing the remote workspace."));
    }, WEB_CACHE_PREPARE_TIMEOUT_MS);
    const cleanup = () => {
      clearTimeout(timer);
      if (channel) {
        channel.port1.removeEventListener("message", onMessage);
        channel.port1.close();
      } else {
        navigator.serviceWorker.removeEventListener("message", onMessage);
      }
    };
    const onMessage = (event) => {
      const message = event.data || {};
      if (message.type !== "web-cache-ready" || message.requestId !== requestId) {
        return;
      }
      cleanup();
      if (message.error) {
        reject(new Error(message.error));
      } else {
        resolve({ ...message, frameUrl: cacheIframeUrl });
      }
    };
    const payload = {
      cacheIframeUrl,
      iframeUrl,
      requestId,
      e2eeKey: remoteWebEndpointRequiresCrypto() ? remoteCrypto?.keyBase64 || "" : "",
      resourceWebTransportUrl: webResourceWebTransportUrl(),
      resourceWebSocketUrl: webResourceWebSocketUrl(),
      transportPreference,
      type: "prepare-web-cache",
      versionUrl: webVersionUrl(),
    };
    if (channel) {
      channel.port1.addEventListener("message", onMessage);
      channel.port1.start();
      worker.postMessage(payload, [channel.port2]);
    } else {
      navigator.serviceWorker.addEventListener("message", onMessage);
      worker.postMessage(payload);
    }
  });
}

function webCacheServiceWorker(registration) {
  if (navigator.serviceWorker.controller) {
    return Promise.resolve(navigator.serviceWorker.controller);
  }
  if (registration?.active) {
    return Promise.resolve(registration.active);
  }

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error("Service worker did not activate in time"));
    }, 15000);
    const cleanup = () => {
      clearTimeout(timer);
      navigator.serviceWorker.removeEventListener("controllerchange", onControllerChange);
      registration?.installing?.removeEventListener("statechange", onStateChange);
      registration?.waiting?.removeEventListener("statechange", onStateChange);
    };
    const maybeResolve = () => {
      const worker =
        navigator.serviceWorker.controller ||
        registration?.active ||
        (registration?.waiting?.state === "activated" ? registration.waiting : null) ||
        (registration?.installing?.state === "activated" ? registration.installing : null);
      if (!worker) {
        return false;
      }
      cleanup();
      resolve(worker);
      return true;
    };
    const onControllerChange = () => {
      maybeResolve();
    };
    const onStateChange = () => {
      maybeResolve();
    };
    registration?.installing?.addEventListener("statechange", onStateChange);
    registration?.waiting?.addEventListener("statechange", onStateChange);
    navigator.serviceWorker.addEventListener("controllerchange", onControllerChange);
    maybeResolve();
  });
}

function normalizeRemoteMode(mode) {
  return REMOTE_MODE_WEB;
}

function remoteModeLabel(mode) {
  return normalizeRemoteMode(mode) === REMOTE_MODE_WEB ? controlStrings.webMode : "Screen";
}

function webTransportUrl() {
  return webTransportUrlWithPath("/wt/session");
}

function webTransportUrlWithPath(pathname, { webEndpoint = false } = {}) {
  if (!remoteOrigin.startsWith("https:")) {
    return "";
  }
  const params = new URLSearchParams({ token });
  if (webEndpoint) {
    applyWebEndpointCryptoParams(params);
  } else {
    applyCryptoParams(params);
  }
  if (remoteCloudUser) {
    params.set("cloudUser", remoteCloudUser);
  }
  if (remoteJwt) {
    params.set("jwt", remoteJwt);
  }
  return `https://${webTransportHost()}${remotePathPrefix}${pathname}?${params.toString()}`;
}

function webTransportHost() {
  const url = new URL(remoteOrigin);
  const hostname = url.hostname.includes(":") ? `[${url.hostname.replace(/^\[|\]$/g, "")}]` : url.hostname;
  return `${hostname}:${url.port || "443"}`;
}

function handleTransportDisconnect(statusText) {
  if (state.remoteMode !== REMOTE_MODE_SCREENCAST) {
    return;
  }
  state.connected = false;
  state.controlSocket = null;
  state.frameConnected = false;
  state.frameSocket = null;
  state.lastSentViewportKey = "";
  state.lastSentPageZoomKey = "";
  state.latestFrame = null;
  state.transportSession = null;
  setStatus(statusText);
  if (emptyState && !state.frameConnected) {
    emptyState.hidden = false;
    setEmptyStateIssue(statusText);
  }
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, statusText);
  }
  scheduleRealtimeReconnect();
}

function scheduleRealtimeReconnect({ immediate = false } = {}) {
  if (state.remoteMode !== REMOTE_MODE_SCREENCAST) {
    return;
  }
  clearReconnectTimer();
  state.reconnectTimer = setTimeout(() => {
    state.reconnectTimer = null;
    if (state.remoteMode === REMOTE_MODE_SCREENCAST) {
      connectRealtime();
    }
  }, immediate ? 0 : RECONNECT_MS);
}

function handleWebBridgeDisconnect(statusText) {
  if (state.remoteMode !== REMOTE_MODE_WEB) {
    return;
  }
  const nextStatus = retryingStatus(statusText || controlStrings.status.webBridgeDisconnectedRetrying);
  state.connected = false;
  setStatus(nextStatus);
  if (emptyState && !state.webFrameLoaded) {
    emptyState.hidden = false;
    setEmptyStateIssue(nextStatus);
  }
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, nextStatus);
  }
  scheduleWebReconnect(nextStatus);
}

function scheduleWebReconnect(statusText, { immediate = false } = {}) {
  if (state.remoteMode !== REMOTE_MODE_WEB) {
    return;
  }
  clearReconnectTimer();
  const delay = immediate ? 0 : state.webReconnectDelayMs;
  state.webReconnectDelayMs = Math.min(WEB_RECONNECT_MAX_MS, Math.max(WEB_RECONNECT_MIN_MS, Math.floor(state.webReconnectDelayMs * 1.6)));
  state.reconnectTimer = setTimeout(() => {
    state.reconnectTimer = null;
    if (state.remoteMode !== REMOTE_MODE_WEB) {
      return;
    }
    setStatus(statusText);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, statusText);
    }
    void connectWebBridgeMode();
  }, delay);
}

function clearReconnectTimer() {
  if (!state.reconnectTimer) {
    return;
  }
  clearTimeout(state.reconnectTimer);
  state.reconnectTimer = null;
}

function scheduleWebBridgeStaleReload(statusText) {
  clearWebBridgeStaleTimer();
  const expectedLastConnectedAt = state.webBridgeLastConnectedAt;
  state.webBridgeStaleTimer = setTimeout(() => {
    state.webBridgeStaleTimer = null;
    if (
      state.remoteMode !== REMOTE_MODE_WEB ||
      state.webBridgeLastConnectedAt !== expectedLastConnectedAt ||
      state.statusText === controlStrings.status.webConnected
    ) {
      return;
    }
    scheduleWebReconnect(statusText, { immediate: true });
  }, WEB_BRIDGE_PARENT_RELOAD_MS);
}

function clearWebBridgeStaleTimer() {
  if (!state.webBridgeStaleTimer) {
    return;
  }
  clearTimeout(state.webBridgeStaleTimer);
  state.webBridgeStaleTimer = null;
}

function retryingStatus(text) {
  const value = String(text || "").trim();
  if (!value) {
    return controlStrings.status.webBridgeDisconnectedRetrying;
  }
  if (/\bretrying\b/i.test(value) || value.includes("重试")) {
    return value;
  }
  return CONTROL_LANGUAGE === "zh" ? `${value}，正在重试` : `${value}, retrying`;
}

function markWebBridgeConnected(reason = "bridge") {
  if (state.remoteMode !== REMOTE_MODE_WEB) {
    return;
  }
  const wasConnected = state.connected && state.webFrameLoaded;
  const connectedAt = Date.now();
  state.connected = true;
  state.webFrameLoaded = true;
  state.webBridgeLastConnectedAt = connectedAt;
  state.webReconnectDelayMs = WEB_RECONNECT_MIN_MS;
  clearReconnectTimer();
  clearWebBridgeStaleTimer();
  applyRemoteModeLayout();
  setStatus(controlStrings.status.webConnected);
  if (!wasConnected) {
    console.info("[remote-control] web bridge connected", { reason });
  }
  if (activeInstanceId) {
    updateInstanceStatus(activeInstanceId, controlStrings.status.webConnected, { lastConnectedAt: connectedAt });
  }
}

function handleWebBridgeStatusMessage(event) {
  if (handleParentBridgeFrameMessage(event)) {
    return;
  }
  const message = event.data || {};
  if (!message || message.type !== WEB_BRIDGE_STATUS_MESSAGE) {
    return;
  }
  if (!isExpectedWebFrameMessage(event)) {
    console.info("[remote-control] ignored web bridge status from unexpected frame", {
      expectedOrigin: webFrameTargetOrigin(),
      origin: event.origin || "",
      status: message.status || "",
    });
    return;
  }
  if (state.remoteMode !== REMOTE_MODE_WEB) {
    return;
  }

  const status = String(message.status || "");
  if (status === "connected") {
    markWebBridgeConnected("status");
    return;
  }

  if (status === "connecting" || status === "reconnecting" || status === "disconnected") {
    const text = status === "connecting"
      ? controlStrings.status.connectingWebBridge
      : controlStrings.status.webBridgeDisconnectedRetrying;
    state.connected = false;
    setStatus(text);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, text);
    }
    scheduleWebBridgeStaleReload(text);
  }
}

function handleParentBridgeFrameMessage(event) {
  const message = event.data || {};
  if (!message || typeof message !== "object") {
    return false;
  }
  const type = message.type;
  if (
    type !== PARENT_BRIDGE_OPEN_MESSAGE &&
    type !== PARENT_BRIDGE_SEND_MESSAGE &&
    type !== PARENT_BRIDGE_CLOSE_MESSAGE
  ) {
    return false;
  }
  if (!isExpectedWebFrameMessage(event)) {
    console.info("[remote-control] ignored parent bridge message from unexpected frame", {
      expectedOrigin: webFrameTargetOrigin(),
      origin: event.origin || "",
      type,
    });
    return true;
  }
  const clientId = typeof message.clientId === "string" ? message.clientId : "";
  if (!clientId) {
    return true;
  }
  if (type === PARENT_BRIDGE_OPEN_MESSAGE) {
    openParentBridgeClient(clientId);
  } else if (type === PARENT_BRIDGE_SEND_MESSAGE) {
    sendParentBridgeClientMessage(clientId, String(message.raw || ""));
  } else if (type === PARENT_BRIDGE_CLOSE_MESSAGE) {
    closeParentBridgeClient(clientId, { notify: false });
  }
  return true;
}

function openParentBridgeClient(clientId) {
  const existing = parentBridgeClients.get(clientId);
  if (existing?.socket?.readyState === WebSocket.OPEN) {
    markWebBridgeConnected("parent-existing");
    postParentBridgeFrameMessage(clientId, { type: PARENT_BRIDGE_OPENED_MESSAGE });
    return;
  }
  closeParentBridgeClient(clientId, { notify: false });

  const socket = new WebSocket(webBridgeSocketUrl());
  const client = {
    opened: false,
    socket,
  };
  parentBridgeClients.set(clientId, client);

  socket.addEventListener("open", () => {
    client.opened = true;
    markWebBridgeConnected("parent-open");
    postParentBridgeFrameMessage(clientId, { type: PARENT_BRIDGE_OPENED_MESSAGE });
  });
  socket.addEventListener("message", (event) => {
    markWebBridgeConnected("parent-message");
    postParentBridgeFrameMessage(clientId, {
      raw: typeof event.data === "string" ? event.data : "",
      type: PARENT_BRIDGE_MESSAGE,
    });
  });
  socket.addEventListener("close", () => {
    if (parentBridgeClients.get(clientId)?.socket !== socket) {
      return;
    }
    parentBridgeClients.delete(clientId);
    postParentBridgeFrameMessage(clientId, { type: PARENT_BRIDGE_CLOSED_MESSAGE });
  });
  socket.addEventListener("error", () => {
    if (!client.opened) {
      postParentBridgeFrameMessage(clientId, {
        error: "Codex bridge websocket failed to connect",
        type: PARENT_BRIDGE_ERROR_MESSAGE,
      });
    }
  });
}

function sendParentBridgeClientMessage(clientId, raw) {
  const socket = parentBridgeClients.get(clientId)?.socket;
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    postParentBridgeFrameMessage(clientId, {
      error: "Codex bridge websocket is not open",
      type: PARENT_BRIDGE_ERROR_MESSAGE,
    });
    return;
  }
  socket.send(raw);
}

function closeParentBridgeClient(clientId, { notify = true } = {}) {
  const client = parentBridgeClients.get(clientId);
  if (!client) {
    return;
  }
  parentBridgeClients.delete(clientId);
  try {
    client.socket?.close();
  } catch {}
  if (notify) {
    postParentBridgeFrameMessage(clientId, { type: PARENT_BRIDGE_CLOSED_MESSAGE });
  }
}

function closeParentBridgeClients() {
  for (const clientId of Array.from(parentBridgeClients.keys())) {
    closeParentBridgeClient(clientId, { notify: false });
  }
  parentBridgeClients = new Map();
}

function postParentBridgeFrameMessage(clientId, payload) {
  if (!webFrame?.contentWindow) {
    return;
  }
  webFrame.contentWindow.postMessage(
    {
      ...payload,
      clientId,
    },
    webFrameTargetOrigin(),
  );
}

function isExpectedWebFrameMessage(event) {
  if (!webFrame?.contentWindow) {
    return false;
  }
  if (event.source === webFrame.contentWindow) {
    return true;
  }
  const expectedOrigin = webFrameTargetOrigin();
  return Boolean(event.origin && expectedOrigin !== "*" && event.origin === expectedOrigin);
}

function webFrameTargetOrigin() {
  try {
    return new URL(webFrame?.src || "", location.href).origin;
  } catch {
    return "*";
  }
}

function logWebTransportFallback(error) {
  if (webTransportFallbackLogged) {
    return;
  }

  webTransportFallbackLogged = true;
  console.info("[transport] WebTransport unavailable, falling back to WebSocket", error);
}

function websocketBasePath(pathname) {
  if (pathname === "/" || pathname === "/index.html") {
    return "";
  }

  if (pathname.endsWith("/index.html")) {
    return pathname.slice(0, -"/index.html".length);
  }

  return pathname.endsWith("/") ? pathname.slice(0, -1) : "";
}

function handleControlMessage(message) {
  if (message.type === "heartbeat") {
    send({ type: "pong", ts: message.ts });
    return;
  }

  if (message.type === "frameMeta") {
    state.latestFrameMeta = message;
    state.frameFormat = message.format || state.frameFormat;
    if (Array.isArray(message.editableRects)) {
      state.editableRects = message.editableRects;
    }
    return;
  }

  if (message.type === "profile") {
    state.frameProfile = message.profile || null;
    return;
  }

  if (message.type === "status") {
    if (message.status?.screencastProfileMode) {
      qualitySelect.value = message.status.screencastProfileMode;
    }
    if (message.status?.screencastProfileSettings) {
      state.frameProfile = message.status.screencastProfileSettings;
    }
    if (message.status?.pageZoomScale && !state.hasStoredPageZoom && !state.pageZoomMenuOpen) {
      setPageZoomScale(message.status.pageZoomScale);
    }
    const nextStatus = message.status?.connected
      ? controlStrings.status.cdpConnected
      : controlStrings.status.waitingForCdp;
    setStatus(nextStatus);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, nextStatus);
    }
    return;
  }

  if (message.type === "warning") {
    setStatus(message.message);
    if (activeInstanceId) {
      updateInstanceStatus(activeInstanceId, message.message || "Warning");
    }
    return;
  }

  if (message.type === "keyboard") {
    if (!message.focus) {
      blurKeyboardProxy();
    }
  }
}

async function renderFrameLoop() {
  const frame = state.latestFrame;
  state.latestFrame = null;

  if (frame) {
    await drawFrameBytes(frame);
  }

  requestAnimationFrame(renderFrameLoop);
}

async function drawFrameBytes(frame) {
  let image = null;
  try {
    const blob = frame instanceof Blob ? frame : new Blob([frame], { type: `image/${state.frameFormat || "jpeg"}` });
    image = await decodeFrame(blob);

    const width = image.width || image.naturalWidth;
    const height = image.height || image.naturalHeight;
    if (screen.width !== width || screen.height !== height) {
      screen.width = width;
      screen.height = height;
    }
    state.screenSize = { height, width };
    fitScreenToWrap();

    screenContext.drawImage(image, 0, 0);
    screen.hidden = false;
    emptyState.hidden = true;

    const meta = state.latestFrameMeta || {};
    const title = meta.target?.title || "Codex";
    const profile = state.frameProfile?.name ? ` · ${state.frameProfile.name}` : "";
    setStatus(`${title} · ${meta.metrics?.width || width}x${meta.metrics?.height || height}${profile}`);
  } catch {
    setStatus(controlStrings.status.frameDecodeFailed);
  } finally {
    cleanupDecodedImage(image);
  }
}

async function decodeFrame(source) {
  if (source instanceof Blob && window.createImageBitmap) {
    try {
      return await createImageBitmap(source);
    } catch {
      // Fall back to Image below for browsers with partial createImageBitmap support.
    }
  }

  const image = new Image();
  image.decoding = "async";
  const objectUrl = source instanceof Blob ? URL.createObjectURL(source) : "";
  image.src = objectUrl || source;
  image.objectUrl = objectUrl;

  if (image.decode) {
    try {
      await image.decode();
      return image;
    } catch {
      // Fall back to onload; decode can reject when a newer frame replaces the URL quickly.
    }
  }

  if (image.complete && image.naturalWidth) {
    return image;
  }

  await new Promise((resolve, reject) => {
    image.onload = resolve;
    image.onerror = reject;
  });
  return image;
}

function cleanupDecodedImage(image) {
  image?.close?.();
  if (image?.objectUrl) {
    URL.revokeObjectURL(image.objectUrl);
  }
}

function startPinch(event) {
  const touches = firstTwoTouches(event);
  if (!touches || !state.screenBaseSize) {
    return;
  }

  cancelPendingScroll();
  state.lastTouch = null;
  state.sidebarSwipe = null;
  state.touchMoved = true;
  const center = touchCenter(touches);
  const centerPoint = pointRelativeToWrap(center.x, center.y);
  state.pinch = {
    startCenterX: centerPoint.x,
    startCenterY: centerPoint.y,
    startDistance: Math.max(1, touchDistance(touches)),
    startPanX: state.screenPanX,
    startPanY: state.screenPanY,
    startZoom: state.zoom,
  };
}

function updatePinch(event) {
  const touches = firstTwoTouches(event);
  if (!touches) {
    return;
  }

  if (!state.pinch) {
    startPinch(event);
    return;
  }

  const center = touchCenter(touches);
  const centerPoint = pointRelativeToWrap(center.x, center.y);
  const nextZoom = clamp(
    state.pinch.startZoom * (touchDistance(touches) / state.pinch.startDistance),
    MIN_SCREEN_ZOOM,
    MAX_SCREEN_ZOOM,
  );
  const contentX = (state.pinch.startCenterX - state.pinch.startPanX) / state.pinch.startZoom;
  const contentY = (state.pinch.startCenterY - state.pinch.startPanY) / state.pinch.startZoom;

  state.zoom = nextZoom;
  state.screenPanX = centerPoint.x - contentX * nextZoom;
  state.screenPanY = centerPoint.y - contentY * nextZoom;
  state.touchMoved = true;
  constrainScreenPan();
  applyScreenViewport();
}

function endPinch() {
  state.pinch = null;
  state.ignoreNextClick = true;
  if (state.zoom <= MIN_SCREEN_ZOOM + 0.02) {
    state.zoom = MIN_SCREEN_ZOOM;
  }
  constrainScreenPan();
  applyScreenViewport();
}

function fitScreenToWrap() {
  if (!state.screenSize) {
    return;
  }

  const wrapRect = screenWrap.getBoundingClientRect();
  if (!wrapRect.width || !wrapRect.height) {
    return;
  }

  const scale = Math.min(wrapRect.width / state.screenSize.width, wrapRect.height / state.screenSize.height);
  const width = Math.max(1, Math.floor(state.screenSize.width * scale));
  const height = Math.max(1, Math.floor(state.screenSize.height * scale));
  state.screenBaseSize = { height, width };
  constrainScreenPan();
  applyScreenViewport();
}

function applyScreenViewport() {
  if (!state.screenBaseSize) {
    return;
  }

  const width = Math.max(1, Math.round(state.screenBaseSize.width * state.zoom));
  const height = Math.max(1, Math.round(state.screenBaseSize.height * state.zoom));
  screen.style.width = `${width}px`;
  screen.style.height = `${height}px`;
  screen.style.transform = state.screenPanX || state.screenPanY ? `translate(${state.screenPanX}px, ${state.screenPanY}px)` : "none";
}

function constrainScreenPan() {
  if (!state.screenBaseSize || state.zoom <= MIN_SCREEN_ZOOM) {
    state.screenPanX = 0;
    state.screenPanY = 0;
    return;
  }

  const wrapRect = screenWrap.getBoundingClientRect();
  const width = state.screenBaseSize.width * state.zoom;
  const height = state.screenBaseSize.height * state.zoom;
  const maxX = Math.max(0, (width - wrapRect.width) / 2);
  const maxY = Math.max(0, (height - wrapRect.height) / 2);
  state.screenPanX = clamp(state.screenPanX, -maxX, maxX);
  state.screenPanY = clamp(state.screenPanY, -maxY, maxY);
}

function firstTwoTouches(event) {
  if (event.touches.length < 2) {
    return null;
  }

  return [event.touches[0], event.touches[1]];
}

function touchDistance([first, second]) {
  return Math.hypot(first.clientX - second.clientX, first.clientY - second.clientY);
}

function touchCenter([first, second]) {
  return {
    x: (first.clientX + second.clientX) / 2,
    y: (first.clientY + second.clientY) / 2,
  };
}

function pointRelativeToWrap(clientX, clientY) {
  const rect = screenWrap.getBoundingClientRect();
  return {
    x: clientX - rect.left - rect.width / 2,
    y: clientY - rect.top - rect.height / 2,
  };
}

function beginSidebarSwipe(touch) {
  if (!touch) {
    state.sidebarSwipe = null;
    return;
  }

  const rect = screenWrap.getBoundingClientRect();
  if (!rect.width || !rect.height) {
    state.sidebarSwipe = null;
    return;
  }

  const edgeWidth = sidebarSwipeEdgeWidth(rect);
  const closeRegionWidth = sidebarSwipeCloseRegionWidth(rect);
  const rightOpenRegionWidth = sidebarSwipeRightOpenRegionWidth(rect);
  const rightGestureWidth = Math.max(closeRegionWidth, rightOpenRegionWidth);
  const x = touch.clientX - rect.left;
  let side = null;
  if (x <= Math.max(edgeWidth, closeRegionWidth)) {
    side = "left";
  } else if (rect.width - x <= rightGestureWidth) {
    side = "right";
  }

  state.sidebarSwipe = side
    ? {
        handled: false,
        side,
        startPoint: normalizedPointFromClient(touch.clientX, touch.clientY),
        startX: touch.clientX,
        startY: touch.clientY,
      }
    : null;
}

function updateSidebarSwipe(touch) {
  const swipe = state.sidebarSwipe;
  if (!swipe) {
    return "inactive";
  }
  if (swipe.handled) {
    return "handled";
  }

  const dx = touch.clientX - swipe.startX;
  const dy = touch.clientY - swipe.startY;
  const absDx = Math.abs(dx);
  const absDy = Math.abs(dy);

  if (absDy > 18 && absDy > absDx) {
    state.sidebarSwipe = null;
    state.lastTouch = { x: touch.clientX, y: touch.clientY, ts: Date.now() };
    return "inactive";
  }

  if (absDx < 8 && absDy < 8) {
    return "inactive";
  }

  const isHorizontal = absDx >= absDy * SIDEBAR_SWIPE_DIRECTION_RATIO;
  const direction = dx > 0 ? "right" : "left";
  if (!isHorizontal) {
    return "inactive";
  }

  state.touchMoved = true;
  state.ignoreNextClick = true;

  if (absDx < SIDEBAR_SWIPE_MIN_DISTANCE_PX || absDy > SIDEBAR_SWIPE_MAX_VERTICAL_PX) {
    return "tracking";
  }

  swipe.handled = true;
  state.lastTouch = { x: touch.clientX, y: touch.clientY, ts: Date.now() };
  cancelPendingScroll();
  send({
    direction,
    type: "sidebarSwipe",
    x: swipe.startPoint?.x,
    y: swipe.startPoint?.y,
  });
  return "handled";
}

function sidebarSwipeEdgeWidth(rect) {
  return clamp(rect.width * SIDEBAR_SWIPE_EDGE_RATIO, SIDEBAR_SWIPE_EDGE_MIN_PX, SIDEBAR_SWIPE_EDGE_MAX_PX);
}

function sidebarSwipeCloseRegionWidth(rect) {
  const width = clamp(
    rect.width * SIDEBAR_SWIPE_CLOSE_REGION_RATIO,
    SIDEBAR_SWIPE_CLOSE_REGION_MIN_PX,
    SIDEBAR_SWIPE_CLOSE_REGION_MAX_PX,
  );
  return Math.min(width, rect.width * 0.45);
}

function sidebarSwipeRightOpenRegionWidth(rect) {
  const width = clamp(
    rect.width * SIDEBAR_SWIPE_RIGHT_OPEN_REGION_RATIO,
    SIDEBAR_SWIPE_RIGHT_OPEN_REGION_MIN_PX,
    SIDEBAR_SWIPE_RIGHT_OPEN_REGION_MAX_PX,
  );
  return Math.min(width, rect.width * 0.35);
}

function cancelPendingScroll() {
  state.pendingScroll = null;
  if (state.scrollTimer) {
    clearTimeout(state.scrollTimer);
    state.scrollTimer = null;
  }
}

function send(payload) {
  if (!state.controlSocket || state.controlSocket.readyState !== TRANSPORT_OPEN) {
    return false;
  }

  state.controlSocket.send(JSON.stringify(payload));
  return true;
}

function queueViewportUpdate() {
  if (state.viewportTimer) {
    clearTimeout(state.viewportTimer);
  }

  state.viewportTimer = setTimeout(() => {
    state.viewportTimer = null;
    sendViewport();
  }, VIEWPORT_SEND_DEBOUNCE_MS);
}

function queuePageZoomUpdate() {
  if (state.pageZoomTimer) {
    clearTimeout(state.pageZoomTimer);
  }

  state.pageZoomTimer = setTimeout(() => {
    state.pageZoomTimer = null;
    sendPageZoom();
  }, PAGE_ZOOM_SEND_DEBOUNCE_MS);
}

function sendPageZoom({ force = false } = {}) {
  const scale = state.pageZoomScale;
  const key = pageZoomKey(scale);
  if (!force && key === state.lastSentPageZoomKey) {
    return;
  }

  const sent = send({
    scale,
    type: "pageZoom",
  });
  if (sent) {
    state.lastSentPageZoomKey = key;
  }
}

function initializePageZoomControl() {
  const storedScale = readStoredPageZoomScale();
  state.hasStoredPageZoom = storedScale !== null;
  setPageZoomScale(storedScale ?? MIN_PAGE_ZOOM_SCALE);
}

function togglePageZoomMenu() {
  if (state.pageZoomMenuOpen) {
    closePageZoomMenu();
    return;
  }

  openPageZoomMenu();
}

function openPageZoomMenu() {
  state.pageZoomMenuOpen = true;
  pageZoomMenu.hidden = false;
  pageZoomButton.setAttribute("aria-expanded", "true");
}

function closePageZoomMenu() {
  state.pageZoomMenuOpen = false;
  pageZoomMenu.hidden = true;
  pageZoomButton.setAttribute("aria-expanded", "false");
}

function setPageZoomScale(scale, { remember = false } = {}) {
  const normalizedScale = normalizePageZoomScale(scale);
  state.pageZoomScale = normalizedScale;
  pageZoomSlider.value = formatPageZoomScale(normalizedScale);
  pageZoomLabel.textContent = `${formatPageZoomScale(normalizedScale)}x`;
  if (remember) {
    rememberPageZoomScale(normalizedScale);
  }
}

function rememberPageZoomScale(scale) {
  state.hasStoredPageZoom = true;
  try {
    localStorage.setItem(PAGE_ZOOM_STORAGE_KEY, pageZoomKey(scale));
  } catch {
    // Storage can be blocked in private browsing modes.
  }
}

function readStoredPageZoomScale() {
  try {
    const value = localStorage.getItem(PAGE_ZOOM_STORAGE_KEY);
    if (!value) {
      return null;
    }

    const scale = Number(value);
    return Number.isFinite(scale) ? normalizePageZoomScale(scale) : null;
  } catch {
    return null;
  }
}

function normalizePageZoomScale(scale) {
  return Math.round(clamp(Number(scale) || MIN_PAGE_ZOOM_SCALE, MIN_PAGE_ZOOM_SCALE, MAX_PAGE_ZOOM_SCALE) * 100) / 100;
}

function formatPageZoomScale(scale) {
  return Number.isInteger(scale) ? String(scale) : scale.toFixed(2).replace(/0$/, "");
}

function pageZoomKey(scale) {
  return normalizePageZoomScale(scale).toFixed(2);
}

function sendViewport() {
  const rect = screenWrap.getBoundingClientRect();
  const width = Math.round(rect.width);
  const height = Math.round(rect.height);
  if (width <= 0 || height <= 0) {
    return;
  }

  const dpr = Math.round((window.devicePixelRatio || 1) * 100) / 100;
  const key = `${width}x${height}@${dpr}`;
  if (key === state.lastSentViewportKey) {
    return;
  }

  const sent = send({
    type: "viewport",
    dpr,
    height,
    width,
  });
  if (sent) {
    state.lastSentViewportKey = key;
  }
}

function flushPointerMove() {
  const point = state.latestPointer;
  state.latestPointer = null;
  if (!point) {
    return;
  }

  send({
    type: "pointerMove",
    ...point,
    ts: Date.now(),
  });
}

function queueScroll(point, deltaX, deltaY) {
  if (!state.pendingScroll) {
    state.pendingScroll = { deltaX: 0, deltaY: 0, x: point.x, y: point.y };
  }

  state.pendingScroll.deltaX += deltaX;
  state.pendingScroll.deltaY += deltaY;
  state.pendingScroll.x = point.x;
  state.pendingScroll.y = point.y;

  if (state.scrollTimer) {
    return;
  }

  state.scrollTimer = setTimeout(flushScroll, SCROLL_SEND_INTERVAL_MS);
}

function flushScroll() {
  const payload = state.pendingScroll;
  state.pendingScroll = null;
  state.scrollTimer = null;

  if (!payload) {
    return;
  }

  send({
    type: "scroll",
    deltaX: payload.deltaX,
    deltaY: payload.deltaY,
    x: payload.x,
    y: payload.y,
  });
}

function focusKeyboardProxy() {
  resetKeyboardProxy();
  try {
    keyboardProxy.focus({ preventScroll: true });
  } catch {
    keyboardProxy.focus();
  }
}

function blurKeyboardProxy() {
  resetKeyboardProxy();
  clearProvisionalText();
  keyboardProxy.blur();
}

function flushKeyboardProxyValue() {
  const text = keyboardProxy.value;
  if (text) {
    if (!isRecentCompositionText(text)) {
      sendProvisionalText(text);
    }
  }
  resetKeyboardProxy();
}

function resetKeyboardProxy() {
  keyboardProxy.value = "";
}

function ignoreNextInputBriefly() {
  state.ignoreNextInput = true;
  setTimeout(() => {
    state.ignoreNextInput = false;
  }, 120);
}

function sendText(text) {
  const now = Date.now();
  if (state.lastSentText === text && now - state.lastSentTextAt < 120) {
    return;
  }

  state.lastSentText = text;
  state.lastSentTextAt = now;
  send({ type: "text", text });
}

function sendProvisionalText(text) {
  sendText(text);
  updateProvisionalForText(text);
}

function sendCommittedText(text) {
  const provisional = currentProvisionalText();
  if (provisional) {
    if (text !== provisional) {
      sendBackspaces(provisional.length);
      sendText(text);
    }
    clearProvisionalText();
    return;
  }

  sendText(text);
}

function sendBackspaces(count) {
  for (let index = 0; index < count; index += 1) {
    send({ type: "key", key: "Backspace" });
  }
}

function updateProvisionalForText(text) {
  if (isProvisionalText(text)) {
    state.provisionalText += text;
    state.provisionalTextAt = Date.now();
    return;
  }

  clearProvisionalText();
}

function updateProvisionalForKey(key) {
  if (key === "Backspace" && state.provisionalText) {
    state.provisionalText = state.provisionalText.slice(0, -1);
    state.provisionalTextAt = Date.now();
    return;
  }

  clearProvisionalText();
}

function currentProvisionalText() {
  if (!state.provisionalText || Date.now() - state.provisionalTextAt > 8000) {
    clearProvisionalText();
    return "";
  }

  return state.provisionalText;
}

function clearProvisionalText() {
  state.provisionalText = "";
  state.provisionalTextAt = 0;
}

function isProvisionalText(text) {
  return /^[A-Za-z0-9'_’-]+$/.test(text);
}

function isRecentCompositionText(text) {
  return state.lastCompositionText === text && Date.now() - state.lastCompositionTs < 500;
}

function committedTextFromBeforeInput(event) {
  if (event.inputType === "insertReplacementText" || event.inputType === "insertFromComposition") {
    return event.data || "";
  }

  return "";
}

function textFromBeforeInput(event) {
  if (event.inputType === "insertText" && event.data) {
    return event.data;
  }

  if (event.inputType === "insertFromPaste") {
    return event.dataTransfer?.getData("text/plain") || event.data || "";
  }

  if (event.inputType === "insertLineBreak" || event.inputType === "insertParagraph") {
    return "";
  }

  return "";
}

function keyFromInputType(inputType) {
  if (inputType === "deleteContentBackward") {
    return "Backspace";
  }

  if (inputType === "deleteContentForward") {
    return "Delete";
  }

  if (inputType === "insertLineBreak" || inputType === "insertParagraph") {
    return "Enter";
  }

  return "";
}

function normalizedPoint(event) {
  return normalizedPointFromClient(event.clientX, event.clientY);
}

function normalizedPointFromClient(clientX, clientY) {
  const rect = screen.getBoundingClientRect();
  if (!rect.width || !rect.height || screen.hidden) {
    return null;
  }

  return {
    x: clamp((clientX - rect.left) / rect.width, 0, 1),
    y: clamp((clientY - rect.top) / rect.height, 0, 1),
  };
}

function isEditablePoint(point) {
  const padding = 0.015;
  return state.editableRects.some((rect) => {
    const left = Number(rect.x) - padding;
    const top = Number(rect.y) - padding;
    const right = Number(rect.x) + Number(rect.width) + padding;
    const bottom = Number(rect.y) + Number(rect.height) + padding;
    return point.x >= left && point.x <= right && point.y >= top && point.y <= bottom;
  });
}

function setStatus(text) {
  if (state.statusText === text) {
    return;
  }

  state.statusText = text;
  if (subtitle) {
    subtitle.textContent = text;
  }
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
