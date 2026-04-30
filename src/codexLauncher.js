import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { resolvePath } from "./config.js";

const DEFAULT_APP_NAMES = ["Codex.app", "OpenAI Codex.app"];
const DEFAULT_QUIT_GRACE_MS = 3000;

export async function launchCodexApp(config, logger = console) {
  if (!config.launch) {
    logger.log("[launcher] launch disabled; waiting for an existing CDP endpoint");
    return null;
  }

  const executablePath = resolveExecutablePath(config);
  if (!executablePath) {
    logger.warn("[launcher] Codex app not found. Set CODEX_APP_PATH or CODEX_EXECUTABLE, or start the app yourself with CDP enabled.");
    logger.warn(`[launcher] expected CDP at http://${config.cdpHost}:${config.cdpPort}`);
    return null;
  }

  const args = [
    `--remote-debugging-port=${config.cdpPort}`,
    "--remote-allow-origins=*",
  ];

  logger.log(`[launcher] starting ${executablePath}`);
  logger.log(`[launcher] CDP port: ${config.cdpPort}`);

  const child = spawn(executablePath, args, {
    detached: true,
    env: {
      ...process.env,
      ELECTRON_ENABLE_LOGGING: process.env.ELECTRON_ENABLE_LOGGING || "1",
    },
    stdio: "ignore",
  });

  child.unref();
  return child;
}

export async function quitCodexApp(child, logger = console, { graceMs = DEFAULT_QUIT_GRACE_MS } = {}) {
  if (!child?.pid) {
    return;
  }

  const pid = child.pid;
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }

  logger.log(`[launcher] quitting Codex app (pid ${pid})`);

  const exited = new Promise((resolve) => {
    child.once("exit", resolve);
  });

  try {
    signalProcessTree(pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") {
      logger.warn(`[launcher] failed to quit Codex app: ${error.message}`);
    }
    return;
  }

  const didExit = await Promise.race([
    exited.then(() => true),
    delay(graceMs).then(() => false),
  ]);

  if (didExit) {
    return;
  }

  logger.warn("[launcher] Codex app did not exit promptly; forcing shutdown");
  try {
    signalProcessTree(pid, "SIGKILL");
  } catch (error) {
    if (error.code !== "ESRCH") {
      logger.warn(`[launcher] failed to force quit Codex app: ${error.message}`);
    }
  }
}

export function resolveExecutablePath(config) {
  if (config.executablePath) {
    const executable = resolvePath(config.executablePath);
    return fileExists(executable) ? executable : "";
  }

  const appPath = findAppBundle(config.appPath);
  if (!appPath) {
    return "";
  }

  return executableFromAppBundle(appPath);
}

function findAppBundle(explicitPath) {
  if (explicitPath) {
    const resolved = resolvePath(explicitPath);
    return directoryExists(resolved) ? resolved : "";
  }

  const candidates = [];
  for (const appName of DEFAULT_APP_NAMES) {
    candidates.push(path.join("/Applications", appName));
    candidates.push(path.join(os.homedir(), "Applications", appName));
  }

  return candidates.find(directoryExists) || "";
}

function executableFromAppBundle(appPath) {
  const infoPath = path.join(appPath, "Contents", "Info.plist");
  const macosDir = path.join(appPath, "Contents", "MacOS");
  const executableName = readBundleExecutable(infoPath);

  if (executableName) {
    const executablePath = path.join(macosDir, executableName);
    if (fileExists(executablePath)) {
      return executablePath;
    }
  }

  const fallbackName = path.basename(appPath, ".app");
  const fallbackPath = path.join(macosDir, fallbackName);
  if (fileExists(fallbackPath)) {
    return fallbackPath;
  }

  try {
    const firstExecutable = fs
      .readdirSync(macosDir)
      .map((entry) => path.join(macosDir, entry))
      .find(fileExists);
    return firstExecutable || "";
  } catch {
    return "";
  }
}

function readBundleExecutable(infoPath) {
  try {
    const plist = fs.readFileSync(infoPath, "utf8");
    const match = plist.match(/<key>CFBundleExecutable<\/key>\s*<string>([^<]+)<\/string>/);
    return match?.[1] || "";
  } catch {
    return "";
  }
}

function fileExists(filePath) {
  try {
    return fs.statSync(filePath).isFile();
  } catch {
    return false;
  }
}

function directoryExists(filePath) {
  try {
    return fs.statSync(filePath).isDirectory();
  } catch {
    return false;
  }
}

function signalProcessTree(pid, signal) {
  if (process.platform === "win32") {
    const args = ["/pid", String(pid), "/T"];
    if (signal === "SIGKILL") {
      args.push("/F");
    }

    const child = spawn("taskkill", args, {
      detached: true,
      stdio: "ignore",
    });
    child.unref();
    return;
  }

  try {
    process.kill(-pid, signal);
  } catch (error) {
    if (error.code !== "ESRCH") {
      throw error;
    }
    process.kill(pid, signal);
  }
}

function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}
