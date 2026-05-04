import crypto from "node:crypto";
import { EventEmitter } from "node:events";

const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const FRAME_BACKPRESSURE_BYTES = 1_000_000;
const FRAME_META_INTERVAL_MS = 250;
const FRAME_PUMP_RETRY_MS = 30;
const HEARTBEAT_INTERVAL_MS = 5000;
const NETWORK_SAMPLE_MS = 1000;

export class MobileWsServer extends EventEmitter {
  constructor({ server, token }, logger = console) {
    super();
    this.controlClients = new Set();
    this.frameClients = new Set();
    this.heartbeatTimer = null;
    this.lastFrameMetaAt = 0;
    this.logger = logger;
    this.networkTimer = null;
    this.rttMs = null;
    this.server = server;
    this.token = token;
  }

  get clientCount() {
    return this.controlClients.size + this.frameClients.size;
  }

  get controlClientCount() {
    return this.controlClients.size;
  }

  get frameClientCount() {
    return this.frameClients.size;
  }

  start() {
    this.server.on("upgrade", (request, socket) => {
      this.handleUpgrade(request, socket);
    });

    this.heartbeatTimer = setInterval(() => {
      this.broadcast({ type: "heartbeat", ts: Date.now() });
    }, HEARTBEAT_INTERVAL_MS).unref();

    this.networkTimer = setInterval(() => {
      this.emit("network", this.networkSnapshot());
    }, NETWORK_SAMPLE_MS).unref();
  }

  stop() {
    clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;
    clearInterval(this.networkTimer);
    this.networkTimer = null;

    for (const client of this.controlClients) {
      client.close();
    }
    for (const client of this.frameClients) {
      client.close();
    }
    this.controlClients.clear();
    this.frameClients.clear();
  }

  broadcast(payload) {
    const message = JSON.stringify(payload);
    for (const client of this.controlClients) {
      client.send(message);
    }
  }

  broadcastFrame(payload) {
    if (this.frameClients.size === 0) {
      return;
    }

    const frame = frameBytesFromPayload(payload);
    if (!frame) {
      return;
    }

    for (const client of this.frameClients) {
      client.sendLatestBinary(frame);
    }

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
    let bufferedAmount = 0;
    let droppedFramesInLast5s = 0;
    const now = Date.now();
    for (const client of this.frameClients) {
      bufferedAmount = Math.max(bufferedAmount, client.bufferedAmount);
      droppedFramesInLast5s += client.droppedFramesSince(now - 5000);
    }

    return {
      bufferedAmount,
      droppedFramesInLast5s,
      frameClientCount: this.frameClients.size,
      rtt: this.controlClients.size > 0 ? this.rttMs : null,
    };
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

  handleUpgrade(request, socket) {
    const url = new URL(request.url || "/", "http://localhost");
    const channel = channelFromPath(url.pathname);

    if (!channel) {
      socket.end("HTTP/1.1 404 Not Found\r\n\r\n");
      return;
    }

    if (url.searchParams.get("token") !== this.token) {
      socket.end("HTTP/1.1 401 Unauthorized\r\n\r\n");
      return;
    }

    const key = request.headers["sec-websocket-key"];
    if (!key) {
      socket.end("HTTP/1.1 400 Bad Request\r\n\r\n");
      return;
    }

    const accept = crypto.createHash("sha1").update(`${key}${GUID}`).digest("base64");
    socket.write([
      "HTTP/1.1 101 Switching Protocols",
      "Upgrade: websocket",
      "Connection: Upgrade",
      `Sec-WebSocket-Accept: ${accept}`,
      "\r\n",
    ].join("\r\n"));

    const client = new WsClient(socket, { channel });
    const clients = channel === "frame" ? this.frameClients : this.controlClients;
    clients.add(client);
    if (channel === "control") {
      this.rttMs = null;
    }
    this.logger.log(`[mobile] connected ${channel} clients: ${clients.size}`);

    client.on("message", (message) => {
      if (channel === "control") {
        this.emit("message", client, message);
      }
    });

    client.on("close", () => {
      clients.delete(client);
      this.logger.log(`[mobile] connected ${channel} clients: ${clients.size}`);
      this.emit("disconnect", client, channel);
    });

    this.emit("connection", client, channel);
  }
}

class WsClient extends EventEmitter {
  constructor(socket, { channel }) {
    super();
    this.buffer = Buffer.alloc(0);
    this.channel = channel;
    this.closed = false;
    this.droppedFrameTimestamps = [];
    this.latestBinary = null;
    this.pumpTimer = null;
    this.sendingBinary = false;
    this.socket = socket;

    socket.setNoDelay?.(true);

    socket.on("data", (chunk) => {
      this.buffer = Buffer.concat([this.buffer, chunk]);
      this.readFrames();
    });

    socket.on("close", () => this.close());
    socket.on("error", () => this.close());
  }

  get bufferedAmount() {
    return this.socket.writableLength + (this.latestBinary?.length || 0);
  }

  send(message) {
    if (this.closed || this.socket.destroyed) {
      return;
    }

    this.socket.write(encodeFrame(Buffer.from(message)));
  }

  sendLatestBinary(message) {
    if (this.closed || this.socket.destroyed) {
      return;
    }

    if (this.latestBinary) {
      this.recordDroppedFrame();
    }

    this.latestBinary = message;
    this.pumpLatestBinary();
  }

  close() {
    if (this.closed) {
      return;
    }

    this.closed = true;
    clearTimeout(this.pumpTimer);
    this.pumpTimer = null;
    this.latestBinary = null;
    this.emit("close");
    try {
      this.socket.end();
    } catch {
      // Socket may already be gone.
    }
  }

  readFrames() {
    while (this.buffer.length >= 2) {
      const first = this.buffer[0];
      const second = this.buffer[1];
      const opcode = first & 0x0f;
      const masked = (second & 0x80) !== 0;
      let payloadLength = second & 0x7f;
      let offset = 2;

      if (payloadLength === 126) {
        if (this.buffer.length < offset + 2) {
          return;
        }
        payloadLength = this.buffer.readUInt16BE(offset);
        offset += 2;
      } else if (payloadLength === 127) {
        if (this.buffer.length < offset + 8) {
          return;
        }
        const high = this.buffer.readUInt32BE(offset);
        const low = this.buffer.readUInt32BE(offset + 4);
        payloadLength = high * 2 ** 32 + low;
        offset += 8;
      }

      const maskLength = masked ? 4 : 0;
      if (this.buffer.length < offset + maskLength + payloadLength) {
        return;
      }

      const mask = masked ? this.buffer.subarray(offset, offset + 4) : null;
      offset += maskLength;
      const payload = this.buffer.subarray(offset, offset + payloadLength);
      this.buffer = this.buffer.subarray(offset + payloadLength);

      if (opcode === 0x8) {
        this.close();
        return;
      }

      if (opcode === 0x9) {
        this.socket.write(encodeFrame(payload, 0xA));
        continue;
      }

      if (opcode !== 0x1) {
        continue;
      }

      const unmasked = Buffer.from(payload);
      if (mask) {
        for (let i = 0; i < unmasked.length; i += 1) {
          unmasked[i] ^= mask[i % 4];
        }
      }

      this.emit("message", unmasked.toString("utf8"));
    }
  }

  pumpLatestBinary() {
    if (this.closed || this.socket.destroyed || this.sendingBinary || !this.latestBinary) {
      return;
    }

    if (this.socket.writableLength > FRAME_BACKPRESSURE_BYTES) {
      this.schedulePump();
      return;
    }

    const message = this.latestBinary;
    this.latestBinary = null;
    this.sendingBinary = true;
    this.socket.write(encodeFrame(message, 0x2), () => {
      this.sendingBinary = false;
      this.pumpLatestBinary();
    });
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
}

function encodeFrame(payload, opcode = 0x1) {
  const length = payload.length;
  let header;

  if (length < 126) {
    header = Buffer.alloc(2);
    header[1] = length;
  } else if (length < 65536) {
    header = Buffer.alloc(4);
    header[1] = 126;
    header.writeUInt16BE(length, 2);
  } else {
    header = Buffer.alloc(10);
    header[1] = 127;
    header.writeUInt32BE(Math.floor(length / 2 ** 32), 2);
    header.writeUInt32BE(length >>> 0, 6);
  }

  header[0] = 0x80 | opcode;
  return Buffer.concat([header, payload]);
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

function channelFromPath(pathname) {
  if (pathname === "/ws/control" || pathname === "/ws") {
    return "control";
  }

  if (pathname === "/ws/frame") {
    return "frame";
  }

  return "";
}
