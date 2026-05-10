#!/usr/bin/env -S deno run --allow-net --allow-read --allow-env --unstable-net

const CAR_FRAME_MAGIC = 0x43415246;
const CAR_STREAM_MAGIC = 0x43415253;
const DEFAULT_ROOM = "default";
const FRAME_META_INTERVAL_MS = 250;
const FRAME_LOG_INTERVAL = numberEnv("RELAY_FRAME_LOG_INTERVAL", 120);
const MAX_DATAGRAM_CHUNKS = numberEnv("WT_MAX_DATAGRAM_CHUNKS", 96);
const MAX_DATAGRAM_PAYLOAD_BYTES = numberEnv("WT_DATAGRAM_PAYLOAD_BYTES", 1100);
const MAX_STREAM_FRAME_BYTES = numberEnv("WT_MAX_STREAM_FRAME_BYTES", 2_000_000);
const MAX_WEBTRANSPORT_SESSIONS = numberEnv("WT_MAX_SESSIONS", 1);
const WT_MIN_FRAME_INTERVAL_MS = numberEnv("WT_MIN_FRAME_INTERVAL_MS", 50);

const TEXT_DECODER = new TextDecoder();
const TEXT_ENCODER = new TextEncoder();

const hostname = Deno.env.get("HOST") || "0.0.0.0";
const port = numberEnv("PORT", 8443);
const certPath = requiredEnv("TLS_CERT");
const keyPath = requiredEnv("TLS_KEY");
const cert = Deno.readTextFileSync(certPath);
const key = Deno.readTextFileSync(keyPath);

const publicDir = new URL("../public/", import.meta.url);
const rooms = new Map();

startHttpServer();
startWebTransportServer();

function startHttpServer() {
  Deno.serve({ cert, hostname, key, port }, async (request) => {
    try {
      const url = new URL(request.url);

      if (url.pathname.endsWith("/api/remote-status")) {
        return handleStatus(url);
      }

      const role = roleFromPath(url.pathname);
      if (role) {
        if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
          return new Response("expected websocket or webtransport", { status: 426 });
        }
        return acceptWebSocket(request, url, role);
      }

      return serveAsset(url);
    } catch (error) {
      console.error("[relay] request failed", error);
      return new Response("internal error", { status: 500 });
    }
  });

  console.log(`[relay] https/ws listening on ${hostname}:${port}`);
}

function startWebTransportServer() {
  const endpoint = new Deno.QuicEndpoint({ hostname, port });
  const listener = endpoint.listen({ alpnProtocols: ["h3"], cert, key });

  console.log(`[relay] webtransport listening on ${hostname}:${port}/udp`);

  (async () => {
    for (;;) {
      let conn;
      try {
        conn = await listener.accept();
      } catch (error) {
        console.warn("[relay] webtransport accept failed", error?.message || error);
        continue;
      }

      handleWebTransportConnection(conn).catch((error) => {
        console.warn("[relay] webtransport connection failed", error?.message || error);
      });
    }
  })();
}

async function acceptWebSocket(request, url, role) {
  const token = url.searchParams.get("token") || "";
  if (!token) {
    return new Response("missing token", { status: 401 });
  }

  const room = roomFor(url.searchParams.get("room") || DEFAULT_ROOM);
  if (role !== "host" && !room.authorizeClient(token)) {
    return new Response(room.host ? "unauthorized" : "remote host not connected", {
      status: room.host ? 401 : 503,
    });
  }

  const { response, socket } = Deno.upgradeWebSocket(request);
  room.acceptWebSocket(socket, role, token);
  return response;
}

async function handleWebTransportConnection(conn) {
  console.log(
    `[relay] webtransport accepted remote=${formatAddr(conn.remoteAddr)} alpn=${conn.protocol || "unknown"} sni=${
      conn.serverName || "unknown"
    }`,
  );
  const wt = await Deno.upgradeWebTransport(conn);
  const url = new URL(wt.url);
  if (!url.pathname.endsWith("/wt/session")) {
    wt.close({ closeCode: 404, reason: "not found" });
    return;
  }

  const token = url.searchParams.get("token") || "";
  if (!token) {
    wt.close({ closeCode: 401, reason: "missing token" });
    return;
  }

  const room = roomFor(url.searchParams.get("room") || DEFAULT_ROOM);
  if (!room.authorizeClient(token)) {
    wt.close({ closeCode: room.host ? 401 : 503, reason: room.host ? "unauthorized" : "remote host not connected" });
    return;
  }

  await wt.ready;
  room.acceptWebTransport(wt, token);
}

class RelayRoom {
  constructor(name) {
    this.frameId = 0;
    this.frameLogCount = 0;
    this.lastFrameMetaAt = 0;
    this.name = name;
    this.sessions = new Map();
  }

  get host() {
    for (const session of this.sessions.values()) {
      if (session.role === "host") {
        return session;
      }
    }
    return null;
  }

  acceptWebSocket(socket, role, token) {
    if (role === "host") {
      this.replaceHost(token);
    }

    const session = new WebSocketRelaySession(this, socket, role, token);
    this.sessions.set(session.id, session);
    session.start();
    this.afterSessionAccepted(session);
  }

  acceptWebTransport(wt, token) {
    const session = new WebTransportRelaySession(this, wt, token);
    this.sessions.set(session.id, session);
    session.start();
    this.afterSessionAccepted(session);
    this.pruneWebTransportSessions(session);
  }

  afterSessionAccepted(session) {
    if (session.role === "host") {
      session.sendJson({ type: "ready", ...this.clientStats() });
      for (const clientSession of this.controlSessions()) {
        session.sendJson({ clientId: clientSession.id, type: "controlConnected" });
      }
    } else if (session.isControlSession) {
      this.sendHost({ clientId: session.id, type: "controlConnected" });
    }

    this.sendHost({ type: "clientStats", ...this.clientStats() });
    console.log(`[relay] room=${this.name} connected ${session.role} sessions=${this.sessions.size}`);
  }

  authorizeClient(token) {
    const host = this.host;
    return Boolean(host && host.token === token);
  }

  broadcastControl(payload) {
    for (const session of this.controlSessions()) {
      session.sendRaw(payload);
    }
  }

  broadcastFrame(payload) {
    const frame = bytesFromPayload(payload);
    if (!frame) {
      console.warn(`[relay] room=${this.name} dropped frame: unsupported payload`);
      return;
    }

    const frameId = ++this.frameId;
    const frameSessions = this.frameSessions();
    this.frameLogCount += 1;
    if (this.frameLogCount === 1 || this.frameLogCount % FRAME_LOG_INTERVAL === 0 || frameSessions.length === 0) {
      console.log(`[relay] room=${this.name} frame=${frameId} bytes=${frame.byteLength} receivers=${frameSessions.length}`);
    }
    for (const session of frameSessions) {
      session.sendFrame(frame, frameId);
    }

    this.maybeBroadcastFrameMeta(payload);
  }

  clientStats() {
    return {
      controlClientCount: this.controlSessions().length,
      frameClientCount: this.frameSessions().length,
    };
  }

  closeControlClient(clientId) {
    const session = this.sessions.get(String(clientId || ""));
    if (session?.isControlSession) {
      session.close(1000, "closed by host");
      return true;
    }
    return false;
  }

  pruneWebTransportSessions(activeSession) {
    const sessions = [...this.sessions.values()].filter((session) => session.role === "webtransport");
    const stale = sessions.filter((session) => session !== activeSession);
    const removeCount = Math.max(0, sessions.length - MAX_WEBTRANSPORT_SESSIONS);
    for (const session of stale.slice(0, removeCount)) {
      session.close(1000, "replaced by newer webtransport session");
    }
  }

  controlSessions() {
    return [...this.sessions.values()].filter((session) => session.isControlSession);
  }

  frameSessions() {
    return [...this.sessions.values()].filter((session) => session.isFrameSession);
  }

  handleControlFromClient(session, payload) {
    this.sendHost({
      clientId: session.id,
      payload: typeof payload === "string" ? payload : JSON.stringify(payload ?? {}),
      type: "controlFromClient",
    });
  }

  handleHostMessage(message) {
    if (typeof message !== "string") {
      this.broadcastFrame(message);
      return;
    }

    let envelope;
    try {
      envelope = JSON.parse(message);
    } catch {
      return;
    }

    if (envelope.type === "controlBroadcast") {
      this.broadcastControl(envelope.payload);
      return;
    }

    if (envelope.type === "controlToClient") {
      this.sendControlClient(envelope.clientId, envelope.payload);
      return;
    }

    if (envelope.type === "closeControl") {
      this.closeControlClient(envelope.clientId);
      return;
    }

    if (envelope.type === "hostClosing") {
      this.broadcastControl(JSON.stringify({ message: "Remote host disconnected", type: "warning" }));
    }
  }

  maybeBroadcastFrameMeta(payload) {
    if (typeof payload !== "object" || !payload) {
      return;
    }

    const now = Date.now();
    if (now - this.lastFrameMetaAt < FRAME_META_INTERVAL_MS) {
      return;
    }

    this.lastFrameMetaAt = now;
    const { bytes: _bytes, data: _data, dataUrl: _dataUrl, ...meta } = payload;
    this.broadcastControl(JSON.stringify({ ...meta, type: "frameMeta" }));
  }

  removeSession(session) {
    if (this.sessions.get(session.id) !== session) {
      return;
    }

    this.sessions.delete(session.id);

    if (session.isControlSession) {
      this.sendHost({ clientId: session.id, type: "controlDisconnected" });
    }

    if (session.role === "host") {
      this.broadcastControl(JSON.stringify({ message: "Remote host disconnected", type: "warning" }));
    } else {
      this.sendHost({ type: "clientStats", ...this.clientStats() });
    }

    console.log(`[relay] room=${this.name} disconnected ${session.role} sessions=${this.sessions.size}`);
  }

  replaceHost(nextToken) {
    for (const session of this.sessions.values()) {
      if (session.role === "host") {
        session.close(1012, "host replaced");
      } else if (session.token !== nextToken) {
        session.close(1008, "token changed");
      }
    }
  }

  sendControlClient(clientId, payload) {
    const session = this.sessions.get(String(clientId || ""));
    if (!session?.isControlSession) {
      return false;
    }
    return session.sendRaw(payload);
  }

  sendHost(envelope) {
    const host = this.host;
    if (!host) {
      return false;
    }
    return host.sendJson(envelope);
  }
}

class RelaySession {
  constructor(room, role, token) {
    this.id = crypto.randomUUID();
    this.role = role;
    this.room = room;
    this.token = token;
  }

  get isControlSession() {
    return this.role === "control" || this.role === "webtransport";
  }

  get isFrameSession() {
    return this.role === "frame" || this.role === "webtransport";
  }

  close() {}

  sendFrame() {}

  sendJson(envelope) {
    return this.sendRaw(JSON.stringify(envelope));
  }

  sendRaw() {
    return false;
  }
}

class WebSocketRelaySession extends RelaySession {
  constructor(room, socket, role, token) {
    super(room, role, token);
    this.socket = socket;
  }

  close(code = 1000, reason = "closed") {
    try {
      this.socket.close(code, reason);
    } catch {
      // Socket may already be gone.
    }
  }

  sendFrame(payload) {
    if (this.role === "frame") {
      return this.sendRaw(payload);
    }
    return false;
  }

  sendRaw(payload) {
    try {
      this.socket.send(payload);
      return true;
    } catch {
      return false;
    }
  }

  start() {
    this.socket.binaryType = "arraybuffer";

    this.socket.addEventListener("message", (event) => {
      if (this.role === "host") {
        this.room.handleHostMessage(event.data);
      } else if (this.role === "control" && typeof event.data === "string") {
        this.room.handleControlFromClient(this, event.data);
      }
    });

    this.socket.addEventListener("close", () => this.room.removeSession(this));
    this.socket.addEventListener("error", () => this.room.removeSession(this));
  }
}

class WebTransportRelaySession extends RelaySession {
  constructor(room, wt, token) {
    super(room, "webtransport", token);
    this.datagramWriter = null;
    this.framePumpTimer = null;
    this.lastFrameSentAt = 0;
    this.latestFrame = null;
    this.pendingControlMessages = [];
    this.sendingFrame = false;
    this.wt = wt;
    this.writer = null;
    this.writeQueue = Promise.resolve();
  }

  close(code = 1000, reason = "closed") {
    try {
      this.wt.close({ closeCode: code, reason });
    } catch {
      // Transport may already be closed.
    }
    if (this.framePumpTimer) {
      clearTimeout(this.framePumpTimer);
      this.framePumpTimer = null;
    }
    this.room.removeSession(this);
  }

  sendFrame(payload, frameId) {
    if (payload.byteLength > MAX_STREAM_FRAME_BYTES) {
      console.warn(`[relay] room=${this.room.name} dropped webtransport frame=${frameId}: ${payload.byteLength} bytes exceeds ${MAX_STREAM_FRAME_BYTES}`);
      return false;
    }

    this.latestFrame = { frameId, payload };
    this.scheduleFramePump();
    return true;
  }

  sendRaw(payload) {
    if (typeof payload !== "string") {
      return false;
    }

    if (!this.writer) {
      this.pendingControlMessages.push(payload);
      return true;
    }

    this.writeControlPayload(payload);
    return true;
  }

  writeControlPayload(payload) {
    const message = TEXT_ENCODER.encode(payload);
    const packet = new Uint8Array(4 + message.byteLength);
    new DataView(packet.buffer).setUint32(0, message.byteLength);
    packet.set(message, 4);
    this.writeQueue = this.writeQueue.then(() => this.writer.write(packet)).catch(() => {
      this.close(1011, "control write failed");
    });
  }

  start() {
    this.datagramWriter = this.wt.datagrams?.writable?.getWriter?.() || null;
    this.readControlStreams();
    this.wt.closed.then(
      () => this.room.removeSession(this),
      () => this.room.removeSession(this),
    );
  }

  async openFrameStream(frameId, payload) {
    if (shouldLogFrame(frameId)) {
      console.log(`[relay] room=${this.room.name} sending webtransport frame=${frameId} via stream bytes=${payload.byteLength}`);
    }
    const stream = await this.wt.createUnidirectionalStream();
    const writer = stream.getWriter();
    try {
      await writer.write(encodeStreamFrame(frameId, payload));
    } finally {
      await writer.close().catch(() => {});
      writer.releaseLock();
    }
  }

  scheduleFramePump(delayMs = 0) {
    if (this.sendingFrame || this.framePumpTimer) {
      return;
    }
    this.framePumpTimer = setTimeout(() => {
      this.framePumpTimer = null;
      this.pumpLatestFrame();
    }, delayMs);
  }

  async pumpLatestFrame() {
    if (this.sendingFrame || !this.latestFrame) {
      return;
    }
    const waitMs = WT_MIN_FRAME_INTERVAL_MS - (Date.now() - this.lastFrameSentAt);
    if (waitMs > 0) {
      this.scheduleFramePump(waitMs);
      return;
    }

    this.sendingFrame = true;
    try {
      const { frameId, payload } = this.latestFrame;
      this.latestFrame = null;
      this.lastFrameSentAt = Date.now();
      if (!this.trySendDatagramFrame(frameId, payload)) {
        await this.openFrameStream(frameId, payload);
      }
    } catch (error) {
      console.warn(`[relay] room=${this.room.name} webtransport frame write failed`, error?.message || error);
      this.close(1011, "frame write failed");
    } finally {
      this.sendingFrame = false;
      if (this.latestFrame) {
        this.scheduleFramePump(WT_MIN_FRAME_INTERVAL_MS);
      }
    }
  }

  async readControlStream(stream) {
    const reader = stream.readable.getReader();
    this.writer = stream.writable.getWriter();
    this.flushPendingControlMessages();
    let buffer = new Uint8Array(0);

    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }

        buffer = concatBytes(buffer, bytesFromValue(value));
        while (buffer.byteLength >= 4) {
          const payloadLength = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength).getUint32(0);
          if (buffer.byteLength < 4 + payloadLength) {
            break;
          }

          const payload = buffer.slice(4, 4 + payloadLength);
          buffer = buffer.slice(4 + payloadLength);
          this.room.handleControlFromClient(this, TEXT_DECODER.decode(payload));
        }
      }
    } catch {
      // Close below handles cleanup.
    }
    this.close(1000, "control stream closed");
  }

  flushPendingControlMessages() {
    const messages = this.pendingControlMessages;
    this.pendingControlMessages = [];
    for (const message of messages) {
      this.writeControlPayload(message);
    }
  }

  async readControlStreams() {
    try {
      for await (const stream of this.wt.incomingBidirectionalStreams) {
        if (this.writer) {
          stream.readable.cancel("control stream already open").catch(() => {});
          stream.writable.abort("control stream already open").catch(() => {});
          continue;
        }
        this.readControlStream(stream);
      }
    } catch {
      this.close(1011, "control stream failed");
    }
  }

  trySendDatagramFrame(frameId, payload) {
    const chunkCount = Math.ceil(payload.byteLength / MAX_DATAGRAM_PAYLOAD_BYTES);
    if (!this.datagramWriter || chunkCount <= 0 || chunkCount > MAX_DATAGRAM_CHUNKS) {
      return false;
    }

    for (let chunkIndex = 0; chunkIndex < chunkCount; chunkIndex += 1) {
      const start = chunkIndex * MAX_DATAGRAM_PAYLOAD_BYTES;
      const chunk = payload.subarray(start, start + MAX_DATAGRAM_PAYLOAD_BYTES);
      this.datagramWriter.write(encodeDatagramFrame(frameId, chunkIndex, chunkCount, chunk)).catch(() => {});
    }
    if (shouldLogFrame(frameId)) {
      console.log(`[relay] room=${this.room.name} sending webtransport frame=${frameId} via datagrams chunks=${chunkCount} bytes=${payload.byteLength}`);
    }
    return true;
  }
}

async function serveAsset(url) {
  const pathname = decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname);
  const resolved = new URL(`.${pathname}`, publicDir);
  if (!resolved.href.startsWith(publicDir.href)) {
    return new Response("forbidden", { status: 403 });
  }

  try {
    const body = await Deno.readFile(resolved);
    return new Response(body, {
      headers: {
        "Cache-Control": "no-store",
        "Content-Type": mimeType(resolved.pathname),
      },
    });
  } catch {
    return new Response("not found", { status: 404 });
  }
}

function bytesFromPayload(payload) {
  if (payload instanceof ArrayBuffer) {
    return new Uint8Array(payload);
  }

  if (ArrayBuffer.isView(payload)) {
    return new Uint8Array(payload.buffer, payload.byteOffset, payload.byteLength);
  }

  return null;
}

function bytesFromValue(value) {
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

function concatBytes(left, right) {
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

function encodeDatagramFrame(frameId, chunkIndex, chunkCount, payload) {
  const packet = new Uint8Array(16 + payload.byteLength);
  const view = new DataView(packet.buffer);
  view.setUint32(0, CAR_FRAME_MAGIC);
  view.setUint8(4, 1);
  view.setUint16(6, chunkCount);
  view.setUint32(8, frameId);
  view.setUint16(12, chunkIndex);
  packet.set(payload, 16);
  return packet;
}

function encodeStreamFrame(frameId, payload, metadata = null) {
  const metadataBytes = metadata ? TEXT_ENCODER.encode(JSON.stringify(metadata)) : new Uint8Array(0);
  const packet = new Uint8Array(16 + metadataBytes.byteLength + payload.byteLength);
  const view = new DataView(packet.buffer);
  view.setUint32(0, CAR_STREAM_MAGIC);
  view.setUint8(4, 1);
  view.setUint16(6, 0);
  view.setUint32(8, frameId);
  view.setUint32(12, metadataBytes.byteLength);
  packet.set(metadataBytes, 16);
  packet.set(payload, 16 + metadataBytes.byteLength);
  return packet;
}

function handleStatus(url) {
  const token = url.searchParams.get("token") || "";
  const room = rooms.get(url.searchParams.get("room") || DEFAULT_ROOM);
  if (room?.host && room.host.token !== token) {
    return json({ error: "unauthorized" }, 401);
  }

  return json({
    hostConnected: Boolean(room?.host),
    ...(room?.clientStats() || { controlClientCount: 0, frameClientCount: 0 }),
  });
}

function json(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    headers: {
      "Cache-Control": "no-store",
      "Content-Type": "application/json; charset=utf-8",
    },
    status,
  });
}

function shouldLogFrame(frameId) {
  return frameId <= 3 || frameId % FRAME_LOG_INTERVAL === 0;
}

function formatAddr(addr) {
  if (!addr || typeof addr !== "object") {
    return "unknown";
  }
  return `${addr.hostname || "unknown"}:${addr.port || 0}`;
}

function mimeType(pathname) {
  if (pathname.endsWith(".css")) {
    return "text/css; charset=utf-8";
  }
  if (pathname.endsWith(".html")) {
    return "text/html; charset=utf-8";
  }
  if (pathname.endsWith(".js")) {
    return "application/javascript; charset=utf-8";
  }
  if (pathname.endsWith(".png")) {
    return "image/png";
  }
  if (pathname.endsWith(".svg")) {
    return "image/svg+xml";
  }
  return "application/octet-stream";
}

function numberEnv(name, fallback) {
  const value = Number(Deno.env.get(name));
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

function requiredEnv(name) {
  const value = Deno.env.get(name);
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function roleFromPath(pathname) {
  if (pathname === "/ws" || pathname.endsWith("/ws/control")) {
    return "control";
  }

  if (pathname.endsWith("/ws/frame")) {
    return "frame";
  }

  if (pathname.endsWith("/ws/host")) {
    return "host";
  }

  if (pathname.endsWith("/wt/session")) {
    return "webtransport";
  }

  return "";
}

function roomFor(name) {
  const roomName = String(name || DEFAULT_ROOM);
  let room = rooms.get(roomName);
  if (!room) {
    room = new RelayRoom(roomName);
    rooms.set(roomName, room);
  }
  return room;
}
