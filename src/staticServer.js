import fs from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const publicDir = path.resolve(__dirname, "..", "public");

const MIME_TYPES = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
};

export function createStaticServer({ bridge, token }) {
  return http.createServer(async (request, response) => {
    try {
      const url = new URL(request.url || "/", "http://localhost");

      if (url.pathname.startsWith("/api/")) {
        await handleApi(request, response, url, bridge, token);
        return;
      }

      await handleStatic(url, response);
    } catch (error) {
      sendJson(response, 500, { error: error.message });
    }
  });
}

async function handleApi(request, response, url, bridge, token) {
  if (url.searchParams.get("token") !== token) {
    sendJson(response, 401, { error: "unauthorized" });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/status") {
    sendJson(response, 200, bridge.status());
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/targets") {
    const targets = await bridge.listTargets();
    sendJson(response, 200, { targets });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/target") {
    const body = await readJsonBody(request);
    await bridge.switchTarget(body.id);
    sendJson(response, 200, bridge.status());
    return;
  }

  sendJson(response, 404, { error: "not found" });
}

async function handleStatic(url, response) {
  const pathname = decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname);
  const resolved = path.resolve(publicDir, `.${pathname}`);

  if (!resolved.startsWith(publicDir)) {
    response.writeHead(403);
    response.end("forbidden");
    return;
  }

  try {
    const body = await fs.readFile(resolved);
    response.writeHead(200, {
      "Cache-Control": "no-store",
      "Content-Type": MIME_TYPES[path.extname(resolved)] || "application/octet-stream",
    });
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end("not found");
  }
}

function sendJson(response, statusCode, payload) {
  response.writeHead(statusCode, {
    "Cache-Control": "no-store",
    "Content-Type": "application/json; charset=utf-8",
  });
  response.end(JSON.stringify(payload));
}

async function readJsonBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }

  const text = Buffer.concat(chunks).toString("utf8");
  return text ? JSON.parse(text) : {};
}
