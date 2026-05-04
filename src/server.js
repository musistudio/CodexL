#!/usr/bin/env node
import { spawn } from "node:child_process";
import QRCode from "qrcode";
import { CdpBridge } from "./cdpClient.js";
import { getLanUrls, helpText, parseConfig } from "./config.js";
import { launchCodexApp, quitCodexApp } from "./codexLauncher.js";
import { MobileWsServer } from "./mobileWsServer.js";
import { prepareCdpPort } from "./ports.js";
import { RemoteRelayClient } from "./remoteRelayClient.js";
import { createStaticServer } from "./staticServer.js";
import { deployWorker } from "./workerDeploy.js";

const config = parseConfig();

if (config.help) {
  process.stdout.write(helpText());
  process.exit(0);
}

if (config.command === "deploy") {
  const code = await deployWorker(config.deployArgs);
  process.exit(code);
}

if (config.modeError) {
  console.error(`[server] ${config.modeError}`);
  process.exit(1);
}

if (config.mode === "remote" && !config.remoteWorkerUrl) {
  console.error("[server] --remote-url is required when --mode remote is enabled");
  process.exit(1);
}

if (config.mode === "remote") {
  try {
    new URL(config.remoteWorkerUrl);
  } catch {
    console.error(`[server] invalid --remote-url: ${config.remoteWorkerUrl}`);
    process.exit(1);
  }
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
const remoteRelay = config.mode === "remote" ? new RemoteRelayClient(config) : null;
const networkStatsByTransport = new Map();
let shuttingDown = false;

if (remoteRelay) {
  console.log(`[remote] full URL: ${remoteRelay.mobileUrl}`);
}

bridge.on("screenshot", (payload) => {
  if (totalFrameClientCount() === 0) {
    return;
  }

  broadcastFrame({ type: "screenshot", ...payload });
});

bridge.on("status", (status) => {
  broadcast({ type: "status", status });
});

bridge.on("warning", (message) => {
  console.warn(`[warning] ${message}`);
  broadcast({ type: "warning", message });
});

bridge.on("profile", (profile) => {
  broadcast({ type: "profile", profile });
});

mobileWs.on("connection", (client, channel) => {
  handleTransportConnection(client, channel);
});

mobileWs.on("disconnect", () => {
  updateScreencastStreaming();
});

mobileWs.on("network", (stats) => {
  updateTransportNetworkStats("lan", stats);
});

mobileWs.on("message", async (client, rawMessage) => {
  try {
    const message = JSON.parse(rawMessage);
    await handleMobileMessage(mobileWs, client, message);
  } catch (error) {
    client.send(JSON.stringify({ type: "error", message: error.message }));
  }
});

if (remoteRelay) {
  remoteRelay.on("connection", (client, channel) => {
    handleTransportConnection(client, channel);
  });

  remoteRelay.on("disconnect", () => {
    updateScreencastStreaming();
  });

  remoteRelay.on("clientStats", () => {
    updateScreencastStreaming();
  });

  remoteRelay.on("network", (stats) => {
    updateTransportNetworkStats("remote", stats);
  });

  remoteRelay.on("message", async (client, rawMessage) => {
    try {
      const message = JSON.parse(rawMessage);
      await handleMobileMessage(remoteRelay, client, message);
    } catch (error) {
      client.send(JSON.stringify({ type: "error", message: error.message }));
    }
  });
}

const codexApp = await launchCodexApp(config);
mobileWs.start();
remoteRelay?.start();
bridge.start();

server.listen(config.port, config.host, () => {
  const urls = getLanUrls(config.host, config.port, config.token);
  console.log(`[server] listening on ${config.host}:${config.port}`);
  if (remoteRelay) {
    console.log("[server] remote mode enabled:");
    console.log(`  full URL: ${remoteRelay.mobileUrl}`);
    console.log("[server] local LAN URLs still available:");
  } else {
    console.log("[server] open one of these URLs on your mobile device:");
  }
  for (const localUrl of urls) {
    console.log(`  ${localUrl}`);
  }
  const qrUrl = remoteRelay?.mobileUrl || urls[0];
  if (qrUrl) {
    void printQrCode(qrUrl);
  }
  console.log(`[server] CDP endpoint: http://${config.cdpHost}:${config.cdpPort}`);

  if (config.openBrowser && qrUrl) {
    openUrl(qrUrl);
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

async function handleMobileMessage(transport, client, message) {
  if (message.type === "pong") {
    transport.notePong(message);
    return;
  }

  if (message.type === "viewport") {
    bridge.setClientViewport(message);
    return;
  }

  if (message.type === "click") {
    const focusKeyboard = await bridge.clickAndCheckEditable(message.x, message.y);
    client.send(JSON.stringify({ type: "keyboard", focus: focusKeyboard }));
    return;
  }

  if (message.type === "pointerMove") {
    await bridge.pointerMove(message.x, message.y);
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
    if (totalFrameClientCount() > 0) {
      await bridge.restartScreencast();
    }
    return;
  }

  if (message.type === "profileMode") {
    await bridge.setScreencastProfileMode(message.mode);
    broadcast({ type: "status", status: bridge.status() });
    return;
  }

  throw new Error(`unknown message type: ${message.type}`);
}

function updateScreencastStreaming() {
  bridge.setScreencastEnabled(totalFrameClientCount() > 0).catch((error) => {
    console.warn(`[warning] screencast update failed: ${error.message}`);
  });
}

function handleTransportConnection(client, channel) {
  if (channel === "control") {
    client.send(JSON.stringify({ type: "status", status: bridge.status() }));
  }
  updateScreencastStreaming();
}

function broadcast(payload) {
  mobileWs.broadcast(payload);
  remoteRelay?.broadcast(payload);
}

function broadcastFrame(payload) {
  mobileWs.broadcastFrame(payload);
  remoteRelay?.broadcastFrame(payload);
}

function totalFrameClientCount() {
  return mobileWs.frameClientCount + (remoteRelay?.frameClientCount || 0);
}

function updateTransportNetworkStats(name, stats) {
  networkStatsByTransport.set(name, stats);
  bridge.updateNetworkStats(aggregateNetworkStats());
}

function aggregateNetworkStats() {
  let bufferedAmount = 0;
  let droppedFramesInLast5s = 0;
  let frameClientCount = 0;
  let rtt = null;

  for (const stats of networkStatsByTransport.values()) {
    bufferedAmount = Math.max(bufferedAmount, Number(stats?.bufferedAmount || 0));
    droppedFramesInLast5s += Math.max(0, Number(stats?.droppedFramesInLast5s || 0));
    frameClientCount += Math.max(0, Number(stats?.frameClientCount || 0));
    const nextRtt = Number(stats?.rtt);
    if (Number.isFinite(nextRtt)) {
      rtt = rtt === null ? Math.max(0, nextRtt) : Math.max(rtt, nextRtt);
    }
  }

  return {
    bufferedAmount,
    droppedFramesInLast5s,
    frameClientCount,
    rtt,
  };
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
  remoteRelay?.stop();
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
