import { EventEmitter } from "node:events";

const CONNECT_RETRY_MS = 1200;
const COMMAND_TIMEOUT_MS = 7000;
const EDITABLE_RECTS_CACHE_MS = 800;
const METRICS_CACHE_MS = 3000;

export class CdpBridge extends EventEmitter {
  constructor(config, logger = console) {
    super();
    this.config = config;
    this.logger = logger;
    this.connected = false;
    this.connecting = false;
    this.currentTarget = null;
    this.editableRectsPromise = null;
    this.lastEditableRects = [];
    this.lastEditableRectsAt = 0;
    this.lastMetrics = null;
    this.lastMetricsAt = 0;
    this.lastScreenshotAt = 0;
    this.lastScreencastAckAt = 0;
    this.pending = new Map();
    this.pendingScreencastAck = null;
    this.requestId = 1;
    this.retryTimer = null;
    this.screencastActive = false;
    this.screencastAckTimer = null;
    this.screencastStartPromise = null;
    this.streamingEnabled = false;
    this.warningState = { key: "", ts: 0 };
    this.ws = null;
  }

  start() {
    this.scheduleConnect(0);
  }

  stop() {
    clearTimeout(this.retryTimer);
    this.closeSocket();
  }

  status() {
    return {
      cdpUrl: `http://${this.config.cdpHost}:${this.config.cdpPort}`,
      connected: this.connected,
      screencastActive: this.screencastActive,
      streamingEnabled: this.streamingEnabled,
      target: this.currentTarget,
    };
  }

  async listTargets() {
    const targets = await fetchJson(this.cdpUrl("/json/list"));
    return targets.map(normalizeTarget);
  }

  async switchTarget(targetId) {
    const targets = await this.listTargets();
    const target = targets.find((candidate) => candidate.id === targetId);
    if (!target) {
      throw new Error(`CDP target not found: ${targetId}`);
    }

    this.connectToTarget(target);
  }

  async click(normalizedX, normalizedY) {
    const point = await this.pointFromNormalized(normalizedX, normalizedY);
    await this.send("Input.dispatchMouseEvent", {
      button: "left",
      clickCount: 1,
      type: "mousePressed",
      x: point.x,
      y: point.y,
    });
    await this.send("Input.dispatchMouseEvent", {
      button: "left",
      clickCount: 1,
      type: "mouseReleased",
      x: point.x,
      y: point.y,
    });
  }

  async clickAndCheckEditable(normalizedX, normalizedY) {
    let editableAtPoint = false;
    try {
      editableAtPoint = await this.isEditableAt(normalizedX, normalizedY);
    } catch (error) {
      this.emitWarning(`editable probe failed: ${error.message}`);
    }

    await this.click(normalizedX, normalizedY);

    let editableFocused = false;
    try {
      editableFocused = await this.hasEditableFocus();
    } catch (error) {
      this.emitWarning(`editable focus probe failed: ${error.message}`);
    }

    return editableAtPoint || editableFocused;
  }

  async scroll(normalizedX, normalizedY, deltaY, deltaX = 0) {
    const point = await this.pointFromNormalized(normalizedX, normalizedY);
    await this.send("Input.dispatchMouseEvent", {
      deltaX,
      deltaY,
      type: "mouseWheel",
      x: point.x,
      y: point.y,
    });
  }

  async insertText(text) {
    if (!text) {
      return;
    }

    await this.send("Input.insertText", { text });
  }

  async key(key) {
    const event = keyEventFor(key);
    await this.send("Input.dispatchKeyEvent", { ...event, type: "keyDown" });
    await this.send("Input.dispatchKeyEvent", { ...event, type: "keyUp" });
  }

  async isEditableAt(normalizedX, normalizedY) {
    const point = await this.pointFromNormalized(normalizedX, normalizedY);
    const result = await this.send("Runtime.evaluate", {
      expression: editableProbeExpression(point.x, point.y),
      returnByValue: true,
    });
    return Boolean(result.result?.value);
  }

  async hasEditableFocus() {
    const result = await this.send("Runtime.evaluate", {
      expression: editableFocusExpression(),
      awaitPromise: true,
      returnByValue: true,
    });
    return Boolean(result.result?.value);
  }

  async getEditableRects({ useCache = true } = {}) {
    const now = Date.now();
    if (useCache && now - this.lastEditableRectsAt < EDITABLE_RECTS_CACHE_MS) {
      return this.lastEditableRects;
    }

    if (this.editableRectsPromise) {
      return this.editableRectsPromise;
    }

    this.editableRectsPromise = this.send("Runtime.evaluate", {
      expression: editableRectsExpression(),
      returnByValue: true,
    }).then((result) => {
      const rects = Array.isArray(result.result?.value) ? result.result.value : [];
      this.lastEditableRects = rects;
      this.lastEditableRectsAt = Date.now();
      return rects;
    }).catch((error) => {
      this.emitWarning(`editable rects failed: ${error.message}`);
      return this.lastEditableRects;
    }).finally(() => {
      this.editableRectsPromise = null;
    });

    return this.editableRectsPromise;
  }

  refreshEditableRects() {
    void this.getEditableRects({ useCache: true });
  }

  async captureAndBroadcast() {
    if (!this.connected) {
      return;
    }

    try {
      const metrics = await this.getViewportMetrics();
      const screenshot = await this.send("Page.captureScreenshot", this.screenshotParams(metrics));
      this.lastScreenshotAt = Date.now();

      this.emit("screenshot", {
        data: screenshot.data,
        format: "jpeg",
        metrics,
        target: this.currentTarget,
        ts: Date.now(),
      });
    } catch (error) {
      this.emitWarning(`screenshot failed: ${error.message}`);
    }
  }

  async setScreencastEnabled(enabled) {
    this.streamingEnabled = Boolean(enabled);
    if (!this.connected) {
      return;
    }

    if (this.streamingEnabled) {
      await this.startScreencast();
      return;
    }

    await this.stopScreencast();
  }

  async scheduleConnect(delayMs = CONNECT_RETRY_MS) {
    if (this.connecting || this.connected) {
      return;
    }

    clearTimeout(this.retryTimer);
    this.retryTimer = setTimeout(() => {
      this.connect().catch((error) => {
        this.emitWarning(`CDP connect failed: ${error.message}`);
        this.scheduleConnect();
      });
    }, delayMs);
  }

  async connect() {
    if (this.connecting || this.connected) {
      return;
    }

    this.connecting = true;
    try {
      const targets = await this.listTargets();
      const target = selectTarget(targets);

      if (!target) {
        throw new Error("no page target with webSocketDebuggerUrl");
      }

      this.connectToTarget(target);
    } finally {
      this.connecting = false;
    }
  }

  connectToTarget(target) {
    this.closeSocket();
    this.connected = false;
    this.currentTarget = target;
    this.emit("status", this.status());
    this.logger.log(`[cdp] connecting to target: ${target.title || target.url || target.id}`);

    const ws = new WebSocket(target.webSocketDebuggerUrl);
    this.ws = ws;

    ws.addEventListener("open", async () => {
      this.connected = true;
      this.emit("status", this.status());
      this.logger.log("[cdp] connected");

      try {
        await Promise.all([
          this.send("Page.enable"),
          this.send("Runtime.enable"),
        ]);
        if (this.streamingEnabled) {
          await this.startScreencast();
        }
      } catch (error) {
        this.emitWarning(`CDP init failed: ${error.message}`);
      }
    });

    ws.addEventListener("message", (event) => {
      this.handleSocketMessage(event.data);
    });

    ws.addEventListener("close", () => {
      this.handleDisconnect("closed");
    });

    ws.addEventListener("error", () => {
      this.handleDisconnect("socket error");
    });
  }

  async startScreencast() {
    if (this.screencastActive) {
      return;
    }

    if (this.screencastStartPromise) {
      await this.screencastStartPromise;
      return;
    }

    this.screencastStartPromise = this.send("Page.startScreencast", {
      everyNthFrame: this.config.screencastEveryNthFrame,
      format: "jpeg",
      maxHeight: this.config.screenshotMaxHeight,
      maxWidth: this.config.screenshotMaxWidth,
      quality: this.config.screenshotQuality,
    }).then(async () => {
      this.screencastActive = true;
      this.emit("status", this.status());
      this.logger.log("[cdp] screencast started");
      this.scheduleInitialScreenshotFallback();

      if (!this.streamingEnabled) {
        await this.stopScreencast();
      }
    }).finally(() => {
      this.screencastStartPromise = null;
    });

    await this.screencastStartPromise;
  }

  async stopScreencast() {
    if (this.screencastStartPromise && !this.screencastActive) {
      try {
        await this.screencastStartPromise;
      } catch {
        return;
      }
    }

    if (!this.screencastActive) {
      this.clearScreencastAck();
      return;
    }

    await this.flushScreencastAck();

    try {
      await this.send("Page.stopScreencast");
    } catch (error) {
      this.emitWarning(`screencast stop failed: ${error.message}`);
    } finally {
      this.screencastActive = false;
      this.clearScreencastAck();
      this.emit("status", this.status());
      this.logger.log("[cdp] screencast stopped");
    }
  }

  async restartScreencast() {
    if (!this.connected) {
      this.streamingEnabled = true;
      return;
    }

    this.streamingEnabled = true;
    if (this.screencastActive) {
      await this.stopScreencast();
    }

    await this.startScreencast();
  }

  async pointFromNormalized(normalizedX, normalizedY) {
    const metrics = await this.getViewportMetrics({ useCache: true });
    return {
      x: clamp(Number(normalizedX), 0, 1) * metrics.width,
      y: clamp(Number(normalizedY), 0, 1) * metrics.height,
    };
  }

  async getViewportMetrics({ useCache = false } = {}) {
    const now = Date.now();
    if (this.lastMetrics && (useCache || now - this.lastMetricsAt < METRICS_CACHE_MS)) {
      return this.lastMetrics;
    }

    try {
      const metrics = await this.send("Page.getLayoutMetrics");
      const viewport = metrics.cssVisualViewport || metrics.visualViewport || metrics.cssLayoutViewport || metrics.layoutViewport || {};
      const width = Number(viewport.clientWidth || viewport.width || 1);
      const height = Number(viewport.clientHeight || viewport.height || 1);
      const scale = Number(viewport.scale || 1);
      const x = Number(viewport.pageX ?? viewport.x ?? 0);
      const y = Number(viewport.pageY ?? viewport.y ?? 0);

      this.lastMetrics = {
        height: Math.max(1, height),
        scale,
        width: Math.max(1, width),
        x: Number.isFinite(x) ? x : 0,
        y: Number.isFinite(y) ? y : 0,
      };
      this.lastMetricsAt = Date.now();
      return this.lastMetrics;
    } catch (error) {
      if (this.lastMetrics) {
        return this.lastMetrics;
      }

      throw error;
    }
  }

  screenshotParams(metrics) {
    const maxWidth = this.config.screenshotMaxWidth || metrics.width;
    const maxHeight = this.config.screenshotMaxHeight || metrics.height;
    const imageScale = Math.min(1, maxWidth / metrics.width, maxHeight / metrics.height);
    const params = {
      captureBeyondViewport: false,
      format: "jpeg",
      quality: this.config.screenshotQuality,
    };

    if (imageScale < 0.999) {
      params.clip = {
        height: metrics.height,
        scale: imageScale,
        width: metrics.width,
        x: metrics.x || 0,
        y: metrics.y || 0,
      };
    }

    return params;
  }

  send(method, params = {}) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("CDP socket is not connected"));
    }

    const id = this.requestId;
    this.requestId += 1;

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP command timed out: ${method}`));
      }, COMMAND_TIMEOUT_MS);

      this.pending.set(id, { method, reject, resolve, timeout });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async handleSocketMessage(data) {
    const text = typeof data === "string" ? data : await data.text?.();
    if (!text) {
      return;
    }

    let payload;
    try {
      payload = JSON.parse(text);
    } catch {
      return;
    }

    if (!payload.id) {
      this.handleCdpEvent(payload);
      return;
    }

    const pending = this.pending.get(payload.id);
    if (!pending) {
      return;
    }

    clearTimeout(pending.timeout);
    this.pending.delete(payload.id);

    if (payload.error) {
      pending.reject(new Error(payload.error.message || pending.method));
      return;
    }

    pending.resolve(payload.result || {});
  }

  handleCdpEvent(payload) {
    if (payload.method === "Page.screencastFrame") {
      this.handleScreencastFrame(payload.params || {});
      return;
    }

    if (payload.method === "Page.screencastVisibilityChanged") {
      this.emit("status", {
        ...this.status(),
        visible: payload.params?.visible !== false,
      });
      return;
    }

    this.emit("event", payload);
  }

  handleScreencastFrame({ data, metadata = {}, sessionId }) {
    if (sessionId !== undefined) {
      this.scheduleScreencastAck(sessionId, { immediate: !this.streamingEnabled });
    }

    if (!data || !this.streamingEnabled) {
      return;
    }

    const metrics = metricsFromScreencastMetadata(metadata, this.lastMetrics);
    this.lastMetrics = metrics;
    this.lastMetricsAt = Date.now();
    this.lastScreenshotAt = Date.now();
    const editableRects = this.lastEditableRects;
    this.refreshEditableRects();

    this.emit("screenshot", {
      data,
      editableRects,
      format: "jpeg",
      metadata,
      metrics,
      target: this.currentTarget,
      ts: Date.now(),
    });
  }

  handleDisconnect(reason) {
    if (!this.ws) {
      return;
    }

    this.logger.warn(`[cdp] disconnected: ${reason}`);
    this.closeSocket();
    this.connected = false;
    this.emit("status", this.status());
    this.scheduleConnect();
  }

  emitWarning(message) {
    const now = Date.now();
    if (this.warningState.key === message && now - this.warningState.ts < 10000) {
      return;
    }

    this.warningState = { key: message, ts: now };
    this.emit("warning", message);
  }

  closeSocket() {
    this.screencastActive = false;
    this.clearScreencastAck();
    this.editableRectsPromise = null;
    this.lastEditableRects = [];
    this.lastEditableRectsAt = 0;
    this.lastMetrics = null;
    this.lastMetricsAt = 0;
    this.lastScreenshotAt = 0;

    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new Error("CDP socket closed"));
    }
    this.pending.clear();

    if (this.ws) {
      const socket = this.ws;
      this.ws = null;
      try {
        socket.close();
      } catch {
        // Ignore close races.
      }
    }
  }

  scheduleScreencastAck(sessionId, { immediate = false } = {}) {
    if (this.pendingScreencastAck !== null && this.pendingScreencastAck !== sessionId) {
      void this.sendScreencastAck(this.pendingScreencastAck);
    }

    this.pendingScreencastAck = sessionId;
    clearTimeout(this.screencastAckTimer);
    this.screencastAckTimer = null;

    const maxFps = Math.max(1, Number(this.config.screencastMaxFps || 1));
    const minInterval = immediate ? 0 : Math.floor(1000 / maxFps);
    const elapsed = Date.now() - this.lastScreencastAckAt;
    const delay = Math.max(0, minInterval - elapsed);

    if (delay === 0) {
      void this.flushScreencastAck();
      return;
    }

    this.screencastAckTimer = setTimeout(() => {
      this.screencastAckTimer = null;
      void this.flushScreencastAck();
    }, delay);
    this.screencastAckTimer.unref?.();
  }

  async flushScreencastAck() {
    clearTimeout(this.screencastAckTimer);
    this.screencastAckTimer = null;

    const sessionId = this.pendingScreencastAck;
    this.pendingScreencastAck = null;
    if (sessionId === null) {
      return;
    }

    await this.sendScreencastAck(sessionId);
  }

  async sendScreencastAck(sessionId) {
    this.lastScreencastAckAt = Date.now();
    try {
      await this.send("Page.screencastFrameAck", { sessionId });
    } catch (error) {
      this.emitWarning(`screencast ack failed: ${error.message}`);
    }
  }

  clearScreencastAck() {
    clearTimeout(this.screencastAckTimer);
    this.screencastAckTimer = null;
    this.pendingScreencastAck = null;
  }

  scheduleInitialScreenshotFallback() {
    const startedAt = Date.now();
    setTimeout(() => {
      if (!this.connected || !this.streamingEnabled || !this.screencastActive) {
        return;
      }

      if (this.lastScreenshotAt >= startedAt) {
        return;
      }

      void this.captureAndBroadcast();
    }, 700).unref?.();
  }

  cdpUrl(pathname) {
    return `http://${this.config.cdpHost}:${this.config.cdpPort}${pathname}`;
  }
}

function selectTarget(targets) {
  const pageTargets = targets.filter((target) => target.webSocketDebuggerUrl && target.type === "page");
  return pageTargets.find((target) => /codex/i.test(`${target.title} ${target.url}`)) || pageTargets[0] || targets.find((target) => target.webSocketDebuggerUrl) || null;
}

function normalizeTarget(target) {
  return {
    description: target.description || "",
    id: target.id,
    title: target.title || "",
    type: target.type || "",
    url: target.url || "",
    webSocketDebuggerUrl: target.webSocketDebuggerUrl || "",
  };
}

function editableProbeExpression(x, y) {
  return `(() => {
    ${editableHelpers()}
    let element = document.elementFromPoint(${JSON.stringify(x)}, ${JSON.stringify(y)});
    while (element && element.shadowRoot) {
      const nested = element.shadowRoot.elementFromPoint(${JSON.stringify(x)}, ${JSON.stringify(y)});
      if (!nested || nested === element) {
        break;
      }
      element = nested;
    }
    return closestEditable(element);
  })()`;
}

function editableFocusExpression() {
  return `(() => {
    ${editableHelpers()}
    let active = document.activeElement;
    while (active && active.shadowRoot && active.shadowRoot.activeElement) {
      active = active.shadowRoot.activeElement;
    }
    return closestEditable(active);
  })()`;
}

function editableRectsExpression() {
  return `(() => {
    ${editableHelpers()}
    const roots = [document];
    const selector = "input, textarea, [contenteditable], [role='combobox'], [role='searchbox'], [role='textbox']";
    const viewportWidth = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
    const viewportHeight = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
    const rects = [];
    const seen = new Set();

    function addElement(element) {
      if (seen.has(element) || !isEditableElement(element)) {
        return;
      }
      seen.add(element);

      const style = window.getComputedStyle(element);
      if (style.visibility === "hidden" || style.display === "none") {
        return;
      }

      const rect = element.getBoundingClientRect();
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(viewportWidth, rect.right);
      const bottom = Math.min(viewportHeight, rect.bottom);
      if (right <= left || bottom <= top) {
        return;
      }

      rects.push({
        x: left / viewportWidth,
        y: top / viewportHeight,
        width: (right - left) / viewportWidth,
        height: (bottom - top) / viewportHeight,
      });
    }

    for (let index = 0; index < roots.length; index += 1) {
      const root = roots[index];
      for (const element of root.querySelectorAll(selector)) {
        addElement(element);
        if (element.shadowRoot) {
          roots.push(element.shadowRoot);
        }
      }
      for (const element of root.querySelectorAll("*")) {
        if (element.shadowRoot) {
          roots.push(element.shadowRoot);
        }
      }
    }

    return rects.slice(0, 200);
  })()`;
}

function editableHelpers() {
  return `
    const nonEditableInputTypes = new Set(["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"]);
    const editableRoles = new Set(["combobox", "searchbox", "textbox"]);

    function isDisabledOrReadonly(element) {
      return element.disabled || element.readOnly || element.getAttribute("aria-disabled") === "true" || element.getAttribute("aria-readonly") === "true";
    }

    function isEditableElement(element) {
      if (!element || element.nodeType !== Node.ELEMENT_NODE) {
        return false;
      }

      const tagName = element.localName;
      if (tagName === "textarea") {
        return !isDisabledOrReadonly(element);
      }

      if (tagName === "input") {
        const type = (element.getAttribute("type") || "text").toLowerCase();
        return !nonEditableInputTypes.has(type) && !isDisabledOrReadonly(element);
      }

      if (element.isContentEditable) {
        return !isDisabledOrReadonly(element);
      }

      const role = (element.getAttribute("role") || "").toLowerCase();
      return editableRoles.has(role) && !isDisabledOrReadonly(element);
    }

    function composedParent(element) {
      if (!element) {
        return null;
      }

      if (element.parentElement) {
        return element.parentElement;
      }

      const root = element.getRootNode && element.getRootNode();
      return root && root.host ? root.host : null;
    }

    function closestEditable(element) {
      let current = element;
      while (current) {
        if (isEditableElement(current)) {
          return true;
        }
        current = composedParent(current);
      }
      return false;
    }
  `;
}

function metricsFromScreencastMetadata(metadata, fallback) {
  const width = Number(metadata.deviceWidth || fallback?.width || 1);
  const height = Number(metadata.deviceHeight || fallback?.height || 1);
  const pageScaleFactor = Number(metadata.pageScaleFactor || fallback?.pageScaleFactor || fallback?.scale || 1);
  const scale = Number(metadata.scale || fallback?.scale || 1);
  const x = Number(metadata.scrollOffsetX ?? fallback?.x ?? 0);
  const y = Number(metadata.scrollOffsetY ?? fallback?.y ?? 0);
  const offsetTop = Number(metadata.offsetTop || 0);

  return {
    height: Math.max(1, height),
    offsetTop: Number.isFinite(offsetTop) ? offsetTop : 0,
    pageScaleFactor: Number.isFinite(pageScaleFactor) ? pageScaleFactor : 1,
    scale: Number.isFinite(scale) ? scale : 1,
    width: Math.max(1, width),
    x: Number.isFinite(x) ? x : 0,
    y: Number.isFinite(y) ? y : 0,
  };
}

function keyEventFor(key) {
  const normalized = String(key || "");
  const special = {
    Backspace: { code: "Backspace", key: "Backspace", windowsVirtualKeyCode: 8 },
    Delete: { code: "Delete", key: "Delete", windowsVirtualKeyCode: 46 },
    Enter: { code: "Enter", key: "Enter", windowsVirtualKeyCode: 13 },
    Escape: { code: "Escape", key: "Escape", windowsVirtualKeyCode: 27 },
    Tab: { code: "Tab", key: "Tab", windowsVirtualKeyCode: 9 },
  };

  if (special[normalized]) {
    return special[normalized];
  }

  const text = normalized.length === 1 ? normalized : "";
  return {
    code: text ? `Key${text.toUpperCase()}` : normalized,
    key: normalized,
    text,
    unmodifiedText: text,
    windowsVirtualKeyCode: text ? text.toUpperCase().charCodeAt(0) : 0,
  };
}

async function fetchJson(url) {
  const response = await fetch(url, {
    signal: AbortSignal.timeout(2500),
  });

  if (!response.ok) {
    throw new Error(`${url} returned ${response.status}`);
  }

  return response.json();
}

function clamp(value, min, max) {
  if (!Number.isFinite(value)) {
    return min;
  }

  return Math.min(max, Math.max(min, value));
}
