use super::*;

pub(super) fn web_bridge_script_response() -> WebResourceResponse {
    WebResourceResponse {
        status: StatusCode::OK,
        content_type: "application/javascript; charset=utf-8".to_string(),
        body: Bytes::from_static(WEB_BRIDGE_SCRIPT.as_bytes()),
    }
}

pub(super) const WEB_BRIDGE_SCRIPT: &str = r#"(() => {
  if (window.__codexWebBridgeInstalled) {
    return;
  }
  window.__codexWebBridgeInstalled = true;

  const pageParams = new URLSearchParams(window.location.search);

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
	  const BRIDGE_HEARTBEAT_INTERVAL_MS = 15000;
	  const BRIDGE_HEARTBEAT_STALE_MS = 30000;
	  const BRIDGE_HEARTBEAT_TIMEOUT_MS = 8000;
	  const BRIDGE_RECONNECT_MIN_DELAY_MS = 250;
	  const BRIDGE_RECONNECT_MAX_DELAY_MS = 5000;
	  const BRIDGE_REQUEST_TIMEOUT_MS = 30000;
	  const BRIDGE_LONG_REQUEST_TIMEOUT_MS = 5 * 60 * 1000;
	  const BRIDGE_STREAM_REQUEST_TIMEOUT_MS = 10 * 60 * 1000;
	  let socket = null;
	  let connectingSocket = null;
	  let transportClient = null;
	  let connectingTransport = null;
	  let webTransportUnavailable = false;
	  let bridgeConnectionStarted = false;
	  let bridgeLastHeartbeatAckAt = 0;
	  let bridgeHeartbeatTimeoutTimer = null;
	  let bridgeReconnectDelayMs = BRIDGE_RECONNECT_MIN_DELAY_MS;
	  let bridgeReconnectTimer = null;
	  let nextMessageId = 1;
	  const E2EE_AAD = new TextEncoder().encode("codexl-remote-e2ee-v1");
	  const E2EE_STORAGE_PREFIX = "codexl.remote.e2ee.v1.";
	  let bridgeCryptoPromise = null;

	  function notifyBridgeStatus(status, detail = {}) {
	    try {
	      window.parent?.postMessage(
	        {
	          ...detail,
	          status,
	          ts: Date.now(),
	          type: BRIDGE_STATUS_MESSAGE,
	        },
	        window.location.origin,
	      );
	    } catch {}
	  }

  function dispatchHostMessage(message) {
    if (!message || typeof message !== "object") {
      return;
    }
    if (message.type === "shared-object-updated" && typeof message.key === "string") {
      sharedObjects[message.key] = message.value;
    }
    window.dispatchEvent(
      new MessageEvent("message", {
        data: message,
        origin: window.location.origin,
        source: window,
      }),
    );
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

	  function markBridgeConnectionOpen() {
	    markBridgeConnectionAlive();
	    bridgeReconnectDelayMs = BRIDGE_RECONNECT_MIN_DELAY_MS;
	    if (bridgeReconnectTimer) {
	      window.clearTimeout(bridgeReconnectTimer);
	      bridgeReconnectTimer = null;
	    }
	    notifyBridgeStatus("connected");
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

	  function handleBridgePayload(payload) {
	    markBridgeConnectionAlive();
	    if (payload?.type === "bridge-heartbeat-ack") {
	      return;
	    }
	    for (const hostMessage of payload.messages || []) {
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
	      scheduleBridgeReconnect();
	    }
	    return pendingResponse;
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

  async function requestWebFolderPickerDirectory(path) {
    const payload = await sendBridgeRequest({
      type: "web-file-picker-list",
      path: typeof path === "string" ? path : "",
    });
    return payload.value || {};
  }

  let activeFolderPicker = null;

  function showWebFolderPicker({ title = "Choose project folder" } = {}) {
    if (activeFolderPicker) {
      return activeFolderPicker;
    }
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
      titleNode.appendChild(createLucideIcon("folderOpen", "codex-web-folder-picker-title-icon"));
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
      pathInput.setAttribute("aria-label", "Folder path");
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
        label: "Select folder",
        title: "Select current folder",
        variant: "default",
      });
      actions.appendChild(selectButton);

      let currentPath = "";
      let parentPath = null;
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
        selectButton.disabled = isBusy || !currentPath;
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
        text.textContent = "Loading folders...";
        loading.appendChild(text);
        list.replaceChildren(loading);
      }

      function renderEntries(entries) {
        list.replaceChildren();
        if (!entries.length) {
          const empty = document.createElement("div");
          empty.className = "codex-web-folder-picker-empty";
          const text = document.createElement("span");
          empty.replaceChildren(createLucideIcon("folderOpen"), text);
          text.textContent = "No folders in this directory.";
          list.appendChild(empty);
          return;
        }
        for (const entry of entries) {
          const row = document.createElement("button");
          row.className = "codex-web-folder-picker-row";
          row.type = "button";
          row.dataset.path = entry.path;
          row.dataset.slot = "directory-item";
          row.title = entry.path || entry.name || "";

          row.appendChild(createLucideIcon("folder", "codex-web-folder-picker-icon"));

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

          row.appendChild(createLucideIcon("chevronRight", "codex-web-folder-picker-row-chevron"));

          list.appendChild(row);
        }
      }

      async function loadDirectory(path) {
        const sequence = ++loadSequence;
        setBusy(true);
        setStatus("Loading folders...", { icon: "loaderCircle" });
        renderLoading();
        try {
          const data = await requestWebFolderPickerDirectory(path);
          if (sequence !== loadSequence) {
            return;
          }
          currentPath = data.path || path || "";
          parentPath = data.parent || null;
          pathInput.value = currentPath;
          renderBreadcrumbs(currentPath);
          renderEntries(Array.isArray(data.entries) ? data.entries : []);
          const entryCount = Array.isArray(data.entries) ? data.entries.length : 0;
          setStatus(
            data.truncated
              ? `Showing first ${entryCount} folders`
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

      list.addEventListener("click", (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const row = target ? target.closest(".codex-web-folder-picker-row") : null;
        if (row && row.dataset.path) {
          void loadDirectory(row.dataset.path);
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
      selectButton.addEventListener("click", () => finish(currentPath || null));
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
      if (await maybeHandleWebFolderPickerMessage(message)) {
        return;
      }
      await sendBridgeRequest(message);
    } catch (error) {
      const hostMessage = bridgeErrorMessage(message, error);
      if (hostMessage) {
        dispatchHostMessage(hostMessage);
      } else {
        console.warn("[codex-web] bridge request failed", error);
      }
    }
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

  bridgeConnectionStarted = true;
  void warmBridgeConnection();
})();
"#;
