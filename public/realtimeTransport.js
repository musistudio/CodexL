const CAR_FRAME_MAGIC = 0x43415246;
const CAR_STREAM_MAGIC = 0x43415253;
const DATAGRAM_HEADER_BYTES = 16;
const MAX_PENDING_DATAGRAM_FRAMES = 6;
const MAX_PENDING_DATAGRAM_FRAME_AGE_MS = 700;
const TRANSPORT_CLOSED = 3;
const TRANSPORT_CONNECTING = 0;
export const TRANSPORT_OPEN = 1;

const TEXT_DECODER = new TextDecoder();
const TEXT_ENCODER = new TextEncoder();

// WebTransport v2:
// - control JSON: client-opened bidirectional stream, uint32 byte length + UTF-8 JSON payload.
// - frame stream: server-opened unidirectional stream, optional "CARS" header + metadata JSON + image bytes.
// - frame datagram: "CARF" header + frame id + chunk index/count + image chunk. Missing chunks drop the frame.
export async function openRealtimeSession({
  controlWebSocketUrl,
  frameWebSocketUrl,
  onWebTransportFallback,
  timeoutMs = 2500,
  transportPreference = "auto",
  webTransportUrl,
}) {
  if (shouldTryWebTransport(transportPreference)) {
    try {
      return await WebTransportSession.connect(webTransportUrl, { timeoutMs });
    } catch (error) {
      onWebTransportFallback?.(error);
    }
  }

  return WebSocketSession.connect({ controlWebSocketUrl, frameWebSocketUrl });
}

function shouldTryWebTransport(transportPreference) {
  if (transportPreference === "websocket" || transportPreference === "ws") {
    return false;
  }

  if (typeof WebTransport !== "function") {
    return false;
  }

  return transportPreference === "webtransport" || transportPreference === "wt" || location.protocol === "https:";
}

class RealtimeSession extends EventTarget {
  constructor(kind) {
    super();
    this.closed = false;
    this.kind = kind;
  }

  close() {
    if (this.closed) {
      return;
    }

    this.closed = true;
    this.closeImpl();
    this.dispatchEvent(new Event("close"));
  }

  closeImpl() {}
}

class RealtimeChannel extends EventTarget {
  constructor(kind) {
    super();
    this.kind = kind;
    this.readyState = TRANSPORT_CONNECTING;
  }

  close() {
    this.closeImpl();
  }

  closeImpl() {}

  dispatchError() {
    this.dispatchEvent(new Event("error"));
  }

  dispatchMessage(data) {
    this.dispatchEvent(messageEvent(data));
  }

  dispatchMetadata(metadata) {
    this.dispatchEvent(metadataEvent(metadata));
  }

  markClosed() {
    if (this.readyState === TRANSPORT_CLOSED) {
      return;
    }

    this.readyState = TRANSPORT_CLOSED;
    this.dispatchEvent(new Event("close"));
  }

  markOpen() {
    this.readyState = TRANSPORT_OPEN;
  }
}

class WebSocketSession extends RealtimeSession {
  constructor(control, frame) {
    super("websocket");
    this.control = control;
    this.frame = frame;

    const closeSession = () => this.close();
    const errorSession = () => {
      this.dispatchEvent(new Event("error"));
      this.close();
    };

    this.control.addEventListener("close", closeSession);
    this.frame.addEventListener("close", closeSession);
    this.control.addEventListener("error", errorSession);
    this.frame.addEventListener("error", errorSession);
  }

  static async connect({ controlWebSocketUrl, frameWebSocketUrl }) {
    let control;
    let frame;
    try {
      const controlPromise = WebSocketChannel.connect(controlWebSocketUrl).then((channel) => {
        control = channel;
        return channel;
      });
      const framePromise = WebSocketChannel.connect(frameWebSocketUrl, { binaryType: "arraybuffer" }).then((channel) => {
        frame = channel;
        return channel;
      });
      await Promise.all([controlPromise, framePromise]);
    } catch (error) {
      control?.close();
      frame?.close();
      throw error;
    }

    return new WebSocketSession(control, frame);
  }

  closeImpl() {
    this.control.close();
    this.frame.close();
  }
}

class WebSocketChannel extends RealtimeChannel {
  constructor(socket) {
    super("websocket");
    this.socket = socket;
  }

  static connect(url, { binaryType } = {}) {
    return new Promise((resolve, reject) => {
      if (typeof WebSocket !== "function") {
        reject(new Error("WebSocket is not available"));
        return;
      }

      const socket = new WebSocket(url);
      if (binaryType) {
        socket.binaryType = binaryType;
      }

      const channel = new WebSocketChannel(socket);
      let opened = false;
      let settled = false;

      const rejectBeforeOpen = (error) => {
        if (settled) {
          return;
        }
        settled = true;
        reject(error);
      };

      socket.addEventListener("open", () => {
        opened = true;
        channel.markOpen();
        if (settled) {
          return;
        }
        settled = true;
        resolve(channel);
      });

      socket.addEventListener("message", (event) => {
        channel.dispatchMessage(event.data);
      });

      socket.addEventListener("close", () => {
        channel.markClosed();
        if (!opened) {
          rejectBeforeOpen(new Error("WebSocket closed before opening"));
        }
      });

      socket.addEventListener("error", () => {
        channel.dispatchError();
        if (!opened) {
          rejectBeforeOpen(new Error("WebSocket failed before opening"));
        }
        socket.close();
      });
    });
  }

  get bufferedAmount() {
    return this.socket.bufferedAmount || 0;
  }

  closeImpl() {
    this.socket.close();
  }

  send(data) {
    this.socket.send(data);
  }
}

class WebTransportSession extends RealtimeSession {
  constructor(transport) {
    super("webtransport");
    this.control = null;
    this.frame = new WebTransportFrameChannel(this);
    this.transport = transport;
  }

  static async connect(url, { timeoutMs }) {
    const transport = new WebTransport(url, {
      congestionControl: "low-latency",
      requireUnreliable: false,
    });
    const session = new WebTransportSession(transport);

    try {
      await withTimeout(transport.ready, timeoutMs, "WebTransport connect timed out", () => session.close());
      await withTimeout(
        session.openControlStream(),
        timeoutMs,
        "WebTransport control stream timed out",
        () => session.close(),
      );
      session.openFrameReceivers();
      session.watchClosed();
      return session;
    } catch (error) {
      session.close();
      throw error;
    }
  }

  closeImpl() {
    this.control?.markClosed();
    this.frame.markClosed();
    try {
      this.transport.close();
    } catch {
      // The transport may already be closed by the browser.
    }
  }

  async openControlStream() {
    if (typeof this.transport.createBidirectionalStream !== "function") {
      throw new Error("WebTransport bidirectional streams are not available");
    }

    const stream = await this.transport.createBidirectionalStream();
    this.control = new WebTransportControlChannel(this, stream.writable.getWriter());
    this.control.markOpen();
    this.control.readLengthPrefixedStream(stream.readable);
  }

  openFrameReceivers() {
    const hasDatagrams = typeof this.transport.datagrams?.readable?.getReader === "function";
    const hasStreams = typeof this.transport.incomingUnidirectionalStreams?.getReader === "function";
    if (!hasDatagrams && !hasStreams) {
      throw new Error("WebTransport frame receive APIs are not available");
    }

    this.frame.markOpen();
    if (hasDatagrams) {
      this.frame.readDatagrams(this.transport.datagrams.readable);
    }
    if (hasStreams) {
      this.frame.readIncomingUnidirectionalStreams(this.transport.incomingUnidirectionalStreams);
    }
  }

  watchClosed() {
    this.transport.closed.then(
      () => this.close(),
      () => this.close(),
    );
  }
}

class WebTransportControlChannel extends RealtimeChannel {
  constructor(session, writer) {
    super("webtransport");
    this.session = session;
    this.writer = writer;
    this.writeQueue = Promise.resolve();
  }

  closeImpl() {
    this.session.close();
  }

  async readLengthPrefixedStream(readable) {
    const reader = readable.getReader();
    let buffer = new Uint8Array(0);

    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }

        buffer = concatBytes(buffer, bytesFromStreamValue(value));
        while (buffer.byteLength >= 4) {
          const payloadLength = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength).getUint32(0);
          if (buffer.byteLength < 4 + payloadLength) {
            break;
          }

          const payload = buffer.slice(4, 4 + payloadLength);
          buffer = buffer.slice(4 + payloadLength);
          this.dispatchMessage(TEXT_DECODER.decode(payload));
        }
      }
    } catch {
      // The session close below drives the reconnect path.
    }
    this.session.close();
  }

  send(data) {
    if (this.readyState !== TRANSPORT_OPEN || !this.writer) {
      return;
    }

    const payload = typeof data === "string" ? TEXT_ENCODER.encode(data) : bytesFromStreamValue(data);
    const packet = new Uint8Array(4 + payload.byteLength);
    new DataView(packet.buffer).setUint32(0, payload.byteLength);
    packet.set(payload, 4);

    this.writeQueue = this.writeQueue.then(() => this.writer.write(packet)).catch(() => {
      this.session.close();
    });
  }
}

class WebTransportFrameChannel extends RealtimeChannel {
  constructor(session) {
    super("webtransport");
    this.latestFrameId = 0;
    this.pendingDatagramFrames = new Map();
    this.session = session;
  }

  closeImpl() {
    this.session.close();
  }

  dispatchFrame({ data, frameId = 0, metadata = null }) {
    if (frameId > 0) {
      if (frameId <= this.latestFrameId) {
        return;
      }
      this.latestFrameId = frameId;
    }

    if (metadata) {
      this.dispatchMetadata(metadata);
    }
    this.dispatchMessage(data);
  }

  handleDatagram(value) {
    const bytes = bytesFromStreamValue(value);
    const packet = parseDatagramFramePacket(bytes);
    if (!packet) {
      this.dispatchFrame({ data: arrayBufferFromBytes(bytes) });
      return;
    }

    if (packet.frameId <= this.latestFrameId) {
      return;
    }

    if (packet.chunkCount <= 1) {
      this.dispatchFrame({ data: arrayBufferFromBytes(packet.payload), frameId: packet.frameId });
      return;
    }

    this.prunePendingDatagramFrames(Date.now());
    let pending = this.pendingDatagramFrames.get(packet.frameId);
    if (!pending) {
      pending = {
        chunks: new Array(packet.chunkCount),
        createdAt: Date.now(),
        received: 0,
        receivedBytes: 0,
      };
      this.pendingDatagramFrames.set(packet.frameId, pending);
    }

    if (pending.chunks.length !== packet.chunkCount || pending.chunks[packet.chunkIndex]) {
      return;
    }

    pending.chunks[packet.chunkIndex] = packet.payload;
    pending.received += 1;
    pending.receivedBytes += packet.payload.byteLength;

    if (pending.received === pending.chunks.length) {
      this.pendingDatagramFrames.delete(packet.frameId);
      this.dispatchFrame({
        data: arrayBufferFromBytes(concatByteList(pending.chunks, pending.receivedBytes)),
        frameId: packet.frameId,
      });
    }
  }

  handleStreamFrame(data) {
    const packet = parseStreamFramePacket(new Uint8Array(data));
    if (!packet) {
      this.dispatchFrame({ data });
      return;
    }

    this.dispatchFrame(packet);
  }

  prunePendingDatagramFrames(now) {
    for (const [frameId, pending] of this.pendingDatagramFrames) {
      if (
        frameId <= this.latestFrameId ||
        now - pending.createdAt > MAX_PENDING_DATAGRAM_FRAME_AGE_MS ||
        this.pendingDatagramFrames.size > MAX_PENDING_DATAGRAM_FRAMES
      ) {
        this.pendingDatagramFrames.delete(frameId);
      }
    }
  }

  async readDatagrams(readable) {
    const reader = readable.getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        this.handleDatagram(value);
      }
    } catch {
      // The session close below drives the reconnect path.
    }
    this.session.close();
  }

  async readFrameStream(readable) {
    try {
      const data = await new Response(readable).arrayBuffer();
      if (data.byteLength > 0) {
        this.handleStreamFrame(data);
      }
    } catch {
      this.session.close();
    }
  }

  async readIncomingUnidirectionalStreams(readable) {
    const reader = readable.getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        this.readFrameStream(value);
      }
    } catch {
      // The session close below drives the reconnect path.
    }
    this.session.close();
  }
}

function arrayBufferFromBytes(bytes) {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

function bytesFromStreamValue(value) {
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

function concatByteList(chunks, totalBytes) {
  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function messageEvent(data) {
  if (typeof MessageEvent === "function") {
    return new MessageEvent("message", { data });
  }

  const event = new Event("message");
  event.data = data;
  return event;
}

function metadataEvent(metadata) {
  const event = new Event("metadata");
  event.detail = metadata;
  return event;
}

function parseDatagramFramePacket(bytes) {
  if (bytes.byteLength < DATAGRAM_HEADER_BYTES) {
    return null;
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0) !== CAR_FRAME_MAGIC || view.getUint8(4) !== 1) {
    return null;
  }

  const chunkCount = view.getUint16(6);
  const frameId = view.getUint32(8);
  const chunkIndex = view.getUint16(12);
  if (chunkCount <= 0 || chunkIndex >= chunkCount || frameId <= 0) {
    return null;
  }

  return {
    chunkCount,
    chunkIndex,
    frameId,
    payload: bytes.slice(DATAGRAM_HEADER_BYTES),
  };
}

function parseStreamFramePacket(bytes) {
  if (bytes.byteLength < 16) {
    return null;
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0) !== CAR_STREAM_MAGIC || view.getUint8(4) !== 1) {
    return null;
  }

  const headerLength = view.getUint16(6);
  const frameId = view.getUint32(8);
  const metadataLength = view.getUint32(12);
  const metadataStart = 16 + headerLength;
  const frameStart = metadataStart + metadataLength;
  if (metadataStart > bytes.byteLength || frameStart > bytes.byteLength) {
    return null;
  }

  let metadata = null;
  if (metadataLength > 0) {
    try {
      metadata = JSON.parse(TEXT_DECODER.decode(bytes.slice(metadataStart, frameStart)));
    } catch {
      metadata = null;
    }
  }

  return {
    data: arrayBufferFromBytes(bytes.slice(frameStart)),
    frameId,
    metadata,
  };
}

function withTimeout(promise, ms, message, onTimeout) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      onTimeout?.();
      reject(new Error(message));
    }, ms);

    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
