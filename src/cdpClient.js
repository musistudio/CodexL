import { EventEmitter } from "node:events";

const CONNECT_RETRY_MS = 1200;
const COMMAND_TIMEOUT_MS = 7000;
const EDITABLE_RECTS_CACHE_MS = 800;
const METRICS_CACHE_MS = 3000;
const SCREENCAST_RESTART_DEBOUNCE_MS = 250;
const SCREENCAST_PROFILE_HYSTERESIS_MS = 4000;
const SCREENCAST_PROFILES = {
  good: {
    everyNthFrame: 2,
    maxHeight: 900,
    maxWidth: 1440,
    name: "good",
    quality: 74,
  },
  medium: {
    everyNthFrame: 2,
    maxHeight: 720,
    maxWidth: 1080,
    name: "medium",
    quality: 60,
  },
  bad: {
    everyNthFrame: 4,
    maxHeight: 480,
    maxWidth: 720,
    name: "bad",
    quality: 42,
  },
};

export class CdpBridge extends EventEmitter {
  constructor(config, logger = console) {
    super();
    this.activeScreencastProfileName = null;
    this.config = config;
    this.logger = logger;
    this.connected = false;
    this.connecting = false;
    this.currentTarget = null;
    this.clientViewport = null;
    this.desiredScreencastProfileName = "good";
    this.editableRectsPromise = null;
    this.lastEditableRects = [];
    this.lastEditableRectsAt = 0;
    this.lastMetrics = null;
    this.lastMetricsAt = 0;
    this.lastScreenshotAt = 0;
    this.lastViewportOverrideKey = "";
    this.pending = new Map();
    this.pendingScreencastProfileName = null;
    this.pendingScreencastProfileSince = 0;
    this.profileRestartPromise = null;
    this.requestId = 1;
    this.retryTimer = null;
    this.screencastActive = false;
    this.screencastProfileMode = "auto";
    this.screencastRestartTimer = null;
    this.screencastStartPromise = null;
    this.streamingEnabled = false;
    this.networkStats = {
      bufferedAmount: 0,
      droppedFramesInLast5s: 0,
      frameClientCount: 0,
      rtt: null,
    };
    this.warningState = { key: "", ts: 0 };
    this.ws = null;
  }

  start() {
    this.scheduleConnect(0);
  }

  stop() {
    clearTimeout(this.retryTimer);
    clearTimeout(this.screencastRestartTimer);
    this.screencastRestartTimer = null;
    this.closeSocket();
  }

  status() {
    return {
      cdpUrl: `http://${this.config.cdpHost}:${this.config.cdpPort}`,
      captureViewport: this.captureViewportSize(),
      clientViewport: this.clientViewport,
      connected: this.connected,
      network: this.networkStats,
      screencastActive: this.screencastActive,
      screencastProfile: this.activeScreencastProfileName || this.desiredScreencastProfileName,
      screencastProfileMode: this.screencastProfileMode,
      screencastProfileSettings: this.screencastProfile(),
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

  async pointerMove(normalizedX, normalizedY) {
    const point = await this.pointFromNormalized(normalizedX, normalizedY);
    await this.send("Input.dispatchMouseEvent", {
      button: "none",
      type: "mouseMoved",
      x: point.x,
      y: point.y,
    });
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
        bytes: Buffer.from(screenshot.data, "base64"),
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

    this.screencastStartPromise = (async () => {
      await this.applyClientViewportOverride();
      const profile = this.screencastProfile();
      await this.send("Page.startScreencast", {
        everyNthFrame: profile.everyNthFrame,
        format: profile.format,
        maxHeight: profile.maxHeight,
        maxWidth: profile.maxWidth,
        quality: profile.quality,
      });

      this.screencastActive = true;
      this.activeScreencastProfileName = profile.name;
      this.emit("profile", profile);
      this.emit("status", this.status());
      this.logger.log(`[cdp] screencast started (${profile.name}: ${profile.maxWidth}x${profile.maxHeight}, q${profile.quality}, every ${profile.everyNthFrame})`);
      this.scheduleInitialScreenshotFallback();

      if (!this.streamingEnabled) {
        await this.stopScreencast();
      }
    })().finally(() => {
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
      return;
    }

    try {
      await this.send("Page.stopScreencast");
    } catch (error) {
      this.emitWarning(`screencast stop failed: ${error.message}`);
    } finally {
      this.screencastActive = false;
      this.activeScreencastProfileName = null;
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

  updateNetworkStats(stats = {}) {
    const rtt = stats.rtt;
    this.networkStats = {
      bufferedAmount: Math.max(0, Number(stats.bufferedAmount || 0)),
      droppedFramesInLast5s: Math.max(0, Number(stats.droppedFramesInLast5s || 0)),
      frameClientCount: Math.max(0, Number(stats.frameClientCount || 0)),
      rtt: rtt !== null && rtt !== undefined && Number.isFinite(Number(rtt)) ? Math.max(0, Number(rtt)) : null,
    };
    this.maybeUpdateScreencastProfile();
  }

  maybeUpdateScreencastProfile() {
    const networkProfileName = profileNameForNetwork(this.networkStats);
    const profileName = profileNameForMode(this.screencastProfileMode, networkProfileName);
    const now = Date.now();

    if (profileName === this.desiredScreencastProfileName) {
      this.pendingScreencastProfileName = null;
      this.pendingScreencastProfileSince = 0;
      return;
    }

    if (profileName !== this.pendingScreencastProfileName) {
      this.pendingScreencastProfileName = profileName;
      this.pendingScreencastProfileSince = now;
      return;
    }

    if (now - this.pendingScreencastProfileSince < SCREENCAST_PROFILE_HYSTERESIS_MS) {
      return;
    }

    void this.switchScreencastProfile(profileName);
  }

  async setScreencastProfileMode(mode) {
    const normalizedMode = String(mode || "auto");
    if (normalizedMode !== "auto" && !SCREENCAST_PROFILES[normalizedMode]) {
      throw new Error(`unknown screencast profile mode: ${mode}`);
    }

    this.screencastProfileMode = normalizedMode;
    this.pendingScreencastProfileName = null;
    this.pendingScreencastProfileSince = 0;

    if (normalizedMode === "auto") {
      this.emit("status", this.status());
      this.maybeUpdateScreencastProfile();
      return;
    }

    await this.switchScreencastProfile(profileNameForMode(normalizedMode, profileNameForNetwork(this.networkStats)));
  }

  setClientViewport(viewport) {
    const nextViewport = normalizeClientViewport(viewport);
    if (!nextViewport) {
      return;
    }

    const currentProfile = this.screencastProfile();
    const currentCaptureViewport = this.captureViewportSize();
    if (sameClientViewport(this.clientViewport, nextViewport)) {
      return;
    }

    this.clientViewport = nextViewport;
    const nextProfile = this.screencastProfile();
    const nextCaptureViewport = this.captureViewportSize();
    this.emit("status", this.status());

    if (profileSizeChanged(currentProfile, nextProfile) || profileSizeChanged(currentCaptureViewport, nextCaptureViewport)) {
      this.scheduleScreencastRestart("viewport");
    }
  }

  async switchScreencastProfile(profileName) {
    if (!SCREENCAST_PROFILES[profileName]) {
      return;
    }

    if (profileName === this.desiredScreencastProfileName && profileName === this.activeScreencastProfileName) {
      this.emit("status", this.status());
      return;
    }

    this.desiredScreencastProfileName = profileName;
    this.pendingScreencastProfileName = null;
    this.pendingScreencastProfileSince = 0;

    if (this.profileRestartPromise) {
      this.emit("status", this.status());
      return;
    }

    if (!this.connected || !this.streamingEnabled || !this.screencastActive) {
      this.emit("status", this.status());
      return;
    }

    this.profileRestartPromise = (async () => {
      const profile = this.screencastProfile();
      this.logger.log(`[cdp] switching screencast profile to ${profile.name}`);
      await this.stopScreencast();
      if (this.connected && this.streamingEnabled) {
        await this.startScreencast();
      }
    })().catch((error) => {
      this.emitWarning(`screencast profile switch failed: ${error.message}`);
    }).finally(() => {
      this.profileRestartPromise = null;
    });

    await this.profileRestartPromise;
  }

  scheduleScreencastRestart(reason) {
    if (!this.connected || !this.streamingEnabled || !this.screencastActive) {
      return;
    }

    clearTimeout(this.screencastRestartTimer);
    this.screencastRestartTimer = setTimeout(() => {
      this.screencastRestartTimer = null;
      if (this.profileRestartPromise || this.screencastStartPromise || !this.connected || !this.streamingEnabled || !this.screencastActive) {
        return;
      }

      this.logger.log(`[cdp] restarting screencast for ${reason}`);
      void this.restartScreencast();
    }, SCREENCAST_RESTART_DEBOUNCE_MS);
    this.screencastRestartTimer.unref?.();
  }

  async applyClientViewportOverride() {
    const size = this.captureViewportSize();
    if (!this.connected || !size) {
      return;
    }

    const key = `${size.width}x${size.height}`;
    if (key === this.lastViewportOverrideKey) {
      return;
    }

    try {
      await this.send("Emulation.setDeviceMetricsOverride", {
        deviceScaleFactor: 1,
        height: size.height,
        mobile: false,
        screenHeight: size.height,
        screenOrientation: {
          angle: size.height >= size.width ? 0 : 90,
          type: size.height >= size.width ? "portraitPrimary" : "landscapePrimary",
        },
        screenWidth: size.width,
        width: size.width,
      });
      this.lastViewportOverrideKey = key;
      this.lastMetrics = null;
      this.lastMetricsAt = 0;
      this.logger.log(`[cdp] viewport override ${key}`);
    } catch (error) {
      this.emitWarning(`viewport override failed: ${error.message}`);
    }
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
    const profile = this.screencastProfile();
    const maxWidth = profile.maxWidth || metrics.width;
    const maxHeight = profile.maxHeight || metrics.height;
    const imageScale = Math.min(1, maxWidth / metrics.width, maxHeight / metrics.height);
    const params = {
      captureBeyondViewport: false,
      format: "jpeg",
      quality: profile.quality,
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
      void this.sendScreencastAck(sessionId);
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
      bytes: Buffer.from(data, "base64"),
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
    clearTimeout(this.screencastRestartTimer);
    this.screencastRestartTimer = null;
    this.screencastActive = false;
    this.activeScreencastProfileName = null;
    this.editableRectsPromise = null;
    this.lastEditableRects = [];
    this.lastEditableRectsAt = 0;
    this.lastMetrics = null;
    this.lastMetricsAt = 0;
    this.lastScreenshotAt = 0;
    this.lastViewportOverrideKey = "";

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

  async sendScreencastAck(sessionId) {
    try {
      await this.send("Page.screencastFrameAck", { sessionId });
    } catch (error) {
      this.emitWarning(`screencast ack failed: ${error.message}`);
    }
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

  captureViewportSize() {
    return captureViewportSizeForClient(this.config, this.clientViewport);
  }

  screencastProfile() {
    const base = SCREENCAST_PROFILES[this.desiredScreencastProfileName] || SCREENCAST_PROFILES.good;
    const size = screencastSizeForViewport(base, this.config, this.clientViewport);
    return {
      everyNthFrame: Math.max(base.everyNthFrame, Number(this.config.screencastEveryNthFrame || 1)),
      format: "jpeg",
      maxHeight: size.height,
      maxWidth: size.width,
      name: base.name,
      quality: Math.min(base.quality, clamp(Number(this.config.screenshotQuality || base.quality), 30, 90)),
    };
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

function normalizeClientViewport(viewport) {
  const width = Number(viewport?.width);
  const height = Number(viewport?.height);
  const dpr = Number(viewport?.dpr || 1);
  if (!Number.isFinite(width) || !Number.isFinite(height) || width < 100 || height < 100) {
    return null;
  }

  return {
    aspect: width / height,
    dpr: clamp(dpr, 1, 4),
    height: Math.round(height),
    width: Math.round(width),
  };
}

function sameClientViewport(current, next) {
  if (!current || !next) {
    return false;
  }

  const aspectDelta = Math.abs(current.aspect - next.aspect);
  return Math.abs(current.width - next.width) < 8 && Math.abs(current.height - next.height) < 8 && aspectDelta < 0.015;
}

function profileSizeChanged(current, next) {
  const currentWidth = Number(current?.maxWidth ?? current?.width ?? 0);
  const currentHeight = Number(current?.maxHeight ?? current?.height ?? 0);
  const nextWidth = Number(next?.maxWidth ?? next?.width ?? 0);
  const nextHeight = Number(next?.maxHeight ?? next?.height ?? 0);
  return Math.abs(currentWidth - nextWidth) >= 16 || Math.abs(currentHeight - nextHeight) >= 16;
}

function captureViewportSizeForClient(config, viewport) {
  if (!viewport) {
    return null;
  }

  return screencastSizeForViewport(SCREENCAST_PROFILES.good, config, viewport);
}

function screencastSizeForViewport(base, config, viewport) {
  if (!viewport) {
    return {
      height: Math.min(base.maxHeight, Number(config.screenshotMaxHeight || base.maxHeight)),
      width: Math.min(base.maxWidth, Number(config.screenshotMaxWidth || base.maxWidth)),
    };
  }

  const configLongEdge = Math.max(
    Number(config.screenshotMaxWidth || base.maxWidth),
    Number(config.screenshotMaxHeight || base.maxHeight),
  );
  const longEdge = Math.round(Math.min(Math.max(base.maxWidth, base.maxHeight), configLongEdge));
  const aspect = clamp(viewport.aspect, 0.25, 4);
  let width;
  let height;

  if (aspect >= 1) {
    width = longEdge;
    height = Math.round(longEdge / aspect);
  } else {
    height = longEdge;
    width = Math.round(longEdge * aspect);
  }

  return {
    height: Math.max(320, height),
    width: Math.max(320, width),
  };
}

function profileNameForNetwork(stats) {
  const bufferedAmount = Number(stats.bufferedAmount || 0);
  const droppedFramesInLast5s = Number(stats.droppedFramesInLast5s || 0);
  const rtt = Number(stats.rtt);

  if (bufferedAmount > 2_000_000 || droppedFramesInLast5s > 20 || (Number.isFinite(rtt) && rtt > 400)) {
    return "bad";
  }

  if (bufferedAmount > 500_000 || (Number.isFinite(rtt) && rtt > 180)) {
    return "medium";
  }

  return "good";
}

function profileNameForMode(mode, networkProfileName) {
  if (mode === "good") {
    return networkProfileName === "bad" ? "medium" : "good";
  }

  if (mode === "bad") {
    return "bad";
  }

  if (mode === "medium") {
    return worseProfileName("medium", networkProfileName);
  }

  return networkProfileName;
}

function worseProfileName(first, second) {
  const ranks = { good: 0, medium: 1, bad: 2 };
  return ranks[first] >= ranks[second] ? first : second;
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
