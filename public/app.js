const token = new URLSearchParams(location.search).get("token") || "";
const MAX_SCREEN_ZOOM = 4;
const MIN_SCREEN_ZOOM = 1;
const SCROLL_SEND_INTERVAL_MS = 45;
const textDecoder = new TextDecoder();

const state = {
  connected: false,
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
  frameDrawInProgress: false,
  frameDrawScheduled: false,
  lastTouch: null,
  pendingScroll: null,
  pendingFrame: null,
  pinch: null,
  scrollTimer: null,
  socket: null,
  screenBaseSize: null,
  screenPanX: 0,
  screenPanY: 0,
  screenSize: null,
  statusText: "",
  touchMoved: false,
  zoom: 1,
};

const screenWrap = document.querySelector("#screenWrap");
const screen = document.querySelector("#screen");
const screenContext = screen.getContext("2d", { alpha: false });
const emptyState = document.querySelector("#emptyState");
const keyboardProxy = document.querySelector("#keyboardProxy");
const subtitle = document.querySelector("#subtitle");

document.querySelector("#refreshButton").addEventListener("click", () => {
  send({ type: "refresh" });
});

if (window.ResizeObserver) {
  const screenResizeObserver = new ResizeObserver(() => {
    fitScreenToWrap();
  });
  screenResizeObserver.observe(screenWrap);
}
window.addEventListener("resize", fitScreenToWrap);
window.visualViewport?.addEventListener("resize", fitScreenToWrap);
window.addEventListener("orientationchange", fitScreenToWrap);

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
      event.preventDefault();
      return;
    }

    state.lastTouch = null;
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
  },
  { passive: true },
);

connect();

function connect() {
  if (!token) {
    setStatus("Missing token");
    return;
  }

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${protocol}//${location.host}/ws?token=${encodeURIComponent(token)}`);
  socket.binaryType = "arraybuffer";
  state.socket = socket;
  setStatus("Connecting");

  socket.addEventListener("open", () => {
    state.connected = true;
    setStatus("Connected");
    send({ type: "refresh" });
  });

  socket.addEventListener("message", (event) => {
    handleSocketMessage(event.data);
  });

  socket.addEventListener("close", () => {
    state.connected = false;
    setStatus("Disconnected, retrying");
    setTimeout(connect, 1000);
  });

  socket.addEventListener("error", () => {
    socket.close();
  });
}

function handleSocketMessage(data) {
  if (typeof data === "string") {
    handleMessage(JSON.parse(data));
    return;
  }

  handleMessage(parseBinaryFrame(data));
}

function handleMessage(message) {
  if (message.type === "screenshot") {
    scheduleScreenshot(message);
    return;
  }

  if (message.type === "status") {
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
    return;
  }
}

function parseBinaryFrame(buffer) {
  const bytes = new Uint8Array(buffer);
  const view = new DataView(buffer);
  const headerLength = view.getUint32(0);
  const headerStart = 4;
  const headerEnd = headerStart + headerLength;
  const header = JSON.parse(textDecoder.decode(bytes.subarray(headerStart, headerEnd)));
  const image = bytes.subarray(headerEnd);
  const blob = new Blob([image], { type: `image/${header.format || "jpeg"}` });
  return {
    ...header,
    blob,
  };
}

function scheduleScreenshot(message) {
  if (Array.isArray(message.editableRects)) {
    state.editableRects = message.editableRects;
  }

  cleanupFrame(state.pendingFrame);
  state.pendingFrame = message;
  if (state.frameDrawScheduled || state.frameDrawInProgress) {
    return;
  }

  state.frameDrawScheduled = true;
  requestAnimationFrame(drawLatestFrame);
}

async function drawLatestFrame() {
  state.frameDrawScheduled = false;
  const frame = state.pendingFrame;
  state.pendingFrame = null;
  if (!frame) {
    return;
  }

  state.frameDrawInProgress = true;
  let image = null;
  try {
    image = await decodeFrame(frame.blob || frame.blobUrl || frame.dataUrl);
    if (state.pendingFrame) {
      return;
    }

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
    const title = frame.target?.title || "Codex";
    setStatus(`${title} · ${frame.metrics?.width || 0}x${frame.metrics?.height || 0}`);
  } catch {
    setStatus("Frame decode failed");
  } finally {
    cleanupDecodedImage(image);
    cleanupFrame(frame);
    state.frameDrawInProgress = false;
    if (state.pendingFrame && !state.frameDrawScheduled) {
      state.frameDrawScheduled = true;
      requestAnimationFrame(drawLatestFrame);
    }
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

function cleanupFrame(frame) {
  if (frame?.blobUrl) {
    URL.revokeObjectURL(frame.blobUrl);
  }
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

function cancelPendingScroll() {
  state.pendingScroll = null;
  if (state.scrollTimer) {
    clearTimeout(state.scrollTimer);
    state.scrollTimer = null;
  }
}

function send(payload) {
  if (!state.socket || state.socket.readyState !== WebSocket.OPEN) {
    return;
  }

  state.socket.send(JSON.stringify(payload));
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
