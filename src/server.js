#!/usr/bin/env node
import { spawn } from "node:child_process";
import QRCode from "qrcode";
import { CdpBridge } from "./cdpClient.js";
import { getLanUrls, helpText, parseConfig } from "./config.js";
import { launchCodexApp, quitCodexApp } from "./codexLauncher.js";
import { MobileWsServer } from "./mobileWsServer.js";
import { prepareCdpPort } from "./ports.js";
import { createStaticServer } from "./staticServer.js";

const config = parseConfig();

if (config.help) {
  process.stdout.write(helpText());
  process.exit(0);
}

try {
  await prepareCdpPort(config);
} catch (error) {
  console.error(`[server] ${error.message}`);
  process.exit(1);
}

const bridge = new CdpBridge(config);
const server = createStaticServer({ bridge, token: config.token });
const mobileWs = new MobileWsServer({ server, token: config.token });
let shuttingDown = false;

bridge.on("screenshot", (payload) => {
  if (mobileWs.clientCount === 0) {
    return;
  }

  mobileWs.broadcastFrame({ type: "screenshot", ...payload });
});

bridge.on("status", (status) => {
  mobileWs.broadcast({ type: "status", status });
});

bridge.on("warning", (message) => {
  console.warn(`[warning] ${message}`);
  mobileWs.broadcast({ type: "warning", message });
});

mobileWs.on("connection", (client) => {
  client.send(JSON.stringify({ type: "status", status: bridge.status() }));
  updateScreencastStreaming();
});

mobileWs.on("disconnect", () => {
  updateScreencastStreaming();
});

mobileWs.on("message", async (client, rawMessage) => {
  try {
    const message = JSON.parse(rawMessage);
    await handleMobileMessage(client, message);
  } catch (error) {
    client.send(JSON.stringify({ type: "error", message: error.message }));
  }
});

const codexApp = await launchCodexApp(config);
mobileWs.start();
bridge.start();

server.listen(config.port, config.host, () => {
  const urls = getLanUrls(config.host, config.port, config.token);
  console.log(`[server] listening on ${config.host}:${config.port}`);
  console.log("[server] open one of these URLs on your mobile device:");
  for (const url of urls) {
    console.log(`  ${url}`);
  }
  if (urls[0]) {
    void printQrCode(urls[0]);
  }
  console.log(`[server] CDP endpoint: http://${config.cdpHost}:${config.cdpPort}`);

  if (config.openBrowser && urls[0]) {
    openUrl(urls[0]);
  }
});

server.on("error", (error) => {
  console.error(`[server] failed to listen on ${config.host}:${config.port}: ${error.message}`);
  void shutdown(1);
});

process.on("SIGINT", () => {
  void shutdown(0);
});
process.on("SIGTERM", () => {
  void shutdown(0);
});

async function handleMobileMessage(client, message) {
  if (message.type === "click") {
    const focusKeyboard = await bridge.clickAndCheckEditable(message.x, message.y);
    client.send(JSON.stringify({ type: "keyboard", focus: focusKeyboard }));
    return;
  }

  if (message.type === "scroll") {
    await bridge.scroll(message.x ?? 0.5, message.y ?? 0.5, message.deltaY ?? 0, message.deltaX ?? 0);
    return;
  }

  if (message.type === "text") {
    await bridge.insertText(String(message.text || ""));
    return;
  }

  if (message.type === "key") {
    await bridge.key(message.key);
    return;
  }

  if (message.type === "refresh") {
    await bridge.restartScreencast();
    return;
  }

  throw new Error(`unknown message type: ${message.type}`);
}

function updateScreencastStreaming() {
  bridge.setScreencastEnabled(mobileWs.clientCount > 0).catch((error) => {
    console.warn(`[warning] screencast update failed: ${error.message}`);
  });
}

async function printQrCode(url) {
  try {
    const qr = await QRCode.toString(url, {
      errorCorrectionLevel: "M",
      small: true,
      type: "terminal",
    });
    console.log("[server] scan this QR code on your mobile device:");
    console.log(qr.trimEnd());
  } catch (error) {
    console.warn(`[server] failed to render QR code: ${error.message}`);
  }
}

async function shutdown(code) {
  if (shuttingDown) {
    return;
  }

  shuttingDown = true;
  console.log("\n[server] shutting down");
  const forceExitTimer = setTimeout(() => process.exit(code), 5000);
  forceExitTimer.unref();

  mobileWs.stop();
  bridge.stop();

  await Promise.allSettled([
    closeServer(),
    quitCodexApp(codexApp),
  ]);

  clearTimeout(forceExitTimer);
  process.exit(code);
}

function closeServer() {
  return new Promise((resolve) => {
    if (!server.listening) {
      resolve();
      return;
    }

    const timeout = setTimeout(resolve, 1000);
    server.close((error) => {
      clearTimeout(timeout);
      if (error) {
        console.warn(`[server] failed to close HTTP server cleanly: ${error.message}`);
      }
      resolve();
    });
  });
}

function openUrl(url) {
  const command = process.platform === "darwin" ? "open" : process.platform === "win32" ? "cmd" : "xdg-open";
  const args = process.platform === "win32" ? ["/c", "start", "", url] : [url];
  const child = spawn(command, args, {
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}
