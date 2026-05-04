import { EventEmitter } from "node:events";

const FRAME_BACKPRESSURE_BYTES = 1_500_000;
const FRAME_META_INTERVAL_MS = 250;
const FRAME_PUMP_RETRY_MS = 30;
const NETWORK_SAMPLE_MS = 1000;
const RECONNECT_MAX_MS = 8000;
const RECONNECT_MIN_MS = 1000;

export class RemoteRelayClient extends EventEmitter {
  constructor({ remoteRoom, remoteWorkerUrl, room = remoteRoom, token, workerUrl = remoteWorkerUrl }, logger = console) {
    super();
    this.clients = new Map();
    this.connected = false;
    this.droppedFrameTimestamps = [];
    this.frameClients = 0;
    this.lastFrameMetaAt = 0;
    this.latestBinary = null;
    this.logger = logger;
    this.networkTimer = null;
    this.pumpTimer = null;
    this.reconnectDelayMs = RECONNECT_MIN_MS;
    this.reconnectTimer = null;
    this.room = room;
    this.rttMs = null;
    this.sendingBinary = false;
    this.socket = null;
    this.stopped = false;
    this.token = token;
    this.workerUrl = workerUrl;
  }

  get clientCount() {
    return this.controlClientCount + this.frameClientCount;
  }

  get controlClientCount() {
    return this.clients.size;
  }

  get frameClientCount() {
    return this.frameClients;
  }

  get mobileUrl() {
    return remoteControlUrl({ room: this.room, token: this.token, workerUrl: this.workerUrl });
  }

  start() {
    this.stopped = false;
    this.connect();

    this.networkTimer = setInterval(() => {
      this.emit("network", this.networkSnapshot());
    }, NETWORK_SAMPLE_MS).unref();
  }

  stop() {
    this.stopped = true;
    clearInterval(this.networkTimer);
    clearTimeout(this.reconnectTimer);
    clearTimeout(this.pumpTimer);
    this.networkTimer = null;
    this.reconnectTimer = null;
    this.pumpTimer = null;
    this.latestBinary = null;
    this.closeSocket();
    this.clearClients();
  }

  broadcast(payload) {
    const message = JSON.stringify(payload);
    return this.sendEnvelope({ payload: message, type: "controlBroadcast" });
  }

  broadcastFrame(payload) {
    if (this.frameClientCount === 0) {
      return;
    }

    const frame = frameBytesFromPayload(payload);
    if (!frame) {
      return;
    }

    this.sendLatestBinary(frame);
    this.maybeBroadcastFrameMeta(payload);
  }

  notePong(message) {
    const ts = Number(message?.ts);
    if (!Number.isFinite(ts) || ts <= 0) {
      return;
    }

    this.rttMs = Math.max(0, Date.now() - ts);
  }

  networkSnapshot() {
    return {
      bufferedAmount: this.bufferedAmount(),
      droppedFramesInLast5s: this.droppedFramesSince(Date.now() - 5000),
      frameClientCount: this.frameClientCount,
      rtt: this.controlClientCount > 0 ? this.rttMs : null,
    };
  }

  connect() {
    if (this.stopped || this.socket) {
      return;
    }

    if (typeof WebSocket !== "function") {
      this.logger.warn("[remote] WebSocket is not available in this Node.js runtime");
      return;
    }

    const url = remoteHostWebSocketUrl({ room: this.room, token: this.token, workerUrl: this.workerUrl });
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    this.socket = socket;

    socket.addEventListener("open", () => {
      if (this.socket !== socket) {
        return;
      }

      this.connected = true;
      this.reconnectDelayMs = RECONNECT_MIN_MS;
      this.logger.log(`[remote] connected to ${this.workerUrl} room=${this.room}`);
      this.emit("ready");
    });

    socket.addEventListener("message", (event) => {
      if (this.socket === socket) {
        this.handleSocketMessage(event.data);
      }
    });

    socket.addEventListener("close", () => {
      if (this.socket === socket) {
        this.handleSocketClose();
      }
    });

    socket.addEventListener("error", () => {
      socket.close();
    });
  }

  handleSocketClose() {
    this.socket = null;
    this.connected = false;
    this.frameClients = 0;
    this.rttMs = null;
    this.latestBinary = null;
    this.sendingBinary = false;
    this.clearClients();
    this.emit("clientStats", this.networkSnapshot());

    if (this.stopped) {
      return;
    }

    const delay = this.reconnectDelayMs;
    this.reconnectDelayMs = Math.min(RECONNECT_MAX_MS, Math.round(this.reconnectDelayMs * 1.6));
    this.logger.warn(`[remote] disconnected, reconnecting in ${delay}ms`);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
    this.reconnectTimer.unref?.();
  }

  handleSocketMessage(data) {
    if (typeof data !== "string") {
      return;
    }

    let message;
    try {
      message = JSON.parse(data);
    } catch {
      return;
    }

    if (message.type === "ready" || message.type === "clientStats") {
      this.updateClientStats(message);
      return;
    }

    if (message.type === "controlConnected") {
      this.attachControlClient(message.clientId);
      return;
    }

    if (message.type === "controlDisconnected") {
      this.detachControlClient(message.clientId);
      return;
    }

    if (message.type === "controlFromClient") {
      const client = this.attachControlClient(message.clientId);
      const payload = typeof message.payload === "string" ? message.payload : JSON.stringify(message.payload ?? {});
      this.emit("message", client, payload);
      return;
    }

    if (message.type === "warning") {
      this.logger.warn(`[remote] ${message.message}`);
    }
  }

  updateClientStats(message) {
    const previousFrameClients = this.frameClients;
    this.frameClients = Math.max(0, Number(message.frameClientCount || 0));
    if (previousFrameClients !== this.frameClients) {
      this.emit("clientStats", this.networkSnapshot());
    }
  }

  attachControlClient(clientId, { silent = false } = {}) {
    const id = String(clientId || "");
    if (!id) {
      return new RemoteControlClient(this, "");
    }

    const existing = this.clients.get(id);
    if (existing) {
      return existing;
    }

    const client = new RemoteControlClient(this, id);
    this.clients.set(id, client);
    this.rttMs = null;
    if (!silent) {
      this.logger.log(`[remote] connected control clients: ${this.clients.size}`);
      this.emit("connection", client, "control");
      this.emit("clientStats", this.networkSnapshot());
    }
    return client;
  }

  detachControlClient(clientId) {
    const id = String(clientId || "");
    const client = this.clients.get(id);
    if (!client) {
      return;
    }

    this.clients.delete(id);
    this.logger.log(`[remote] connected control clients: ${this.clients.size}`);
    this.emit("disconnect", client, "control");
    this.emit("clientStats", this.networkSnapshot());
  }

  clearClients() {
    for (const client of this.clients.values()) {
      this.emit("disconnect", client, "control");
    }
    this.clients.clear();
  }

  sendEnvelope(envelope) {
    if (!this.isOpen()) {
      return false;
    }

    try {
      this.socket.send(JSON.stringify(envelope));
      return true;
    } catch {
      return false;
    }
  }

  sendLatestBinary(message) {
    if (!this.isOpen()) {
      return;
    }

    if (this.latestBinary) {
      this.recordDroppedFrame();
    }

    this.latestBinary = message;
    this.pumpLatestBinary();
  }

  pumpLatestBinary() {
    if (!this.isOpen() || this.sendingBinary || !this.latestBinary) {
      return;
    }

    if (this.socket.bufferedAmount > FRAME_BACKPRESSURE_BYTES) {
      this.schedulePump();
      return;
    }

    const message = this.latestBinary;
    this.latestBinary = null;
    this.sendingBinary = true;
    try {
      this.socket.send(arrayBufferFromBuffer(message));
    } finally {
      this.sendingBinary = false;
    }
    this.pumpLatestBinary();
  }

  schedulePump() {
    if (this.pumpTimer) {
      return;
    }

    this.pumpTimer = setTimeout(() => {
      this.pumpTimer = null;
      this.pumpLatestBinary();
    }, FRAME_PUMP_RETRY_MS);
    this.pumpTimer.unref?.();
  }

  maybeBroadcastFrameMeta(payload) {
    const now = Date.now();
    if (now - this.lastFrameMetaAt < FRAME_META_INTERVAL_MS) {
      return;
    }

    this.lastFrameMetaAt = now;
    const { bytes, data, dataUrl, ...meta } = payload;
    this.broadcast({ ...meta, type: "frameMeta" });
  }

  bufferedAmount() {
    return Math.max(0, Number(this.socket?.bufferedAmount || 0) + (this.latestBinary?.length || 0));
  }

  recordDroppedFrame() {
    const now = Date.now();
    this.droppedFrameTimestamps.push(now);
    this.pruneDroppedFrames(now - 5000);
  }

  droppedFramesSince(since) {
    this.pruneDroppedFrames(since);
    return this.droppedFrameTimestamps.length;
  }

  pruneDroppedFrames(since) {
    while (this.droppedFrameTimestamps.length > 0 && this.droppedFrameTimestamps[0] < since) {
      this.droppedFrameTimestamps.shift();
    }
  }

  closeSocket() {
    const socket = this.socket;
    this.socket = null;
    if (!socket) {
      return;
    }

    try {
      socket.close();
    } catch {
      // Socket may already be closed.
    }
  }

  isOpen() {
    return this.socket && this.socket.readyState === WebSocket.OPEN;
  }
}

class RemoteControlClient {
  constructor(relay, id) {
    this.id = id;
    this.relay = relay;
  }

  send(message) {
    return this.relay.sendEnvelope({
      clientId: this.id,
      payload: message,
      type: "controlToClient",
    });
  }

  close() {
    return this.relay.sendEnvelope({
      clientId: this.id,
      type: "closeControl",
    });
  }
}

export function remoteControlUrl({ room, token, workerUrl }) {
  const url = urlWithBasePath(workerUrl, "/");
  url.searchParams.set("room", room);
  url.searchParams.set("token", token);
  return url.toString();
}

function remoteHostWebSocketUrl({ room, token, workerUrl }) {
  const url = urlWithBasePath(workerUrl, "/ws/host");
  url.protocol = url.protocol === "http:" ? "ws:" : "wss:";
  url.searchParams.set("room", room);
  url.searchParams.set("token", token);
  return url.toString();
}

function urlWithBasePath(workerUrl, pathname) {
  const url = new URL(workerUrl);
  const basePath = url.pathname.replace(/\/+$/, "");
  url.pathname = `${basePath}${pathname}`;
  url.search = "";
  url.hash = "";
  return url;
}

function frameBytesFromPayload(payload) {
  if (Buffer.isBuffer(payload.bytes)) {
    return payload.bytes;
  }

  if (Buffer.isBuffer(payload.data)) {
    return payload.data;
  }

  const imageBase64 = typeof payload.data === "string" ? payload.data : dataUrlToBase64(payload.dataUrl || "");
  return imageBase64 ? Buffer.from(imageBase64, "base64") : null;
}

function dataUrlToBase64(dataUrl) {
  const commaIndex = dataUrl.indexOf(",");
  return commaIndex >= 0 ? dataUrl.slice(commaIndex + 1) : dataUrl;
}

function arrayBufferFromBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}
