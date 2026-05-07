import { EventEmitter } from "node:events";

const CONNECT_RETRY_MS = 1200;
const COMMAND_TIMEOUT_MS = 7000;
const EDITABLE_RECTS_CACHE_MS = 800;
const METRICS_CACHE_MS = 3000;
const DEFAULT_PAGE_ZOOM_SCALE = 1;
const MIN_PAGE_ZOOM_SCALE = 1;
const MAX_PAGE_ZOOM_SCALE = 3;
const RESTORE_RESOLUTION_BINDING = "__codexAppRemotelyRestoreResolution";
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
    this.legacyPageZoomCleared = false;
    this.pageZoomScale = DEFAULT_PAGE_ZOOM_SCALE;
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
    this.viewportOverrideSuspended = false;
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
      pageZoomScale: this.pageZoomScale,
      screencastActive: this.screencastActive,
      screencastProfile: this.activeScreencastProfileName || this.desiredScreencastProfileName,
      screencastProfileMode: this.screencastProfileMode,
      screencastProfileSettings: this.screencastProfile(),
      streamingEnabled: this.streamingEnabled,
      target: this.currentTarget,
      viewportOverrideSuspended: this.viewportOverrideSuspended,
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

  async setSidebar(side, action = "open", { emitWarning = true } = {}) {
    const normalizedSide = side === "right" ? "right" : "left";
    const normalizedAction = action === "close" ? "close" : "open";
    const result = await this.send("Runtime.evaluate", {
      awaitPromise: true,
      expression: setSidebarExpression(normalizedSide, normalizedAction),
      returnByValue: true,
    });
    const response = result.result?.value;
    if (result.exceptionDetails || !response?.ok) {
      const reason = result.exceptionDetails?.text || response?.reason || "no matching control";
      if (emitWarning) {
        this.emitWarning(`${normalizedSide} sidebar ${normalizedAction} failed: ${reason}`);
      }
      return {
        ...response,
        action: normalizedAction,
        ok: false,
        reason,
        side: normalizedSide,
      };
    }

    if (this.streamingEnabled) {
      void this.captureAndBroadcast();
    }

    return {
      ...response,
      action: normalizedAction,
      side: normalizedSide,
    };
  }

  async applySidebarSwipe(direction, startPoint = null) {
    const normalizedDirection = direction === "left" ? "left" : "right";
    const closeSide = normalizedDirection === "right" ? "right" : "left";
    const openSide = normalizedDirection === "right" ? "left" : "right";
    const closeResult = await this.setSidebar(closeSide, "close", {
      emitWarning: false,
    });

    if (closeResult?.clicked) {
      return closeResult;
    }

    if (closeResult?.sideOpen) {
      return closeResult;
    }

    if (await this.isScrollableAt(startPoint?.x, startPoint?.y)) {
      return {
        ok: true,
        reason: "gesture started on a scrollable component",
        skipped: true,
      };
    }

    return this.setSidebar(openSide, "open");
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

  async isScrollableAt(normalizedX, normalizedY) {
    const x = Number(normalizedX);
    const y = Number(normalizedY);
    if (!Number.isFinite(x) || !Number.isFinite(y)) {
      return false;
    }

    const point = await this.pointFromNormalized(x, y);
    const result = await this.send("Runtime.evaluate", {
      expression: scrollableProbeExpression(point.x, point.y),
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
      this.legacyPageZoomCleared = false;
      this.emit("status", this.status());
      this.logger.log("[cdp] connected");

      try {
        await Promise.all([
          this.send("Page.enable"),
          this.send("Runtime.enable"),
        ]);
        await this.installRestoreResolutionMenu();
        await this.applyPageZoomScale();
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

  async setPageZoomScale(scale) {
    const nextScale = normalizePageZoomScale(scale);
    if (nextScale === this.pageZoomScale) {
      await this.applyPageZoomScale();
      this.emit("status", this.status());
      return;
    }

    this.pageZoomScale = nextScale;
    await this.applyPageZoomScale();
    this.emit("status", this.status());
  }

  setClientViewport(viewport) {
    const nextViewport = normalizeClientViewport(viewport);
    if (!nextViewport) {
      return;
    }

    const currentProfile = this.screencastProfile();
    const currentCaptureViewport = this.captureViewportSize();
    if (sameClientViewport(this.clientViewport, nextViewport) && !this.viewportOverrideSuspended) {
      return;
    }

    const resumeViewportOverride = this.viewportOverrideSuspended;
    this.clientViewport = nextViewport;
    this.viewportOverrideSuspended = false;

    const nextProfile = this.screencastProfile();
    const nextCaptureViewport = this.captureViewportSize();
    this.emit("status", this.status());

    if (resumeViewportOverride || profileSizeChanged(currentProfile, nextProfile) || profileSizeChanged(currentCaptureViewport, nextCaptureViewport)) {
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
    if (this.viewportOverrideSuspended) {
      return;
    }

    const size = this.captureViewportSize();
    if (!this.connected || !size) {
      return;
    }

    const zoomScale = normalizePageZoomScale(this.pageZoomScale);
    const emulatedSize = emulatedViewportSizeForCaptureSize(size, zoomScale);
    const key = `${size.width}x${size.height}@${formatPageZoomScale(zoomScale)}x`;
    if (key === this.lastViewportOverrideKey) {
      return;
    }

    try {
      await this.send("Emulation.setDeviceMetricsOverride", {
        deviceScaleFactor: zoomScale,
        height: emulatedSize.height,
        mobile: false,
        screenHeight: emulatedSize.height,
        screenOrientation: {
          angle: emulatedSize.height >= emulatedSize.width ? 0 : 90,
          type: emulatedSize.height >= emulatedSize.width ? "portraitPrimary" : "landscapePrimary",
        },
        screenWidth: emulatedSize.width,
        width: emulatedSize.width,
      });
      this.lastViewportOverrideKey = key;
      this.lastMetrics = null;
      this.lastMetricsAt = 0;
      this.logger.log(`[cdp] viewport override ${key} (${emulatedSize.width}x${emulatedSize.height} css px)`);
    } catch (error) {
      this.emitWarning(`viewport override failed: ${error.message}`);
    }
  }

  async clearClientViewportOverride() {
    if (!this.connected || this.lastViewportOverrideKey === "desktop") {
      return;
    }

    try {
      await this.send("Emulation.clearDeviceMetricsOverride");
      this.lastViewportOverrideKey = "desktop";
      this.lastMetrics = null;
      this.lastMetricsAt = 0;
      this.logger.log("[cdp] viewport override cleared");
    } catch (error) {
      this.emitWarning(`viewport restore failed: ${error.message}`);
    }
  }

  async restoreDesktopResolution() {
    if (!this.connected) {
      return;
    }

    const wasScreencastActive = this.screencastActive;
    this.viewportOverrideSuspended = true;

    if (wasScreencastActive) {
      await this.stopScreencast();
    }

    await this.clearClientViewportOverride();
    this.emit("status", this.status());
    this.logger.log("[cdp] restored desktop resolution");
    void this.injectRestoreResolutionMenu();

    if (wasScreencastActive && this.streamingEnabled) {
      await this.startScreencast();
    } else if (this.streamingEnabled) {
      await this.captureAndBroadcast();
    }
  }

  async installRestoreResolutionMenu() {
    if (!this.connected) {
      return;
    }

    try {
      await this.send("Runtime.addBinding", { name: RESTORE_RESOLUTION_BINDING });
    } catch (error) {
      if (!/already exists/i.test(error.message)) {
        this.emitWarning(`restore resolution binding failed: ${error.message}`);
      }
    }

    try {
      await this.send("Page.addScriptToEvaluateOnNewDocument", {
        source: restoreResolutionMenuExpression(RESTORE_RESOLUTION_BINDING),
      });
    } catch (error) {
      this.emitWarning(`restore resolution preload failed: ${error.message}`);
    }

    await this.injectRestoreResolutionMenu();
  }

  async applyPageZoomScale() {
    if (!this.connected) {
      return;
    }

    const targetScale = normalizePageZoomScale(this.pageZoomScale);
    this.pageZoomScale = targetScale;

    await this.clearLegacyPageZoom();
    await this.applyClientViewportOverride();
    this.logger.log(`[cdp] content scale ${formatPageZoomScale(this.pageZoomScale)}x`);

    this.lastEditableRects = [];
    this.lastEditableRectsAt = 0;
    this.lastMetrics = null;
    this.lastMetricsAt = 0;
    if (this.streamingEnabled) {
      void this.captureAndBroadcast();
    }
  }

  async clearLegacyPageZoom() {
    if (!this.connected || this.legacyPageZoomCleared) {
      return;
    }

    await Promise.allSettled([
      this.send("Emulation.setPageScaleFactor", {
        pageScaleFactor: DEFAULT_PAGE_ZOOM_SCALE,
      }),
      this.send("Runtime.evaluate", {
        expression: clearLegacyCssPageZoomExpression(),
        returnByValue: true,
      }),
    ]);
    this.legacyPageZoomCleared = true;
  }

  async injectRestoreResolutionMenu() {
    if (!this.connected) {
      return;
    }

    try {
      await this.send("Runtime.evaluate", {
        awaitPromise: true,
        expression: restoreResolutionMenuExpression(RESTORE_RESOLUTION_BINDING),
        returnByValue: true,
      });
    } catch (error) {
      this.emitWarning(`restore resolution menu inject failed: ${error.message}`);
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
    if (payload.method === "Runtime.bindingCalled" && payload.params?.name === RESTORE_RESOLUTION_BINDING) {
      void this.restoreDesktopResolution();
      return;
    }

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
    this.legacyPageZoomCleared = false;

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
    return captureViewportSizeForClient(this.config, this.effectiveClientViewport());
  }

  screencastProfile() {
    const base = SCREENCAST_PROFILES[this.desiredScreencastProfileName] || SCREENCAST_PROFILES.good;
    const size = screencastSizeForViewport(base, this.config, this.effectiveClientViewport());
    return {
      everyNthFrame: Math.max(base.everyNthFrame, Number(this.config.screencastEveryNthFrame || 1)),
      format: "jpeg",
      maxHeight: size.height,
      maxWidth: size.width,
      name: base.name,
      quality: Math.min(base.quality, clamp(Number(this.config.screenshotQuality || base.quality), 30, 90)),
    };
  }

  effectiveClientViewport() {
    return this.viewportOverrideSuspended ? null : this.clientViewport;
  }
}

function selectTarget(targets) {
  const pageTargets = targets.filter((target) => target.webSocketDebuggerUrl && target.type === "page");
  return pageTargets.find((target) => /codex/i.test(`${target.title} ${target.url}`)) || pageTargets[0] || targets.find((target) => target.webSocketDebuggerUrl) || null;
}

function normalizeTarget(target) {
  return {
    description: target.description || "",
    devtoolsFrontendUrl: target.devtoolsFrontendUrl || "",
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

function normalizePageZoomScale(scale) {
  return Math.round(clamp(Number(scale) || DEFAULT_PAGE_ZOOM_SCALE, MIN_PAGE_ZOOM_SCALE, MAX_PAGE_ZOOM_SCALE) * 100) / 100;
}

function emulatedViewportSizeForCaptureSize(size, zoomScale) {
  const scale = normalizePageZoomScale(zoomScale);
  return {
    height: Math.max(1, Math.round(Number(size.height || 1) / scale)),
    width: Math.max(1, Math.round(Number(size.width || 1) / scale)),
  };
}

function formatPageZoomScale(scale) {
  return Number.isInteger(scale) ? String(scale) : scale.toFixed(2).replace(/0$/, "");
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

function restoreResolutionMenuExpression(bindingName) {
  return `(() => {
    const bindingName = ${JSON.stringify(bindingName)};
    const hostId = "codex-app-remotely-restore-resolution";

    function install() {
      if (!document.documentElement || document.getElementById(hostId)) {
        return true;
      }

      const host = document.createElement("div");
      host.id = hostId;
      host.style.cssText = [
        "position:fixed",
        "top:8px",
        "right:136px",
        "z-index:2147483647",
        "width:auto",
        "height:auto",
        "pointer-events:auto",
      ].join(";");

      const root = host.attachShadow({ mode: "open" });
      const style = document.createElement("style");
      style.textContent = \`
        button {
          align-items: center;
          background: rgba(32, 36, 42, 0.94);
          border: 1px solid rgba(120, 130, 145, 0.42);
          border-radius: 7px;
          color: rgba(245, 247, 250, 0.94);
          cursor: default;
          display: inline-flex;
          font: 600 12px/1 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
          height: 28px;
          justify-content: center;
          letter-spacing: 0;
          padding: 0 10px;
          white-space: nowrap;
        }
        button:hover {
          background: rgba(52, 58, 67, 0.96);
          border-color: rgba(160, 170, 185, 0.56);
        }
        button:active {
          background: rgba(76, 86, 101, 0.98);
        }
      \`;

      const button = document.createElement("button");
      button.type = "button";
      button.textContent = "恢复分辨率";
      button.title = "恢复桌面端原本的分辨率";
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        const binding = window[bindingName];
        if (typeof binding === "function") {
          binding(JSON.stringify({ type: "restoreDesktopResolution" }));
        }
      });

      root.append(style, button);
      document.documentElement.appendChild(host);
      return true;
    }

    function ensureInstalled() {
      try {
        install();
      } catch {
        return false;
      }
      return true;
    }

    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", ensureInstalled, { once: true });
    } else {
      ensureInstalled();
    }

    if (!window.__codexAppRemotelyRestoreResolutionObserver) {
      window.__codexAppRemotelyRestoreResolutionObserver = new MutationObserver(() => {
        if (!document.getElementById(hostId)) {
          ensureInstalled();
        }
      });
      window.__codexAppRemotelyRestoreResolutionObserver.observe(document.documentElement, {
        childList: true,
        subtree: true,
      });
    }

    return true;
  })()`;
}

function setSidebarExpression(side, action) {
  return `(() => {
    const side = ${JSON.stringify(side === "right" ? "right" : "left")};
    const action = ${JSON.stringify(action === "close" ? "close" : "open")};
    const viewportWidth = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
    const viewportHeight = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
    const selector = "button, [role='button'], [role='menuitem'], [aria-label][tabindex], [title][tabindex]";
    const roots = [document];
    const seen = new Set();
    const candidates = [];
    const commonSidebarWords = ["sidebar", "side bar", "side panel", "侧边栏", "边栏"];
    const leftWords = ["left", "primary", "nav", "navigation", "activity", "history", "project", "projects", "workspace", "左", "左侧", "项目", "对话"];
    const rightWords = ["right", "secondary", "auxiliary", "details", "detail", "panel", "inspector", "右", "右侧", "面板", "详情"];
    const openStateWords = ["open", "show", "expand", "打开", "显示", "展开"];
    const toggleWords = ["toggle", "切换"];
    const actionWords = [...openStateWords, ...toggleWords];
    const closeWords = ["close", "hide", "collapse", "关闭", "隐藏", "收起"];
    const closeStateWords = ["close", "collapse", "关闭", "收起"];
    const negativeWords = ["back", "forward", "refresh", "reload", "zoom", "quality", "new chat", "search", "settings", "terminal", "folder", "send", "microphone", "model", "刷新", "搜索", "设置", "新对话"];
    const rightToolbarNoiseWords = ["terminal", "console", "shell", "folder", "files", "file browser", "command line", "终端", "控制台", "文件夹", "文件"];

    function text(value) {
      return String(value || "").replace(/\\s+/g, " ").trim();
    }

    function labelFor(element) {
      return [
        element.getAttribute("aria-label"),
        element.getAttribute("title"),
        element.getAttribute("data-testid"),
        element.getAttribute("data-test-id"),
        element.id,
        element.textContent,
      ].map(text).filter(Boolean).join(" ");
    }

    function includesAny(label, words) {
      return words.some((word) => label.includes(word));
    }

    function isInteractive(element) {
      const role = (element.getAttribute("role") || "").toLowerCase();
      return element.localName === "button" || role === "button" || role === "menuitem" || element.hasAttribute("tabindex");
    }

    function visibleRect(element) {
      if (!isInteractive(element)) {
        return null;
      }

      const style = window.getComputedStyle(element);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
        return null;
      }

      const rect = element.getBoundingClientRect();
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(viewportWidth, rect.right);
      const bottom = Math.min(viewportHeight, rect.bottom);
      if (right <= left || bottom <= top || right - left < 8 || bottom - top < 8) {
        return null;
      }

      return {
        bottom,
        height: bottom - top,
        left,
        right,
        top,
        width: right - left,
      };
    }

    function expandedState(element) {
      const value = element.getAttribute("aria-expanded") ?? element.getAttribute("aria-pressed");
      if (value === "true") {
        return true;
      }
      if (value === "false") {
        return false;
      }
      return null;
    }

    function isUnlabeledEdgeControl(rect, lowerLabel) {
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const compactIcon = rect.width <= 56 && rect.height <= 56;
      const edgeBand = Math.min(96, viewportWidth * 0.16);
      const farEdge = side === "left" ? centerX <= edgeBand : centerX >= viewportWidth - edgeBand;
      return !lowerLabel && compactIcon && farEdge && centerY <= Math.min(140, viewportHeight * 0.24);
    }

    function visiblePanelRect(element) {
      if (!element || element.nodeType !== Node.ELEMENT_NODE) {
        return null;
      }

      const style = window.getComputedStyle(element);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
        return null;
      }

      const rect = element.getBoundingClientRect();
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(viewportWidth, rect.right);
      const bottom = Math.min(viewportHeight, rect.bottom);
      const width = right - left;
      const height = bottom - top;
      if (width < 110 || height < viewportHeight * 0.45 || width > viewportWidth * 0.62 || right <= left || bottom <= top) {
        return null;
      }

      if (top > viewportHeight * 0.35 || bottom < viewportHeight * 0.65) {
        return null;
      }

      return { bottom, height, left, right, top, width };
    }

    function isSidebarPanelForSide(rect, targetSide) {
      if (!rect) {
        return false;
      }
      const centerX = rect.left + rect.width / 2;
      const edgeAllowance = Math.max(8, Math.min(48, viewportWidth * 0.06));
      if (targetSide === "left") {
        return rect.left <= edgeAllowance && centerX <= viewportWidth * 0.42;
      }

      const touchesRightEdge = rect.right >= viewportWidth - edgeAllowance;
      const reachesRightArea = rect.right >= viewportWidth * 0.82;
      const startsInRightHalf = rect.left >= viewportWidth * 0.36;
      const mostlyRightPanel = centerX >= viewportWidth * 0.58 && rect.width <= viewportWidth * 0.5;
      return touchesRightEdge || (reachesRightArea && startsInRightHalf && mostlyRightPanel);
    }

    function controlIndicatesSideOpen(element, targetSide) {
      const rect = visibleRect(element);
      if (!rect) {
        return false;
      }

      const centerX = rect.left + rect.width / 2;
      const sideZone = targetSide === "left" ? centerX <= viewportWidth * 0.35 : centerX >= viewportWidth * 0.65;
      if (!sideZone) {
        return false;
      }

      const lowerLabel = labelFor(element).toLowerCase();
      const targetWords = targetSide === "left" ? leftWords : rightWords;
      const relevant =
        includesAny(lowerLabel, commonSidebarWords) ||
        includesAny(lowerLabel, targetWords) ||
        includesAny(lowerLabel, closeWords) ||
        includesAny(lowerLabel, toggleWords);

      if (!relevant) {
        return false;
      }

      const expanded = expandedState(element);
      return expanded === true || includesAny(lowerLabel, closeStateWords);
    }

    function isSideOpen(targetSide) {
      const panelRoots = [document];
      const panelSeen = new Set();
      for (let index = 0; index < panelRoots.length; index += 1) {
        const root = panelRoots[index];
        for (const element of root.querySelectorAll("*")) {
          if (panelSeen.has(element)) {
            continue;
          }
          panelSeen.add(element);
          if (controlIndicatesSideOpen(element, targetSide)) {
            return true;
          }
          const rect = visiblePanelRect(element);
          if (isSidebarPanelForSide(rect, targetSide)) {
            return true;
          }
          if (element.shadowRoot && !panelRoots.includes(element.shadowRoot)) {
            panelRoots.push(element.shadowRoot);
          }
        }
      }
      return false;
    }

    const sideOpen = isSideOpen(side);

    function scoreCandidate(rect, lowerLabel, expanded) {
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const sideZone = side === "left" ? centerX <= viewportWidth * 0.35 : centerX >= viewportWidth * 0.65;
      const compactIcon = rect.width <= 56 && rect.height <= 56;
      const unlabeledEdgeControl = isUnlabeledEdgeControl(rect, lowerLabel);
      let score = sideZone ? 35 : -90;

      if (centerY <= Math.min(140, viewportHeight * 0.24)) {
        score += 35;
      } else if (centerY <= viewportHeight * 0.4) {
        score += 10;
      } else {
        score -= 60;
      }

      if (side === "left") {
        score += Math.max(0, (viewportWidth * 0.28 - centerX) / Math.max(1, viewportWidth * 0.28)) * 30;
      } else {
        score += Math.max(0, (centerX - viewportWidth * 0.72) / Math.max(1, viewportWidth * 0.28)) * 30;
      }

      if (includesAny(lowerLabel, commonSidebarWords)) {
        score += 80;
      }
      if (includesAny(lowerLabel, side === "left" ? leftWords : rightWords)) {
        score += 55;
      }
      if (action === "close") {
        if (expanded === true) {
          score += 80;
        } else if (expanded === false) {
          score -= 20;
        }
        if (includesAny(lowerLabel, closeWords)) {
          score += 80;
        }
        if (includesAny(lowerLabel, toggleWords)) {
          score += 45;
        }
        if (includesAny(lowerLabel, openStateWords) && !includesAny(lowerLabel, closeWords) && !includesAny(lowerLabel, toggleWords)) {
          score -= 60;
        }
      } else {
        if (expanded === false) {
          score += 20;
        } else if (expanded === true) {
          score += 40;
        }
        if (includesAny(lowerLabel, actionWords)) {
          score += 25;
        }
        if (includesAny(lowerLabel, closeWords)) {
          score -= 25;
        }
      }
      if (includesAny(lowerLabel, negativeWords)) {
        score -= 70;
      }
      if (side === "right" && action === "open" && includesAny(lowerLabel, rightToolbarNoiseWords)) {
        score -= 160;
      }
      if (!lowerLabel) {
        score -= 50;
      }
      if (compactIcon) {
        score += 12;
      }
      if (unlabeledEdgeControl && (action === "open" || (action === "close" && sideOpen))) {
        score += 95;
      }

      return score;
    }

    function scoreRightSidebarToggle(candidate) {
      if (side !== "right") {
        return -Infinity;
      }

      const rect = candidate.rect;
      const lowerLabel = candidate.lowerLabel;
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const compactIcon = rect.width <= 56 && rect.height <= 56;
      const unlabeledEdgeControl = isUnlabeledEdgeControl(rect, lowerLabel);
      let score = centerX >= viewportWidth * 0.65 ? 35 : -100;

      score += Math.max(0, (centerX - viewportWidth * 0.72) / Math.max(1, viewportWidth * 0.28)) * 35;
      if (centerY <= Math.min(140, viewportHeight * 0.24)) {
        score += 35;
      } else {
        score -= 50;
      }
      if (includesAny(lowerLabel, commonSidebarWords)) {
        score += 80;
      }
      if (includesAny(lowerLabel, rightWords)) {
        score += 55;
      }
      if (includesAny(lowerLabel, actionWords)) {
        score += 25;
      }
      if (includesAny(lowerLabel, closeWords)) {
        score -= 35;
      }
      if (includesAny(lowerLabel, negativeWords) || includesAny(lowerLabel, rightToolbarNoiseWords)) {
        score -= 160;
      }
      if (!lowerLabel) {
        score -= 50;
      }
      if (compactIcon) {
        score += 12;
      }
      if (unlabeledEdgeControl) {
        score += 95;
      }

      return score;
    }

    function dispatchClick(element, rect) {
      const clientX = rect.left + rect.width / 2;
      const clientY = rect.top + rect.height / 2;
      const base = {
        bubbles: true,
        button: 0,
        cancelable: true,
        clientX,
        clientY,
        composed: true,
        view: window,
      };

      try {
        element.focus({ preventScroll: true });
      } catch {
        try {
          element.focus();
        } catch {
          // Ignore focus failures for non-focusable controls.
        }
      }

      if (window.PointerEvent) {
        element.dispatchEvent(new PointerEvent("pointerdown", { ...base, buttons: 1, isPrimary: true, pointerId: 1, pointerType: "mouse" }));
      }
      element.dispatchEvent(new MouseEvent("mousedown", { ...base, buttons: 1 }));
      if (window.PointerEvent) {
        element.dispatchEvent(new PointerEvent("pointerup", { ...base, buttons: 0, isPrimary: true, pointerId: 1, pointerType: "mouse" }));
      }
      element.dispatchEvent(new MouseEvent("mouseup", { ...base, buttons: 0 }));
      element.dispatchEvent(new MouseEvent("click", { ...base, buttons: 0 }));
    }

    for (let index = 0; index < roots.length; index += 1) {
      const root = roots[index];
      for (const element of root.querySelectorAll(selector)) {
        if (seen.has(element)) {
          continue;
        }
        seen.add(element);

        const rect = visibleRect(element);
        if (!rect) {
          continue;
        }

        const label = labelFor(element);
        const lowerLabel = label.toLowerCase();
        const expanded = expandedState(element);
        const score = scoreCandidate(rect, lowerLabel, expanded);
        if (score > 0) {
          candidates.push({
            element,
            expanded,
            label,
            lowerLabel,
            rect,
            score,
          });
        }
      }

      for (const element of root.querySelectorAll("*")) {
        if (element.shadowRoot && !roots.includes(element.shadowRoot)) {
          roots.push(element.shadowRoot);
        }
      }
    }

    candidates.sort((first, second) => second.score - first.score);
    if (action === "close" && side === "right" && sideOpen) {
      const toggleCandidate = candidates
        .map((candidate) => ({
          ...candidate,
          toggleScore: scoreRightSidebarToggle(candidate),
        }))
        .sort((first, second) => second.toggleScore - first.toggleScore)[0];

      if (toggleCandidate && toggleCandidate.toggleScore >= 70) {
        dispatchClick(toggleCandidate.element, toggleCandidate.rect);
        return {
          clicked: true,
          label: toggleCandidate.label,
          ok: true,
          score: toggleCandidate.toggleScore,
          side,
          sideOpen,
          via: "right-sidebar-toggle",
        };
      }
    }

    const best = candidates[0];
    const threshold = action === "close" ? 60 : 70;
    if (!best || best.score < threshold) {
      return {
        ok: false,
        reason: \`no matching sidebar \${action} control\`,
        sideOpen,
      };
    }

    if (action === "close") {
      const hasCloseState = includesAny(best.lowerLabel, closeWords);
      const hasToggle = includesAny(best.lowerLabel, toggleWords);
      const hasOpenEdgeToggle = sideOpen && isUnlabeledEdgeControl(best.rect, best.lowerLabel);
      const hasOnlyOpenState = best.expanded === false && includesAny(best.lowerLabel, openStateWords) && !hasCloseState && !hasToggle;
      if (hasOnlyOpenState) {
        return {
          alreadyClosed: true,
          label: best.label,
          ok: true,
          score: best.score,
          side,
          sideOpen,
        };
      }
      if (best.expanded !== true && !hasCloseState && (!hasToggle || !sideOpen) && !hasOpenEdgeToggle) {
        return {
          ok: false,
          reason: "no matching sidebar close control",
          sideOpen,
        };
      }

      dispatchClick(best.element, best.rect);
      return {
        clicked: true,
        label: best.label,
        ok: true,
        score: best.score,
        side,
        sideOpen,
      };
    }

    if (best.expanded === true || (includesAny(best.lowerLabel, closeStateWords) && !includesAny(best.lowerLabel, actionWords))) {
      return {
        alreadyOpen: true,
        label: best.label,
        ok: true,
        score: best.score,
        side,
        sideOpen,
      };
    }

    dispatchClick(best.element, best.rect);
    return {
      clicked: true,
      label: best.label,
      ok: true,
      score: best.score,
      side,
      sideOpen,
    };
  })()`;
}

function clearLegacyCssPageZoomExpression() {
  return `(() => {
    const root = document.documentElement;
    if (root?.getAttribute("data-codex-app-remotely-page-zoom")) {
      root.style.zoom = "";
      root.removeAttribute("data-codex-app-remotely-page-zoom");
    }
    return true;
  })()`;
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

function scrollableProbeExpression(x, y) {
  return `(() => {
    ${scrollableHelpers()}
    let element = document.elementFromPoint(${JSON.stringify(x)}, ${JSON.stringify(y)});
    while (element && element.shadowRoot) {
      const nested = element.shadowRoot.elementFromPoint(${JSON.stringify(x)}, ${JSON.stringify(y)});
      if (!nested || nested === element) {
        break;
      }
      element = nested;
    }
    return closestScrollable(element);
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

function scrollableHelpers() {
  return `
    const viewportWidth = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
    const viewportHeight = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
    const scrollableOverflowValues = new Set(["auto", "scroll", "overlay"]);

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

    function canScrollAxis(scrollSize, clientSize, overflowValue) {
      return scrollSize - clientSize > 2 && scrollableOverflowValues.has(overflowValue);
    }

    function isScrollableElement(element) {
      if (!element || element.nodeType !== Node.ELEMENT_NODE) {
        return false;
      }

      if (element === document.documentElement || element === document.body) {
        return false;
      }

      const style = window.getComputedStyle(element);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
        return false;
      }

      const rect = element.getBoundingClientRect();
      if (rect.width < 16 || rect.height < 16) {
        return false;
      }

      if (rect.width >= viewportWidth * 0.96 && rect.height >= viewportHeight * 0.96) {
        return false;
      }

      return (
        canScrollAxis(element.scrollWidth, element.clientWidth, style.overflowX) ||
        canScrollAxis(element.scrollHeight, element.clientHeight, style.overflowY)
      );
    }

    function closestScrollable(element) {
      let current = element;
      while (current) {
        if (isScrollableElement(current)) {
          return true;
        }
        current = composedParent(current);
      }
      return false;
    }
  `;
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
