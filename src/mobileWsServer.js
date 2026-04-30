import crypto from "node:crypto";
import { EventEmitter } from "node:events";

const GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

export class MobileWsServer extends EventEmitter {
  constructor({ server, token }, logger = console) {
    super();
    this.clients = new Set();
    this.heartbeatTimer = null;
    this.logger = logger;
    this.server = server;
    this.token = token;
  }

  get clientCount() {
    return this.clients.size;
  }

  start() {
    this.server.on("upgrade", (request, socket) => {
      this.handleUpgrade(request, socket);
    });

    this.heartbeatTimer = setInterval(() => {
      this.broadcast({ type: "heartbeat", ts: Date.now() });
    }, 15000).unref();
  }

  stop() {
    clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;

    for (const client of this.clients) {
      client.close();
    }
    this.clients.clear();
  }

  broadcast(payload) {
    const message = JSON.stringify(payload);
    for (const client of this.clients) {
      client.send(message);
    }
  }

  broadcastFrame(payload) {
    const packet = encodeFramePacket(payload);
    for (const client of this.clients) {
      client.sendBinary(packet);
    }
  }

  handleUpgrade(request, socket) {
    const url = new URL(request.url || "/", "http://localhost");

    if (url.pathname !== "/ws") {
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

    const client = new WsClient(socket);
    this.clients.add(client);
    this.logger.log(`[mobile] connected clients: ${this.clients.size}`);

    client.on("message", (message) => {
      this.emit("message", client, message);
    });

    client.on("close", () => {
      this.clients.delete(client);
      this.logger.log(`[mobile] connected clients: ${this.clients.size}`);
      this.emit("disconnect", client);
    });

    this.emit("connection", client);
  }
}

class WsClient extends EventEmitter {
  constructor(socket) {
    super();
    this.buffer = Buffer.alloc(0);
    this.closed = false;
    this.pendingBinary = null;
    this.socket = socket;
    this.waitingForDrain = false;

    socket.setNoDelay?.(true);

    socket.on("data", (chunk) => {
      this.buffer = Buffer.concat([this.buffer, chunk]);
      this.readFrames();
    });

    socket.on("drain", () => {
      this.waitingForDrain = false;
      this.flushPendingBinary();
    });

    socket.on("close", () => this.close());
    socket.on("error", () => this.close());
  }

  send(message) {
    if (this.closed || this.socket.destroyed) {
      return;
    }

    this.socket.write(encodeFrame(Buffer.from(message)));
  }

  sendBinary(message) {
    if (this.closed || this.socket.destroyed) {
      return;
    }

    if (this.waitingForDrain) {
      this.pendingBinary = message;
      return;
    }

    this.writeBinary(message);
  }

  close() {
    if (this.closed) {
      return;
    }

    this.closed = true;
    this.pendingBinary = null;
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

  flushPendingBinary() {
    if (this.closed || this.socket.destroyed || !this.pendingBinary) {
      return;
    }

    const message = this.pendingBinary;
    this.pendingBinary = null;
    this.writeBinary(message);
  }

  writeBinary(message) {
    const canContinue = this.socket.write(encodeFrame(message, 0x2));
    if (!canContinue) {
      this.waitingForDrain = true;
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

function encodeFramePacket(payload) {
  const imageBase64 = payload.data || dataUrlToBase64(payload.dataUrl || "");
  const image = Buffer.from(imageBase64, "base64");
  const headerPayload = { ...payload, data: undefined, dataUrl: undefined };
  const header = Buffer.from(JSON.stringify(headerPayload), "utf8");
  const length = Buffer.alloc(4);
  length.writeUInt32BE(header.length, 0);
  return Buffer.concat([length, header, image]);
}

function dataUrlToBase64(dataUrl) {
  const commaIndex = dataUrl.indexOf(",");
  return commaIndex >= 0 ? dataUrl.slice(commaIndex + 1) : dataUrl;
}
