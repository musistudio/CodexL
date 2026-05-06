const urlParams = new URLSearchParams(location.search);
const token = urlParams.get("token") || "";
const remoteRoom = urlParams.get("room") || "default";
const wsBasePath = websocketBasePath();
const MAX_SCREEN_ZOOM = 4;
const MIN_SCREEN_ZOOM = 1;
const POINTER_MOVE_SEND_INTERVAL_MS = 33;
const SCROLL_SEND_INTERVAL_MS = 45;
const VIEWPORT_SEND_DEBOUNCE_MS = 120;
const PAGE_ZOOM_SEND_DEBOUNCE_MS = 120;
const PAGE_ZOOM_STORAGE_KEY = "codex-app-remotely.pageZoomScale";
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
  viewportTimer: null,
  zoom: 1,
};

const screenWrap = document.querySelector("#screenWrap");
const screen = document.querySelector("#screen");
const screenContext = screen.getContext("2d", { alpha: false });
const emptyState = document.querySelector("#emptyState");
const keyboardProxy = document.querySelector("#keyboardProxy");
const pageZoomButton = document.querySelector("#pageZoomButton");
const pageZoomControl = document.querySelector("#pageZoomControl");
const pageZoomLabel = document.querySelector("#pageZoomLabel");
const pageZoomMenu = document.querySelector("#pageZoomMenu");
const pageZoomSlider = document.querySelector("#pageZoomSlider");
const qualitySelect = document.querySelector("#qualitySelect");
const subtitle = document.querySelector("#subtitle");

document.querySelector("#refreshButton").addEventListener("click", () => {
  send({ type: "refresh" });
});

qualitySelect.addEventListener("change", () => {
  send({ type: "profileMode", mode: qualitySelect.value });
});

pageZoomButton.addEventListener("click", () => {
  togglePageZoomMenu();
});

pageZoomSlider.addEventListener("input", () => {
  setPageZoomScale(pageZoomSlider.value, { remember: true });
  queuePageZoomUpdate();
});

pageZoomSlider.addEventListener("change", () => {
  setPageZoomScale(pageZoomSlider.value, { remember: true });
  sendPageZoom({ force: true });
});

document.addEventListener("pointerdown", (event) => {
  if (state.pageZoomMenuOpen && !pageZoomControl.contains(event.target)) {
    closePageZoomMenu();
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && state.pageZoomMenuOpen) {
    closePageZoomMenu();
  }
});

if (window.ResizeObserver) {
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

screenWrap.addEventListener("click", (event) => {
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

screenWrap.addEventListener(
  "mousemove",
  (event) => {
    const point = normalizedPoint(event);
    if (point) {
      state.latestPointer = point;
    }
  },
  { passive: true },
);

keyboardProxy.addEventListener("keydown", (event) => {
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

keyboardProxy.addEventListener("beforeinput", (event) => {
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

keyboardProxy.addEventListener("input", () => {
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

keyboardProxy.addEventListener("compositionstart", () => {
  state.composing = true;
});

keyboardProxy.addEventListener("compositionend", (event) => {
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

screenWrap.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    const point = normalizedPoint(event) || { x: 0.5, y: 0.5 };
    queueScroll(point, event.deltaX, event.deltaY);
  },
  { passive: false },
);

screenWrap.addEventListener(
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

screenWrap.addEventListener(
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

screenWrap.addEventListener(
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

screenWrap.addEventListener(
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

initializePageZoomControl();
connectControl();
connectFrame();
setInterval(flushPointerMove, POINTER_MOVE_SEND_INTERVAL_MS);
requestAnimationFrame(renderFrameLoop);

function connectControl() {
  if (!token) {
    setStatus("Missing token");
    return;
  }

  const socket = new WebSocket(wsUrl("/ws/control"));
  state.controlSocket = socket;
  setStatus("Connecting");

  socket.addEventListener("open", () => {
    state.connected = true;
    state.lastSentViewportKey = "";
    state.lastSentPageZoomKey = "";
    setStatus("Connected");
    sendViewport();
    sendPageZoom({ force: true });
    send({ type: "refresh" });
  });

  socket.addEventListener("message", (event) => {
    if (typeof event.data === "string") {
      handleControlMessage(JSON.parse(event.data));
    }
  });

  socket.addEventListener("close", () => {
    if (state.controlSocket !== socket) {
      return;
    }

    state.connected = false;
    state.lastSentViewportKey = "";
    state.lastSentPageZoomKey = "";
    setStatus("Control disconnected, retrying");
    setTimeout(connectControl, 1000);
  });

  socket.addEventListener("error", () => {
    socket.close();
  });
}

function connectFrame() {
  if (!token) {
    return;
  }

  const socket = new WebSocket(wsUrl("/ws/frame"));
  socket.binaryType = "arraybuffer";
  state.frameSocket = socket;

  socket.addEventListener("open", () => {
    state.frameConnected = true;
  });

  socket.addEventListener("message", (event) => {
    if (typeof event.data !== "string") {
      state.latestFrame = event.data;
    }
  });

  socket.addEventListener("close", () => {
    if (state.frameSocket !== socket) {
      return;
    }

    state.frameConnected = false;
    state.latestFrame = null;
    setTimeout(connectFrame, 1000);
  });

  socket.addEventListener("error", () => {
    socket.close();
  });
}

function wsUrl(pathname) {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const params = new URLSearchParams({ token });
  if (remoteRoom) {
    params.set("room", remoteRoom);
  }
  return `${protocol}//${location.host}${wsBasePath}${pathname}?${params.toString()}`;
}

function websocketBasePath() {
  if (location.pathname === "/" || location.pathname === "/index.html") {
    return "";
  }

  if (location.pathname.endsWith("/index.html")) {
    return location.pathname.slice(0, -"/index.html".length);
  }

  return location.pathname.endsWith("/") ? location.pathname.slice(0, -1) : "";
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
    setStatus(message.status?.connected ? "CDP connected" : "Waiting for CDP");
    return;
  }

  if (message.type === "warning") {
    setStatus(message.message);
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
    setStatus("Frame decode failed");
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
  send({ direction, type: "sidebarSwipe" });
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
  if (!state.controlSocket || state.controlSocket.readyState !== WebSocket.OPEN) {
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
  subtitle.textContent = text;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
