import { DurableObject } from "cloudflare:workers";

const DEFAULT_ROOM = "default";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (isRelayRequest(url.pathname)) {
      const room = url.searchParams.get("room") || DEFAULT_ROOM;
      const id = env.REMOTE_SESSIONS.idFromName(room);
      return env.REMOTE_SESSIONS.get(id).fetch(request);
    }

    if (env.ASSETS) {
      return env.ASSETS.fetch(request);
    }

    return new Response("codex-app-remotely worker", {
      headers: { "Content-Type": "text/plain; charset=utf-8" },
    });
  },
};

export class RemoteSession extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sessions = new Map();

    for (const ws of this.ctx.getWebSockets()) {
      const attachment = ws.deserializeAttachment();
      if (attachment?.role) {
        this.sessions.set(ws, attachment);
      }
    }
  }

  async fetch(request) {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname.endsWith("/api/remote-status")) {
      return this.handleStatus(url);
    }

    const role = roleFromPath(url.pathname);
    if (!role) {
      return new Response("not found", { status: 404 });
    }

    if (request.headers.get("Upgrade") !== "websocket") {
      return new Response("expected websocket", { status: 426 });
    }

    const token = url.searchParams.get("token") || "";
    if (!token) {
      return new Response("missing token", { status: 401 });
    }

    if (role !== "host") {
      const host = this.hostSession();
      if (!host) {
        return new Response("remote host not connected", { status: 503 });
      }
      if (host.session.token !== token) {
        return new Response("unauthorized", { status: 401 });
      }
    }

    return this.acceptWebSocket(role, token);
  }

  acceptWebSocket(role, token) {
    if (role === "host") {
      this.replaceHost(token);
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    const session = {
      connectedAt: Date.now(),
      id: crypto.randomUUID(),
      role,
      token,
    };

    this.ctx.acceptWebSocket(server, [role]);
    server.serializeAttachment(session);
    this.sessions.set(server, session);

    if (role === "host") {
      this.send(server, { type: "ready", ...this.clientStats() });
      for (const clientSession of this.controlSessions()) {
        this.send(server, { clientId: clientSession.session.id, type: "controlConnected" });
      }
    } else if (role === "control") {
      this.sendHost({ clientId: session.id, type: "controlConnected" });
    }

    this.sendHost({ type: "clientStats", ...this.clientStats() });

    return new Response(null, {
      status: 101,
      webSocket: client,
    });
  }

  async webSocketMessage(ws, message) {
    const session = this.sessionFor(ws);
    if (!session) {
      ws.close(1008, "unknown session");
      return;
    }

    if (session.role === "host") {
      this.handleHostMessage(message);
      return;
    }

    if (session.role === "control" && typeof message === "string") {
      this.sendHost({
        clientId: session.id,
        payload: message,
        type: "controlFromClient",
      });
    }
  }

  async webSocketClose(ws, code, reason) {
    this.removeSession(ws);
    ws.close(code, reason);
  }

  async webSocketError(ws) {
    this.removeSession(ws);
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

  handleStatus(url) {
    const token = url.searchParams.get("token") || "";
    const host = this.hostSession();
    if (host && host.session.token !== token) {
      return new Response(JSON.stringify({ error: "unauthorized" }), {
        headers: { "Content-Type": "application/json; charset=utf-8" },
        status: 401,
      });
    }

    return new Response(JSON.stringify({ hostConnected: Boolean(host), ...this.clientStats() }), {
      headers: {
        "Cache-Control": "no-store",
        "Content-Type": "application/json; charset=utf-8",
      },
    });
  }

  removeSession(ws) {
    const session = this.sessionFor(ws);
    this.sessions.delete(ws);

    if (!session) {
      return;
    }

    if (session.role === "control") {
      this.sendHost({ clientId: session.id, type: "controlDisconnected" });
    }

    if (session.role === "host") {
      this.broadcastControl(JSON.stringify({ message: "Remote host disconnected", type: "warning" }));
    } else {
      this.sendHost({ type: "clientStats", ...this.clientStats() });
    }
  }

  replaceHost(nextToken) {
    for (const [ws, session] of this.sessions) {
      if (session.role === "host") {
        this.sessions.delete(ws);
        ws.close(1012, "host replaced");
      } else if (session.token !== nextToken) {
        this.sessions.delete(ws);
        ws.close(1008, "token changed");
      }
    }
  }

  hostSession() {
    for (const [ws, session] of this.sessions) {
      if (session.role === "host") {
        return { session, ws };
      }
    }

    return null;
  }

  controlSessions() {
    return [...this.sessions].filter(([, session]) => session.role === "control").map(([ws, session]) => ({ session, ws }));
  }

  frameSessions() {
    return [...this.sessions].filter(([, session]) => session.role === "frame").map(([ws, session]) => ({ session, ws }));
  }

  clientStats() {
    return {
      controlClientCount: this.controlSessions().length,
      frameClientCount: this.frameSessions().length,
    };
  }

  sessionFor(ws) {
    const session = this.sessions.get(ws) || ws.deserializeAttachment();
    if (session?.role && !this.sessions.has(ws)) {
      this.sessions.set(ws, session);
    }
    return session;
  }

  sendHost(envelope) {
    const host = this.hostSession();
    if (!host) {
      return false;
    }

    return this.send(host.ws, envelope);
  }

  sendControlClient(clientId, payload) {
    const targetId = String(clientId || "");
    for (const { session, ws } of this.controlSessions()) {
      if (session.id === targetId) {
        return this.sendRaw(ws, payload);
      }
    }

    return false;
  }

  closeControlClient(clientId) {
    const targetId = String(clientId || "");
    for (const { session, ws } of this.controlSessions()) {
      if (session.id === targetId) {
        ws.close(1000, "closed by host");
        return true;
      }
    }

    return false;
  }

  broadcastControl(payload) {
    for (const { ws } of this.controlSessions()) {
      this.sendRaw(ws, payload);
    }
  }

  broadcastFrame(payload) {
    for (const { ws } of this.frameSessions()) {
      this.sendRaw(ws, payload);
    }
  }

  send(ws, envelope) {
    return this.sendRaw(ws, JSON.stringify(envelope));
  }

  sendRaw(ws, payload) {
    try {
      ws.send(payload);
      return true;
    } catch {
      return false;
    }
  }
}

function isRelayRequest(pathname) {
  return Boolean(roleFromPath(pathname)) || pathname.endsWith("/api/remote-status");
}

function roleFromPath(pathname) {
  if (
    pathname === "/ws" ||
    pathname === "/wt" ||
    pathname.endsWith("/ws/control") ||
    pathname.endsWith("/wt/control") ||
    pathname.endsWith("/wt/session")
  ) {
    return "control";
  }

  if (pathname.endsWith("/ws/frame") || pathname.endsWith("/wt/frame")) {
    return "frame";
  }

  if (pathname.endsWith("/ws/host") || pathname.endsWith("/wt/host")) {
    return "host";
  }

  return "";
}
