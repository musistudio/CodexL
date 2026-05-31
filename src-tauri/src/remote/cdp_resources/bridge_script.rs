use super::*;

pub(super) fn web_bridge_script_response() -> WebResourceResponse {
    WebResourceResponse {
        status: StatusCode::OK,
        content_type: "application/javascript; charset=utf-8".to_string(),
        body: Bytes::from_static(WEB_BRIDGE_SCRIPT.as_bytes()),
    }
}

pub(super) const WEB_BRIDGE_SCRIPT: &str = r#"(() => {
  installCryptoRandomUuidPolyfill();

  if (window.__codexWebBridgeInstalled) {
    return;
  }
  window.__codexWebBridgeInstalled = true;

  const pageParams = new URLSearchParams(window.location.search);

  function installCryptoRandomUuidPolyfill() {
    const scope = typeof globalThis !== "undefined" ? globalThis : window;
    const cryptoObject = scope.crypto || window.crypto;
    if (cryptoObject && typeof cryptoObject.randomUUID === "function") {
      return;
    }

    const randomBytes = () => {
      const bytes = new Uint8Array(16);
      try {
        if (cryptoObject && typeof cryptoObject.getRandomValues === "function") {
          cryptoObject.getRandomValues(bytes);
          return bytes;
        }
      } catch {}
      for (let index = 0; index < bytes.length; index += 1) {
        bytes[index] = Math.floor(Math.random() * 256) & 0xff;
      }
      return bytes;
    };
    const randomUUID = () => {
      const bytes = randomBytes();
      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;
      const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    };
    const installOn = (target) => {
      if (!target || typeof target.randomUUID === "function") {
        return true;
      }
      try {
        Object.defineProperty(target, "randomUUID", {
          configurable: true,
          value: randomUUID,
          writable: true,
        });
        return true;
      } catch {
        return false;
      }
    };

    if (installOn(cryptoObject)) {
      return;
    }
    if (cryptoObject && installOn(Object.getPrototypeOf(cryptoObject))) {
      return;
    }
    try {
      Object.defineProperty(scope, "crypto", {
        configurable: true,
        value: { randomUUID },
      });
    } catch {}
  }

	  function bridgeSocketUrl() {
	    const configuredUrl = pageParams.get("codexBridgeUrl");
	    let url;
    try {
      url = configuredUrl
        ? new URL(configuredUrl, window.location.href)
        : new URL("./_bridge", window.location.href);
    } catch {
      url = new URL("./_bridge", window.location.href);
    }
    if (url.protocol === "https:") {
      url.protocol = "wss:";
    } else if (url.protocol === "http:") {
      url.protocol = "ws:";
    } else if (url.protocol !== "ws:" && url.protocol !== "wss:") {
      url = new URL("./_bridge", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    }
    const token = pageParams.get("token");
    if (token) {
      url.searchParams.set("token", token);
    }
	    return url;
	  }

	  function bridgeWebTransportUrl() {
	    const configuredUrl = pageParams.get("codexBridgeTransportUrl");
	    if (!configuredUrl) {
	      return null;
	    }
	    let url;
	    try {
	      url = new URL(configuredUrl, window.location.href);
	    } catch {
	      return null;
	    }
	    if (url.protocol !== "https:") {
	      return null;
	    }
    const token = pageParams.get("token");
    if (token) {
      url.searchParams.set("token", token);
    }
	    return url;
	  }

	  const bridgeUrl = bridgeSocketUrl();
	  const bridgeTransportUrl = bridgeWebTransportUrl();
	  const transportPreference = (pageParams.get("transport") || "auto").toLowerCase();
	  const sharedObjects = (window.__codexWebSharedObjects ||= Object.create(null));
	  const pending = new Map();
	  const BRIDGE_STATUS_MESSAGE = "codex-web-bridge-status";
	  const PARENT_BRIDGE_OPEN_MESSAGE = "codex-web-parent-bridge-open";
	  const PARENT_BRIDGE_OPENED_MESSAGE = "codex-web-parent-bridge-opened";
	  const PARENT_BRIDGE_SEND_MESSAGE = "codex-web-parent-bridge-send";
	  const PARENT_BRIDGE_MESSAGE = "codex-web-parent-bridge-message";
	  const PARENT_BRIDGE_CLOSE_MESSAGE = "codex-web-parent-bridge-close";
	  const PARENT_BRIDGE_CLOSED_MESSAGE = "codex-web-parent-bridge-closed";
	  const PARENT_BRIDGE_ERROR_MESSAGE = "codex-web-parent-bridge-error";
	  const BRIDGE_HEARTBEAT_INTERVAL_MS = 15000;
	  const BRIDGE_HEARTBEAT_STALE_MS = 30000;
	  const BRIDGE_HEARTBEAT_TIMEOUT_MS = 8000;
	  const BRIDGE_RECONNECT_MIN_DELAY_MS = 250;
	  const BRIDGE_RECONNECT_MAX_DELAY_MS = 5000;
	  const BRIDGE_REQUEST_TIMEOUT_MS = 30000;
	  const BRIDGE_LONG_REQUEST_TIMEOUT_MS = 5 * 60 * 1000;
	  const BRIDGE_STREAM_REQUEST_TIMEOUT_MS = 10 * 60 * 1000;
	  const BRIDGE_SNAPSHOT_REQUEST_COOLDOWN_MS = 1000;
	  const BRIDGE_HYDRATION_SNAPSHOT_DELAY_MS = 100;
	  const BRIDGE_PEER_HYDRATION_SNAPSHOT_FALLBACK_MS = 1500;
	  const BRIDGE_THREAD_HYDRATION_METHODS = new Set([
	    "thread/list",
	    "thread/read",
	    "thread/search",
	    "thread/turns/list",
	    "turn/list",
	  ]);
	  const BRIDGE_FOLLOWER_HOST_RESPONSE_METHODS = new Set([
	    "thread-follower-command-approval-decision",
	    "thread-follower-file-approval-decision",
	    "thread-follower-permissions-request-approval-response",
	    "thread-follower-submit-mcp-server-elicitation-response",
	    "thread-follower-submit-user-input",
	  ]);
	  const CODEX_APP_NOTIFICATION_TYPES = new Set([
	    "terminal-attached",
	    "terminal-data",
	    "terminal-error",
	    "terminal-exit",
	    "terminal-init-log",
	    "close-terminal-session",
	    "shared-object-updated",
	    "query-cache-invalidate",
	    "tray-menu-threads-changed",
	  ]);
	  let socket = null;
	  let connectingSocket = null;
	  let transportClient = null;
	  let connectingTransport = null;
	  let parentBridgeOpen = false;
	  let parentBridgeListenerInstalled = false;
	  let parentBridgeStopHeartbeat = null;
	  let parentBridgeOpenTimer = null;
	  let connectingParentBridge = null;
	  let resolveParentBridge = null;
	  let rejectParentBridge = null;
	  let webTransportUnavailable = false;
	  let bridgeConnectionStarted = false;
	  let bridgeLastHeartbeatAckAt = 0;
	  let bridgeHeartbeatTimeoutTimer = null;
	  let bridgeReconnectDelayMs = BRIDGE_RECONNECT_MIN_DELAY_MS;
	  let bridgeReconnectTimer = null;
	  let nextMessageId = 1;
	  let lastNotificationSeq = 0;
	  let lastSnapshotRequestAt = 0;
	  const peerHydrationRequests = new Map();
	  const bridgeClientId = `codex-web-bridge-frame-${Date.now()}-${Math.random().toString(36).slice(2)}`;
	  const E2EE_AAD = new TextEncoder().encode("codexl-remote-e2ee-v1");
	  const E2EE_STORAGE_PREFIX = "codexl.remote.e2ee.v1.";
	  let bridgeCryptoPromise = null;

	  function parentBridgeTargetOrigin() {
	    return pageParams.get("codexBridgeParentOrigin") || "*";
	  }

	  function shouldUseParentBridge() {
	    return pageParams.get("codexBridgeParent") === "1" && window.parent && window.parent !== window;
	  }

	  function parentBridgeConnection() {
	    return {
	      send(raw) {
	        return sendParentBridgeRaw(raw);
	      },
	    };
	  }

	  function postParentBridgeMessage(message) {
	    window.parent.postMessage(
	      {
	        ...message,
	        clientId: bridgeClientId,
	      },
	      parentBridgeTargetOrigin(),
	    );
	  }

	  function notifyBridgeStatus(status, detail = {}) {
	    try {
	      window.parent?.postMessage(
	        {
	          ...detail,
	          clientId: bridgeClientId,
	          status,
	          ts: Date.now(),
	          type: BRIDGE_STATUS_MESSAGE,
	        },
	        parentBridgeTargetOrigin(),
	      );
	    } catch {}
	  }

  function bridgeDebug(...args) {
    try {
      console.info("[codex-web][bridge]", ...args);
    } catch {}
  }

  function bridgeMessageLabel(message) {
    if (!message || typeof message !== "object") {
      return String(message);
    }
    const type = message.type || "?";
    if (type === "fetch") {
      let endpoint = "";
      try {
        endpoint = new URL(message.url, window.location.href).pathname.replace(/^\//, "");
      } catch {}
      return `fetch:${endpoint}:${message.requestId || ""}`;
    }
    if (type === "fetch-response") {
      const status = message.status != null ? `:${message.status}` : "";
      const error = message.error ? `:${String(message.error).replace(/\s+/g, " ").slice(0, 120)}` : "";
      return `fetch-response:${message.requestId || ""}:${message.responseType || ""}${status}${error}`;
    }
    if (type === "ipc-broadcast") {
      const params = message.params || {};
      if (message.method === "client-status-changed") {
        return `ipc-broadcast:client-status-changed:${params.status || ""}:${params.clientType || ""}:${params.clientId || ""}`;
      }
      const change = params.change || {};
      const turnCount =
        change.type === "snapshot" && Array.isArray(change.conversationState?.turns)
          ? `:turns=${change.conversationState.turns.length}`
          : "";
      return `ipc-broadcast:${message.method || ""}:${change.type || ""}:${params.conversationId || ""}${turnCount}`;
    }
    if (type === "thread-stream-state-changed") {
      const change = message.change || {};
      const turnCount =
        change.type === "snapshot" && Array.isArray(change.conversationState?.turns)
          ? `:turns=${change.conversationState.turns.length}`
          : "";
      return `thread-stream-state-changed:${change.type || ""}:${message.conversationId || ""}${turnCount}`;
    }
    if (type === "mcp-request") {
      return `mcp-request:${message.request?.method || ""}`;
    }
    if (type === "mcp-notification") {
      return `mcp-notification:${message.method || ""}`;
    }
    if (type === "mcp-response") {
      return `mcp-response:${message.message?.method || message.message?.id || ""}`;
    }
    return type;
  }

  function bridgePayloadLabels(payload) {
    const messages = Array.isArray(payload?.messages) ? payload.messages : [];
    const labels = messages.slice(0, 8).map(bridgeMessageLabel);
    if (messages.length > labels.length) {
      labels.push(`+${messages.length - labels.length}`);
    }
    return labels.join(",") || "-";
  }

  function dispatchHostMessage(message) {
    if (!message || typeof message !== "object") {
      return;
    }
    const hostMessage = stripBridgeMetadata(normalizeHostMessageForCodexApp(message));
    if (maybeHandleRemoteWorkspaceRootRequest(hostMessage)) {
      return;
    }
    bridgeDebug("dispatchHostMessage", bridgeMessageLabel(hostMessage));
    if (hostMessage.type === "shared-object-updated" && typeof hostMessage.key === "string") {
      sharedObjects[hostMessage.key] = hostMessage.value;
    }
    window.dispatchEvent(
      new MessageEvent("message", {
        data: hostMessage,
        origin: window.location.origin,
        source: window,
      }),
    );
  }

  function normalizeHostMessageForCodexApp(message) {
    if (
      message?.type === "mcp-notification" &&
      message.method === "thread-stream-state-changed" &&
      message.params &&
      typeof message.params === "object"
    ) {
      message = {
        type: "thread-stream-state-changed",
        sourceClientId: message.sourceClientId || message.clientId,
        version: message.version,
        ...message.params,
      };
    }
    if (message?.type !== "thread-stream-state-changed") {
      return message;
    }
    return {
      type: "ipc-broadcast",
      method: "thread-stream-state-changed",
      sourceClientId: message.sourceClientId || message.clientId || "codex-web-bridge-host-view",
      version: Number.isFinite(message.version) ? message.version : 6,
      params: stripBridgeMetadata(message),
    };
  }

  function stripBridgeMetadata(message) {
    if (!message || typeof message !== "object") {
      return message;
    }
    const hostMessage = { ...message };
    delete hostMessage.__codexWebBridgeNotificationForwarded;
    delete hostMessage.__codexWebBridgeNotificationSeq;
    delete hostMessage.__codexWebBridgeNotificationQueuedAt;
    delete hostMessage.__codexEventSource;
    delete hostMessage.__codexEventChannel;
    delete hostMessage.__codexEventMethod;
    return hostMessage;
  }

  function isBridgeNotification(message) {
    return (
      message &&
      typeof message === "object" &&
      (message.__codexWebBridgeNotificationSeq != null ||
        message.__codexEventSource === "cdp-webview")
    );
  }

  function shouldDispatchBridgeNotificationToCodexApp(message) {
    if (!message || typeof message !== "object") {
      return false;
    }
    if (message.type === "ipc-broadcast") {
      return (
        message.method === "thread-stream-state-changed" ||
        message.method === "client-status-changed"
      );
    }
    if (message.type === "thread-stream-state-changed") {
      return true;
    }
    if (message.type === "mcp-notification" || message.type === "mcp-request") {
      return true;
    }
    if (message.type === "mcp-response") {
      return Boolean(message.message && typeof message.message.method === "string");
    }
    return CODEX_APP_NOTIFICATION_TYPES.has(message.type);
  }

  function bridgeIpcRequestBody(message) {
    if (!message || typeof message !== "object" || message.type !== "fetch") {
      return null;
    }
    if (typeof message.url !== "string") {
      return null;
    }
    let url;
    try {
      url = new URL(message.url, window.location.href);
    } catch {
      return null;
    }
    if (url.protocol !== "vscode:" || url.hostname !== "codex" || url.pathname !== "/ipc-request") {
      return null;
    }
    if (typeof message.body === "string") {
      try {
        return JSON.parse(message.body);
      } catch {
        return null;
      }
    }
    if (message.body && typeof message.body === "object") {
      return message.body;
    }
    return null;
  }

  function bridgeFollowerHostResponseRequest(message) {
    if (!isBridgeNotification(message)) {
      return null;
    }
    const body = bridgeIpcRequestBody(message);
    if (!body || typeof body !== "object") {
      return null;
    }
    if (!BRIDGE_FOLLOWER_HOST_RESPONSE_METHODS.has(body.method)) {
      return null;
    }
    const targetClientId = typeof body.targetClientId === "string" ? body.targetClientId : "";
    if (!targetClientId.includes("codex-web")) {
      return null;
    }
    const params = body.params && typeof body.params === "object" ? body.params : null;
    if (!params || params.requestId == null) {
      return null;
    }
    return { method: body.method, params };
  }

  function followerHostResponseResult(request) {
    if (
      request.method === "thread-follower-command-approval-decision" ||
      request.method === "thread-follower-file-approval-decision"
    ) {
      return { decision: request.params.decision };
    }
    return request.params.response;
  }

  async function forwardFollowerHostResponseToCodexHost(message, request) {
    const result = followerHostResponseResult(request);
    if (result === undefined) {
      return;
    }
    await sendBridgeRequest({
      type: "mcp-response",
      hostId: message.hostId || request.params.hostId || "local",
      response: {
        id: request.params.requestId,
        result,
      },
    });
    await sendBridgeRequest({
      type: "desktop-notification-hide",
      notificationId: `approval-${request.params.requestId}`,
    });
  }

  function maybeForwardFollowerHostResponse(message) {
    const request = bridgeFollowerHostResponseRequest(message);
    if (!request) {
      return false;
    }
    void forwardFollowerHostResponseToCodexHost(message, request).catch((error) => {
      console.warn("[codex-web] bridge follower host response failed", error);
    });
    return true;
  }

  function recordBridgeEvent(message) {
    if (!message || typeof message !== "object") {
      return;
    }
    const events = (window.__codexWebBridgeEvents ||= []);
    events.push(message);
    if (events.length > 2048) {
      events.splice(0, events.length - 2048);
    }
    try {
      window.dispatchEvent(new CustomEvent("codex-web-bridge-event", { detail: message }));
    } catch {}
    try {
      window.parent?.postMessage(
        {
          event: stripBridgeMetadata(message),
          method: message.__codexEventMethod,
          source: message.__codexEventSource || "bridge",
          type: "codex-web-bridge-event",
        },
        window.location.origin,
      );
    } catch {}
  }

  function bridgeErrorMessage(message, error) {
    const text = error && error.message ? error.message : String(error);
    if (message && message.type === "fetch" && message.requestId) {
      return {
        type: "fetch-response",
        requestId: message.requestId,
        responseType: "error",
        status: 500,
        error: text,
      };
    }
    if (message && message.type === "fetch-stream" && message.requestId) {
      return {
        type: "fetch-stream-error",
        requestId: message.requestId,
        error: text,
      };
    }
    if (message && message.type === "mcp-request" && message.request?.id != null) {
      return {
        type: "mcp-response",
        hostId: message.hostId,
        message: {
          id: message.request.id,
          error: {
            code: -32000,
            message: text,
          },
        },
      };
    }
    return null;
  }

  function rejectPending(error) {
    for (const [id, entry] of pending) {
      window.clearTimeout(entry.timer);
      pending.delete(id);
      entry.reject(error);
    }
  }

	  function clearBridgeHeartbeatTimeout() {
	    if (bridgeHeartbeatTimeoutTimer) {
	      window.clearTimeout(bridgeHeartbeatTimeoutTimer);
	      bridgeHeartbeatTimeoutTimer = null;
	    }
	  }

	  function markBridgeConnectionAlive() {
	    bridgeLastHeartbeatAckAt = Date.now();
	    clearBridgeHeartbeatTimeout();
	  }

	  function closeBridgeConnection() {
	    clearBridgeHeartbeatTimeout();
	    if (parentBridgeOpen || connectingParentBridge) {
	      try {
	        postParentBridgeMessage({ type: PARENT_BRIDGE_CLOSE_MESSAGE });
	      } catch {}
	    }
	    parentBridgeOpen = false;
	    connectingParentBridge = null;
	    resolveParentBridge = null;
	    rejectParentBridge = null;
	    if (parentBridgeOpenTimer) {
	      window.clearTimeout(parentBridgeOpenTimer);
	      parentBridgeOpenTimer = null;
	    }
	    try {
	      parentBridgeStopHeartbeat?.();
	    } catch {}
	    parentBridgeStopHeartbeat = null;
	    try {
	      socket?.close();
	    } catch {}
	    try {
	      transportClient?.close();
	    } catch {}
	  }

	  function isBridgeConnectionStale() {
	    return (
	      bridgeLastHeartbeatAckAt > 0 &&
	      Date.now() - bridgeLastHeartbeatAckAt > BRIDGE_HEARTBEAT_STALE_MS
	    );
	  }

	  function startBridgeHeartbeat(sendHeartbeat) {
	    const timer = window.setInterval(() => {
	      Promise.resolve()
	        .then(() => sendHeartbeat(JSON.stringify({ type: "bridge-heartbeat" })))
	        .catch((error) => {
	          console.warn("[codex-web] bridge heartbeat failed", error);
	          closeBridgeConnection();
	          scheduleBridgeReconnect();
	        });
	      try {
	        clearBridgeHeartbeatTimeout();
	        bridgeHeartbeatTimeoutTimer = window.setTimeout(() => {
	          bridgeHeartbeatTimeoutTimer = null;
	          closeBridgeConnection();
	          scheduleBridgeReconnect();
	        }, BRIDGE_HEARTBEAT_TIMEOUT_MS);
	      } catch (error) {
	        console.warn("[codex-web] bridge heartbeat failed", error);
	        closeBridgeConnection();
	        scheduleBridgeReconnect();
	      }
	    }, BRIDGE_HEARTBEAT_INTERVAL_MS);
	    return () => {
	      window.clearInterval(timer);
	      clearBridgeHeartbeatTimeout();
	    };
	  }

	  function ensureParentBridgeListener() {
	    if (parentBridgeListenerInstalled) {
	      return;
	    }
	    parentBridgeListenerInstalled = true;
	    window.addEventListener("message", handleParentBridgeMessage);
	  }

	  function openParentBridgeConnection() {
	    if (parentBridgeOpen) {
	      return Promise.resolve(parentBridgeConnection());
	    }
	    if (connectingParentBridge) {
	      return connectingParentBridge;
	    }
	    ensureParentBridgeListener();
	    connectingParentBridge = new Promise((resolve, reject) => {
	      resolveParentBridge = resolve;
	      rejectParentBridge = reject;
	      if (parentBridgeOpenTimer) {
	        window.clearTimeout(parentBridgeOpenTimer);
	      }
	      parentBridgeOpenTimer = window.setTimeout(() => {
	        parentBridgeOpenTimer = null;
	        connectingParentBridge = null;
	        resolveParentBridge = null;
	        rejectParentBridge = null;
	        reject(new Error("Timed out waiting for parent Codex bridge"));
	      }, BRIDGE_HEARTBEAT_TIMEOUT_MS);
	      postParentBridgeMessage({ type: PARENT_BRIDGE_OPEN_MESSAGE });
	    });
	    return connectingParentBridge;
	  }

	  async function sendParentBridgeRaw(raw) {
	    if (!parentBridgeOpen) {
	      throw new Error("Parent Codex bridge is not open");
	    }
	    postParentBridgeMessage({
	      raw: await encryptBridgeText(raw),
	      type: PARENT_BRIDGE_SEND_MESSAGE,
	    });
	  }

	  function settleParentBridgeOpen() {
	    if (parentBridgeOpenTimer) {
	      window.clearTimeout(parentBridgeOpenTimer);
	      parentBridgeOpenTimer = null;
	    }
	    parentBridgeOpen = true;
	    connectingParentBridge = null;
	    const resolve = resolveParentBridge;
	    resolveParentBridge = null;
	    rejectParentBridge = null;
	    try {
	      parentBridgeStopHeartbeat?.();
	    } catch {}
	    parentBridgeStopHeartbeat = startBridgeHeartbeat(sendParentBridgeRaw);
	    markBridgeConnectionOpen();
	    resolve?.(parentBridgeConnection());
	  }

	  function handleParentBridgeClosed(reason) {
	    const error = new Error(reason || "Parent Codex bridge closed");
	    parentBridgeOpen = false;
	    if (parentBridgeOpenTimer) {
	      window.clearTimeout(parentBridgeOpenTimer);
	      parentBridgeOpenTimer = null;
	    }
	    try {
	      parentBridgeStopHeartbeat?.();
	    } catch {}
	    parentBridgeStopHeartbeat = null;
	    if (connectingParentBridge) {
	      const reject = rejectParentBridge;
	      connectingParentBridge = null;
	      resolveParentBridge = null;
	      rejectParentBridge = null;
	      reject?.(error);
	      return;
	    }
	    notifyBridgeStatus("disconnected");
	    rejectPending(error);
	    scheduleBridgeReconnect();
	  }

	  async function handleParentBridgeMessage(event) {
	    const message = event.data || {};
	    if (!message || message.clientId !== bridgeClientId) {
	      return;
	    }
	    if (message.type === PARENT_BRIDGE_OPENED_MESSAGE) {
	      settleParentBridgeOpen();
	      return;
	    }
	    if (message.type === PARENT_BRIDGE_MESSAGE) {
	      try {
	        handleBridgePayload(JSON.parse(await decryptBridgeText(String(message.raw || ""))));
	      } catch (error) {
	        console.warn("[codex-web] invalid parent bridge payload", error);
	      }
	      return;
	    }
	    if (message.type === PARENT_BRIDGE_CLOSED_MESSAGE || message.type === PARENT_BRIDGE_ERROR_MESSAGE) {
	      handleParentBridgeClosed(String(message.error || message.reason || ""));
	    }
	  }

	  function markBridgeConnectionOpen() {
	    markBridgeConnectionAlive();
	    bridgeReconnectDelayMs = BRIDGE_RECONNECT_MIN_DELAY_MS;
	    if (bridgeReconnectTimer) {
	      window.clearTimeout(bridgeReconnectTimer);
	      bridgeReconnectTimer = null;
	    }
	    notifyBridgeStatus("connected");
	    window.setTimeout(() => requestHostSnapshot("bridge-connected"), 0);
	  }

	  function scheduleBridgeReconnect() {
	    if (!bridgeConnectionStarted || bridgeReconnectTimer) {
	      return;
	    }
	    const delay = bridgeReconnectDelayMs;
	    notifyBridgeStatus("reconnecting", { delayMs: delay });
	    bridgeReconnectDelayMs = Math.min(
	      BRIDGE_RECONNECT_MAX_DELAY_MS,
	      Math.max(BRIDGE_RECONNECT_MIN_DELAY_MS, Math.floor(bridgeReconnectDelayMs * 1.6)),
	    );
	    bridgeReconnectTimer = window.setTimeout(() => {
	      bridgeReconnectTimer = null;
	      void warmBridgeConnection();
	    }, delay);
	  }

	  async function warmBridgeConnection() {
	    notifyBridgeStatus("connecting");
	    try {
	      const connection = await openBridgeConnection();
	      markBridgeConnectionOpen();
	      return connection;
	    } catch (error) {
	      scheduleBridgeReconnect();
	      return null;
	    }
	  }

	  function requestHostSnapshot(reason, options = {}) {
	    const force = options.force === true;
	    const now = Date.now();
	    if (!force && now - lastSnapshotRequestAt < BRIDGE_SNAPSHOT_REQUEST_COOLDOWN_MS) {
	      return;
	    }
	    lastSnapshotRequestAt = now;
	    void sendBridgeRequest({
	      type: "codex-web-bridge-request-snapshot",
	      clientId: bridgeClientId,
	      reason,
	    }).catch((error) => {
	      console.warn("[codex-web] bridge snapshot request failed", error);
	    });
	  }

	  function mcpRequestMethod(message) {
	    return message?.type === "mcp-request" && typeof message.request?.method === "string"
	      ? message.request.method
	      : null;
	  }

	  function mcpRequestId(message) {
	    return message?.type === "mcp-request" && message.request?.id != null
	      ? String(message.request.id)
	      : null;
	  }

	  function mcpResponseId(message) {
	    return message?.type === "mcp-response" && message.message?.id != null
	      ? String(message.message.id)
	      : null;
	  }

	  function isThreadHydrationRequest(message) {
	    return BRIDGE_THREAD_HYDRATION_METHODS.has(mcpRequestMethod(message));
	  }

	  function shouldRefreshStreamSnapshotAfterHostMessage(message) {
	    return isThreadHydrationRequest(message) || message?.type === "thread-role-request";
	  }

	  function scheduleHostSnapshotAfterViewHydration(message) {
	    if (!shouldRefreshStreamSnapshotAfterHostMessage(message)) {
	      return;
	    }
	    window.setTimeout(
	      () => requestHostSnapshot("remote-view-hydrated", { force: true }),
	      BRIDGE_HYDRATION_SNAPSHOT_DELAY_MS,
	    );
	  }

	  function shouldAskRemoteOwnerForSnapshot(message) {
	    return isBridgeNotification(message) && shouldRefreshStreamSnapshotAfterHostMessage(message);
	  }

	  function peerHydrationKey(hostId, requestId) {
	    return `${hostId || ""}:${requestId}`;
	  }

	  function dispatchRemoteOwnerSnapshotRequestForClient(clientId, reason) {
	    dispatchHostMessage({
	      type: "ipc-broadcast",
	      method: "client-status-changed",
	      sourceClientId: clientId,
	      version: 0,
	      params: {
	        clientId,
	        clientType: "web",
	        status: "connected",
	      },
	      reason,
	    });
	  }

	  function dispatchRemoteOwnerSnapshotRequest(message, reason) {
	    const clientId =
	      message.sourceClientId || message.clientId || "codex-web-bridge-peer-view";
	    dispatchRemoteOwnerSnapshotRequestForClient(clientId, reason);
	  }

	  function trackRemoteOwnerSnapshotRequest(message) {
	    if (!isBridgeNotification(message)) {
	      return;
	    }
	    if (shouldAskRemoteOwnerForSnapshot(message)) {
	      const requestId = mcpRequestId(message);
	      if (!requestId) {
	        window.setTimeout(
	          () => dispatchRemoteOwnerSnapshotRequest(message, "peer-view-history-loaded-fallback"),
	          BRIDGE_PEER_HYDRATION_SNAPSHOT_FALLBACK_MS,
	        );
	        return;
	      }
	      const key = peerHydrationKey(message.hostId, requestId);
	      const existing = peerHydrationRequests.get(key);
	      if (existing?.timer) {
	        window.clearTimeout(existing.timer);
	      }
	      const clientId =
	        message.sourceClientId || message.clientId || "codex-web-bridge-peer-view";
	      const timer = window.setTimeout(() => {
	        const pending = peerHydrationRequests.get(key);
	        if (!pending) {
	          return;
	        }
	        peerHydrationRequests.delete(key);
	        dispatchRemoteOwnerSnapshotRequestForClient(
	          pending.clientId,
	          "peer-view-history-loaded-fallback",
	        );
	      }, BRIDGE_PEER_HYDRATION_SNAPSHOT_FALLBACK_MS);
	      peerHydrationRequests.set(key, { clientId, timer });
	      return;
	    }
	    const responseId = mcpResponseId(message);
	    if (!responseId) {
	      return;
	    }
	    const key = peerHydrationKey(message.hostId, responseId);
	    const pending = peerHydrationRequests.get(key);
	    if (!pending) {
	      return;
	    }
	    peerHydrationRequests.delete(key);
	    if (pending.timer) {
	      window.clearTimeout(pending.timer);
	    }
	    dispatchRemoteOwnerSnapshotRequestForClient(
	      pending.clientId,
	      "peer-view-history-loaded",
	    );
	  }

	  function shouldDispatchNotification(message) {
	    const rawSeq = message?.__codexWebBridgeNotificationSeq;
	    const seq = typeof rawSeq === "number" ? rawSeq : Number(rawSeq);
	    if (!Number.isFinite(seq) || seq <= 0) {
	      return true;
	    }
	    if (lastNotificationSeq > 0 && seq <= lastNotificationSeq) {
	      return false;
	    }
	    if (lastNotificationSeq > 0 && seq !== lastNotificationSeq + 1) {
	      requestHostSnapshot("notification-sequence-gap", { force: true });
	    }
	    lastNotificationSeq = seq;
	    return true;
	  }

	  function handleBridgePayload(payload) {
	    markBridgeConnectionAlive();
	    if (payload?.type === "bridge-heartbeat-ack") {
	      return;
	    }
      bridgeDebug(
        "handleBridgePayload",
        `id=${payload?.id || ""}`,
        `messages=${Array.isArray(payload?.messages) ? payload.messages.length : 0}`,
        bridgePayloadLabels(payload),
        payload?.error ? `error=${payload.error}` : "",
      );
	    for (const hostMessage of payload.messages || []) {
	      if (hostMessage?.type === "codex-web-bridge-notification-gap") {
	        requestHostSnapshot(hostMessage.reason || "notification-gap", { force: true });
	        continue;
	      }
	      if (!shouldDispatchNotification(hostMessage)) {
	        continue;
	      }
	      recordBridgeEvent(hostMessage);
	      trackRemoteOwnerSnapshotRequest(hostMessage);
	      if (maybeForwardFollowerHostResponse(hostMessage)) {
	        continue;
	      }
	      if (
	        isBridgeNotification(hostMessage) &&
	        !shouldDispatchBridgeNotificationToCodexApp(hostMessage)
	      ) {
	        continue;
	      }
	      dispatchHostMessage(hostMessage);
	    }
    if (!payload.id) {
      return;
    }
    const entry = pending.get(String(payload.id));
    if (!entry) {
      return;
    }
    window.clearTimeout(entry.timer);
    pending.delete(String(payload.id));
    if (payload.error) {
      entry.reject(new Error(payload.error));
      return;
    }
	    entry.resolve(payload);
	  }

	  function shouldTryBridgeTransport() {
	    if (!bridgeTransportUrl || webTransportUnavailable) {
	      return false;
	    }
	    if (transportPreference === "websocket" || transportPreference === "ws") {
	      return false;
	    }
	    return typeof WebTransport === "function";
	  }

	  function openBridgeTransport() {
	    if (transportClient && !transportClient.closed) {
	      return Promise.resolve(transportClient);
	    }
	    if (connectingTransport) {
	      return connectingTransport;
	    }
	    connectingTransport = createBridgeTransportClient().then(
	      (client) => {
	        connectingTransport = null;
	        transportClient = client;
	        return client;
	      },
	      (error) => {
	        connectingTransport = null;
	        transportClient = null;
	        throw error;
	      },
	    );
	    return connectingTransport;
	  }

	  async function createBridgeTransportClient() {
	    const transport = new WebTransport(bridgeTransportUrl.href, {
	      congestionControl: "low-latency",
	      requireUnreliable: false,
	    });
	    let reader = null;
	    let ready = false;
	    let stopHeartbeat = null;
	    let writer = null;
	    const close = () => {
	      if (client.closed) {
	        return;
	      }
	      const wasReady = ready;
	      const isCurrentClient = transportClient === client;
	      client.closed = true;
	      if (isCurrentClient) {
	        transportClient = null;
	      }
	      try {
	        reader?.cancel();
	      } catch {}
	      try {
	        writer?.close();
	      } catch {}
	      try {
	        transport.close();
	      } catch {}
	      try {
	        stopHeartbeat?.();
	      } catch {}
	      if (wasReady && isCurrentClient) {
	        notifyBridgeStatus("disconnected");
	        rejectPending(new Error("Codex bridge WebTransport closed"));
	        scheduleBridgeReconnect();
	      }
	    };
	    const client = {
	      closed: false,
	      close,
	      send(raw) {
	        if (client.closed || !writer) {
	          throw new Error("Codex bridge WebTransport is not open");
	        }
	        writeLengthPrefixedBridgeTransport(writer, raw).catch((error) => {
	          close();
	          console.warn("[codex-web] bridge WebTransport send failed", error);
	        });
	      },
	    };
	    try {
	      await transport.ready;
	      const stream = await transport.createBidirectionalStream();
	      reader = stream.readable.getReader();
	      writer = stream.writable.getWriter();
	      ready = true;
	      stopHeartbeat = startBridgeHeartbeat((raw) => client.send(raw));
	      markBridgeConnectionOpen();
	    } catch (error) {
	      close();
	      throw error;
	    }
	    readLengthPrefixedBridgeTransport(reader, (payload) => {
	      try {
	        handleBridgePayload(JSON.parse(payload));
	      } catch (error) {
	        console.warn("[codex-web] invalid bridge WebTransport payload", error);
	      }
	    }).then(close, close);
	    transport.closed.then(close, close);
	    return client;
	  }

	  async function readLengthPrefixedBridgeTransport(reader, onPayload) {
	    const decoder = new TextDecoder();
	    let buffer = new Uint8Array(0);
	    for (;;) {
	      const { done, value } = await reader.read();
	      if (done) {
	        return;
	      }
	      buffer = concatBridgeBytes(buffer, bridgeBytesFromStreamValue(value));
	      while (buffer.byteLength >= 4) {
	        const payloadLength = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength).getUint32(0);
	        if (buffer.byteLength < 4 + payloadLength) {
	          break;
	        }
	        const payload = buffer.slice(4, 4 + payloadLength);
	        buffer = buffer.slice(4 + payloadLength);
	        const text = await decryptBridgeText(decoder.decode(payload));
	        await onPayload(text);
	      }
	    }
	  }

	  async function writeLengthPrefixedBridgeTransport(writer, text) {
	    text = await encryptBridgeText(text);
	    const payload = new TextEncoder().encode(text);
	    const packet = new Uint8Array(4 + payload.byteLength);
	    new DataView(packet.buffer).setUint32(0, payload.byteLength);
	    packet.set(payload, 4);
	    return writer.write(packet);
	  }

	  function bridgeBytesFromStreamValue(value) {
	    if (value instanceof Uint8Array) {
	      return value;
	    }
	    if (value instanceof ArrayBuffer) {
	      return new Uint8Array(value);
	    }
	    if (ArrayBuffer.isView(value)) {
	      return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
	    }
	    return new Uint8Array(0);
	  }

	  function concatBridgeBytes(left, right) {
	    if (left.byteLength === 0) {
	      return right;
	    }
	    if (right.byteLength === 0) {
	      return left;
	    }
	    const bytes = new Uint8Array(left.byteLength + right.byteLength);
	    bytes.set(left, 0);
	    bytes.set(right, left.byteLength);
	    return bytes;
	  }

	  async function openBridgeConnection() {
	    if (shouldUseParentBridge()) {
	      return await openParentBridgeConnection();
	    }
	    if (shouldTryBridgeTransport()) {
	      try {
	        return await openBridgeTransport();
	      } catch (error) {
	        webTransportUnavailable = true;
	        console.info("[codex-web] bridge WebTransport unavailable, falling back to WebSocket", error);
	      }
	    }
	    const ws = await openBridgeSocket();
	    return {
	      send(raw) {
	        return sendBridgeSocketRaw(ws, raw);
	      },
	    };
	  }

	  function bridgeRequiresCrypto() {
	    return (
	      bridgeUrl.searchParams.get("e2ee") === "v1" ||
	      bridgeUrl.searchParams.get("requirePassword") === "1" ||
	      bridgeTransportUrl?.searchParams.get("e2ee") === "v1" ||
	      bridgeTransportUrl?.searchParams.get("requirePassword") === "1"
	    );
	  }

	  async function bridgeCryptoKey() {
	    if (!bridgeRequiresCrypto()) {
	      return null;
	    }
	    if (!bridgeCryptoPromise) {
	      bridgeCryptoPromise = (async () => {
	        const token = bridgeUrl.searchParams.get("token") || pageParams.get("token") || "";
	        const rawKey = window.sessionStorage?.getItem(`${E2EE_STORAGE_PREFIX}${token}`) || "";
	        if (!rawKey || !window.crypto?.subtle) {
	          throw new Error("Codex bridge password key is missing");
	        }
	        return window.crypto.subtle.importKey(
	          "raw",
	          base64UrlDecode(rawKey),
	          { name: "AES-GCM" },
	          false,
	          ["decrypt", "encrypt"],
	        );
	      })();
	    }
	    return bridgeCryptoPromise;
	  }

	  async function encryptBridgeText(raw) {
	    const key = await bridgeCryptoKey();
	    if (!key) {
	      return raw;
	    }
	    const nonce = window.crypto.getRandomValues(new Uint8Array(12));
	    const encrypted = new Uint8Array(
	      await window.crypto.subtle.encrypt(
	        { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
	        key,
	        new TextEncoder().encode(String(raw || "")),
	      ),
	    );
	    return JSON.stringify({
	      type: "e2ee",
	      version: 1,
	      nonce: base64UrlEncode(nonce),
	      payload: base64UrlEncode(encrypted),
	    });
	  }

	  async function decryptBridgeText(raw) {
	    const key = await bridgeCryptoKey();
	    if (!key) {
	      return raw;
	    }
	    const envelope = JSON.parse(String(raw || ""));
	    if (envelope?.type !== "e2ee" || envelope.version !== 1) {
	      throw new Error("Encrypted Codex bridge payload is required");
	    }
	    const decrypted = await window.crypto.subtle.decrypt(
	      {
	        additionalData: E2EE_AAD,
	        iv: base64UrlDecode(String(envelope.nonce || "")),
	        name: "AES-GCM",
	      },
	      key,
	      base64UrlDecode(String(envelope.payload || "")),
	    );
	    return new TextDecoder().decode(decrypted);
	  }

	  function base64UrlEncode(value) {
	    let binary = "";
	    for (const byte of value) {
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

	  async function sendBridgeSocketRaw(ws, raw) {
	    if (ws.readyState !== WebSocket.OPEN) {
	      throw new Error("Codex bridge websocket is not open");
	    }
	    ws.send(await encryptBridgeText(raw));
	  }

	  function openBridgeSocket() {
	    if (socket && socket.readyState === WebSocket.OPEN) {
	      return Promise.resolve(socket);
    }
    if (connectingSocket) {
      return connectingSocket;
    }
    connectingSocket = new Promise((resolve, reject) => {
      let opened = false;
      let stopHeartbeat = null;
      const ws = new WebSocket(bridgeUrl.href);
      socket = ws;

      ws.addEventListener("open", () => {
        opened = true;
        connectingSocket = null;
        stopHeartbeat = startBridgeHeartbeat((raw) => sendBridgeSocketRaw(ws, raw));
        markBridgeConnectionOpen();
        resolve(ws);
      });
      ws.addEventListener("message", (event) => {
        void handleBridgeSocketMessage(event.data);
      });
      const handleBridgeSocketMessage = async (data) => {
        try {
          handleBridgePayload(JSON.parse(await decryptBridgeText(data)));
        } catch (error) {
          console.warn("[codex-web] invalid bridge websocket payload", error);
        }
      };
      ws.addEventListener("close", () => {
        const error = new Error("Codex bridge websocket closed");
        const isCurrentSocket = socket === ws;
        if (isCurrentSocket) {
          socket = null;
        }
        try {
          stopHeartbeat?.();
        } catch {}
        if (!opened) {
          connectingSocket = null;
          reject(error);
        }
        if (isCurrentSocket) {
          notifyBridgeStatus("disconnected");
          rejectPending(error);
        }
        if (opened && isCurrentSocket) {
          scheduleBridgeReconnect();
        }
      });
      ws.addEventListener("error", () => {
        if (!opened) {
          connectingSocket = null;
          socket = null;
          reject(new Error("Codex bridge websocket failed to connect"));
        }
      });
    });
    return connectingSocket;
  }

	  function bridgeRequestTimeoutMs(message) {
	    if (message?.type === "fetch-stream") {
	      return BRIDGE_STREAM_REQUEST_TIMEOUT_MS;
	    }
	    if (message?.type === "mcp-request") {
	      return BRIDGE_LONG_REQUEST_TIMEOUT_MS;
	    }
	    return BRIDGE_REQUEST_TIMEOUT_MS;
	  }

	  async function sendBridgeRequest(message) {
	    bridgeConnectionStarted = true;
	    if (isBridgeConnectionStale()) {
	      closeBridgeConnection();
	    }
	    const id = String(nextMessageId++);
      bridgeDebug("sendBridgeRequest", `id=${id}`, bridgeMessageLabel(message));
	    const timeoutMs = bridgeRequestTimeoutMs(message);
	    const pendingResponse = new Promise((resolve, reject) => {
	      const timer = window.setTimeout(() => {
	        pending.delete(id);
	        reject(new Error("Timed out waiting for Codex bridge websocket response"));
	      }, timeoutMs);
	      pending.set(id, { message, reject, resolve, timer });
	    });
	    try {
	      const connection = await openBridgeConnection();
	      await connection.send(JSON.stringify({ id, message }));
	    } catch (error) {
	      const entry = pending.get(id);
	      if (entry) {
	        window.clearTimeout(entry.timer);
	        pending.delete(id);
	        entry.reject(error);
	      }
        bridgeDebug("sendBridgeRequest failed", `id=${id}`, bridgeMessageLabel(message), error?.message || String(error));
	      scheduleBridgeReconnect();
	    }
	    return pendingResponse;
	  }

  function deliverCodexLPluginBridgeResponse(response, fallbackId = null) {
    const normalized =
      response && typeof response === "object"
        ? response
        : {
            id: fallbackId,
            ok: false,
            error: "Invalid CodexL plugin bridge response",
          };
    const deliver = () => {
      const runtime = window.__codexlPluginRuntime;
      if (!runtime || typeof runtime.acceptCdpResponse !== "function") {
        return false;
      }
      runtime.acceptCdpResponse(normalized);
      return true;
    };
    if (!deliver()) {
      window.setTimeout(deliver, 0);
    }
  }

  function installCodexLPluginBridge() {
    if (window.__codexlPluginBridge?.__codexlRemoteWebBridge) {
      return;
    }
    const bridge = (raw) => {
      let pluginRequest;
      let pluginRequestId = null;
      try {
        pluginRequest = JSON.parse(String(raw || "{}"));
        if (pluginRequest && pluginRequest.id != null) {
          pluginRequestId = String(pluginRequest.id);
        }
      } catch (error) {
        deliverCodexLPluginBridgeResponse({
          id: null,
          ok: false,
          error: `Invalid CodexL plugin request: ${error?.message || String(error)}`,
        });
        return false;
      }
      void sendBridgeRequest({
        type: "codexl-plugin-bridge",
        pluginRequest,
      })
        .then((payload) => {
          deliverCodexLPluginBridgeResponse(payload?.codexlPluginResponse, pluginRequestId);
        })
        .catch((error) => {
          deliverCodexLPluginBridgeResponse({
            id: pluginRequestId,
            ok: false,
            error: error?.message || String(error),
          });
        });
      return true;
    };
    try {
      Object.defineProperty(bridge, "__codexlRemoteWebBridge", {
        configurable: true,
        value: true,
      });
      Object.defineProperty(window, "__codexlPluginBridge", {
        configurable: true,
        value: bridge,
        writable: true,
      });
    } catch {
      bridge.__codexlRemoteWebBridge = true;
      window.__codexlPluginBridge = bridge;
    }
  }

  function codexLCurrentBridgeScriptUrl() {
    const current = document.currentScript;
    if (current?.src) {
      return current.src;
    }
    const scripts = Array.from(document.scripts || []).reverse();
    const bridgeScript = scripts.find((script) =>
      /(?:^|\/)_(?:codexl_)?bridge\.js(?:[?#]|$)/.test(script.src || "")
    );
    return bridgeScript?.src || "";
  }

  function codexLPluginRuntimeScriptUrl() {
    const configuredUrl =
      pageParams.get("codexlPluginRuntimeUrl") || pageParams.get("codexlRuntimeUrl");
    if (configuredUrl) {
      try {
        return new URL(configuredUrl, window.location.href).toString();
      } catch {}
    }
    const configuredBaseUrl = pageParams.get("codexlRuntimeBaseUrl");
    if (configuredBaseUrl) {
      try {
        const baseUrl = new URL(
          configuredBaseUrl.endsWith("/") ? configuredBaseUrl : `${configuredBaseUrl}/`,
          window.location.href
        );
        return new URL("_codexl_plugin.js", baseUrl).toString();
      } catch {}
    }
    const bridgeScriptUrl = codexLCurrentBridgeScriptUrl();
    if (bridgeScriptUrl) {
      try {
        const url = new URL(bridgeScriptUrl, window.location.href);
        url.search = "";
        url.hash = "";
        if (url.pathname.endsWith("/_codexl_bridge.js")) {
          url.pathname = url.pathname.replace(/_codexl_bridge\.js$/, "_codexl_plugin.js");
          return url.toString();
        }
        if (url.pathname.endsWith("/_bridge.js")) {
          url.pathname = url.pathname.replace(/_bridge\.js$/, "_codexl_plugin.js");
          return url.toString();
        }
        return new URL("_codexl_plugin.js", url).toString();
      } catch {}
    }
    try {
      return new URL("./_codexl_plugin.js", window.location.href).toString();
    } catch {
      return "";
    }
  }

  function codexLScriptUrlKey(src) {
    try {
      const url = new URL(src, window.location.href);
      url.search = "";
      url.hash = "";
      return url.toString();
    } catch {
      return String(src || "").split('#')[0].split('?')[0];
    }
  }

  function hasCodexLPluginRuntimeScript(src) {
    if (window.__codexlPluginRuntime && !window.__codexlPluginRuntime.closed) {
      return true;
    }
    const targetKey = codexLScriptUrlKey(src);
    return Array.from(document.scripts || []).some((script) => {
      const scriptSrc = script.src || script.getAttribute("src") || "";
      if (!scriptSrc) {
        return false;
      }
      if (script.dataset?.codexlRuntime === "plugin") {
        return true;
      }
      if (codexLScriptUrlKey(scriptSrc) === targetKey) {
        return true;
      }
      return /(?:^|\/)_codexl_plugin\.js(?:[?#]|$)/.test(scriptSrc);
    });
  }

  function escapeCodexLHtmlAttribute(value) {
    return String(value)
      .replace(/&/g, "&amp;")
      .replace(/"/g, "&quot;")
      .replace(/</g, "&lt;");
  }

  function installCodexLPluginRuntimeEntry() {
    if (window.__codexlPluginRuntimeEntryInstalled) {
      return;
    }
    const src = codexLPluginRuntimeScriptUrl();
    if (!src || hasCodexLPluginRuntimeScript(src)) {
      return;
    }
    window.__codexlPluginRuntimeEntryInstalled = true;
    if (document.readyState === "loading" && document.currentScript) {
      document.write(
        `<script src="${escapeCodexLHtmlAttribute(src)}" data-codexl-runtime="plugin"><\/script>`
      );
      return;
    }
    const script = document.createElement("script");
    script.src = src;
    script.async = false;
    script.dataset.codexlRuntime = "plugin";
    (document.head || document.documentElement).appendChild(script);
  }

  const LUCIDE_ICON_PATHS = {
    alertCircle: [
      ["circle", { cx: "12", cy: "12", r: "10" }],
      ["line", { x1: "12", x2: "12", y1: "8", y2: "12" }],
      ["line", { x1: "12", x2: "12.01", y1: "16", y2: "16" }],
    ],
    arrowUp: [
      ["path", { d: "m5 12 7-7 7 7" }],
      ["path", { d: "M12 19V5" }],
    ],
    check: [["path", { d: "M20 6 9 17l-5-5" }]],
    chevronRight: [["path", { d: "m9 18 6-6-6-6" }]],
    cornerDownRight: [
      ["polyline", { points: "15 10 20 15 15 20" }],
      ["path", { d: "M4 4v7a4 4 0 0 0 4 4h12" }],
    ],
    file: [
      ["path", { d: "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" }],
      ["path", { d: "M14 2v4a2 2 0 0 0 2 2h4" }],
    ],
    folder: [
      [
        "path",
        {
          d: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9l-.81-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z",
        },
      ],
    ],
    folderOpen: [
      [
        "path",
        {
          d: "m6 14 1.5-2.9A2 2 0 0 1 9.24 9H20a2 2 0 0 1 1.75 2.96l-2.5 4.55A2 2 0 0 1 17.5 18H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2A2 2 0 0 0 12.07 6H20a2 2 0 0 1 2 2v1",
        },
      ],
    ],
    hardDrive: [
      ["line", { x1: "22", x2: "2", y1: "12", y2: "12" }],
      [
        "path",
        {
          d: "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
        },
      ],
      ["line", { x1: "6", x2: "6.01", y1: "16", y2: "16" }],
      ["line", { x1: "10", x2: "10.01", y1: "16", y2: "16" }],
    ],
    home: [
      ["path", { d: "m3 9 9-7 9 7" }],
      ["path", { d: "M9 22V12h6v10" }],
      ["path", { d: "M21 9v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9" }],
    ],
    loaderCircle: [["path", { d: "M21 12a9 9 0 1 1-6.219-8.56" }]],
    refreshCw: [
      ["path", { d: "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" }],
      ["path", { d: "M21 3v5h-5" }],
      ["path", { d: "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" }],
      ["path", { d: "M8 16H3v5" }],
    ],
    x: [
      ["path", { d: "M18 6 6 18" }],
      ["path", { d: "m6 6 12 12" }],
    ],
  };

  function createLucideIcon(name, className = "") {
    const icon = LUCIDE_ICON_PATHS[name] || LUCIDE_ICON_PATHS.folder;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("data-lucide", name);
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "16");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "2");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("aria-hidden", "true");
    if (className) {
      svg.setAttribute("class", className);
    }
    for (const [tag, attrs] of icon) {
      const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
      for (const [key, value] of Object.entries(attrs)) {
        node.setAttribute(key, value);
      }
      svg.appendChild(node);
    }
    return svg;
  }

  function createWebPickerButton({
    className = "",
    icon,
    label,
    size = "default",
    title,
    variant = "outline",
  } = {}) {
    const button = document.createElement("button");
    button.className = [
      "codex-web-folder-picker-button",
      `codex-web-folder-picker-button-${variant}`,
      `codex-web-folder-picker-button-size-${size}`,
      className,
    ]
      .filter(Boolean)
      .join(" ");
    button.type = "button";
    button.dataset.slot = "button";
    if (title) {
      button.title = title;
      button.setAttribute("aria-label", title);
    }
    if (icon) {
      button.appendChild(createLucideIcon(icon, "codex-web-folder-picker-button-icon"));
    }
    if (label) {
      const labelNode = document.createElement("span");
      labelNode.className = "codex-web-folder-picker-button-label";
      labelNode.textContent = label;
      button.appendChild(labelNode);
    }
    return button;
  }

  function webFolderPickerBreadcrumbs(path) {
    const value = String(path || "");
    if (!value) {
      return [];
    }
    const separator = value.includes("\\") ? "\\" : "/";
    const segments = [];
    const driveMatch = value.match(/^[A-Za-z]:[\\/]?/);
    let rest = value;
    let current = "";
    if (driveMatch) {
      current = driveMatch[0].replace(/[\\/]$/, "");
      rest = value.slice(driveMatch[0].length);
      segments.push({ icon: "hardDrive", label: current, path: `${current}${separator}` });
    } else if (value.startsWith("/")) {
      current = "";
      rest = value.slice(1);
      segments.push({ icon: "home", label: "/", path: "/" });
    }
    for (const part of rest.split(/[\\/]+/).filter(Boolean)) {
      current = current
        ? `${current}${separator}${part}`
        : value.startsWith("/")
          ? `${separator}${part}`
          : part;
      segments.push({ label: part, path: current });
    }
    return segments;
  }

  function ensureWebFolderPickerStyle() {
    if (document.getElementById("codex-web-folder-picker-style")) {
      return;
    }
    const style = document.createElement("style");
    style.id = "codex-web-folder-picker-style";
    style.textContent = `
      .codex-web-folder-picker-backdrop {
        --codex-picker-background: var(--background, #ffffff);
        --codex-picker-foreground: var(--foreground, #0f172a);
        --codex-picker-muted: var(--muted, #f4f4f5);
        --codex-picker-muted-foreground: var(--muted-foreground, #71717a);
        --codex-picker-border: var(--border, rgba(24, 24, 27, 0.12));
        --codex-picker-input: var(--input, rgba(24, 24, 27, 0.16));
        --codex-picker-primary: var(--primary, #18181b);
        --codex-picker-primary-foreground: var(--primary-foreground, #fafafa);
        --codex-picker-accent: var(--accent, #f4f4f5);
        --codex-picker-accent-foreground: var(--accent-foreground, #18181b);
        --codex-picker-destructive: var(--destructive, #dc2626);
        --codex-picker-ring: var(--ring, #71717a);
        position: fixed;
        inset: 0;
        z-index: 2147483647;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 16px;
        background: rgba(9, 9, 11, 0.54);
        color: var(--codex-picker-foreground);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        backdrop-filter: blur(3px);
      }
      .codex-web-folder-picker-panel {
        width: min(780px, calc(100vw - 24px));
        height: min(680px, calc(100vh - 24px));
        display: flex;
        flex-direction: column;
        overflow: hidden;
        border: 1px solid var(--codex-picker-border);
        border-radius: 8px;
        background: var(--codex-picker-background);
        box-shadow: 0 24px 72px rgba(9, 9, 11, 0.32);
      }
      .codex-web-folder-picker-header {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 16px 18px;
        border-bottom: 1px solid var(--codex-picker-border);
      }
      .codex-web-folder-picker-title {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: 10px;
        font-size: 16px;
        font-weight: 600;
        letter-spacing: 0;
      }
      .codex-web-folder-picker-title-icon {
        width: 18px;
        height: 18px;
        color: var(--codex-picker-muted-foreground);
      }
      .codex-web-folder-picker-toolbar {
        display: flex;
        align-items: center;
        gap: 6px;
      }
      .codex-web-folder-picker-path-row {
        display: flex;
        gap: 8px;
        padding: 12px 18px 8px;
      }
      .codex-web-folder-picker-input-shell {
        flex: 1;
        min-width: 0;
        height: 36px;
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 0 10px;
        border: 1px solid var(--codex-picker-input);
        border-radius: 6px;
        background: var(--codex-picker-background);
      }
      .codex-web-folder-picker-input-shell:focus-within {
        border-color: var(--codex-picker-ring);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--codex-picker-ring) 18%, transparent);
      }
      .codex-web-folder-picker-input-icon {
        width: 15px;
        height: 15px;
        flex: 0 0 auto;
        color: var(--codex-picker-muted-foreground);
      }
      .codex-web-folder-picker-path {
        width: 100%;
        min-width: 0;
        height: 34px;
        border: 0;
        outline: 0;
        background: transparent;
        color: var(--codex-picker-foreground);
        font: inherit;
        font-size: 13px;
      }
      .codex-web-folder-picker-button {
        height: 36px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 7px;
        white-space: nowrap;
        border: 1px solid transparent;
        border-radius: 6px;
        font: inherit;
        font-size: 13px;
        font-weight: 500;
        line-height: 1;
        cursor: pointer;
        transition: background-color 120ms ease, border-color 120ms ease, color 120ms ease, opacity 120ms ease;
      }
      .codex-web-folder-picker-button:focus-visible {
        outline: 2px solid var(--codex-picker-ring);
        outline-offset: 2px;
      }
      .codex-web-folder-picker-button:disabled {
        cursor: default;
        opacity: 0.5;
      }
      .codex-web-folder-picker-button-default {
        padding: 0 14px;
        border-color: var(--codex-picker-primary);
        background: var(--codex-picker-primary);
        color: var(--codex-picker-primary-foreground);
      }
      .codex-web-folder-picker-button-default:hover:not(:disabled) {
        opacity: 0.92;
      }
      .codex-web-folder-picker-button-outline {
        padding: 0 12px;
        border-color: var(--codex-picker-border);
        background: var(--codex-picker-background);
        color: var(--codex-picker-foreground);
      }
      .codex-web-folder-picker-button-outline:hover:not(:disabled),
      .codex-web-folder-picker-button-ghost:hover:not(:disabled) {
        background: var(--codex-picker-accent);
        color: var(--codex-picker-accent-foreground);
      }
      .codex-web-folder-picker-button-ghost {
        padding: 0 10px;
        background: transparent;
        color: var(--codex-picker-foreground);
      }
      .codex-web-folder-picker-button-icon-only {
        width: 36px;
        padding: 0;
      }
      .codex-web-folder-picker-button-icon {
        width: 16px;
        height: 16px;
        flex: 0 0 auto;
      }
      .codex-web-folder-picker-breadcrumbs {
        display: flex;
        align-items: center;
        gap: 2px;
        min-height: 32px;
        padding: 0 18px 10px;
        overflow-x: auto;
        border-bottom: 1px solid var(--codex-picker-border);
      }
      .codex-web-folder-picker-breadcrumb {
        height: 26px;
        max-width: 180px;
        display: inline-flex;
        align-items: center;
        gap: 6px;
        flex: 0 0 auto;
        border: 0;
        border-radius: 6px;
        padding: 0 8px;
        background: transparent;
        color: var(--codex-picker-muted-foreground);
        font: inherit;
        font-size: 12px;
        cursor: pointer;
      }
      .codex-web-folder-picker-breadcrumb:hover {
        background: var(--codex-picker-accent);
        color: var(--codex-picker-accent-foreground);
      }
      .codex-web-folder-picker-breadcrumb-current {
        color: var(--codex-picker-foreground);
        font-weight: 500;
      }
      .codex-web-folder-picker-breadcrumb-label {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .codex-web-folder-picker-breadcrumb-separator {
        width: 14px;
        height: 14px;
        flex: 0 0 auto;
        color: var(--codex-picker-muted-foreground);
      }
      .codex-web-folder-picker-list {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 8px;
        background: color-mix(in srgb, var(--codex-picker-muted) 48%, var(--codex-picker-background));
      }
      .codex-web-folder-picker-row {
        width: 100%;
        min-height: 44px;
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 8px 10px;
        border: 1px solid transparent;
        border-radius: 6px;
        background: transparent;
        color: var(--codex-picker-foreground);
        font: inherit;
        font-size: 13px;
        text-align: left;
        cursor: pointer;
      }
      .codex-web-folder-picker-row:hover,
      .codex-web-folder-picker-row:focus-visible {
        border-color: var(--codex-picker-border);
        background: var(--codex-picker-background);
        outline: 0;
      }
      .codex-web-folder-picker-row-selected {
        border-color: var(--codex-picker-ring);
        background: var(--codex-picker-background);
      }
      .codex-web-folder-picker-icon {
        width: 18px;
        height: 18px;
        flex: 0 0 18px;
        color: var(--codex-picker-muted-foreground);
      }
      .codex-web-folder-picker-row-chevron {
        width: 16px;
        height: 16px;
        flex: 0 0 16px;
        color: var(--codex-picker-muted-foreground);
      }
      .codex-web-folder-picker-row-body {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
      }
      .codex-web-folder-picker-name {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
      }
      .codex-web-folder-picker-entry-path {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        color: var(--codex-picker-muted-foreground);
        font-size: 12px;
      }
      .codex-web-folder-picker-status {
        min-height: 36px;
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 0 18px;
        color: var(--codex-picker-muted-foreground);
        font-size: 12px;
        border-top: 1px solid var(--codex-picker-border);
      }
      .codex-web-folder-picker-status-error {
        color: var(--codex-picker-destructive);
      }
      .codex-web-folder-picker-actions {
        display: flex;
        align-items: center;
        gap: 8px;
        justify-content: flex-end;
        padding: 12px 18px;
        border-top: 1px solid var(--codex-picker-border);
      }
      .codex-web-folder-picker-empty,
      .codex-web-folder-picker-loading {
        min-height: 220px;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 10px;
        color: var(--codex-picker-muted-foreground);
        text-align: center;
        font-size: 13px;
      }
      .codex-web-folder-picker-empty svg,
      .codex-web-folder-picker-loading svg {
        width: 28px;
        height: 28px;
      }
      .codex-web-folder-picker-spin {
        animation: codex-web-folder-picker-spin 0.9s linear infinite;
      }
      @keyframes codex-web-folder-picker-spin {
        to {
          transform: rotate(360deg);
        }
      }
      @media (prefers-color-scheme: dark) {
        .codex-web-folder-picker-backdrop {
          --codex-picker-background: var(--background, #09090b);
          --codex-picker-foreground: var(--foreground, #fafafa);
          --codex-picker-muted: var(--muted, #18181b);
          --codex-picker-muted-foreground: var(--muted-foreground, #a1a1aa);
          --codex-picker-border: var(--border, rgba(250, 250, 250, 0.12));
          --codex-picker-input: var(--input, rgba(250, 250, 250, 0.14));
          --codex-picker-primary: var(--primary, #fafafa);
          --codex-picker-primary-foreground: var(--primary-foreground, #18181b);
          --codex-picker-accent: var(--accent, #18181b);
          --codex-picker-accent-foreground: var(--accent-foreground, #fafafa);
        }
      }
      @media (max-width: 640px) {
        .codex-web-folder-picker-backdrop {
          padding: 8px;
          align-items: stretch;
        }
        .codex-web-folder-picker-panel {
          width: 100%;
          height: 100%;
          max-height: none;
        }
        .codex-web-folder-picker-path-row {
          flex-wrap: wrap;
        }
        .codex-web-folder-picker-input-shell {
          flex-basis: 100%;
        }
        .codex-web-folder-picker-actions {
          flex-direction: column-reverse;
        }
        .codex-web-folder-picker-actions .codex-web-folder-picker-button {
          width: 100%;
        }
      }
    `;
    document.head.appendChild(style);
  }

  async function requestWebFolderPickerDirectory(path, { imagesOnly = false, mode = "directory" } = {}) {
    const payload = await sendBridgeRequest({
      type: "web-file-picker-list",
      imagesOnly: !!imagesOnly,
      mode,
      path: typeof path === "string" ? path : "",
    });
    return payload.value || {};
  }

  let activeFolderPicker = null;

  function showWebFolderPicker({
    imagesOnly = false,
    mode = "directory",
    multiple = false,
    selectLabel = null,
    title = "Choose project folder",
  } = {}) {
    if (activeFolderPicker) {
      return activeFolderPicker;
    }
    const fileMode = mode === "file";
    activeFolderPicker = new Promise((resolve) => {
      ensureWebFolderPickerStyle();
      const backdrop = document.createElement("div");
      backdrop.className = "codex-web-folder-picker-backdrop";
      backdrop.dataset.slot = "dialog-overlay";

      const panel = document.createElement("div");
      panel.className = "codex-web-folder-picker-panel";
      panel.dataset.slot = "dialog-content";
      panel.setAttribute("role", "dialog");
      panel.setAttribute("aria-modal", "true");
      backdrop.appendChild(panel);

      const header = document.createElement("div");
      header.className = "codex-web-folder-picker-header";
      header.dataset.slot = "dialog-header";
      panel.appendChild(header);

      const titleNode = document.createElement("div");
      titleNode.className = "codex-web-folder-picker-title";
      titleNode.dataset.slot = "dialog-title";
      titleNode.appendChild(createLucideIcon(fileMode ? "file" : "folderOpen", "codex-web-folder-picker-title-icon"));
      const titleText = document.createElement("span");
      titleText.textContent = title;
      titleNode.appendChild(titleText);
      header.appendChild(titleNode);

      const toolbar = document.createElement("div");
      toolbar.className = "codex-web-folder-picker-toolbar";
      header.appendChild(toolbar);

      const homeButton = createWebPickerButton({
        className: "codex-web-folder-picker-button-icon-only",
        icon: "home",
        size: "icon",
        title: "Home",
        variant: "ghost",
      });
      toolbar.appendChild(homeButton);

      const upButton = createWebPickerButton({
        className: "codex-web-folder-picker-button-icon-only",
        icon: "arrowUp",
        size: "icon",
        title: "Parent folder",
        variant: "ghost",
      });
      toolbar.appendChild(upButton);

      const refreshButton = createWebPickerButton({
        className: "codex-web-folder-picker-button-icon-only",
        icon: "refreshCw",
        size: "icon",
        title: "Refresh",
        variant: "ghost",
      });
      toolbar.appendChild(refreshButton);

      const closeButton = createWebPickerButton({
        className: "codex-web-folder-picker-button-icon-only",
        icon: "x",
        size: "icon",
        title: "Cancel",
        variant: "ghost",
      });
      toolbar.appendChild(closeButton);

      const pathRow = document.createElement("div");
      pathRow.className = "codex-web-folder-picker-path-row";
      panel.appendChild(pathRow);

      const pathShell = document.createElement("div");
      pathShell.className = "codex-web-folder-picker-input-shell";
      pathShell.dataset.slot = "input";
      pathRow.appendChild(pathShell);

      pathShell.appendChild(createLucideIcon("hardDrive", "codex-web-folder-picker-input-icon"));

      const pathInput = document.createElement("input");
      pathInput.className = "codex-web-folder-picker-path";
      pathInput.dataset.slot = "input";
      pathInput.type = "text";
      pathInput.spellcheck = false;
      pathInput.setAttribute("aria-label", fileMode ? "Directory path" : "Folder path");
      pathShell.appendChild(pathInput);

      const goButton = createWebPickerButton({
        icon: "cornerDownRight",
        label: "Go",
        title: "Open path",
        variant: "outline",
      });
      pathRow.appendChild(goButton);

      const breadcrumbs = document.createElement("div");
      breadcrumbs.className = "codex-web-folder-picker-breadcrumbs";
      breadcrumbs.dataset.slot = "breadcrumb";
      panel.appendChild(breadcrumbs);

      const list = document.createElement("div");
      list.className = "codex-web-folder-picker-list";
      list.dataset.slot = "scroll-area";
      panel.appendChild(list);

      const status = document.createElement("div");
      status.className = "codex-web-folder-picker-status";
      status.dataset.slot = "dialog-description";
      panel.appendChild(status);

      const actions = document.createElement("div");
      actions.className = "codex-web-folder-picker-actions";
      actions.dataset.slot = "dialog-footer";
      panel.appendChild(actions);

      const cancelButton = createWebPickerButton({
        label: "Cancel",
        title: "Cancel",
        variant: "outline",
      });
      actions.appendChild(cancelButton);

      const selectButton = createWebPickerButton({
        icon: "check",
        label: selectLabel || (fileMode ? (multiple ? "Select files" : "Select file") : "Select folder"),
        title: fileMode ? "Select chosen files" : "Select current folder",
        variant: "default",
      });
      actions.appendChild(selectButton);

      let currentPath = "";
      let currentEntries = [];
      let parentPath = null;
      let selectedFilePaths = new Set();
      let loadSequence = 0;

      function finish(value) {
        document.removeEventListener("keydown", onKeyDown, true);
        backdrop.remove();
        activeFolderPicker = null;
        resolve(value);
      }

      function setBusy(isBusy) {
        goButton.disabled = isBusy;
        homeButton.disabled = isBusy;
        refreshButton.disabled = isBusy;
        selectButton.disabled = isBusy || (fileMode ? selectedFilePaths.size === 0 : !currentPath);
        upButton.disabled = isBusy || !parentPath;
      }

      function setStatus(text, { error = false, icon = null } = {}) {
        status.className = `codex-web-folder-picker-status${error ? " codex-web-folder-picker-status-error" : ""}`;
        status.replaceChildren();
        if (icon) {
          status.appendChild(createLucideIcon(icon, error ? "" : "codex-web-folder-picker-icon"));
        }
        const textNode = document.createElement("span");
        textNode.textContent = text;
        status.appendChild(textNode);
      }

      function renderBreadcrumbs(path) {
        breadcrumbs.replaceChildren();
        const segments = webFolderPickerBreadcrumbs(path);
        segments.forEach((segment, index) => {
          if (index > 0) {
            breadcrumbs.appendChild(createLucideIcon("chevronRight", "codex-web-folder-picker-breadcrumb-separator"));
          }
          const item = document.createElement("button");
          item.className = `codex-web-folder-picker-breadcrumb${
            index === segments.length - 1 ? " codex-web-folder-picker-breadcrumb-current" : ""
          }`;
          item.type = "button";
          item.dataset.path = segment.path;
          item.dataset.slot = "breadcrumb-item";
          item.title = segment.path;
          if (segment.icon) {
            item.appendChild(createLucideIcon(segment.icon));
          }
          const label = document.createElement("span");
          label.className = "codex-web-folder-picker-breadcrumb-label";
          label.textContent = segment.label;
          item.appendChild(label);
          breadcrumbs.appendChild(item);
        });
      }

      function renderLoading() {
        const loading = document.createElement("div");
        loading.className = "codex-web-folder-picker-loading";
        loading.appendChild(createLucideIcon("loaderCircle", "codex-web-folder-picker-spin"));
        const text = document.createElement("span");
        text.textContent = fileMode ? "Loading files..." : "Loading folders...";
        loading.appendChild(text);
        list.replaceChildren(loading);
      }

      function renderEntries(entries) {
        list.replaceChildren();
        if (!entries.length) {
          const empty = document.createElement("div");
          empty.className = "codex-web-folder-picker-empty";
          const text = document.createElement("span");
          empty.replaceChildren(createLucideIcon(fileMode ? "file" : "folderOpen"), text);
          text.textContent = fileMode
            ? imagesOnly
              ? "No image files in this directory."
              : "No files in this directory."
            : "No folders in this directory.";
          list.appendChild(empty);
          return;
        }
        for (const entry of entries) {
          const kind = entry.kind === "file" ? "file" : "directory";
          const isFile = kind === "file";
          const row = document.createElement("button");
          row.className = `codex-web-folder-picker-row${
            isFile && selectedFilePaths.has(entry.path) ? " codex-web-folder-picker-row-selected" : ""
          }`;
          row.type = "button";
          row.dataset.path = entry.path;
          row.dataset.kind = kind;
          row.dataset.slot = isFile ? "file-item" : "directory-item";
          if (isFile) {
            row.setAttribute("aria-selected", selectedFilePaths.has(entry.path) ? "true" : "false");
          }
          row.title = entry.path || entry.name || "";

          row.appendChild(createLucideIcon(isFile ? "file" : "folder", "codex-web-folder-picker-icon"));

          const body = document.createElement("span");
          body.className = "codex-web-folder-picker-row-body";
          row.appendChild(body);

          const name = document.createElement("span");
          name.className = "codex-web-folder-picker-name";
          name.textContent = entry.name || entry.path;
          body.appendChild(name);

          const entryPath = document.createElement("span");
          entryPath.className = "codex-web-folder-picker-entry-path";
          entryPath.textContent = entry.path || "";
          body.appendChild(entryPath);

          if (!isFile) {
            row.appendChild(createLucideIcon("chevronRight", "codex-web-folder-picker-row-chevron"));
          }

          list.appendChild(row);
        }
      }

      async function loadDirectory(path) {
        const sequence = ++loadSequence;
        setBusy(true);
        setStatus(fileMode ? "Loading files..." : "Loading folders...", { icon: "loaderCircle" });
        renderLoading();
        try {
          const data = await requestWebFolderPickerDirectory(path, { imagesOnly, mode });
          if (sequence !== loadSequence) {
            return;
          }
          currentPath = data.path || path || "";
          currentEntries = Array.isArray(data.entries) ? data.entries : [];
          parentPath = data.parent || null;
          selectedFilePaths.clear();
          pathInput.value = currentPath;
          renderBreadcrumbs(currentPath);
          renderEntries(currentEntries);
          const entryCount = currentEntries.length;
          const directoryCount = currentEntries.filter((entry) => entry.kind !== "file").length;
          const fileCount = currentEntries.filter((entry) => entry.kind === "file").length;
          setStatus(
            data.truncated
              ? `Showing first ${entryCount} items`
              : fileMode
                ? `${directoryCount} ${directoryCount === 1 ? "folder" : "folders"}, ${fileCount} ${fileCount === 1 ? "file" : "files"}`
                : `${entryCount} ${entryCount === 1 ? "folder" : "folders"}`,
          );
        } catch (error) {
          if (sequence !== loadSequence) {
            return;
          }
          setStatus(error && error.message ? error.message : String(error), {
            error: true,
            icon: "alertCircle",
          });
          list.replaceChildren();
        } finally {
          if (sequence === loadSequence) {
            setBusy(false);
          }
        }
      }

      function onKeyDown(event) {
        if (event.key === "Escape") {
          event.preventDefault();
          finish(null);
        }
      }

      function toggleFileSelection(path) {
        if (!path) {
          return;
        }
        if (!multiple) {
          selectedFilePaths.clear();
          selectedFilePaths.add(path);
        } else if (selectedFilePaths.has(path)) {
          selectedFilePaths.delete(path);
        } else {
          selectedFilePaths.add(path);
        }
        renderEntries(currentEntries);
        setBusy(false);
      }

      list.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const row = target ? target.closest(".codex-web-folder-picker-row") : null;
        if (row && row.dataset.path) {
          if (fileMode && row.dataset.kind === "file") {
            toggleFileSelection(row.dataset.path);
          } else {
            void loadDirectory(row.dataset.path);
          }
        }
      });
      list.addEventListener("dblclick", (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const row = target ? target.closest(".codex-web-folder-picker-row") : null;
        if (fileMode && row && row.dataset.kind === "file" && row.dataset.path) {
          selectedFilePaths.add(row.dataset.path);
          finish(Array.from(selectedFilePaths));
        }
      });
      breadcrumbs.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const item = target ? target.closest(".codex-web-folder-picker-breadcrumb") : null;
        if (item && item.dataset.path) {
          void loadDirectory(item.dataset.path);
        }
      });
      homeButton.addEventListener("click", () => {
        void loadDirectory("");
      });
      upButton.addEventListener("click", () => {
        if (parentPath) {
          void loadDirectory(parentPath);
        }
      });
      refreshButton.addEventListener("click", () => {
        void loadDirectory(currentPath);
      });
      goButton.addEventListener("click", () => {
        void loadDirectory(pathInput.value);
      });
      pathInput.addEventListener("keydown", (event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          void loadDirectory(pathInput.value);
        }
      });
      cancelButton.addEventListener("click", () => finish(null));
      closeButton.addEventListener("click", () => finish(null));
      selectButton.addEventListener("click", () => {
        finish(fileMode ? Array.from(selectedFilePaths) : currentPath || null);
      });
      backdrop.addEventListener("click", (event) => {
        if (event.target === backdrop) {
          finish(null);
        }
      });
      document.addEventListener("keydown", onKeyDown, true);
      (document.body || document.documentElement).appendChild(backdrop);
      pathInput.focus();
      void loadDirectory("");
    });
    return activeFolderPicker;
  }

  function webPickerBasename(path) {
    return String(path || "").split(/[\\/]+/).filter(Boolean).pop() || String(path || "");
  }

  function webPickerFetchEndpoint(message) {
    if (message?.type !== "fetch" || typeof message.url !== "string") {
      return "";
    }
    try {
      const url = new URL(message.url, window.location.href);
      if (url.protocol !== "vscode:" || url.hostname !== "codex") {
        return "";
      }
      return url.pathname.replace(/^\/+/, "");
    } catch {
      const prefix = "vscode://codex/";
      return message.url.startsWith(prefix) ? message.url.slice(prefix.length).split(/[?#]/, 1)[0] : "";
    }
  }

  function webPickerFetchParams(message) {
    let body = message?.body;
    if (typeof body === "string") {
      if (!body.trim()) {
        body = {};
      } else {
        try {
          body = JSON.parse(body);
        } catch {
          body = {};
        }
      }
    }
    if (!body || typeof body !== "object") {
      return {};
    }
    return body.params && typeof body.params === "object" ? body.params : body;
  }

  function dispatchWebPickerFetchSuccess(requestId, body) {
    dispatchHostMessage({
      type: "fetch-response",
      requestId,
      responseType: "success",
      status: 200,
      headers: { "content-type": "application/json" },
      bodyJsonString: JSON.stringify(body),
    });
  }

  function webPickerFetchMessage(endpoint, params) {
    return {
      type: "fetch",
      requestId: `codex-web-picker-${Date.now()}-${Math.random().toString(36).slice(2)}`,
      url: `vscode://codex/${endpoint}`,
      body: JSON.stringify({ params }),
    };
  }

  async function sendWebPickerFetch(endpoint, params) {
    const message = webPickerFetchMessage(endpoint, params);
    const payload = await sendBridgeRequest(message);
    const response = (payload.messages || []).find(
      (item) => item?.type === "fetch-response" && item.requestId === message.requestId,
    );
    if (response?.responseType === "error") {
      throw new Error(response.error || `Fetch failed: ${endpoint}`);
    }
    return response;
  }

  async function maybeHandleWebFilePickerFetch(message) {
    if (webPickerFetchEndpoint(message) !== "pick-files") {
      return false;
    }
    const params = webPickerFetchParams(message);
    const pickerTitle =
      typeof params.pickerTitle === "string" && params.pickerTitle.trim()
        ? params.pickerTitle
        : "Select files";
    const selectedPaths = await showWebFolderPicker({
      imagesOnly: params.imagesOnly === true,
      mode: "file",
      multiple: true,
      selectLabel: "Select files",
      title: pickerTitle,
    });
    const files = Array.isArray(selectedPaths)
      ? selectedPaths.map((path) => ({
          fsPath: path,
          label: webPickerBasename(path),
          path,
        }))
      : [];
    dispatchWebPickerFetchSuccess(message.requestId, { files });
    return true;
  }

  function maybeHandleRemoteWorkspaceRootRequest(message) {
    if (message?.type !== "remote-workspace-root-requested") {
      return false;
    }
    void handleRemoteWorkspaceRootRequest(message).catch((error) => {
      console.warn("[codex-web] remote workspace root picker failed", error);
    });
    return true;
  }

  async function handleRemoteWorkspaceRootRequest(message) {
    const mode = message.mode === "pick" ? "pick" : "add";
    const root = await showWebFolderPicker({
      title: mode === "pick" ? "Choose project folder" : "Add project folder",
    });
    if (!root) {
      return;
    }
    if (mode === "pick") {
      dispatchHostMessage({ type: "workspace-root-option-picked", root });
      return;
    }
    const setActive = message.setActive !== false;
    await sendWebPickerFetch("add-workspace-root-option", {
      hostId: message.hostId || "local",
      root,
      setActive,
    });
    dispatchHostMessage({ type: "workspace-root-option-added", root });
    if (setActive) {
      dispatchHostMessage({
        type: "navigate-to-route",
        path: "/",
        state: { focusComposerNonce: Date.now() },
      });
    }
  }

  async function maybeHandleWebFolderPickerMessage(message) {
    if (message.type === "electron-pick-workspace-root-option") {
      const root = await showWebFolderPicker({ title: "Choose project folder" });
      if (root) {
        dispatchHostMessage({ type: "workspace-root-option-picked", root });
      }
      return true;
    }
    if (message.type === "electron-add-new-workspace-root-option" && !message.root) {
      const root = await showWebFolderPicker({ title: "Add project folder" });
      if (root) {
        await forwardToCodexHost({ ...message, root });
      }
      return true;
    }
    return false;
  }

  async function forwardToCodexHost(message) {
    if (!message || typeof message !== "object") {
      return;
    }
    try {
      bridgeDebug("forwardToCodexHost", bridgeMessageLabel(message));
      if (await maybeHandleWebFilePickerFetch(message)) {
        return;
      }
      if (await maybeHandleWebFolderPickerMessage(message)) {
        return;
      }
      await sendBridgeRequest(message);
      scheduleHostSnapshotAfterViewHydration(message);
    } catch (error) {
      const hostMessage = bridgeErrorMessage(message, error);
      if (hostMessage) {
        dispatchHostMessage(hostMessage);
      } else {
        console.warn("[codex-web] bridge request failed", error);
      }
    }
  }

  const appHostRpcMainObject = {
    services: {
      codexMicro: null,
      hotkeyWindowHotkeys: null,
      primaryRuntime: null,
      projectWritableRoots: null,
    },
  };

  function appHostRpcDevalue(value) {
    if (value === undefined) {
      return ["undefined"];
    }
    if (
      value === null ||
      typeof value === "string" ||
      typeof value === "boolean"
    ) {
      return value;
    }
    if (typeof value === "number") {
      if (value === Infinity) {
        return ["inf"];
      }
      if (value === -Infinity) {
        return ["-inf"];
      }
      if (Number.isNaN(value)) {
        return ["nan"];
      }
      return value;
    }
    if (Array.isArray(value)) {
      return [value.map(appHostRpcDevalue)];
    }
    if (typeof value === "object") {
      const result = {};
      for (const key of Object.keys(value)) {
        result[key] = appHostRpcDevalue(value[key]);
      }
      return result;
    }
    return ["undefined"];
  }

  function appHostRpcEvaluate(exportsTable, expression) {
    if (!Array.isArray(expression)) {
      return undefined;
    }
    if (expression[0] !== "pipeline" && expression[0] !== "import") {
      return undefined;
    }
    const exportId = expression[1];
    let value = exportsTable[exportId]?.value;
    const path = Array.isArray(expression[2]) ? expression[2] : [];
    for (const segment of path) {
      if (value == null) {
        return undefined;
      }
      value = value[segment];
    }
    if (expression.length > 3 && typeof value === "function") {
      return value();
    }
    return value;
  }

  function installCodexAppHostRpcBridge() {
    if (window.__codexAppHostRpcBridgeInstalled) {
      return;
    }
    window.__codexAppHostRpcBridgeInstalled = true;
    window.addEventListener(
      "message",
      (event) => {
        const data = event?.data;
        if (!data || data.type !== "connect-app-host") {
          return;
        }
        const port = data.port || event.ports?.[0];
        if (!port) {
          return;
        }
        const exportsTable = [{ value: appHostRpcMainObject }];
        const sendRpcPacket = (packet) => {
          port.postMessage(JSON.stringify(packet));
        };
        port.addEventListener("message", (portEvent) => {
          if (portEvent.data === null) {
            port.close();
            return;
          }
          if (typeof portEvent.data !== "string") {
            return;
          }
          let packet;
          try {
            packet = JSON.parse(portEvent.data);
          } catch {
            return;
          }
          if (!Array.isArray(packet)) {
            return;
          }
          if (packet[0] === "push" && packet.length > 1) {
            exportsTable.push({ value: appHostRpcEvaluate(exportsTable, packet[1]) });
            return;
          }
          if (packet[0] === "pull" && typeof packet[1] === "number") {
            sendRpcPacket([
              "resolve",
              packet[1],
              appHostRpcDevalue(exportsTable[packet[1]]?.value),
            ]);
            return;
          }
          if (packet[0] === "release" && typeof packet[1] === "number") {
            delete exportsTable[packet[1]];
            return;
          }
          if (packet[0] === "abort") {
            port.close();
          }
        });
        port.start?.();
      },
      true,
    );
  }

  const electronBridge = (window.electronBridge ||= {});
  electronBridge.sendMessageFromView = async (message) => {
    await forwardToCodexHost(message);
  };
  electronBridge.getSharedObjectSnapshotValue = (key) => sharedObjects[key];

  window.addEventListener("codex-message-from-view", (event) => {
    if (event.__codexForwardedViaBridge) {
      return;
    }
    void forwardToCodexHost(event.detail);
  });

  installCodexAppHostRpcBridge();
  installCodexLPluginBridge();
  installCodexLPluginRuntimeEntry();
  bridgeConnectionStarted = true;
  void warmBridgeConnection();
})();
"#;
