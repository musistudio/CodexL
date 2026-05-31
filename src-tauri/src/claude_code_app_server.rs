use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
#[cfg(unix)]
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub const PROVIDER_NAME: &str = "claude-code";

const PROTOCOL_VERSION: &str = "2025-11-25";
const BIN_ENV: &str = "CODEXL_CLAUDE_CODE_BIN";
const BASE_ARGS_ENV: &str = "CODEXL_CLAUDE_CODE_BASE_ARGS";
const EXTRA_ARGS_ENV: &str = "CODEXL_CLAUDE_CODE_EXTRA_ARGS";
const MODEL_ENV: &str = "CODEXL_CLAUDE_CODE_MODEL";
const PERMISSION_MODE_ENV: &str = "CODEXL_CLAUDE_CODE_PERMISSION_MODE";
const PERMISSION_PROMPT_TOOL_ENV: &str = "CODEXL_CLAUDE_CODE_PERMISSION_PROMPT_TOOL";
const TURN_IDLE_TIMEOUT_MS_ENV: &str = "CODEXL_CLAUDE_CODE_TURN_IDLE_TIMEOUT_MS";
const PERMISSION_APPROVAL_TIMEOUT_MS_ENV: &str =
    "CODEXL_CLAUDE_CODE_PERMISSION_APPROVAL_TIMEOUT_MS";
const CODEX_APP_SERVER_PROXY_ENV: &str = "CODEXL_CLAUDE_CODE_PROXY_CODEX_APP_SERVER";
const APP_SERVER_LOG_PATH_ENV: &str = "CODEXL_CLAUDE_CODE_APP_SERVER_LOG";
const CONTEXT_WINDOW_ENV: &str = "CODEXL_CLAUDE_CODE_CONTEXT_WINDOW";
const CLAUDE_PATH_ENV: &str = "CLAUDE_PATH";
const CLAUDE_PATH_OVERRIDE_ENV: &str = "CODEXL_CLAUDE_PATH";
const BROWSER_PLUGIN_NAME: &str = "browser";
const BROWSER_USE_PLUGIN_NAME: &str = "browser-use";
const COMPUTER_USE_PLUGIN_NAME: &str = "computer-use";
const OPENAI_BUNDLED_MARKETPLACE_NAME: &str = "openai-bundled";
const PROTECTED_BUNDLED_PLUGIN_NAMES: &[&str] = &[
    COMPUTER_USE_PLUGIN_NAME,
    BROWSER_PLUGIN_NAME,
    BROWSER_USE_PLUGIN_NAME,
];
const DEFAULT_MODEL: &str = "claude-code";
const DEFAULT_PERMISSION_PROMPT_TOOL: &str = "stdio";
const DEFAULT_CLAUDE_CONTEXT_WINDOW: i64 = 200_000;
const CLAUDE_ONE_M_CONTEXT_WINDOW: i64 = 1_000_000;
const DEFAULT_TURN_IDLE_TIMEOUT_MS: u64 = 60 * 60 * 1000;
const DEFAULT_PERMISSION_APPROVAL_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const MIN_NATIVE_CLAUDE_BYTES: u64 = 5 * 1024 * 1024;
const CLAUDE_THREAD_NAMES_FILE: &str = "codex-app-thread-names.json";
const CLAUDE_TITLE_MATCH_MAX_DELTA_SECONDS: u64 = 6 * 60 * 60;
const CLAUDE_RESULT_EXIT_GRACE_MS: u64 = 500;
const CLAUDE_THREAD_STREAM_STATE_HEARTBEAT_MS: u64 = 1_000;
const CLAUDE_CHILD_ENV_REMOVALS: &[&str] = &[
    "DISABLE_AUTOUPDATER",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS",
];
const COMPUTER_USE_NODE_RELAY_NODE_ENV: &str = "CODEXL_COMPUTER_USE_NODE_RELAY_NODE";
const COMPUTER_USE_NODE_RELAY_SCRIPT: &str = r#"
const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const readline = require("node:readline");

const DEFAULT_TOOL_CALL_TIMEOUT_MS = 90 * 1000;
const DEFAULT_LIST_APPS_TIMEOUT_MS = 30 * 1000;
const DEFAULT_GET_APP_STATE_TIMEOUT_MS = 20 * 1000;

function envDurationMs(name, defaultMs) {
  const raw = process.env[name];
  if (raw === undefined || raw === null || String(raw).trim() === "") return defaultMs;
  const parsed = Number(String(raw).trim());
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : defaultMs;
}

function envOptionalDurationMs(name) {
  const raw = process.env[name];
  if (raw === undefined || raw === null || String(raw).trim() === "") return null;
  const parsed = Number(String(raw).trim());
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

const configuredToolCallTimeoutMs = envOptionalDurationMs("CODEXL_COMPUTER_USE_TOOL_CALL_TIMEOUT_MS");
const toolCallTimeoutMs = configuredToolCallTimeoutMs ?? DEFAULT_TOOL_CALL_TIMEOUT_MS;

function logPath() {
  const explicit = process.env.CODEXL_CLAUDE_CODE_APP_SERVER_LOG;
  if (explicit && !/^(0|false|off|none)$/i.test(explicit)) return explicit;
  if (process.platform === "darwin") {
    return path.join(os.homedir(), "Library", "Logs", "com.openai.codex", "claude-code-app-server.log");
  }
  return path.join(os.homedir(), ".codexl", "claude-code-app-server.log");
}

function logEvent(event, fields = {}) {
  try {
    const file = logPath();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.appendFileSync(file, JSON.stringify({ tsMs: Date.now(), event, ...fields }) + "\n");
  } catch {
  }
}

function jsonRpcId(value) {
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return null;
}

function messageSummary(line) {
  try {
    const value = JSON.parse(line);
    const result = value && value.result;
    return {
      id: jsonRpcId(value && value.id),
      method: value && value.method,
      toolName: value && value.params && value.params.name,
      hasResult: !!result,
      hasError: !!(value && value.error),
      resultKeys: result && typeof result === "object" && !Array.isArray(result) ? Object.keys(result) : [],
      contentTypes: result && Array.isArray(result.content)
        ? result.content.map((item) => item && item.type).filter(Boolean)
        : [],
    };
  } catch {
    return { nonJson: true, preview: line.slice(0, 500) };
  }
}

function parseArgs(argv) {
  const options = {
    serverName: "",
    threadId: "",
    turnId: "",
    sessionId: "",
    cwd: "",
    command: "",
    args: [],
  };
  let commandIndex = -1;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--") {
      commandIndex = index + 1;
      break;
    }
    if (arg === "--server-name") options.serverName = argv[++index] || "";
    else if (arg === "--thread-id") options.threadId = argv[++index] || "";
    else if (arg === "--turn-id") options.turnId = argv[++index] || "";
    else if (arg === "--session-id") options.sessionId = argv[++index] || "";
    else if (arg === "--cwd") options.cwd = argv[++index] || "";
  }
  if (commandIndex < 0 || !argv[commandIndex]) {
    throw new Error("missing Computer Use child command");
  }
  options.command = argv[commandIndex];
  options.args = argv.slice(commandIndex + 1);
  return options;
}

function ensureObject(parent, key) {
  if (!parent[key] || typeof parent[key] !== "object" || Array.isArray(parent[key])) {
    parent[key] = {};
  }
  return parent[key];
}

function isBlockedComputerUseEnvKey(key) {
  const upper = key.toUpperCase();
  const blockedKeys = new Set([
    "CODEX_HOME",
    "CODEX_CLI_PATH",
    "CODEXL_REAL_CODEX_CLI_PATH",
    "CODEXL_CLAUDE_CODE_APP_SERVER_LOG",
    "CODEXL_CLAUDE_CODE_BIN",
    "CODEXL_CLAUDE_CODE_ARGS",
    "CODEXL_CLAUDE_CODE_EXTRA_ARGS",
    "CODEXL_CLAUDE_CODE_MODEL",
    "CODEXL_CLAUDE_CODE_PERMISSION_MODE",
    "CODEXL_CLAUDE_CODE_PERMISSION_PROMPT_TOOL",
    "CODEXL_CLAUDE_CODE_PROXY_CODEX_APP_SERVER",
    "CODEXL_COMPUTER_USE_NODE_RELAY_NODE",
    "CODEXL_COMPUTER_USE_TOOL_CALL_TIMEOUT_MS",
    "DISABLE_AUTOUPDATER",
  ]);
  return (
    blockedKeys.has(upper) ||
    upper.includes("CLAUDE") ||
    upper.startsWith("ANTHROPIC_") ||
    upper.startsWith("CCR_")
  );
}

function sanitizedComputerUseEnv(options) {
  const allowedKeys = [
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LOGNAME",
    "PATH",
    "SHELL",
    "SSH_AUTH_SOCK",
    "TERM",
    "TMPDIR",
    "USER",
    "XPC_FLAGS",
    "XPC_SERVICE_NAME",
    "__CFBundleIdentifier",
    "__CF_USER_TEXT_ENCODING",
  ];
  const env = {};
  for (const key of allowedKeys) {
    if (process.env[key] && !isBlockedComputerUseEnvKey(key)) {
      env[key] = process.env[key];
    }
  }
  if (!env.PATH) env.PATH = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";
  if (!env.TMPDIR) env.TMPDIR = os.tmpdir();
  env.CODEX_SESSION_ID = options.sessionId;
  env.CODEX_THREAD_ID = options.threadId;
  env.CODEX_TURN_ID = options.turnId;
  return env;
}

function isListAppsToolCall(message) {
  return !!(
    message &&
    message.method === "tools/call" &&
    message.params &&
    message.params.name === "list_apps"
  );
}

function isGetAppStateToolCall(message) {
  return !!(
    message &&
    message.method === "tools/call" &&
    message.params &&
    message.params.name === "get_app_state"
  );
}

function timeoutMsForToolCall(message) {
  if (configuredToolCallTimeoutMs !== null) return configuredToolCallTimeoutMs;
  if (isListAppsToolCall(message)) return DEFAULT_LIST_APPS_TIMEOUT_MS;
  if (isGetAppStateToolCall(message)) return DEFAULT_GET_APP_STATE_TIMEOUT_MS;
  return toolCallTimeoutMs;
}

function appNameFromPath(appPath) {
  return path.basename(appPath).replace(/\.app$/i, "");
}

function listRunningAppNames() {
  if (process.platform !== "darwin") return new Set();
  try {
    const result = spawnSync("/usr/bin/osascript", [
      "-e",
      'tell application "System Events" to get name of application processes whose background only is false',
    ], {
      encoding: "utf8",
      env: sanitizedComputerUseEnv(options),
      timeout: 3000,
    });
    if (result.status !== 0 || !result.stdout) return new Set();
    return new Set(result.stdout.split(",").map((name) => name.trim()).filter(Boolean));
  } catch {
    return new Set();
  }
}

function listAppBundlesFromDirectory(root, depth, output, seen) {
  if (!root || depth < 0) return;
  let entries = [];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
    const appPath = path.join(root, entry.name);
    if (/\.app$/i.test(entry.name)) {
      const realPath = appPath.endsWith(path.sep) ? appPath : `${appPath}${path.sep}`;
      if (!seen.has(realPath)) {
        seen.add(realPath);
        output.push({ name: appNameFromPath(appPath), path: realPath });
      }
      continue;
    }
    if (depth > 0) {
      listAppBundlesFromDirectory(appPath, depth - 1, output, seen);
    }
  }
}

function fallbackListAppsText() {
  const roots = [
    "/Applications",
    path.join(os.homedir(), "Applications"),
    "/System/Applications",
    "/System/Applications/Utilities",
    "/System/Library/CoreServices",
  ];
  const apps = [];
  const seen = new Set();
  for (const root of roots) {
    listAppBundlesFromDirectory(root, 2, apps, seen);
  }
  const running = listRunningAppNames();
  apps.sort((a, b) => {
    const runningDelta = Number(running.has(b.name)) - Number(running.has(a.name));
    if (runningDelta !== 0) return runningDelta;
    return a.name.localeCompare(b.name);
  });
  return apps
    .slice(0, 300)
    .map((app) => `${app.name} — ${app.path}${running.has(app.name) ? " [running]" : ""}`)
    .join("\n");
}

function fallbackListAppsResponse(id, reason) {
  return {
    jsonrpc: "2.0",
    id,
    result: {
      _meta: {
        "codexl/fallback": {
          source: "macos-app-bundles",
          reason,
        },
      },
      content: [
        {
          type: "text",
          text: fallbackListAppsText(),
        },
      ],
    },
  };
}

function respondWithFallbackListApps(message, reason) {
  const requestId = jsonRpcId(message && message.id) || "unknown";
  logEvent("computer_use_node_relay_list_apps_fallback", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    requestId,
    error: reason,
  });
  process.stdout.write(JSON.stringify(fallbackListAppsResponse(message.id, reason)) + "\n");
}

function appArgument(message) {
  const args = message && message.params && message.params.arguments;
  const value = args && typeof args === "object" && !Array.isArray(args) ? args.app : null;
  return typeof value === "string" && value.trim() ? value.trim() : "";
}

function appProcessName(app) {
  if (!app) return "";
  if (app.includes("/")) return appNameFromPath(app);
  if (/\.app$/i.test(app)) return appNameFromPath(app);
  return app;
}

function appleScriptString(value) {
  return String(value || "").replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function runSync(command, args, timeout) {
  try {
    const result = spawnSync(command, args, {
      encoding: "utf8",
      env: childEnv,
      timeout,
    });
    return {
      status: result.status,
      stdout: result.stdout || "",
      stderr: result.stderr || "",
      error: result.error ? String(result.error && result.error.message || result.error) : "",
    };
  } catch (error) {
    return { status: null, stdout: "", stderr: "", error: String(error && error.message || error) };
  }
}

function runOsascript(script, timeout = 3000) {
  return runSync("/usr/bin/osascript", ["-e", script], timeout);
}

function openAppForFallback(app) {
  if (process.platform !== "darwin" || !app) return { attempted: false, status: null, stderr: "" };
  const args = app.includes("/") ? [app] : ["-a", app];
  const result = runSync("/usr/bin/open", args, 5000);
  return { attempted: true, status: result.status, stderr: result.stderr || result.error || "" };
}

function appIsRunning(appName) {
  if (process.platform !== "darwin" || !appName) return false;
  const escaped = appleScriptString(appName);
  const result = runOsascript(`tell application "System Events" to exists process "${escaped}"`, 3000);
  return /^true$/i.test(result.stdout.trim());
}

function waitForAppRunning(appName, attempts = 5) {
  for (let index = 0; index < attempts; index += 1) {
    if (appIsRunning(appName)) return true;
    runSync("/bin/sleep", ["1"], 1500);
  }
  return appIsRunning(appName);
}

function appWindowNames(appName) {
  if (process.platform !== "darwin" || !appName) return { windows: [], error: "" };
  const escaped = appleScriptString(appName);
  const result = runOsascript(
    `tell application "System Events" to tell process "${escaped}" to get name of windows`,
    4000,
  );
  if (result.status !== 0) {
    return { windows: [], error: result.stderr || result.error || result.stdout };
  }
  const windows = result.stdout
    .split(",")
    .map((name) => name.trim())
    .filter(Boolean);
  return { windows, error: "" };
}

function frontmostAppName() {
  if (process.platform !== "darwin") return "";
  const result = runOsascript(
    'tell application "System Events" to get name of first application process whose frontmost is true',
    3000,
  );
  return result.status === 0 ? result.stdout.trim() : "";
}

function screenshotContentForFallback() {
  if (process.platform !== "darwin") return null;
  const file = path.join(os.tmpdir(), `codexl-computer-use-fallback-${process.pid}-${Date.now()}.jpg`);
  const result = runSync("/usr/sbin/screencapture", ["-x", "-t", "jpg", file], 8000);
  if (result.status !== 0) {
    logEvent("computer_use_node_relay_get_app_state_fallback_screenshot_error", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      error: result.stderr || result.error || result.stdout,
    });
    return null;
  }
  try {
    const data = fs.readFileSync(file).toString("base64");
    fs.unlinkSync(file);
    return { type: "image", data, mimeType: "image/jpeg" };
  } catch (error) {
    try { fs.unlinkSync(file); } catch {}
    logEvent("computer_use_node_relay_get_app_state_fallback_screenshot_read_error", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      error: String(error && error.message || error),
    });
    return null;
  }
}

function fallbackGetAppStateResponse(id, message, reason) {
  const app = appArgument(message);
  const processName = appProcessName(app);
  const openResult = openAppForFallback(app);
  const running = waitForAppRunning(processName);
  const windows = appWindowNames(processName);
  const frontmost = frontmostAppName();
  const content = [
    {
      type: "text",
      text: [
        "Computer Use get_app_state fallback was used because the native Computer Use MCP client did not respond.",
        `Reason: ${reason}`,
        `Requested app: ${app || "(missing)"}`,
        `Process name: ${processName || "(unknown)"}`,
        `Open attempted: ${openResult.attempted ? "yes" : "no"}`,
        `Open status: ${openResult.status === null ? "unknown" : openResult.status}`,
        openResult.stderr ? `Open stderr: ${openResult.stderr.trim()}` : "",
        `Running: ${running ? "yes" : "no"}`,
        `Frontmost app: ${frontmost || "(unknown)"}`,
        `Windows: ${windows.windows.length ? windows.windows.join(" | ") : "(none found)"}`,
        windows.error ? `Accessibility/window error: ${windows.error.trim()}` : "",
        "Screenshot: full-screen fallback image is attached when macOS screen capture succeeds.",
      ].filter(Boolean).join("\n"),
    },
  ];
  const screenshot = screenshotContentForFallback();
  if (screenshot) content.push(screenshot);
  return {
    jsonrpc: "2.0",
    id,
    result: {
      _meta: {
        "codexl/fallback": {
          source: "macos-open-system-events-screencapture",
          reason,
        },
      },
      content,
    },
  };
}

function fallbackResponseForToolCall(message, reason) {
  if (isGetAppStateToolCall(message)) {
    return fallbackGetAppStateResponse(message.id, message, reason);
  }
  return null;
}

function injectTurnMetadata(line, options) {
  let value;
  try {
    value = JSON.parse(line);
  } catch {
    return line;
  }
  if (!value || value.method !== "tools/call") return line;
  const metadata = {
    type: "thread-id",
    "thread-id": options.threadId,
    threadId: options.threadId,
    "turn-id": options.turnId,
    turnId: options.turnId,
    session_id: options.sessionId,
    turn_id: options.turnId,
    codex_session_id: options.sessionId,
    codex_thread_id: options.threadId,
    cwd: options.cwd,
    source: "claude-code",
    server: options.serverName,
  };
  const params = ensureObject(value, "params");
  const meta = ensureObject(params, "_meta");
  meta["x-codex-turn-metadata"] = metadata;
  meta.codexTurnMetadata = metadata;
  const headers = ensureObject(params, "headers");
  headers["x-codex-turn-metadata"] = JSON.stringify(metadata);
  return JSON.stringify(value);
}

const options = parseArgs(process.argv.slice(2));
logEvent("computer_use_node_relay_start", {
  serverName: options.serverName,
  threadId: options.threadId,
  turnId: options.turnId,
  sessionId: options.sessionId,
  command: options.command,
  args: options.args,
});
const childEnv = sanitizedComputerUseEnv(options);
const childProcesses = new Set();
const pendingMainToolCalls = new Map();
const staleMainToolCallResponseIds = new Set();
const internalMainResponseIds = new Set();
const retiredMainChildPids = new Set();
let initializeParams = {
  clientInfo: { name: "codexl-computer-use-relay", version: "1" },
  capabilities: {},
};
let shuttingDown = false;

function spawnComputerUseChild(label) {
  const child = spawn(options.command, options.args, {
    cwd: process.cwd(),
    env: childEnv,
    stdio: ["pipe", "pipe", "pipe"],
  });
  childProcesses.add(child);
  logEvent("computer_use_node_relay_child_spawned", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    label,
    pid: child.pid,
    detached: false,
    envKeys: Object.keys(childEnv).sort(),
  });
  child.stderr.on("data", (chunk) => {
    process.stderr.write(chunk);
    logEvent("computer_use_node_relay_child_stderr", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      label,
      preview: chunk.toString("utf8").slice(0, 1000),
    });
  });
  child.on("error", (error) => {
    logEvent("computer_use_node_relay_child_error", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      label,
      error: String(error && error.message || error),
    });
  });
  child.on("close", (code, signal) => {
    childProcesses.delete(child);
    rejectPendingMainToolCalls(`Computer Use MCP child closed${signal ? ` (${signal})` : ""}`);
    logEvent("computer_use_node_relay_child_close", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      label,
      code,
      signal,
    });
    if (label === "main" && retiredMainChildPids.delete(child.pid)) {
      return;
    }
    if (label === "main" && !shuttingDown) {
      process.exit(code ?? (signal ? 1 : 0));
    }
  });
  child.stdin.on("error", (error) => {
    logEvent("computer_use_node_relay_child_stdin_error", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      label,
      error: String(error && error.message || error),
    });
  });
  return child;
}

let child = null;
function attachMainChild(nextChild) {
  child = nextChild;
  let stdoutBuffer = "";
  child.stdout.on("data", (chunk) => {
    stdoutBuffer += chunk.toString("utf8");
    let index;
    while ((index = stdoutBuffer.indexOf("\n")) >= 0) {
      const line = stdoutBuffer.slice(0, index).trim();
      stdoutBuffer = stdoutBuffer.slice(index + 1);
      if (!line) continue;
      handleMainChildStdoutLine(line);
    }
  });
}
attachMainChild(spawnComputerUseChild("main"));
process.stdout.on("drain", () => child.stdout.resume());

function parseJsonLine(line) {
  try {
    return JSON.parse(line);
  } catch {
    return null;
  }
}

function jsonRpcErrorResponse(id, message) {
  return {
    jsonrpc: "2.0",
    id,
    error: {
      code: -32000,
      message,
    },
  };
}

function markStaleMainToolCallResponseId(requestId) {
  staleMainToolCallResponseIds.add(requestId);
  setTimeout(() => staleMainToolCallResponseIds.delete(requestId), 5 * 60 * 1000).unref();
}

function failMainToolCall(requestId, message, error, event) {
  const pending = pendingMainToolCalls.get(requestId);
  if (!pending) return;
  if (pending.timeout) clearTimeout(pending.timeout);
  pendingMainToolCalls.delete(requestId);
  markStaleMainToolCallResponseId(requestId);
  logEvent(event, {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    requestId,
    toolName: message && message.params && message.params.name,
    error,
  });
  const fallback = fallbackResponseForToolCall(message, error);
  if (fallback) {
    logEvent("computer_use_node_relay_main_tool_call_fallback", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      requestId,
      toolName: message && message.params && message.params.name,
      error,
    });
    process.stdout.write(JSON.stringify(fallback) + "\n");
  } else {
    process.stdout.write(JSON.stringify(jsonRpcErrorResponse(message.id, error)) + "\n");
  }
  if (event === "computer_use_node_relay_main_tool_call_timeout") {
    restartMainChild(error);
  }
}

function rejectPendingMainToolCalls(error) {
  for (const [requestId, pending] of Array.from(pendingMainToolCalls.entries())) {
    failMainToolCall(requestId, pending.message, error, "computer_use_node_relay_main_tool_call_error");
  }
}

function sendMainToolCall(line, message) {
  const requestId = jsonRpcId(message.id);
  if (!requestId) {
    if (!child.stdin.write(line + "\n")) {
      rl.pause();
    }
    return;
  }
  staleMainToolCallResponseIds.delete(requestId);
  const timeoutMs = timeoutMsForToolCall(message);
  const pending = {
    message,
    timeout: null,
  };
  if (Number.isFinite(timeoutMs) && timeoutMs > 0) {
    pending.timeout = setTimeout(() => {
      failMainToolCall(
        requestId,
        message,
        `timeout main-tool-call-${requestId}`,
        "computer_use_node_relay_main_tool_call_timeout",
      );
    }, timeoutMs);
    pending.timeout.unref();
  }
  pendingMainToolCalls.set(requestId, pending);
  logEvent("computer_use_node_relay_main_tool_call_send", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    requestId,
    toolName: message && message.params && message.params.name,
    timeoutMs,
  });
  if (!child.stdin.write(line + "\n", (error) => {
    if (error) {
      failMainToolCall(
        requestId,
        message,
        String(error && error.message || error),
        "computer_use_node_relay_main_tool_call_stdin_error",
      );
    }
  })) {
    rl.pause();
  }
}

function sendInternalMainRequest(message, reason) {
  const requestId = jsonRpcId(message && message.id);
  if (requestId) internalMainResponseIds.add(requestId);
  logEvent("computer_use_node_relay_main_internal_send", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    requestId,
    method: message && message.method,
    reason,
  });
  child.stdin.write(JSON.stringify(message) + "\n");
}

function restartMainChild(reason) {
  if (shuttingDown) return;
  const previous = child;
  if (previous && previous.pid) retiredMainChildPids.add(previous.pid);
  logEvent("computer_use_node_relay_main_restart", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    reason,
    oldPid: previous && previous.pid,
  });
  if (previous && !previous.killed) previous.kill("SIGTERM");
  attachMainChild(spawnComputerUseChild("main"));
  const restartId = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  sendInternalMainRequest({
    jsonrpc: "2.0",
    id: `codexl-restart-init-${restartId}`,
    method: "initialize",
    params: initializeParams,
  }, reason);
  child.stdin.write(JSON.stringify({
    jsonrpc: "2.0",
    method: "notifications/initialized",
    params: {},
  }) + "\n");
  sendInternalMainRequest({
    jsonrpc: "2.0",
    id: `codexl-restart-tools-${restartId}`,
    method: "tools/list",
    params: {},
  }, reason);
}

function handleMainChildStdoutLine(line) {
  logEvent("computer_use_node_relay_child_stdout", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    label: "main",
    ...messageSummary(line),
  });
  const value = parseJsonLine(line);
  const responseId = value && !value.method ? jsonRpcId(value.id) : null;
  if (responseId && internalMainResponseIds.has(responseId)) {
    internalMainResponseIds.delete(responseId);
    logEvent("computer_use_node_relay_main_internal_response", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      requestId: responseId,
      hasResult: !!value.result,
      hasError: !!value.error,
    });
    return;
  }
  if (responseId && staleMainToolCallResponseIds.has(responseId)) {
    staleMainToolCallResponseIds.delete(responseId);
    logEvent("computer_use_node_relay_main_tool_call_late_response_dropped", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      requestId: responseId,
    });
    return;
  }
  if (responseId && pendingMainToolCalls.has(responseId)) {
    const pending = pendingMainToolCalls.get(responseId);
    if (pending.timeout) clearTimeout(pending.timeout);
    pendingMainToolCalls.delete(responseId);
    logEvent("computer_use_node_relay_main_tool_call_response", {
      serverName: options.serverName,
      threadId: options.threadId,
      turnId: options.turnId,
      requestId: responseId,
      hasResult: !!value.result,
      hasError: !!value.error,
    });
  }
  if (!process.stdout.write(line + "\n")) {
    child.stdout.pause();
  }
}

function shutdown(signal) {
  if (shuttingDown) return;
  shuttingDown = true;
  logEvent("computer_use_node_relay_signal", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    signal,
    childPid: child.pid,
  });
  for (const process of childProcesses) {
    if (!process.killed) process.kill(signal);
  }
  setTimeout(() => process.exit(1), 1000).unref();
}
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("SIGTERM", () => shutdown("SIGTERM"));

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on("line", (line) => {
  const transformed = injectTurnMetadata(line, options);
  const message = parseJsonLine(transformed);
  logEvent("computer_use_node_relay_stdin", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
    injected: transformed !== line,
    ...messageSummary(transformed),
  });
  if (message && message.method === "initialize" && message.params) {
    initializeParams = message.params;
  }
  if (message && isListAppsToolCall(message)) {
    respondWithFallbackListApps(message, "handled by codexl relay without invoking Computer Use MCP list_apps");
    return;
  }
  if (message && message.method === "tools/call") {
    sendMainToolCall(transformed, message);
    return;
  }
  if (!child.stdin.write(transformed + "\n")) {
    rl.pause();
  }
});
child.stdin.on("drain", () => rl.resume());
rl.on("close", () => {
  logEvent("computer_use_node_relay_stdin_close", {
    serverName: options.serverName,
    threadId: options.threadId,
    turnId: options.turnId,
  });
  child.stdin.end();
});
"#;
const CLAUDE_STREAM_JSON_ARGS: &[&str] = &[
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--include-partial-messages",
];

type SharedOutput<W> = Arc<Mutex<W>>;
type SharedState = Arc<Mutex<ClaudeAppServerState>>;

#[derive(Debug, Clone)]
struct RunOptions {
    workspace_name: Option<String>,
}

#[derive(Debug)]
struct ClaudeAppServerState {
    active_processes: BTreeMap<(String, String), u32>,
    app_responses: BTreeMap<String, Value>,
    interrupted_turns: BTreeSet<(String, String)>,
    threads: BTreeMap<String, ClaudeThread>,
    workspace_name: Option<String>,
}

#[derive(Debug, Clone)]
struct ClaudeThread {
    id: String,
    session_id: String,
    claude_session_id: String,
    path: Option<String>,
    preview: String,
    cwd: String,
    model: String,
    created_at: i64,
    updated_at: i64,
    archived: bool,
    name: Option<String>,
    turns: Vec<ClaudeTurn>,
    latest_token_usage_info: Option<Value>,
}

#[derive(Debug, Clone)]
struct ClaudeGeneratedTitle {
    source_prompt: String,
    title: Option<String>,
    cwd: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct ClaudeTurn {
    id: String,
    input: Vec<Value>,
    tool_items: Vec<Value>,
    agent_text: String,
    status: TurnStatus,
    error: Option<String>,
    started_at: i64,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug)]
struct TurnWork {
    thread_id: String,
    turn_id: String,
    agent_item_id: String,
    cli_item_id: String,
    claude_session_id: String,
    cwd: String,
    prompt: String,
    resume_existing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaleActiveProcess {
    thread_id: String,
    turn_id: String,
    pid: u32,
}

#[derive(Debug)]
struct ClaudeRunResult {
    text: String,
    error: Option<String>,
    duration_ms: i64,
    tool_items: Vec<Value>,
    agent_item_streamed: bool,
    latest_token_usage_info: Option<Value>,
}

#[derive(Debug, Default)]
struct ClaudeStreamState {
    emitted_text: String,
    pending_agent_text: String,
    suppressed_agent_prefix: String,
    result_text: Option<String>,
    result_error: Option<String>,
    latest_token_usage_info: Option<Value>,
    latest_model: Option<String>,
    agent_item_started: bool,
    reasoning_item_started: bool,
    reasoning_text: String,
    saw_tool_call: bool,
    seen_tool_ids: BTreeSet<String>,
    tool_block_by_index: BTreeMap<i64, String>,
    tool_input_deltas: BTreeMap<String, String>,
    tool_calls: BTreeMap<String, ClaudeToolCallState>,
    completed_tool_ids: BTreeSet<String>,
    completed_tool_items: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ClaudeToolCallState {
    name: String,
    arguments: Value,
    started_at_ms: i64,
    started_emitted: bool,
    kind: ClaudeToolItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeToolItemKind {
    CommandExecution,
    CollabAgentToolCall,
    McpToolCall,
}

pub fn run_stdio_app_server(args: Vec<OsString>) -> Result<i32, String> {
    run_stdio_app_server_with_io(args, std::io::stdin(), std::io::stdout())
}

pub(crate) fn run_stdio_app_server_with_io<R, W>(
    args: Vec<OsString>,
    input: R,
    output: W,
) -> Result<i32, String>
where
    R: Read,
    W: Write + Send + 'static,
{
    let options = parse_options(args);
    claude_code_log_event(
        "app_server_start",
        json!({
            "workspaceName": options.workspace_name,
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
        }),
    );
    let state = Arc::new(Mutex::new(ClaudeAppServerState {
        active_processes: BTreeMap::new(),
        app_responses: BTreeMap::new(),
        interrupted_turns: BTreeSet::new(),
        threads: BTreeMap::new(),
        workspace_name: options.workspace_name,
    }));
    let output = Arc::new(Mutex::new(output));
    let mut workers = Vec::new();
    let mut reader = BufReader::new(input);
    let mut line = Vec::new();

    loop {
        line.clear();
        let size = reader
            .read_until(b'\n', &mut line)
            .map_err(|err| format!("failed to read app-server stdin: {}", err))?;
        if size == 0 {
            break;
        }
        if let Some(worker) = handle_client_line(&line, Arc::clone(&state), Arc::clone(&output))? {
            workers.push(worker);
        }
    }

    for worker in workers {
        worker
            .join()
            .map_err(|_| "claude-code turn worker panicked".to_string())?;
    }
    claude_code_log_event(
        "app_server_stop",
        json!({
            "pid": std::process::id(),
        }),
    );
    Ok(0)
}

fn parse_options(args: Vec<OsString>) -> RunOptions {
    let mut workspace_name = None;
    let args = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace-name" => {
                index += 1;
                workspace_name = args.get(index).map(|value| value.trim().to_string());
            }
            _ => {}
        }
        index += 1;
    }
    RunOptions { workspace_name }
}

fn claude_code_log_event(event: &str, fields: Value) {
    let Some(path) = claude_code_app_server_log_path() else {
        return;
    };
    let mut object = serde_json::Map::new();
    object.insert("tsMs".to_string(), json!(now_millis()));
    object.insert("event".to_string(), json!(event));
    if let Value::Object(fields) = fields {
        for (key, value) in fields {
            object.insert(key, value);
        }
    } else {
        object.insert("data".to_string(), fields);
    }
    let Ok(line) = serde_json::to_string(&Value::Object(object)) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn claude_code_app_server_log_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(APP_SERVER_LOG_PATH_ENV) {
        let value = value.to_string_lossy();
        let value = value.trim();
        if value.is_empty()
            || matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "none"
            )
        {
            return None;
        }
        return Some(expand_log_path(value));
    }
    if cfg!(test) {
        return None;
    }
    user_home_dir_for_log().map(|home| {
        if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Logs")
                .join("com.openai.codex")
                .join("claude-code-app-server.log")
        } else {
            home.join(".codexl").join("claude-code-app-server.log")
        }
    })
}

fn expand_log_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = user_home_dir_for_log() {
            return home;
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = user_home_dir_for_log() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

fn user_home_dir_for_log() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.to_string_lossy().trim().is_empty())
        .map(PathBuf::from)
}

fn log_json_rpc_id(id: &Value) -> Value {
    json_rpc_id_key(id)
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn log_request_params_summary(params: &Value) -> Value {
    match params {
        Value::Object(map) => json!({
            "kind": "object",
            "keys": map.keys().cloned().collect::<Vec<_>>(),
        }),
        Value::Array(values) => json!({
            "kind": "array",
            "len": values.len(),
        }),
        Value::Null => json!({ "kind": "null" }),
        _ => json!({ "kind": "scalar" }),
    }
}

fn log_text_preview(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn turn_work_log_fields(work: &TurnWork) -> Value {
    json!({
        "threadId": &work.thread_id,
        "turnId": &work.turn_id,
        "claudeSessionId": &work.claude_session_id,
        "cwd": &work.cwd,
        "resumeExisting": work.resume_existing,
        "titleGeneration": is_claude_title_generation_prompt(&work.prompt),
    })
}

fn stream_log_summary(stream: &ClaudeStreamState) -> Value {
    json!({
        "sawToolCall": stream.saw_tool_call,
        "toolCalls": stream
            .tool_calls
            .iter()
            .map(|(id, state)| {
                json!({
                    "id": id,
                    "name": &state.name,
                    "startedEmitted": state.started_emitted,
                    "completed": stream.completed_tool_ids.contains(id),
                })
            })
            .collect::<Vec<_>>(),
        "completedToolIds": stream.completed_tool_ids.iter().cloned().collect::<Vec<_>>(),
        "resultSeen": claude_stream_result_seen(stream),
        "tokenUsageSeen": stream.latest_token_usage_info.is_some(),
        "latestModel": stream.latest_model.as_deref(),
        "emittedTextBytes": stream.emitted_text.len(),
        "pendingAgentTextBytes": stream.pending_agent_text.len(),
    })
}

fn claude_message_log_summary(message: &Value) -> Value {
    let message_type = message.get("type").and_then(Value::as_str);
    let mut summary = serde_json::Map::new();
    summary.insert(
        "type".to_string(),
        message_type.map(Value::from).unwrap_or(Value::Null),
    );
    if let Some(parent_tool_use_id) = message.get("parent_tool_use_id").and_then(Value::as_str) {
        summary.insert("parentToolUseId".to_string(), json!(parent_tool_use_id));
    }
    match message_type {
        Some("stream_event") => {
            if let Some(event) = message.get("event") {
                summary.insert(
                    "streamEventType".to_string(),
                    event
                        .get("type")
                        .and_then(Value::as_str)
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                );
                if let Some(content_block) = event.get("content_block") {
                    summary.insert(
                        "contentBlockType".to_string(),
                        content_block
                            .get("type")
                            .and_then(Value::as_str)
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    );
                    summary.insert(
                        "toolId".to_string(),
                        content_block
                            .get("id")
                            .and_then(Value::as_str)
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    );
                    summary.insert(
                        "toolName".to_string(),
                        content_block
                            .get("name")
                            .and_then(Value::as_str)
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    );
                }
            }
        }
        Some("assistant") | Some("user") => {
            if let Some(content) = message
                .get("message")
                .and_then(|message| message.get("content"))
            {
                summary.insert(
                    "contentTypes".to_string(),
                    claude_content_type_summary(content),
                );
            }
        }
        Some("result") => {
            summary.insert(
                "isError".to_string(),
                json!(message
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)),
            );
            summary.insert(
                "usage".to_string(),
                json!(claude_result_usage_summary(message)),
            );
            summary.insert(
                "resultPreview".to_string(),
                json!(message
                    .get("result")
                    .and_then(Value::as_str)
                    .map(|value| log_text_preview(value, 300))),
            );
        }
        Some("control_request") => {
            summary.insert(
                "requestId".to_string(),
                json!(claude_control_request_id(message)),
            );
            summary.insert(
                "subtype".to_string(),
                json!(claude_control_request_subtype(message)),
            );
            summary.insert(
                "toolName".to_string(),
                json!(claude_permission_tool_name(message)),
            );
            summary.insert(
                "serverName".to_string(),
                json!(claude_permission_server_name(message)),
            );
            if let Some(request) = message.get("request").and_then(Value::as_object) {
                summary.insert(
                    "requestKeys".to_string(),
                    json!(request.keys().cloned().collect::<Vec<_>>()),
                );
            }
            summary.insert(
                "input".to_string(),
                claude_permission_request_input(message)
                    .map(log_request_params_summary)
                    .unwrap_or_else(|| json!({ "kind": "missing" })),
            );
        }
        Some("system") => {
            if let Some(map) = message.as_object() {
                summary.insert(
                    "keys".to_string(),
                    json!(map.keys().cloned().collect::<Vec<_>>()),
                );
            }
            for key in ["subtype", "session_id", "model", "cwd"] {
                if let Some(value) = message.get(key).and_then(Value::as_str) {
                    summary.insert(key.to_string(), json!(value));
                }
            }
            if let Some(preview) =
                first_non_empty_string_at(message, &["/message", "/content", "/error"])
            {
                summary.insert(
                    "preview".to_string(),
                    json!(log_text_preview(&preview, 300)),
                );
            }
            if let Some(tools) = message.get("tools").and_then(Value::as_array) {
                summary.insert("toolCount".to_string(), json!(tools.len()));
                summary.insert(
                    "toolNames".to_string(),
                    json!(tools
                        .iter()
                        .filter_map(|tool| first_non_empty_string_at(
                            tool,
                            &["/name", "/tool_name"]
                        ))
                        .take(50)
                        .collect::<Vec<_>>()),
                );
            }
            let mcp_servers = message
                .get("mcp_servers")
                .or_else(|| message.get("mcpServers"));
            if let Some(mcp_servers) = mcp_servers {
                summary.insert(
                    "mcpServers".to_string(),
                    claude_system_mcp_servers_log_summary(mcp_servers),
                );
            }
        }
        _ => {}
    }
    Value::Object(summary)
}

fn claude_system_mcp_servers_log_summary(value: &Value) -> Value {
    match value {
        Value::Array(servers) => json!(servers
            .iter()
            .map(|server| {
                json!({
                    "name": first_non_empty_string_at(server, &["/name", "/server_name", "/serverName"]),
                    "status": first_non_empty_string_at(server, &["/status", "/state"]),
                    "error": first_non_empty_string_at(server, &["/error", "/message"])
                        .map(|value| log_text_preview(&value, 300)),
                })
            })
            .collect::<Vec<_>>()),
        Value::Object(servers) => json!(servers
            .iter()
            .map(|(name, server)| {
                json!({
                    "name": name,
                    "status": first_non_empty_string_at(server, &["/status", "/state"]),
                    "error": first_non_empty_string_at(server, &["/error", "/message"])
                        .map(|value| log_text_preview(&value, 300)),
                })
            })
            .collect::<Vec<_>>()),
        _ => log_request_params_summary(value),
    }
}

fn claude_content_type_summary(content: &Value) -> Value {
    let values = match content {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        Value::Object(_) => vec![content
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object")
            .to_string()],
        _ => vec!["scalar".to_string()],
    };
    json!(values)
}

#[derive(Debug, Clone)]
struct McpMetadataRelayOptions {
    server_name: String,
    thread_id: String,
    turn_id: String,
    session_id: String,
    cwd: String,
    command: String,
    args: Vec<String>,
}

pub fn run_mcp_metadata_relay(args: Vec<OsString>) -> Result<i32, String> {
    let options = parse_mcp_metadata_relay_options(args)?;
    claude_code_log_event(
        "mcp_metadata_relay_start",
        json!({
            "serverName": &options.server_name,
            "threadId": &options.thread_id,
            "turnId": &options.turn_id,
            "sessionId": &options.session_id,
            "command": &options.command,
            "args": &options.args,
        }),
    );
    maybe_launch_computer_use_service(&options);
    let mut command = Command::new(&options.command);
    command
        .args(&options.args)
        .env("CODEX_SESSION_ID", &options.session_id)
        .env("CODEX_THREAD_ID", &options.thread_id)
        .env("CODEX_TURN_ID", &options.turn_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to launch MCP metadata relay child: {}", err))?;
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open MCP metadata relay child stdin".to_string())?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture MCP metadata relay child stdout".to_string())?;
    let stdout_options = options.clone();
    let stdout_handle = thread::spawn(move || {
        forward_mcp_child_stdout(child_stdout, stdout_options);
    });

    let mut stdin = BufReader::new(std::io::stdin());
    let mut line = Vec::new();
    loop {
        line.clear();
        let size = stdin
            .read_until(b'\n', &mut line)
            .map_err(|err| format!("failed to read MCP metadata relay stdin: {}", err))?;
        if size == 0 {
            break;
        }
        let transformed = inject_mcp_codex_turn_metadata(&line, &options);
        child_stdin
            .write_all(&transformed)
            .and_then(|_| child_stdin.flush())
            .map_err(|err| format!("failed to write MCP metadata relay child stdin: {}", err))?;
    }
    drop(child_stdin);
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for MCP metadata relay child: {}", err))?;
    let _ = stdout_handle.join();
    claude_code_log_event(
        "mcp_metadata_relay_stop",
        json!({
            "serverName": &options.server_name,
            "threadId": &options.thread_id,
            "turnId": &options.turn_id,
            "success": status.success(),
            "status": status.to_string(),
        }),
    );
    Ok(status
        .code()
        .unwrap_or(if status.success() { 0 } else { 1 }))
}

fn forward_mcp_child_stdout<R>(child_stdout: R, options: McpMetadataRelayOptions)
where
    R: Read,
{
    let mut reader = BufReader::new(child_stdout);
    let mut stdout = std::io::stdout();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                log_mcp_child_stdout_line(&line, &options);
                if stdout
                    .write_all(&line)
                    .and_then(|_| stdout.flush())
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn log_mcp_child_stdout_line(line: &[u8], options: &McpMetadataRelayOptions) {
    let trimmed = trim_json_line(line);
    let Ok(value) = serde_json::from_slice::<Value>(trimmed) else {
        claude_code_log_event(
            "mcp_metadata_relay_child_stdout_non_json",
            json!({
                "serverName": &options.server_name,
                "threadId": &options.thread_id,
                "turnId": &options.turn_id,
                "linePreview": log_text_preview(&String::from_utf8_lossy(trimmed), 500),
            }),
        );
        return;
    };
    let result = value.get("result");
    let content_types = result
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|item| first_non_empty_string_at(item, &["/type"]))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    claude_code_log_event(
        "mcp_metadata_relay_child_stdout",
        json!({
            "serverName": &options.server_name,
            "threadId": &options.thread_id,
            "turnId": &options.turn_id,
            "method": value.get("method").and_then(Value::as_str),
            "id": value.get("id").map(log_json_rpc_id).unwrap_or(Value::Null),
            "hasResult": result.is_some(),
            "hasError": value.get("error").is_some(),
            "isError": result
                .and_then(|result| result.get("isError"))
                .and_then(Value::as_bool),
            "result": result.map(log_request_params_summary).unwrap_or(Value::Null),
            "contentTypes": content_types,
        }),
    );
}

#[cfg(target_os = "macos")]
fn maybe_launch_computer_use_service(options: &McpMetadataRelayOptions) {
    maybe_launch_computer_use_service_path(
        &options.server_name,
        &options.thread_id,
        &options.turn_id,
        &options.command,
    );
}

#[cfg(target_os = "macos")]
fn maybe_launch_computer_use_service_for_command(
    server_name: &str,
    work: &TurnWork,
    command: &str,
) {
    maybe_launch_computer_use_service_path(server_name, &work.thread_id, &work.turn_id, command);
}

#[cfg(not(target_os = "macos"))]
fn maybe_launch_computer_use_service_for_command(
    _server_name: &str,
    _work: &TurnWork,
    _command: &str,
) {
}

#[cfg(target_os = "macos")]
fn maybe_launch_computer_use_service_path(
    server_name: &str,
    thread_id: &str,
    turn_id: &str,
    command: &str,
) {
    let Some(app_path) = computer_use_service_app_from_client_command(command) else {
        return;
    };
    let result = Command::new("open").arg(&app_path).status();
    claude_code_log_event(
        "computer_use_service_launch",
        json!({
            "serverName": server_name,
            "threadId": thread_id,
            "turnId": turn_id,
            "appPath": app_path.to_string_lossy(),
            "success": result.as_ref().map(|status| status.success()).unwrap_or(false),
            "status": result
                .as_ref()
                .ok()
                .map(|status| status.to_string()),
            "error": result.err().map(|err| err.to_string()),
        }),
    );
}

#[cfg(not(target_os = "macos"))]
fn maybe_launch_computer_use_service(_options: &McpMetadataRelayOptions) {}

#[cfg(target_os = "macos")]
fn computer_use_service_app_from_client_command(command: &str) -> Option<PathBuf> {
    Path::new(command)
        .ancestors()
        .find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("Codex Computer Use.app")
        })
        .map(Path::to_path_buf)
        .filter(|path| path.is_dir())
}

fn parse_mcp_metadata_relay_options(
    args: Vec<OsString>,
) -> Result<McpMetadataRelayOptions, String> {
    let args = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let mut server_name = String::new();
    let mut thread_id = String::new();
    let mut turn_id = String::new();
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut command_index = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--server-name" => {
                index += 1;
                server_name = args.get(index).cloned().unwrap_or_default();
            }
            "--thread-id" => {
                index += 1;
                thread_id = args.get(index).cloned().unwrap_or_default();
            }
            "--turn-id" => {
                index += 1;
                turn_id = args.get(index).cloned().unwrap_or_default();
            }
            "--session-id" => {
                index += 1;
                session_id = args.get(index).cloned().unwrap_or_default();
            }
            "--cwd" => {
                index += 1;
                cwd = args.get(index).cloned().unwrap_or_default();
            }
            "--" => {
                command_index = Some(index + 1);
                break;
            }
            _ => {}
        }
        index += 1;
    }
    let command_index = command_index
        .ok_or_else(|| "missing -- before MCP metadata relay child command".to_string())?;
    let command = args
        .get(command_index)
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "missing MCP metadata relay child command".to_string())?;
    Ok(McpMetadataRelayOptions {
        server_name,
        thread_id,
        turn_id,
        session_id,
        cwd,
        command,
        args: args.into_iter().skip(command_index + 1).collect(),
    })
}

fn inject_mcp_codex_turn_metadata(line: &[u8], options: &McpMetadataRelayOptions) -> Vec<u8> {
    let trimmed = trim_json_line(line);
    let Ok(mut value) = serde_json::from_slice::<Value>(trimmed) else {
        return line.to_vec();
    };
    let should_inject = value.get("method").and_then(Value::as_str) == Some("tools/call");
    if !should_inject {
        return line.to_vec();
    }
    let metadata = mcp_codex_turn_metadata(options);
    let metadata_header = serde_json::to_string(&metadata).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        let params = object
            .entry("params".to_string())
            .or_insert_with(|| json!({}));
        if !params.is_object() {
            *params = json!({});
        }
        if let Some(params) = params.as_object_mut() {
            let meta = params
                .entry("_meta".to_string())
                .or_insert_with(|| json!({}));
            if !meta.is_object() {
                *meta = json!({});
            }
            if let Some(meta) = meta.as_object_mut() {
                meta.insert("x-codex-turn-metadata".to_string(), metadata.clone());
                meta.insert("codexTurnMetadata".to_string(), metadata.clone());
            }
            let headers = params
                .entry("headers".to_string())
                .or_insert_with(|| json!({}));
            if !headers.is_object() {
                *headers = json!({});
            }
            if let Some(headers) = headers.as_object_mut() {
                headers.insert(
                    "x-codex-turn-metadata".to_string(),
                    Value::String(metadata_header),
                );
            }
        }
    }
    claude_code_log_event(
        "mcp_metadata_relay_injected",
        json!({
            "serverName": &options.server_name,
            "threadId": &options.thread_id,
            "turnId": &options.turn_id,
            "method": value.get("method").and_then(Value::as_str),
            "id": value.get("id").map(log_json_rpc_id).unwrap_or(Value::Null),
        }),
    );
    let mut output = serde_json::to_vec(&value).unwrap_or_else(|_| trimmed.to_vec());
    output.push(b'\n');
    output
}

fn mcp_codex_turn_metadata(options: &McpMetadataRelayOptions) -> Value {
    json!({
        "type": "thread-id",
        "thread-id": &options.thread_id,
        "threadId": &options.thread_id,
        "turn-id": &options.turn_id,
        "turnId": &options.turn_id,
        "session_id": &options.session_id,
        "turn_id": &options.turn_id,
        "codex_session_id": &options.session_id,
        "codex_thread_id": &options.thread_id,
        "cwd": &options.cwd,
        "source": "claude-code",
        "server": &options.server_name,
    })
}

fn handle_client_line<W>(
    line: &[u8],
    state: SharedState,
    output: SharedOutput<W>,
) -> Result<Option<thread::JoinHandle<()>>, String>
where
    W: Write + Send + 'static,
{
    let value = match serde_json::from_slice::<Value>(trim_json_line(line)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "[codexl-claude-code] ignoring invalid JSON-RPC line: {}",
                err
            );
            claude_code_log_event(
                "client_line_invalid_json",
                json!({
                    "error": err.to_string(),
                    "bytes": line.len(),
                }),
            );
            return Ok(None);
        }
    };

    if value.get("method").is_none() {
        if let Some(response_id) = value.get("id").and_then(json_rpc_id_key) {
            let response = value
                .get("result")
                .cloned()
                .or_else(|| {
                    value
                        .get("error")
                        .cloned()
                        .map(|error| json!({ "error": error }))
                })
                .unwrap_or(Value::Null);
            let mut state = lock_state(&state)?;
            state.app_responses.insert(response_id, response);
            claude_code_log_event(
                "app_response_stashed",
                json!({
                    "id": log_json_rpc_id(value.get("id").unwrap_or(&Value::Null)),
                    "hasError": value.get("error").is_some(),
                    "resultSummary": log_request_params_summary(value.get("result").unwrap_or(&Value::Null)),
                }),
            );
        }
        return Ok(None);
    }

    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method == "notifications/initialized" || method == "initialized" {
        return Ok(None);
    }
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    claude_code_log_event(
        "client_request",
        json!({
            "method": method,
            "id": log_json_rpc_id(&id),
            "params": log_request_params_summary(&params),
        }),
    );

    if should_inject_codex_app_method(method) {
        if let Some(result) = standalone_codex_app_result(method, &params) {
            claude_code_log_event(
                "codex_app_method_satisfied",
                json!({
                    "method": method,
                    "id": log_json_rpc_id(&id),
                    "result": log_request_params_summary(&result),
                }),
            );
            write_response(&output, id, result)?;
        } else {
            claude_code_log_event(
                "codex_app_method_unsupported",
                json!({
                    "method": method,
                    "id": log_json_rpc_id(&id),
                }),
            );
            write_error(
                &output,
                id,
                -32601,
                format!("Claude Code app-server does not support method: {}", method),
            )?;
        }
        return Ok(None);
    }

    match method {
        "initialize" => {
            let protocol_version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            write_response(
                &output,
                id,
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": { "experimentalApi": true },
                    "serverInfo": {
                        "name": "codexl-claude-code-app-server",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "userAgent": format!("codexl-claude-code-app-server/{}", env!("CARGO_PKG_VERSION")),
                    "codexHome": crate::config::default_codex_home(),
                    "platformFamily": std::env::consts::FAMILY,
                    "platformOs": std::env::consts::OS,
                }),
            )?;
        }
        "thread/start" => {
            let (response, _notification) = {
                let mut state = lock_state(&state)?;
                state.start_thread(&params)
            };
            write_response(&output, id, response)?;
        }
        "thread/resume" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.resume_thread(&params)?
            };
            write_response(&output, id, response)?;
            write_notification(&output, notification)?;
        }
        "thread/read" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_read(&params)?
            };
            write_response(&output, id, response)?;
        }
        "thread/list" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_list(&params)
            };
            write_response(&output, id, response)?;
        }
        "thread/loaded/list" => {
            let response = {
                let state = lock_state(&state)?;
                json!({
                    "data": state
                        .threads
                        .values()
                        .filter(|thread| !is_claude_title_generation_thread(thread))
                        .map(|thread| thread.id.clone())
                        .collect::<Vec<_>>(),
                    "nextCursor": Value::Null,
                })
            };
            write_response(&output, id, response)?;
        }
        "thread/turns/list" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_turns_list(&params)?
            };
            write_response(&output, id, response)?;
        }
        "thread/turns/items/list" => {
            write_response(
                &output,
                id,
                json!({ "data": [], "nextCursor": Value::Null }),
            )?;
        }
        "thread/archive" => {
            let notification = {
                let mut state = lock_state(&state)?;
                state.set_archived(&params, true)
            };
            write_response(&output, id, json!({}))?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/unarchive" => {
            let notification = {
                let mut state = lock_state(&state)?;
                state.set_archived(&params, false)
            };
            write_response(&output, id, json!({}))?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/unsubscribe" => {
            write_response(&output, id, json!({ "status": "notSubscribed" }))?;
        }
        "thread/name/set" => {
            let notification = {
                let mut state = lock_state(&state)?;
                state.set_thread_name(&params)
            };
            write_response(&output, id, json!({}))?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/metadata/update" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_read(&params)?
            };
            write_response(&output, id, response)?;
        }
        "turn/start" => {
            let (response, notifications, work, stale_processes) = {
                let mut state = lock_state(&state)?;
                state.start_turn(&params)?
            };
            write_response(&output, id, response)?;
            for stale_process in stale_processes {
                claude_code_log_event(
                    "turn_start_terminate_stale_process",
                    json!({
                        "threadId": stale_process.thread_id,
                        "turnId": stale_process.turn_id,
                        "pid": stale_process.pid,
                    }),
                );
                terminate_process_group(stale_process.pid);
            }
            claude_code_log_event(
                "turn_start_response_sent",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "notificationCount": notifications.len(),
                    "titleGeneration": is_claude_title_generation_prompt(&work.prompt),
                }),
            );
            for notification in notifications {
                write_notification(&output, notification)?;
            }
            let worker_state = Arc::clone(&state);
            let worker_output = Arc::clone(&output);
            claude_code_log_event("turn_worker_spawn", turn_work_log_fields(&work));
            return Ok(Some(thread::spawn(move || {
                run_turn_worker(work, worker_state, worker_output);
            })));
        }
        "turn/interrupt" => {
            let pid = {
                let mut state = lock_state(&state)?;
                state.interrupt_turn(&params)
            };
            write_response(&output, id, json!({}))?;
            if let Some(pid) = pid {
                claude_code_log_event(
                    "turn_interrupt_terminate_process",
                    json!({
                        "pid": pid,
                    }),
                );
                terminate_process_group(pid);
            } else {
                claude_code_log_event(
                    "turn_interrupt_no_process",
                    json!({
                        "params": log_request_params_summary(&params),
                    }),
                );
            }
        }
        "model/list" => {
            write_response(
                &output,
                id,
                json!({ "data": [], "nextCursor": Value::Null }),
            )?;
        }
        "modelProvider/capabilities/read" => {
            write_response(
                &output,
                id,
                json!({
                    "namespaceTools": false,
                    "imageGeneration": false,
                    "webSearch": false,
                }),
            )?;
        }
        "account/read" => {
            let workspace_name = {
                let state = lock_state(&state)?;
                state.workspace_name.clone()
            };
            write_response(
                &output,
                id,
                claude_code_mock_account_read_result(workspace_name.as_deref()),
            )?;
        }
        "getAuthStatus" => {
            let workspace_name = {
                let state = lock_state(&state)?;
                state.workspace_name.clone()
            };
            write_response(
                &output,
                id,
                claude_code_mock_auth_status_result(&params, workspace_name.as_deref()),
            )?;
        }
        "permissionProfile/list"
        | "skills/list"
        | "plugin/list"
        | "app/list"
        | "mcpServerStatus/list"
        | "experimentalFeature/list" => {
            write_response(
                &output,
                id,
                json!({ "data": [], "nextCursor": Value::Null }),
            )?;
        }
        "hooks/list" => {
            write_response(&output, id, json!({ "data": [] }))?;
        }
        "collaborationMode/list" => {
            write_response(&output, id, json!({ "data": [] }))?;
        }
        "config/read" => {
            write_response(&output, id, config_read_response(&params))?;
        }
        "configRequirements/read" => {
            write_response(&output, id, json!({ "requirements": Value::Null }))?;
        }
        "config/mcpServer/reload" | "memory/reset" => {
            write_response(&output, id, json!({}))?;
        }
        _ => {
            write_error(
                &output,
                id,
                -32601,
                format!("Claude Code app-server does not support method: {}", method),
            )?;
        }
    }
    Ok(None)
}

fn should_inject_codex_app_method(method: &str) -> bool {
    !is_claude_code_owned_method(method)
}

fn claude_code_mock_account_read_result(workspace_name: Option<&str>) -> Value {
    json!({
        "account": {
            "type": "chatgpt",
            "email": claude_code_mock_account_email(workspace_name),
            "planType": "unknown",
        },
        "requiresOpenaiAuth": false,
    })
}

fn claude_code_mock_auth_status_result(params: &Value, workspace_name: Option<&str>) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("authMethod".to_string(), json!("chatgpt"));
    result.insert(
        "account".to_string(),
        claude_code_mock_account_read_result(workspace_name)
            .get("account")
            .cloned()
            .unwrap_or(Value::Null),
    );
    if params
        .get("includeToken")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        result.insert("authToken".to_string(), Value::Null);
    }
    result.insert("requiresOpenaiAuth".to_string(), json!(false));
    Value::Object(result)
}

fn claude_code_mock_account_email(workspace_name: Option<&str>) -> String {
    workspace_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PROVIDER_NAME)
        .to_string()
}

fn is_claude_code_owned_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "thread/start"
            | "thread/resume"
            | "thread/read"
            | "thread/list"
            | "thread/loaded/list"
            | "thread/turns/list"
            | "thread/turns/items/list"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/unsubscribe"
            | "thread/name/set"
            | "thread/metadata/update"
            | "turn/start"
            | "turn/interrupt"
            | "account/read"
            | "getAuthStatus"
            | "config/read"
    )
}

fn standalone_codex_app_result(method: &str, params: &Value) -> Option<Value> {
    if method_removes_protected_bundled_plugin(method, params) {
        return Some(json!({}));
    }

    if should_proxy_codex_app_method(method) {
        if let Some(result) = codex_cli_app_server_method_result(method, params) {
            return Some(normalize_proxied_codex_app_result(method, params, result));
        }
    }

    match method {
        "config/mcpServer/reload"
        | "memory/reset"
        | "experimentalFeature/enablement/set"
        | "marketplace/add" => Some(json!({})),
        "remoteControl/status/read" => Some(json!({
            "enabled": false,
            "status": "unavailable",
        })),
        "configRequirements/read" => Some(json!({ "requirements": Value::Null })),
        "extension/list" | "extensions/list" => Some(json!({
            "data": standalone_extension_list(),
            "nextCursor": Value::Null,
        })),
        "hooks/list" => Some(json!({ "data": standalone_hooks_list(params) })),
        "collaborationMode/list" => Some(json!({ "data": [] })),
        "modelProvider/capabilities/read" => Some(json!({
            "namespaceTools": false,
            "imageGeneration": false,
            "webSearch": false,
        })),
        "thread/goal/get" | "thread/goal/set" | "thread/goal/clear" => {
            Some(json!({ "goal": Value::Null }))
        }
        "skills/list" => Some(json!({
            "data": standalone_skill_list(),
            "nextCursor": Value::Null,
        })),
        "plugin/list" => Some(standalone_plugin_list_result()),
        "plugin/read" => Some(standalone_plugin_read_result(params)),
        "plugin/install" => Some(standalone_plugin_install_result(params)),
        method if method.starts_with("plugin/") || method.starts_with("marketplace/") => {
            Some(json!({}))
        }
        "app/list" => Some(json!({
            "data": standalone_app_list(),
            "nextCursor": Value::Null,
        })),
        "mcpServerStatus/list" => Some(json!({
            "data": standalone_mcp_server_status_list(),
            "nextCursor": Value::Null,
        })),
        "model/list" | "permissionProfile/list" | "experimentalFeature/list" => Some(json!({
            "data": [],
            "nextCursor": Value::Null,
        })),
        _ => None,
    }
}

fn normalize_proxied_codex_app_result(method: &str, params: &Value, result: Value) -> Value {
    match method {
        "plugin/list" => merge_proxied_plugin_list_result(result),
        "plugin/read" if result.get("plugin").map_or(true, Value::is_null) => {
            standalone_plugin_read_result(params)
        }
        _ => result,
    }
}

fn merge_proxied_plugin_list_result(mut result: Value) -> Value {
    let fallback = standalone_plugin_list_result();
    let Some(result_object) = result.as_object_mut() else {
        return fallback;
    };
    let Some(fallback_object) = fallback.as_object() else {
        return result;
    };

    merge_plugin_array_field(result_object, fallback_object, "data");
    merge_marketplace_array_field(result_object, fallback_object);
    for key in ["marketplaceLoadErrors", "featuredPluginIds", "nextCursor"] {
        if !result_object.contains_key(key) {
            if let Some(value) = fallback_object.get(key) {
                result_object.insert(key.to_string(), value.clone());
            }
        }
    }
    keep_local_protected_bundled_plugins_available(result_object, fallback_object);
    result
}

fn method_removes_protected_bundled_plugin(method: &str, params: &Value) -> bool {
    matches!(
        method,
        "plugin/uninstall" | "plugin/remove" | "plugin/delete"
    ) && plugin_request_targets_protected_bundled_plugin(params)
}

fn keep_local_protected_bundled_plugins_available(
    result_object: &mut Map<String, Value>,
    fallback_object: &Map<String, Value>,
) {
    for plugin_name in PROTECTED_BUNDLED_PLUGIN_NAMES {
        keep_local_protected_bundled_plugin_available(result_object, fallback_object, plugin_name);
    }
}

fn keep_local_protected_bundled_plugin_available(
    result_object: &mut Map<String, Value>,
    fallback_object: &Map<String, Value>,
    plugin_name: &str,
) {
    let fallback_plugin = fallback_object
        .get("data")
        .and_then(Value::as_array)
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin_matches_name(plugin, plugin_name))
        })
        .map(available_protected_bundled_plugin);

    if let Some(plugin) = fallback_plugin.as_ref() {
        let result_plugins = result_object
            .entry("data".to_string())
            .or_insert_with(|| json!([]));
        if !result_plugins.is_array() {
            *result_plugins = json!([]);
        }
        if let Some(result_plugins) = result_plugins.as_array_mut() {
            upsert_protected_bundled_plugin(result_plugins, plugin, plugin_name);
        }
    } else if let Some(result_plugins) = result_object.get_mut("data").and_then(Value::as_array_mut)
    {
        normalize_existing_protected_bundled_plugins(result_plugins);
    }

    keep_local_protected_bundled_marketplace_plugin_available(
        result_object,
        fallback_object,
        plugin_name,
    );
}

fn keep_local_protected_bundled_marketplace_plugin_available(
    result_object: &mut Map<String, Value>,
    fallback_object: &Map<String, Value>,
    plugin_name: &str,
) {
    let Some(fallback_marketplace) = fallback_object
        .get("marketplaces")
        .and_then(Value::as_array)
        .and_then(|marketplaces| {
            marketplaces
                .iter()
                .find(|marketplace| marketplace_is_openai_bundled(marketplace))
        })
    else {
        return;
    };
    let fallback_plugin = fallback_marketplace
        .get("plugins")
        .and_then(Value::as_array)
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin_matches_name(plugin, plugin_name))
        })
        .map(available_protected_bundled_plugin);

    let Some(fallback_plugin) = fallback_plugin else {
        return;
    };

    let result_marketplaces = result_object
        .entry("marketplaces".to_string())
        .or_insert_with(|| json!([]));
    if !result_marketplaces.is_array() {
        *result_marketplaces = json!([]);
    }
    let Some(result_marketplaces) = result_marketplaces.as_array_mut() else {
        return;
    };
    let result_marketplace_index = result_marketplaces
        .iter()
        .position(|marketplace| marketplace_is_openai_bundled(marketplace))
        .unwrap_or_else(|| {
            result_marketplaces.push(fallback_marketplace.clone());
            result_marketplaces.len() - 1
        });
    let Some(result_marketplace) = result_marketplaces.get_mut(result_marketplace_index) else {
        return;
    };
    let Some(result_object) = result_marketplace.as_object_mut() else {
        return;
    };
    let plugins = result_object
        .entry("plugins".to_string())
        .or_insert_with(|| json!([]));
    if !plugins.is_array() {
        *plugins = json!([]);
    }
    if let Some(plugins) = plugins.as_array_mut() {
        upsert_protected_bundled_plugin(plugins, &fallback_plugin, plugin_name);
    }
}

fn upsert_protected_bundled_plugin(plugins: &mut Vec<Value>, plugin: &Value, plugin_name: &str) {
    let plugin = available_protected_bundled_plugin(plugin);
    if let Some(existing) = plugins
        .iter_mut()
        .find(|value| plugin_matches_name(value, plugin_name))
    {
        *existing = plugin;
    } else {
        plugins.push(plugin);
    }
}

fn normalize_existing_protected_bundled_plugins(plugins: &mut [Value]) {
    for plugin in plugins {
        if plugin_is_protected_bundled(plugin) {
            *plugin = available_protected_bundled_plugin(plugin);
        }
    }
}

fn available_protected_bundled_plugin(plugin: &Value) -> Value {
    let mut plugin = plugin.clone();
    if let Some(object) = plugin.as_object_mut() {
        object.insert("installed".to_string(), Value::Bool(true));
        object.insert("enabled".to_string(), Value::Bool(true));
        object.insert("availability".to_string(), json!("AVAILABLE"));
        object.insert("installPolicy".to_string(), json!("AVAILABLE"));
        object.remove("disabled");
    }
    plugin
}

fn plugin_request_targets_protected_bundled_plugin(params: &Value) -> bool {
    plugin_request_name(params)
        .map(plugin_name_is_protected_bundled)
        .unwrap_or(false)
}

fn plugin_is_protected_bundled(value: &Value) -> bool {
    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(plugin_name_is_protected_bundled)
        || value
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(plugin_name_is_protected_bundled)
        || value
            .pointer("/source/path")
            .and_then(Value::as_str)
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .is_some_and(plugin_name_is_protected_bundled)
}

fn plugin_matches_name(value: &Value, plugin_name: &str) -> bool {
    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|value| plugin_name_matches(value, plugin_name))
        || value
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|value| plugin_name_matches(value, plugin_name))
        || value
            .pointer("/source/path")
            .and_then(Value::as_str)
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|value| plugin_name_matches(value, plugin_name))
}

fn plugin_name_is_protected_bundled(value: &str) -> bool {
    PROTECTED_BUNDLED_PLUGIN_NAMES
        .iter()
        .any(|plugin_name| plugin_name_matches(value, plugin_name))
}

fn plugin_name_matches(value: &str, plugin_name: &str) -> bool {
    value
        .split('@')
        .next()
        .unwrap_or(value)
        .trim()
        .eq_ignore_ascii_case(plugin_name)
}

fn marketplace_is_openai_bundled(value: &Value) -> bool {
    ["name", "path"].into_iter().any(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains(OPENAI_BUNDLED_MARKETPLACE_NAME))
    })
}

fn merge_plugin_array_field(
    result_object: &mut Map<String, Value>,
    fallback_object: &Map<String, Value>,
    key: &str,
) {
    let Some(fallback_values) = fallback_object.get(key).and_then(Value::as_array) else {
        return;
    };
    let result_values = result_object
        .entry(key.to_string())
        .or_insert_with(|| json!([]));
    if !result_values.is_array() {
        *result_values = json!([]);
    }
    if let Some(result_values) = result_values.as_array_mut() {
        append_missing_values_by_key(result_values, fallback_values, plugin_list_item_key);
    }
}

fn merge_marketplace_array_field(
    result_object: &mut Map<String, Value>,
    fallback_object: &Map<String, Value>,
) {
    let Some(fallback_marketplaces) = fallback_object
        .get("marketplaces")
        .and_then(Value::as_array)
    else {
        return;
    };
    let result_marketplaces = result_object
        .entry("marketplaces".to_string())
        .or_insert_with(|| json!([]));
    if !result_marketplaces.is_array() {
        *result_marketplaces = json!([]);
    }
    let Some(result_marketplaces) = result_marketplaces.as_array_mut() else {
        return;
    };
    for fallback_marketplace in fallback_marketplaces {
        if let Some(result_marketplace) = result_marketplaces.iter_mut().find(|marketplace| {
            marketplace_list_item_key(marketplace)
                .zip(marketplace_list_item_key(fallback_marketplace))
                .is_some_and(|(left, right)| left == right)
        }) {
            merge_marketplace_plugins(result_marketplace, fallback_marketplace);
            fill_missing_marketplace_fields(result_marketplace, fallback_marketplace);
        } else {
            result_marketplaces.push(fallback_marketplace.clone());
        }
    }
}

fn merge_marketplace_plugins(result_marketplace: &mut Value, fallback_marketplace: &Value) {
    let Some(result_object) = result_marketplace.as_object_mut() else {
        return;
    };
    let Some(fallback_object) = fallback_marketplace.as_object() else {
        return;
    };
    merge_plugin_array_field(result_object, fallback_object, "plugins");
}

fn fill_missing_marketplace_fields(result_marketplace: &mut Value, fallback_marketplace: &Value) {
    let Some(result_object) = result_marketplace.as_object_mut() else {
        return;
    };
    let Some(fallback_object) = fallback_marketplace.as_object() else {
        return;
    };
    for key in ["name", "path", "interface"] {
        if !result_object.contains_key(key) {
            if let Some(value) = fallback_object.get(key) {
                result_object.insert(key.to_string(), value.clone());
            }
        }
    }
}

fn append_missing_values_by_key(
    result_values: &mut Vec<Value>,
    fallback_values: &[Value],
    key_fn: fn(&Value) -> Option<String>,
) {
    let mut seen = result_values
        .iter()
        .filter_map(key_fn)
        .collect::<BTreeSet<_>>();
    for fallback_value in fallback_values {
        let key = key_fn(fallback_value);
        if key.as_ref().is_some_and(|key| !seen.insert(key.clone())) {
            continue;
        }
        result_values.push(fallback_value.clone());
    }
}

fn plugin_list_item_key(value: &Value) -> Option<String> {
    value
        .get("id")
        .or_else(|| value.get("name"))
        .or_else(|| value.get("path"))
        .or_else(|| value.pointer("/source/path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn marketplace_list_item_key(value: &Value) -> Option<String> {
    value
        .get("path")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn should_proxy_codex_app_method(method: &str) -> bool {
    if method.starts_with("plugin/")
        || method.starts_with("marketplace/")
        || method.starts_with("skills/")
        || method.starts_with("hooks/")
        || method.starts_with("mcpServer/")
        || method.starts_with("mcpServerStatus/")
    {
        return true;
    }
    matches!(
        method,
        "extension/list"
            | "extensions/list"
            | "hooks/list"
            | "skills/list"
            | "plugin/list"
            | "plugin/read"
            | "plugin/install"
            | "app/list"
            | "mcpServerStatus/list"
            | "marketplace/add"
            | "marketplace/remove"
            | "marketplace/upgrade"
            | "experimentalFeature/enablement/set"
            | "config/mcpServer/reload"
    )
}

fn standalone_skill_list() -> Vec<Value> {
    let mut skills = Vec::new();
    let mut seen = BTreeSet::new();
    for root in codex_resource_roots("skills")
        .into_iter()
        .chain(codex_resource_roots("plugins"))
    {
        collect_skill_files(&root, 0, &mut |path| {
            let key = canonical_key(path);
            if seen.insert(key) {
                if let Some(skill) = skill_json_from_path(path) {
                    skills.push(skill);
                }
            }
        });
    }
    skills.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    skills
}

fn skill_json_from_path(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    let fallback_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("skill")
        .to_string();
    let name = front_matter_value(&content, "name")
        .or_else(|| markdown_title(&content))
        .unwrap_or(fallback_name);
    let description = front_matter_value(&content, "description")
        .or_else(|| markdown_first_paragraph(&content))
        .unwrap_or_default();
    Some(json!({
        "id": name,
        "name": name,
        "title": markdown_title(&content).unwrap_or_else(|| name.clone()),
        "description": description,
        "path": path.to_string_lossy().to_string(),
        "skillPath": path.to_string_lossy().to_string(),
        "source": "filesystem",
        "enabled": true,
    }))
}

fn collect_skill_files<F>(dir: &Path, depth: usize, visitor: &mut F)
where
    F: FnMut(&Path),
{
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        {
            visitor(&path);
        } else if metadata.is_dir() {
            collect_skill_files(&path, depth + 1, visitor);
        }
    }
}

#[derive(Debug, Clone)]
struct StandalonePluginEntry {
    marketplace_name: String,
    marketplace_path: String,
    manifest_path: PathBuf,
    package_dir: PathBuf,
    plugin: Value,
}

fn codex_cli_app_server_method_result(method: &str, params: &Value) -> Option<Value> {
    if !codex_cli_app_server_proxy_enabled() {
        return None;
    }
    let executable = codex_cli_app_server_executable()?;
    let request_id = "__codexl_claude_code_proxy_request__";
    let initialize_id = "__codexl_claude_code_proxy_initialize__";
    let input = format!(
        "{}\n{}\n{}\n",
        json!({
            "id": initialize_id,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "codexl-claude-code-app-server",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        }),
        json!({
            "id": request_id,
            "method": method,
            "params": params,
        })
    );
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .arg("--analytics-default-enabled")
        .env("CODEX_HOME", codex_cli_app_server_codex_home(method))
        .env_remove("CODEX_CLI_PATH")
        .env_remove("CODEXL_REAL_CODEX_CLI_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        stdin.write_all(input.as_bytes()).ok()?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_str) != Some(request_id) {
            continue;
        }
        if value.get("error").is_some() {
            return None;
        }
        return value.get("result").cloned();
    }
    None
}

fn codex_cli_app_server_proxy_enabled() -> bool {
    std::env::var(CODEX_APP_SERVER_PROXY_ENV)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(true)
}

fn codex_cli_app_server_codex_home(method: &str) -> String {
    let active_home = active_codex_home_for_proxy();
    if method_uses_global_plugin_home(method) {
        return codex_home_with_plugins(active_home.as_deref())
            .or(active_home)
            .unwrap_or_else(crate::config::default_codex_home);
    }
    active_home.unwrap_or_else(crate::config::default_codex_home)
}

fn active_codex_home_for_proxy() -> Option<String> {
    std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let config = crate::config::AppConfig::load();
            config.active_codex_home().map(str::to_string)
        })
}

fn method_uses_global_plugin_home(method: &str) -> bool {
    method.starts_with("plugin/")
        || method.starts_with("marketplace/")
        || method.starts_with("skills/")
        || method.starts_with("hooks/")
        || matches!(method, "app/list" | "extension/list" | "extensions/list")
}

fn codex_home_with_plugins(active_home: Option<&str>) -> Option<String> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(active_home) = active_home {
        push_unique_path(
            &mut candidates,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(active_home)),
        );
    }
    if let Some(default_home) = global_default_codex_home_candidate() {
        push_unique_path(&mut candidates, &mut seen, default_home);
    }
    let config = crate::config::AppConfig::load();
    push_unique_path(
        &mut candidates,
        &mut seen,
        PathBuf::from(crate::config::normalize_home_path(&config.codex_home)),
    );
    for profile in config.codex_home_profiles {
        push_unique_path(
            &mut candidates,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(&profile.path)),
        );
    }
    for profile in config.provider_profiles {
        push_unique_path(
            &mut candidates,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(&profile.codex_home)),
        );
        push_unique_path(
            &mut candidates,
            &mut seen,
            crate::config::generated_codex_home(&profile),
        );
    }
    candidates
        .into_iter()
        .find(|home| codex_home_has_plugin_cache(home))
        .map(|home| home.to_string_lossy().to_string())
}

fn global_default_codex_home_candidate() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("CODEXL_CODEX_HOME") {
        let value = value.trim();
        if !value.is_empty() {
            return Some(PathBuf::from(crate::config::normalize_home_path(value)));
        }
    }
    if cfg!(windows) {
        std::env::var("USERPROFILE")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                let combined = format!("{}{}", drive.trim(), path.trim());
                (!combined.trim().is_empty()).then(|| PathBuf::from(combined))
            })
            .map(|home| home.join(".codex"))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|home| PathBuf::from(home).join(".codex"))
    }
}

fn codex_home_has_plugin_cache(home: &Path) -> bool {
    collect_limited_plugin_manifests(&home.join("plugins"), 0, 1) > 0
}

fn collect_limited_plugin_manifests(dir: &Path, depth: usize, limit: usize) -> usize {
    if depth > 8 || limit == 0 {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some("plugin.json")
        {
            count += 1;
        } else if metadata.is_dir() {
            count += collect_limited_plugin_manifests(&path, depth + 1, limit - count);
        }
        if count >= limit {
            break;
        }
    }
    count
}

fn codex_cli_app_server_executable() -> Option<String> {
    for key in [
        "CODEXL_REAL_CODEX_CLI_PATH",
        "CODEXL_BUNDLED_CODEX_CLI_PATH",
    ] {
        if let Ok(value) = std::env::var(key) {
            if let Some(executable) = codex_cli_executable_candidate(&value) {
                return Some(executable);
            }
        }
    }
    let config = crate::config::AppConfig::load();
    let resolved_codex_cli =
        crate::launcher::resolve_codex_cli_executable(None, &config.codex_path);
    for value in [config.codex_path.as_str(), resolved_codex_cli.as_str()] {
        if let Some(executable) = codex_cli_executable_candidate(value) {
            return Some(executable);
        }
    }
    for app in [
        "/Applications/Codex.app/Contents/MacOS/Codex",
        "/Applications/OpenAI Codex.app/Contents/MacOS/OpenAI Codex",
    ] {
        if let Some(executable) = codex_cli_executable_candidate(app) {
            return Some(executable);
        }
    }
    None
}

fn codex_cli_executable_candidate(value: &str) -> Option<String> {
    let value = value.trim();
    if !codex_cli_executable_usable(value) {
        return None;
    }
    if let Some(path) = bundled_codex_cli_path(value) {
        return Some(path.to_string_lossy().to_string());
    }
    let path = Path::new(value);
    if path.is_file() {
        return Some(value.to_string());
    }
    executable_on_path(value).map(|path| path.to_string_lossy().to_string())
}

fn bundled_codex_cli_path(codex_app_executable: &str) -> Option<PathBuf> {
    let executable = PathBuf::from(codex_app_executable.trim());
    let file_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(contents_dir) = executable.parent().and_then(|parent| parent.parent()) {
        if let Some(candidate) = [
            contents_dir.join("Resources").join(file_name),
            contents_dir.join("resources").join(file_name),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
        {
            return Some(candidate);
        }
    }
    (executable.is_file() && !path_is_macos_app_main_executable(&executable)).then_some(executable)
}

fn codex_cli_executable_usable(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains("codexl-codex-cli-middleware") {
        return false;
    }
    if let Ok(current) = std::env::current_exe() {
        if Path::new(value) == current {
            return false;
        }
    }
    let path = Path::new(value);
    (!path_is_macos_app_main_executable(path) && path.is_file())
        || bundled_codex_cli_path(value).is_some()
        || executable_on_path(value).is_some()
}

fn path_is_macos_app_main_executable(path: &Path) -> bool {
    path.to_string_lossy().contains(".app/Contents/MacOS/")
}

fn executable_on_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() || Path::new(value).components().count() != 1 {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in executable_name_candidates(value) {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn executable_name_candidates(value: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(value).extension().is_none() {
        vec![value.to_string(), format!("{value}.exe")]
    } else {
        vec![value.to_string()]
    }
}

fn standalone_plugin_list_result() -> Value {
    let entries = standalone_plugin_entries();
    let mut marketplaces = BTreeMap::<String, (String, Vec<Value>)>::new();
    let mut data = Vec::new();
    for entry in entries {
        data.push(entry.plugin.clone());
        marketplaces
            .entry(entry.marketplace_name)
            .or_insert_with(|| (entry.marketplace_path, Vec::new()))
            .1
            .push(entry.plugin);
    }
    let mut marketplace_values = marketplaces
        .into_iter()
        .map(|(name, (path, mut plugins))| {
            plugins.sort_by(|left, right| {
                left.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(right.get("id").and_then(Value::as_str).unwrap_or_default())
            });
            json!({
                "name": name,
                "path": path,
                "interface": Value::Null,
                "plugins": plugins,
            })
        })
        .collect::<Vec<_>>();
    marketplace_values.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    data.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(right.get("id").and_then(Value::as_str).unwrap_or_default())
    });
    json!({
        "marketplaces": marketplace_values,
        "marketplaceLoadErrors": [],
        "featuredPluginIds": [],
        "data": data,
        "nextCursor": Value::Null,
    })
}

fn standalone_plugin_entries() -> Vec<StandalonePluginEntry> {
    let mut plugins = Vec::new();
    let mut seen = BTreeSet::new();
    for root in codex_resource_roots("plugins") {
        collect_json_manifest_files(&root, 0, &["plugin.json"], &mut |path| {
            if manifest_is_inside_codex_app_dir(path, ".codex-app") {
                return;
            }
            let key = canonical_key(path);
            if seen.insert(key) {
                if let Some(plugin) = plugin_entry_from_manifest_path(path) {
                    plugins.push(plugin);
                }
            }
        });
    }
    plugins.sort_by(|left, right| {
        left.plugin
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .plugin
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    plugins
}

fn plugin_entry_from_manifest_path(path: &Path) -> Option<StandalonePluginEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&content).ok()?;
    let object = value.as_object()?;
    let package_dir = plugin_package_dir_for_manifest(path);
    let marketplace_name = plugin_marketplace_name(path);
    let marketplace_path = plugin_marketplace_path(path, &marketplace_name);
    let fallback_name = package_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin")
        .to_string();
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&fallback_name)
        .to_string();
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if marketplace_name == "filesystem" {
                name.clone()
            } else {
                format!("{}@{}", name, marketplace_name)
            }
        });
    let local_version = plugin_local_version(&package_dir, &name, object);
    let keywords = object
        .get("keywords")
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let plugin = json!({
        "id": id,
        "name": name,
        "shareContext": object.get("shareContext").cloned().unwrap_or(Value::Null),
        "source": {
            "type": "local",
            "path": path_to_string(package_dir.clone()),
        },
        "version": object.get("version").cloned().unwrap_or(Value::Null),
        "localVersion": local_version
            .map(Value::String)
            .unwrap_or(Value::Null),
        "installed": true,
        "enabled": object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "installPolicy": object
            .get("installPolicy")
            .and_then(Value::as_str)
            .unwrap_or("AVAILABLE"),
        "authPolicy": object
            .get("authPolicy")
            .and_then(Value::as_str)
            .unwrap_or("ON_INSTALL"),
        "availability": object
            .get("availability")
            .and_then(Value::as_str)
            .unwrap_or("AVAILABLE"),
        "interface": plugin_interface_json(object.get("interface"), &package_dir),
        "keywords": keywords,
        "path": path.to_string_lossy().to_string(),
    });
    Some(StandalonePluginEntry {
        marketplace_name,
        marketplace_path,
        manifest_path: path.to_path_buf(),
        package_dir,
        plugin,
    })
}

fn plugin_local_version(
    package_dir: &Path,
    plugin_name: &str,
    manifest: &Map<String, Value>,
) -> Option<String> {
    manifest
        .get("localVersion")
        .or_else(|| manifest.get("local_version"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let parent_name = package_dir.parent()?.file_name()?.to_str()?;
            if !parent_name.eq_ignore_ascii_case(plugin_name) {
                return None;
            }
            package_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            manifest
                .get("version")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn standalone_plugin_read_result(params: &Value) -> Value {
    let Some(entry) = find_standalone_plugin_entry(params) else {
        return json!({ "plugin": Value::Null });
    };
    json!({ "plugin": standalone_plugin_detail(&entry) })
}

fn standalone_plugin_install_result(params: &Value) -> Value {
    let auth_policy = find_standalone_plugin_entry(params)
        .and_then(|entry| {
            entry
                .plugin
                .get("authPolicy")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "ON_INSTALL".to_string());
    json!({
        "authPolicy": auth_policy,
        "appsNeedingAuth": [],
    })
}

fn find_standalone_plugin_entry(params: &Value) -> Option<StandalonePluginEntry> {
    let plugin_name = plugin_request_name(params)?;
    let marketplace_path = params
        .get("marketplacePath")
        .or_else(|| params.get("marketplace_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    standalone_plugin_entries()
        .into_iter()
        .find(|entry| plugin_entry_matches_request(entry, plugin_name, marketplace_path))
}

fn plugin_request_name(params: &Value) -> Option<&str> {
    [
        "pluginName",
        "plugin_name",
        "pluginId",
        "plugin_id",
        "name",
        "id",
    ]
    .into_iter()
    .find_map(|key| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn plugin_entry_matches_request(
    entry: &StandalonePluginEntry,
    plugin_name: &str,
    marketplace_path: Option<&str>,
) -> bool {
    let Some(entry_name) = entry.plugin.get("name").and_then(Value::as_str) else {
        return false;
    };
    let entry_id = entry.plugin.get("id").and_then(Value::as_str);
    let source_name = entry
        .plugin
        .pointer("/source/path")
        .and_then(Value::as_str)
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str());
    let name_matches = [Some(entry_name), entry_id, source_name]
        .into_iter()
        .flatten()
        .any(|candidate| {
            candidate == plugin_name
                || candidate
                    .strip_suffix(&format!("@{}", entry.marketplace_name))
                    .is_some_and(|value| value == plugin_name)
        });
    if !name_matches {
        return false;
    }
    let Some(marketplace_path) = marketplace_path else {
        return true;
    };
    plugin_marketplace_path_matches(entry, marketplace_path)
}

fn plugin_marketplace_path_matches(entry: &StandalonePluginEntry, marketplace_path: &str) -> bool {
    let marketplace_path = marketplace_path.trim();
    if marketplace_path.is_empty() {
        return true;
    }
    if marketplace_path == entry.marketplace_path {
        return true;
    }
    let entry_key = canonical_key(Path::new(&entry.marketplace_path));
    let request_key = canonical_key(Path::new(marketplace_path));
    if entry_key == request_key {
        return true;
    }
    marketplace_path.contains(&entry.marketplace_name)
        || entry.marketplace_path.contains(marketplace_path)
        || marketplace_path.contains(&entry.marketplace_path)
}

fn standalone_plugin_detail(entry: &StandalonePluginEntry) -> Value {
    let manifest = read_json_file(&entry.manifest_path).unwrap_or_else(|| json!({}));
    json!({
        "marketplaceName": entry.marketplace_name,
        "marketplacePath": entry.marketplace_path,
        "summary": entry.plugin,
        "description": manifest
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "skills": plugin_skill_details(entry, &manifest),
        "hooks": plugin_hook_details(entry, &manifest),
        "apps": plugin_app_details(entry, &manifest),
        "mcpServers": plugin_mcp_server_names(entry, &manifest),
    })
}

fn plugin_skill_details(entry: &StandalonePluginEntry, manifest: &Value) -> Vec<Value> {
    let mut skill_paths = Vec::new();
    let mut seen = BTreeSet::new();
    for path in plugin_manifest_paths(manifest.get("skills"), &entry.package_dir) {
        collect_plugin_skill_paths(&path, &mut skill_paths, &mut seen);
    }
    if skill_paths.is_empty() {
        collect_plugin_skill_paths(
            &entry.package_dir.join("skills"),
            &mut skill_paths,
            &mut seen,
        );
    }
    let plugin_name = entry
        .plugin
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("plugin");
    let mut skills = skill_paths
        .into_iter()
        .filter_map(|path| plugin_skill_detail(plugin_name, &path))
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    skills
}

fn collect_plugin_skill_paths(
    path: &Path,
    skill_paths: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<String>,
) {
    if path.is_file() && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
        push_unique_path(skill_paths, seen, path.to_path_buf());
    } else {
        collect_skill_files(path, 0, &mut |skill_path| {
            push_unique_path(skill_paths, seen, skill_path.to_path_buf());
        });
    }
}

fn plugin_skill_detail(plugin_name: &str, path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    let fallback_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("skill")
        .to_string();
    let skill_name = front_matter_value(&content, "name")
        .or_else(|| markdown_title(&content))
        .unwrap_or(fallback_name);
    let full_name = if skill_name.contains(':') {
        skill_name.clone()
    } else {
        format!("{}:{}", plugin_name, skill_name)
    };
    let description = front_matter_value(&content, "description")
        .or_else(|| markdown_first_paragraph(&content))
        .unwrap_or_default();
    Some(json!({
        "name": full_name,
        "description": description,
        "shortDescription": Value::Null,
        "interface": Value::Null,
        "path": path.to_string_lossy().to_string(),
        "enabled": true,
    }))
}

fn plugin_hook_details(entry: &StandalonePluginEntry, manifest: &Value) -> Vec<Value> {
    if let Some(hooks) = manifest.get("hooks").and_then(Value::as_array) {
        return hooks.clone();
    }
    plugin_manifest_paths(manifest.get("hooks"), &entry.package_dir)
        .into_iter()
        .filter_map(|path| read_json_file(&path))
        .flat_map(|value| {
            value
                .get("hooks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

fn plugin_app_details(entry: &StandalonePluginEntry, manifest: &Value) -> Vec<Value> {
    let mut apps = Vec::new();
    for path in plugin_manifest_paths(manifest.get("apps"), &entry.package_dir) {
        let Some(value) = read_json_file(&path) else {
            continue;
        };
        if let Some(values) = value.get("apps").and_then(Value::as_array) {
            apps.extend(values.clone());
            continue;
        }
        if let Some(values) = value.get("apps").and_then(Value::as_object) {
            for (name, app) in values {
                let mut app = app.clone();
                if let Some(object) = app.as_object_mut() {
                    object
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.clone()));
                }
                apps.push(app);
            }
        }
    }
    apps
}

fn plugin_mcp_server_names(entry: &StandalonePluginEntry, manifest: &Value) -> Vec<String> {
    let mut names = BTreeSet::new();
    if let Some(servers) = manifest.get("mcpServers").and_then(Value::as_object) {
        names.extend(servers.keys().cloned());
    }
    for path in plugin_manifest_paths(manifest.get("mcpServers"), &entry.package_dir) {
        let Some(value) = read_json_file(&path) else {
            continue;
        };
        if let Some(servers) = value.get("mcpServers").and_then(Value::as_object) {
            names.extend(servers.keys().cloned());
        }
    }
    names.into_iter().collect()
}

fn plugin_manifest_paths(value: Option<&Value>, package_dir: &Path) -> Vec<PathBuf> {
    match value {
        Some(Value::String(path)) => non_empty_string(path)
            .map(|path| vec![resolve_plugin_manifest_path(package_dir, &path)])
            .unwrap_or_default(),
        Some(Value::Array(paths)) => paths
            .iter()
            .filter_map(Value::as_str)
            .filter_map(non_empty_string)
            .map(|path| resolve_plugin_manifest_path(package_dir, &path))
            .collect(),
        _ => Vec::new(),
    }
}

fn resolve_plugin_manifest_path(package_dir: &Path, value: &str) -> PathBuf {
    let expanded = crate::config::normalize_home_path(value.trim());
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        package_dir.join(path)
    }
}

fn read_json_file(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&content).ok()
}

fn plugin_marketplace_name(path: &Path) -> String {
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>();
    for window in components.windows(2) {
        if window[0] == "cache" && !window[1].is_empty() {
            return window[1].clone();
        }
    }
    "filesystem".to_string()
}

fn plugin_marketplace_path(path: &Path, marketplace_name: &str) -> String {
    if marketplace_name == "filesystem" {
        return path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_string_lossy()
            .to_string();
    }
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.file_name().and_then(|name| name.to_str()) == Some(marketplace_name) {
            return candidate.to_string_lossy().to_string();
        }
        current = candidate.parent();
    }
    path.parent()
        .unwrap_or_else(|| Path::new(""))
        .to_string_lossy()
        .to_string()
}

fn plugin_interface_json(interface: Option<&Value>, package_dir: &Path) -> Value {
    let Some(interface) = interface.and_then(Value::as_object) else {
        return Value::Null;
    };
    let mut normalized = serde_json::Map::new();
    for (key, value) in interface {
        let normalized_key = match key.as_str() {
            "websiteURL" => "websiteUrl",
            "privacyPolicyURL" => "privacyPolicyUrl",
            "termsOfServiceURL" => "termsOfServiceUrl",
            other => other,
        };
        let normalized_value = match normalized_key {
            "composerIcon" | "logo" => plugin_asset_value(package_dir, value),
            "screenshots" => plugin_asset_array(package_dir, value),
            _ => value.clone(),
        };
        normalized.insert(normalized_key.to_string(), normalized_value);
    }
    normalized
        .entry("composerIconUrl".to_string())
        .or_insert(Value::Null);
    normalized
        .entry("logoUrl".to_string())
        .or_insert(Value::Null);
    normalized
        .entry("screenshotUrls".to_string())
        .or_insert_with(|| json!([]));
    Value::Object(normalized)
}

fn plugin_asset_array(package_dir: &Path, value: &Value) -> Value {
    let Some(values) = value.as_array() else {
        return json!([]);
    };
    Value::Array(
        values
            .iter()
            .map(|value| plugin_asset_value(package_dir, value))
            .collect(),
    )
}

fn plugin_asset_value(package_dir: &Path, value: &Value) -> Value {
    let Some(raw) = value.as_str() else {
        return value.clone();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || Path::new(trimmed).is_absolute()
    {
        return Value::String(trimmed.to_string());
    }
    Value::String(package_dir.join(trimmed).to_string_lossy().to_string())
}

fn standalone_app_list() -> Vec<Value> {
    let mut apps = Vec::new();
    let mut seen = BTreeSet::new();
    for kind in ["apps", "connectors"] {
        for root in codex_resource_roots(kind) {
            collect_json_manifest_files(
                &root,
                0,
                &["app.json", "connector.json", "plugin.json"],
                &mut |path| {
                    let key = canonical_key(path);
                    if seen.insert(key) {
                        if let Some(app) = manifest_json_from_path(path, "app") {
                            apps.push(app);
                        }
                    }
                },
            );
        }
    }
    apps.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(right.get("id").and_then(Value::as_str).unwrap_or_default())
    });
    apps
}

fn collect_json_manifest_files<F>(dir: &Path, depth: usize, names: &[&str], visitor: &mut F)
where
    F: FnMut(&Path),
{
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| names.contains(&name))
        {
            visitor(&path);
        } else if metadata.is_dir() {
            collect_json_manifest_files(&path, depth + 1, names, visitor);
        }
    }
}

fn manifest_json_from_path(path: &Path, kind: &str) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut value = serde_json::from_str::<Value>(&content).ok()?;
    let object = value.as_object_mut()?;
    let fallback_name = plugin_package_dir_for_manifest(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(kind)
        .to_string();
    let fallback_id = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&fallback_name)
        .to_string();
    if object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        object.insert("id".to_string(), Value::String(fallback_id.clone()));
    }
    if object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        object.insert("name".to_string(), Value::String(fallback_name));
    }
    object.insert("type".to_string(), Value::String(kind.to_string()));
    object.insert(
        "source".to_string(),
        Value::String("filesystem".to_string()),
    );
    object.insert(
        "path".to_string(),
        Value::String(path.to_string_lossy().to_string()),
    );
    object
        .entry("enabled".to_string())
        .or_insert(Value::Bool(true));
    Some(value)
}

fn manifest_is_inside_codex_app_dir(path: &Path, dir_name: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_string_lossy() == dir_name)
}

fn standalone_mcp_server_status_list() -> Vec<Value> {
    let mut servers = BTreeMap::<String, StandaloneMcpServer>::new();
    for config_path in codex_config_paths() {
        let Ok(content) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        for server in parse_mcp_servers_from_config(&content, &config_path) {
            servers.entry(server.name.clone()).or_insert(server);
        }
    }
    for config_path in codex_plugin_mcp_config_paths() {
        let Ok(content) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
        for server in parse_mcp_servers_from_json(&value, &config_path, base_dir, "plugin") {
            servers.entry(server.name.clone()).or_insert(server);
        }
    }
    servers
        .into_values()
        .map(StandaloneMcpServer::to_json)
        .collect()
}

fn claude_code_capability_args(work: &TurnWork, launch_services: bool) -> Vec<String> {
    if is_claude_title_generation_prompt(&work.prompt) {
        return Vec::new();
    }
    let mut args = Vec::new();
    if let Some(mcp_config) = claude_code_mcp_config_json(work, launch_services) {
        args.push("--mcp-config".to_string());
        args.push(mcp_config);
    }
    args
}

fn claude_code_mcp_config_json(work: &TurnWork, launch_services: bool) -> Option<String> {
    let mut mcp_servers = serde_json::Map::new();
    for server in standalone_mcp_server_status_list() {
        if server.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(name) = server.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(command) = server
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let mut config = serde_json::Map::new();
        config.insert("command".to_string(), Value::String(command.to_string()));
        if let Some(args) = server.get("args").and_then(Value::as_array) {
            config.insert("args".to_string(), Value::Array(args.clone()));
        }
        if let Some(cwd) = server
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            config.insert("cwd".to_string(), Value::String(cwd.to_string()));
        }
        if let Some(env) = server.get("env").filter(|env| env.is_object()) {
            config.insert("env".to_string(), env.clone());
        }
        let claude_name = claude_code_mcp_server_name(name, &mcp_servers);
        if claude_code_mcp_server_is_computer_use(name, command) {
            configure_computer_use_mcp_server(&mut config, work, &claude_name, launch_services);
        } else if claude_code_mcp_server_requires_metadata_relay(name, command) {
            wrap_mcp_server_with_metadata_relay(&mut config, work, &claude_name);
        }
        mcp_servers.insert(claude_name, Value::Object(config));
    }
    if mcp_servers.is_empty() {
        return None;
    }
    serde_json::to_string(&json!({ "mcpServers": mcp_servers })).ok()
}

fn claude_code_mcp_config_log_summary(work: &TurnWork) -> Value {
    let Some(config) = claude_code_mcp_config_json(work, false) else {
        return json!({
            "injected": false,
            "servers": [],
        });
    };
    let servers = serde_json::from_str::<Value>(&config)
        .ok()
        .and_then(|value| value.get("mcpServers").and_then(Value::as_object).cloned())
        .map(|servers| {
            servers
                .into_iter()
                .map(|(name, server)| {
                    json!({
                        "name": name,
                        "command": server.get("command").and_then(Value::as_str),
                        "cwd": server.get("cwd").and_then(Value::as_str),
                        "args": server
                            .get("args")
                            .and_then(Value::as_array)
                            .map(|args| args.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                            .unwrap_or_default(),
                        "envKeys": server
                            .get("env")
                            .and_then(Value::as_object)
                            .map(|env| env.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "injected": true,
        "servers": servers,
    })
}

fn claude_code_mcp_server_is_computer_use(name: &str, command: &str) -> bool {
    let lower_name = name.trim().to_ascii_lowercase();
    lower_name == "computer-use" || command.contains("SkyComputerUseClient")
}

fn claude_code_mcp_server_requires_metadata_relay(_name: &str, _command: &str) -> bool {
    false
}

fn configure_computer_use_mcp_server(
    config: &mut serde_json::Map<String, Value>,
    work: &TurnWork,
    server_name: &str,
    launch_service: bool,
) {
    add_claude_code_mcp_turn_env(config, work);
    if launch_service {
        if let Some(command) = config.get("command").and_then(Value::as_str) {
            maybe_launch_computer_use_service_for_command(server_name, work, command);
        }
    }
    if !wrap_computer_use_mcp_server_with_node_relay(config, work, server_name) {
        wrap_mcp_server_with_metadata_relay(config, work, server_name);
    }
}

fn wrap_computer_use_mcp_server_with_node_relay(
    config: &mut serde_json::Map<String, Value>,
    work: &TurnWork,
    server_name: &str,
) -> bool {
    let Some(node) = computer_use_node_relay_node_path() else {
        return false;
    };
    let Some(script_path) = ensure_computer_use_node_relay_script() else {
        return false;
    };
    let Some(command) = config
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return false;
    };
    let real_args = config
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut args = vec![
        script_path.to_string_lossy().to_string(),
        "--server-name".to_string(),
        server_name.to_string(),
        "--thread-id".to_string(),
        work.thread_id.clone(),
        "--turn-id".to_string(),
        work.turn_id.clone(),
        "--session-id".to_string(),
        work.claude_session_id.clone(),
        "--cwd".to_string(),
        work.cwd.clone(),
        "--".to_string(),
        command,
    ];
    args.extend(real_args);
    config.insert(
        "command".to_string(),
        Value::String(node.to_string_lossy().to_string()),
    );
    config.insert(
        "args".to_string(),
        Value::Array(args.into_iter().map(Value::String).collect()),
    );
    add_claude_code_mcp_turn_env(config, work);
    true
}

fn computer_use_node_relay_node_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(COMPUTER_USE_NODE_RELAY_NODE_ENV) {
        let value = value.to_string_lossy();
        let value = value.trim();
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "off"
        ) {
            return None;
        }
        let path = expand_log_path(value);
        return path.is_file().then_some(path);
    }
    computer_use_node_relay_node_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

fn computer_use_node_relay_node_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(
        "/Applications/Codex.app/Contents/Resources/node",
    )];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(if cfg!(windows) { "node.exe" } else { "node" }));
        }
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ]);
    candidates
}

fn ensure_computer_use_node_relay_script() -> Option<PathBuf> {
    let path = std::env::temp_dir().join("codexl-computer-use-mcp-relay.cjs");
    let should_write = std::fs::read_to_string(&path)
        .map(|current| current != COMPUTER_USE_NODE_RELAY_SCRIPT)
        .unwrap_or(true);
    if should_write && std::fs::write(&path, COMPUTER_USE_NODE_RELAY_SCRIPT).is_err() {
        return None;
    }
    Some(path)
}

fn wrap_mcp_server_with_metadata_relay(
    config: &mut serde_json::Map<String, Value>,
    work: &TurnWork,
    server_name: &str,
) {
    let Some(command) = config
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let real_args = config
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    let mut args = vec![
        crate::cli_middleware::CLAUDE_CODE_MCP_METADATA_RELAY_RUN_MODE_ARG.to_string(),
        "--server-name".to_string(),
        server_name.to_string(),
        "--thread-id".to_string(),
        work.thread_id.clone(),
        "--turn-id".to_string(),
        work.turn_id.clone(),
        "--session-id".to_string(),
        work.claude_session_id.clone(),
        "--cwd".to_string(),
        work.cwd.clone(),
        "--".to_string(),
        command,
    ];
    args.extend(real_args);
    config.insert(
        "command".to_string(),
        Value::String(current_exe.to_string_lossy().to_string()),
    );
    config.insert(
        "args".to_string(),
        Value::Array(args.into_iter().map(Value::String).collect()),
    );
    add_claude_code_mcp_turn_env(config, work);
}

fn add_claude_code_mcp_turn_env(config: &mut serde_json::Map<String, Value>, work: &TurnWork) {
    let mut env = config
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    env.insert(
        "CODEX_SESSION_ID".to_string(),
        json!(work.claude_session_id),
    );
    env.insert("CODEX_TURN_ID".to_string(), json!(work.turn_id));
    env.insert("CODEX_THREAD_ID".to_string(), json!(work.thread_id));
    config.insert("env".to_string(), Value::Object(env));
}

fn claude_code_mcp_server_name(name: &str, existing: &serde_json::Map<String, Value>) -> String {
    let base = if claude_code_reserved_mcp_server_name(name) {
        format!("codex-{name}")
    } else {
        name.to_string()
    };
    let mut candidate = base.clone();
    let mut index = 2;
    while existing.contains_key(&candidate) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn claude_code_reserved_mcp_server_name(name: &str) -> bool {
    matches!(name.trim().to_ascii_lowercase().as_str(), "computer-use")
}

#[derive(Debug, Clone, Default)]
struct StandaloneMcpServer {
    name: String,
    command: Option<String>,
    args: Vec<String>,
    enabled: bool,
    config_path: String,
    cwd: Option<String>,
    env: Option<Value>,
    source: String,
}

impl StandaloneMcpServer {
    fn to_json(self) -> Value {
        json!({
            "id": self.name,
            "name": self.name,
            "serverName": self.name,
            "server_name": self.name,
            "status": if self.enabled { "configured" } else { "disabled" },
            "enabled": self.enabled,
            "command": self.command,
            "args": self.args,
            "transport": "stdio",
            "cwd": self.cwd,
            "env": self.env.unwrap_or_else(|| json!({})),
            "source": self.source,
            "configPath": self.config_path,
            "config_path": self.config_path,
            "error": Value::Null,
        })
    }
}

fn parse_mcp_servers_from_config(content: &str, config_path: &Path) -> Vec<StandaloneMcpServer> {
    let mut servers = Vec::new();
    let mut current: Option<StandaloneMcpServer> = None;
    let mut in_env_table = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(server) = current.take() {
                servers.push(server);
            }
            let table = line.trim_matches(['[', ']']);
            if let Some(name) = mcp_server_name_from_table(table) {
                if table.contains(".env") {
                    in_env_table = true;
                    current = None;
                } else {
                    in_env_table = false;
                    current = Some(StandaloneMcpServer {
                        name,
                        enabled: true,
                        config_path: config_path.to_string_lossy().to_string(),
                        source: "config".to_string(),
                        ..StandaloneMcpServer::default()
                    });
                }
            } else {
                current = None;
                in_env_table = false;
            }
            continue;
        }
        if in_env_table {
            continue;
        }
        let Some(server) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "command" => server.command = parse_toml_string(value),
            "args" => server.args = parse_toml_string_array(value),
            "enabled" => server.enabled = parse_toml_bool(value).unwrap_or(server.enabled),
            "disabled" => {
                if let Some(disabled) = parse_toml_bool(value) {
                    server.enabled = !disabled;
                }
            }
            _ => {}
        }
    }
    if let Some(server) = current {
        servers.push(server);
    }
    servers
}

fn codex_plugin_mcp_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for root in codex_resource_roots("plugins") {
        collect_json_manifest_files(&root, 0, &["plugin.json"], &mut |path| {
            let Some(mcp_path) = plugin_manifest_mcp_config_path(path) else {
                return;
            };
            push_unique_path(&mut paths, &mut seen, mcp_path);
        });
        collect_json_manifest_files(&root, 0, &[".mcp.json"], &mut |path| {
            push_unique_path(&mut paths, &mut seen, path.to_path_buf());
        });
    }
    paths
}

fn plugin_manifest_mcp_config_path(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&content).ok()?;
    let mcp_servers = value.get("mcpServers")?.as_str()?.trim();
    if mcp_servers.is_empty() {
        return None;
    }
    let mcp_path = PathBuf::from(mcp_servers);
    if mcp_path.is_absolute() {
        return Some(mcp_path);
    }
    Some(plugin_package_dir_for_manifest(path).join(mcp_path))
}

fn plugin_package_dir_for_manifest(path: &Path) -> PathBuf {
    let manifest_dir = path.parent().unwrap_or_else(|| Path::new("."));
    if manifest_dir.file_name().and_then(|value| value.to_str()) == Some(".codex-plugin") {
        return manifest_dir.parent().unwrap_or(manifest_dir).to_path_buf();
    }
    manifest_dir.to_path_buf()
}

fn parse_mcp_servers_from_json(
    value: &Value,
    config_path: &Path,
    base_dir: &Path,
    source: &str,
) -> Vec<StandaloneMcpServer> {
    let Some(servers_object) = value.get("mcpServers").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut servers = Vec::new();
    for (name, server_value) in servers_object {
        let Some(server_object) = server_value.as_object() else {
            continue;
        };
        let cwd_path = server_object
            .get("cwd")
            .and_then(Value::as_str)
            .map(|cwd| resolve_mcp_config_path(base_dir, cwd));
        let command_base = cwd_path.as_deref().unwrap_or(base_dir);
        let mut command = server_object
            .get("command")
            .and_then(Value::as_str)
            .map(|command| resolve_mcp_command(command_base, command));
        let mut cwd = cwd_path.map(path_to_string);
        if standalone_mcp_server_is_computer_use(name, command.as_deref()) {
            if let Some(global_command) = global_computer_use_client_command() {
                command = Some(global_command.to_string_lossy().to_string());
                cwd = global_computer_use_app_dir().map(|path| path.to_string_lossy().to_string());
            }
        }
        let args = server_object
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let enabled = server_object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && !server_object
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        servers.push(StandaloneMcpServer {
            name: name.to_string(),
            command,
            args,
            enabled,
            config_path: config_path.to_string_lossy().to_string(),
            cwd,
            env: server_object
                .get("env")
                .filter(|env| env.is_object())
                .cloned(),
            source: source.to_string(),
        });
    }
    servers
}

fn resolve_mcp_config_path(base_dir: &Path, value: &str) -> PathBuf {
    let value = value.trim();
    if value.is_empty() {
        return base_dir.to_path_buf();
    }
    let expanded = crate::config::normalize_home_path(value);
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn resolve_mcp_command(base_dir: &Path, command: &str) -> String {
    let command = command.trim();
    if command.is_empty() {
        return String::new();
    }
    let expanded = crate::config::normalize_home_path(command);
    let path = PathBuf::from(&expanded);
    if path.is_absolute() || path.components().count() > 1 {
        path_to_string(if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        })
    } else {
        expanded
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.canonicalize()
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn standalone_mcp_server_is_computer_use(name: &str, command: Option<&str>) -> bool {
    name.trim().eq_ignore_ascii_case("computer-use")
        || command
            .map(|command| command.contains("SkyComputerUseClient"))
            .unwrap_or(false)
}

fn global_computer_use_client_command() -> Option<PathBuf> {
    let app_dir = global_computer_use_app_dir()?;
    let command = app_dir
        .join("Codex Computer Use.app")
        .join("Contents")
        .join("SharedSupport")
        .join("SkyComputerUseClient.app")
        .join("Contents")
        .join("MacOS")
        .join("SkyComputerUseClient");
    command.is_file().then_some(command)
}

fn global_computer_use_app_dir() -> Option<PathBuf> {
    let path = global_default_codex_home_candidate()?.join("computer-use");
    path.is_dir().then_some(path)
}

fn mcp_server_name_from_table(table: &str) -> Option<String> {
    let rest = table.strip_prefix("mcp_servers.")?;
    let name = rest.split('.').next()?.trim().trim_matches('"');
    (!name.is_empty()).then(|| name.to_string())
}

fn standalone_hooks_list(_params: &Value) -> Vec<Value> {
    Vec::new()
}

fn standalone_extension_list() -> Vec<Value> {
    [
        crate::extensions::builtin_bot_gateway_status(),
        crate::extensions::builtin_next_ai_gateway_status(),
    ]
    .into_iter()
    .filter_map(|status| serde_json::to_value(status).ok())
    .collect()
}

fn codex_resource_roots(kind: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for home in codex_home_candidates() {
        push_unique_path(&mut roots, &mut seen, home.join(kind));
        push_unique_path(
            &mut roots,
            &mut seen,
            home.join("vendor_imports").join(kind),
        );
    }
    roots
}

fn codex_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for home in codex_home_candidates() {
        push_unique_path(&mut paths, &mut seen, home.join("config.toml"));
    }
    paths
}

fn codex_home_candidates() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let mut seen = BTreeSet::new();
    if let Ok(value) = std::env::var("CODEX_HOME") {
        push_unique_path(
            &mut homes,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(&value)),
        );
    }
    push_unique_path(
        &mut homes,
        &mut seen,
        PathBuf::from(crate::config::default_codex_home()),
    );
    if let Some(global_home) = global_default_codex_home_candidate() {
        push_unique_path(&mut homes, &mut seen, global_home);
    }
    let config = crate::config::AppConfig::load();
    push_unique_path(
        &mut homes,
        &mut seen,
        PathBuf::from(crate::config::normalize_home_path(&config.codex_home)),
    );
    for profile in config.codex_home_profiles {
        push_unique_path(
            &mut homes,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(&profile.path)),
        );
    }
    for profile in config.provider_profiles {
        push_unique_path(
            &mut homes,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(&profile.codex_home)),
        );
        push_unique_path(
            &mut homes,
            &mut seen,
            crate::config::generated_codex_home(&profile),
        );
    }
    homes
}

fn push_unique_path(paths: &mut Vec<PathBuf>, seen: &mut BTreeSet<String>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    let key = canonical_key(&path);
    if seen.insert(key) {
        paths.push(path);
    }
}

fn canonical_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn front_matter_value(content: &str, key: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == key {
            return Some(trim_quoted(value.trim()).to_string());
        }
    }
    None
}

fn markdown_title(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn markdown_first_paragraph(content: &str) -> Option<String> {
    let mut in_front_matter = false;
    let mut front_matter_seen = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" && !front_matter_seen {
            in_front_matter = true;
            front_matter_seen = true;
            continue;
        }
        if trimmed == "---" && in_front_matter {
            in_front_matter = false;
            continue;
        }
        if in_front_matter || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

fn parse_toml_string(value: &str) -> Option<String> {
    let value = strip_toml_comment(value).trim();
    if value.is_empty() {
        return None;
    }
    Some(trim_quoted(value).to_string())
}

fn parse_toml_string_array(value: &str) -> Vec<String> {
    let value = strip_toml_comment(value).trim();
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|part| trim_quoted(part.trim()).to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_toml_bool(value: &str) -> Option<bool> {
    match strip_toml_comment(value).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn strip_toml_comment(value: &str) -> &str {
    value.split('#').next().unwrap_or(value)
}

fn trim_quoted(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'').trim()
}

impl ClaudeAppServerState {
    fn start_thread(&mut self, params: &Value) -> (Value, Value) {
        let id = new_uuid_v4();
        let cwd = normalize_cwd(params.get("cwd").and_then(Value::as_str));
        let model = model_from_params(params);
        let now = now_seconds();
        let name = self.workspace_name.clone();
        let thread = ClaudeThread {
            id: id.clone(),
            session_id: id.clone(),
            claude_session_id: id,
            path: None,
            preview: String::new(),
            cwd,
            model,
            created_at: now,
            updated_at: now,
            archived: false,
            name,
            turns: Vec::new(),
            latest_token_usage_info: None,
        };
        let response = thread_runtime_response(&thread, false);
        let notification = json!({
            "method": "thread/started",
            "params": { "thread": thread.to_json(false) },
        });
        self.threads.insert(thread.id.clone(), thread);
        (response, notification)
    }

    fn resume_thread(&mut self, params: &Value) -> Result<(Value, Value), String> {
        let thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(new_uuid_v4);
        let lookup_thread_id = strip_local_thread_prefix(&thread_id);
        if !self.threads.contains_key(&thread_id) && !self.threads.contains_key(lookup_thread_id) {
            let thread = load_claude_thread_from_params(params, self.workspace_name.clone())
                .or_else(|| load_claude_thread_by_id(lookup_thread_id, self.workspace_name.clone()))
                .ok_or_else(|| format!("thread not found: {}", thread_id))?;
            self.threads.insert(thread.id.clone(), thread);
        }
        let thread = self
            .threads
            .get(&thread_id)
            .or_else(|| self.threads.get(lookup_thread_id))
            .or_else(|| {
                self.threads.values().find(|thread| {
                    thread.path.as_deref()
                        == params
                            .get("path")
                            .and_then(Value::as_str)
                            .filter(|path| !path.trim().is_empty())
                })
            })
            .ok_or_else(|| format!("thread not loaded: {}", thread_id))?;
        if is_claude_title_generation_thread(thread) {
            return Err(format!("thread not found: {}", thread_id));
        }
        let include_turns = !params
            .get("excludeTurns")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut response_thread = thread.clone();
        let inline_titles = load_claude_inline_thread_titles();
        apply_inline_claude_thread_title(
            &mut response_thread,
            &inline_titles,
            self.workspace_name.as_deref(),
        );
        let generated_titles = self.generated_titles();
        apply_generated_titles_to_single_claude_thread(
            &mut response_thread,
            &generated_titles,
            self.workspace_name.as_deref(),
        );
        let response = thread_runtime_response(&response_thread, include_turns);
        let notification = json!({
            "method": "thread/started",
            "params": { "thread": response_thread.to_json(false) },
        });
        Ok((response, notification))
    }

    fn thread_read(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_param(params, "threadId")?;
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        let include_turns = params
            .get("includeTurns")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(thread) = self
            .threads
            .get(thread_id)
            .or_else(|| self.threads.get(lookup_thread_id))
        {
            if is_claude_title_generation_thread(thread) {
                return Err(format!("thread not found: {}", thread_id));
            }
            let mut thread = thread.clone();
            let inline_titles = load_claude_inline_thread_titles();
            apply_inline_claude_thread_title(
                &mut thread,
                &inline_titles,
                self.workspace_name.as_deref(),
            );
            let generated_titles = self.generated_titles();
            apply_generated_titles_to_single_claude_thread(
                &mut thread,
                &generated_titles,
                self.workspace_name.as_deref(),
            );
            return Ok(json!({ "thread": thread.to_json(include_turns) }));
        }
        let mut thread = load_claude_thread_by_id(lookup_thread_id, self.workspace_name.clone())
            .ok_or_else(|| format!("thread not found: {}", thread_id))?;
        let generated_titles = self.generated_titles();
        apply_generated_titles_to_single_claude_thread(
            &mut thread,
            &generated_titles,
            self.workspace_name.as_deref(),
        );
        Ok(json!({ "thread": thread.to_json(include_turns) }))
    }

    fn thread_list(&self, params: &Value) -> Value {
        let archived = params
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut threads = load_claude_threads(self.workspace_name.clone());
        let mut generated_titles = load_claude_generated_titles();
        let inline_titles = load_claude_inline_thread_titles();
        for thread in self.threads.values() {
            if let Some(generated_title) = claude_generated_title_from_thread(thread) {
                generated_titles.push(generated_title);
                continue;
            }
            let mut thread = thread.clone();
            apply_inline_claude_thread_title(
                &mut thread,
                &inline_titles,
                self.workspace_name.as_deref(),
            );
            threads.insert(thread.id.clone(), thread);
        }
        apply_generated_titles_to_claude_threads(
            &mut threads,
            &generated_titles,
            self.workspace_name.as_deref(),
        );
        let mut data = threads
            .values()
            .filter(|thread| thread_matches_list_params(thread, params, archived))
            .map(|thread| thread.to_json(false))
            .collect::<Vec<_>>();
        let sort_key = match params.get("sortKey").and_then(Value::as_str) {
            Some("created_at") | Some("createdAt") => "createdAt",
            Some("updated_at") | Some("updatedAt") => "updatedAt",
            _ => "createdAt",
        };
        let sort_desc = !matches!(
            params.get("sortDirection").and_then(Value::as_str),
            Some("asc")
        );
        data.sort_by(|left, right| {
            let left_key = left.get(sort_key).and_then(Value::as_i64);
            let right_key = right.get(sort_key).and_then(Value::as_i64);
            if sort_desc {
                right_key.cmp(&left_key)
            } else {
                left_key.cmp(&right_key)
            }
        });
        if let Some(limit) = params.get("limit").and_then(Value::as_u64) {
            data.truncate(limit as usize);
        }
        json!({
            "data": data,
            "nextCursor": Value::Null,
            "backwardsCursor": Value::Null,
        })
    }

    fn thread_turns_list(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_param(params, "threadId")?;
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        let loaded_thread;
        let thread = if let Some(thread) = self
            .threads
            .get(thread_id)
            .or_else(|| self.threads.get(lookup_thread_id))
        {
            thread
        } else {
            loaded_thread = load_claude_thread_by_id(lookup_thread_id, self.workspace_name.clone())
                .ok_or_else(|| format!("thread not found: {}", thread_id))?;
            &loaded_thread
        };
        if is_claude_title_generation_thread(thread) {
            return Err(format!("thread not found: {}", thread_id));
        }
        let mut turns = thread.turns.clone();
        if !matches!(
            params.get("sortDirection").and_then(Value::as_str),
            Some("asc")
        ) {
            turns.reverse();
        }
        if let Some(limit) = params.get("limit").and_then(Value::as_u64) {
            turns.truncate(limit as usize);
        }
        Ok(json!({
            "data": turns.iter().map(|turn| turn.to_json(true)).collect::<Vec<_>>(),
            "nextCursor": Value::Null,
            "backwardsCursor": Value::Null,
        }))
    }

    fn set_archived(&mut self, params: &Value, archived: bool) -> Option<Value> {
        let thread_id = params.get("threadId").and_then(Value::as_str)?;
        let thread = self.threads.get_mut(thread_id)?;
        thread.archived = archived;
        thread.updated_at = now_seconds();
        Some(json!({
            "method": if archived { "thread/archived" } else { "thread/unarchived" },
            "params": { "threadId": thread_id },
        }))
    }

    fn set_thread_name(&mut self, params: &Value) -> Option<Value> {
        let thread_id = params.get("threadId").and_then(Value::as_str)?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        persist_claude_thread_name(thread_id, name.as_deref());
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        let thread = if let Some(thread) = self.threads.get_mut(thread_id) {
            thread
        } else {
            self.threads.get_mut(lookup_thread_id)?
        };
        thread.name = name.clone();
        thread.updated_at = now_seconds();
        Some(json!({
            "method": "thread/name/updated",
            "params": {
                "threadId": thread_id,
                "name": name,
            },
        }))
    }

    fn start_turn(
        &mut self,
        params: &Value,
    ) -> Result<(Value, Vec<Value>, TurnWork, Vec<StaleActiveProcess>), String> {
        let thread_id = required_param(params, "threadId")?.to_string();
        {
            let thread = self
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| format!("thread not found: {}", thread_id))?;
            if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
                thread.cwd = normalize_cwd(Some(cwd));
            }
            if let Some(model) = params.get("model").and_then(Value::as_str) {
                thread.model = model.to_string();
            }
        }
        let input = params
            .get("input")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let prompt = prompt_from_input(&input);
        let is_title_generation = is_claude_title_generation_prompt(&prompt);
        let now = now_seconds();
        let stale_processes = if is_title_generation {
            Vec::new()
        } else {
            self.interrupt_active_processes_for_thread(&thread_id, now)
        };
        if !stale_processes.is_empty() {
            claude_code_log_event(
                "turn_start_interrupted_stale_processes",
                json!({
                    "threadId": &thread_id,
                    "count": stale_processes.len(),
                    "processes": stale_processes
                        .iter()
                        .map(|process| json!({
                            "turnId": process.turn_id,
                            "pid": process.pid,
                        }))
                        .collect::<Vec<_>>(),
                }),
            );
        }
        let thread = self
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| format!("thread not found: {}", thread_id))?;
        if thread.preview.is_empty() {
            thread.preview = prompt.chars().take(160).collect();
        }
        let resume_existing = thread.turns.iter().any(|turn| {
            matches!(
                turn.status,
                TurnStatus::Completed | TurnStatus::Interrupted | TurnStatus::Failed
            )
        });
        let turn = ClaudeTurn {
            id: format!("turn-{}", new_uuid_v4()),
            input,
            tool_items: Vec::new(),
            agent_text: String::new(),
            status: TurnStatus::InProgress,
            error: None,
            started_at: now,
            completed_at: None,
            duration_ms: None,
        };
        let turn_id = turn.id.clone();
        let user_item = turn.user_item_json();
        let agent_item_id = agent_item_id_for_turn(&turn_id);
        let cli_item_id = cli_item_id_for_turn(&turn_id);
        let response_turn = turn.to_json(false);
        thread.updated_at = now;
        thread.turns.push(turn);
        let work = TurnWork {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            agent_item_id,
            cli_item_id,
            claude_session_id: thread.claude_session_id.clone(),
            cwd: thread.cwd.clone(),
            prompt,
            resume_existing,
        };
        claude_code_log_event(
            "turn_start_prepared",
            json!({
                "threadId": &work.thread_id,
                "turnId": &work.turn_id,
                "claudeSessionId": &work.claude_session_id,
                "cwd": &work.cwd,
                "resumeExisting": work.resume_existing,
                "titleGeneration": is_title_generation,
                "promptPreview": log_text_preview(&work.prompt, 200),
            }),
        );
        let notifications = if is_title_generation {
            thread.archived = true;
            vec![claude_thread_archived_notification(&thread_id)]
        } else {
            vec![
                claude_thread_started_notification(thread),
                json!({
                    "method": "turn/started",
                    "params": {
                        "threadId": thread_id,
                        "turn": response_turn.clone(),
                    },
                }),
                json!({
                    "method": "item/started",
                    "params": {
                        "threadId": thread_id,
                        "turnId": turn_id,
                        "item": user_item,
                        "startedAtMs": now_millis(),
                    },
                }),
                claude_thread_stream_state_changed_notification(thread),
            ]
        };
        Ok((
            json!({ "turn": response_turn.clone() }),
            notifications,
            work,
            stale_processes,
        ))
    }

    fn interrupt_active_processes_for_thread(
        &mut self,
        thread_id: &str,
        completed_at: i64,
    ) -> Vec<StaleActiveProcess> {
        let stale_keys = self
            .active_processes
            .keys()
            .filter(|(active_thread_id, _)| active_thread_id == thread_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut stale_processes = Vec::new();
        for key in stale_keys {
            if let Some(pid) = self.active_processes.remove(&key) {
                self.interrupted_turns.insert(key.clone());
                stale_processes.push(StaleActiveProcess {
                    thread_id: key.0,
                    turn_id: key.1,
                    pid,
                });
            }
        }
        if let Some(thread) = self.threads.get_mut(thread_id) {
            for stale_process in &stale_processes {
                if let Some(turn) = thread
                    .turns
                    .iter_mut()
                    .find(|turn| turn.id == stale_process.turn_id)
                {
                    if turn.status == TurnStatus::InProgress {
                        turn.status = TurnStatus::Interrupted;
                        turn.completed_at = Some(completed_at);
                        turn.duration_ms = Some(
                            completed_at
                                .saturating_sub(turn.started_at)
                                .saturating_mul(1000),
                        );
                    }
                }
            }
            if !stale_processes.is_empty() {
                thread.updated_at = completed_at;
            }
        }
        stale_processes
    }

    fn interrupt_turn(&mut self, params: &Value) -> Option<u32> {
        let thread_id = params.get("threadId").and_then(Value::as_str)?;
        let requested_turn_id = params.get("turnId").and_then(Value::as_str);
        let thread = self.threads.get_mut(thread_id)?;
        let turn_id = requested_turn_id
            .filter(|turn_id| thread.turns.iter().any(|turn| turn.id == *turn_id))
            .map(str::to_string)
            .or_else(|| {
                thread
                    .turns
                    .iter()
                    .rev()
                    .find(|turn| turn.status == TurnStatus::InProgress)
                    .map(|turn| turn.id.clone())
            })?;
        let turn = thread.turns.iter_mut().find(|turn| turn.id == turn_id)?;
        turn.status = TurnStatus::Interrupted;
        thread.updated_at = now_seconds();
        let key = (thread_id.to_string(), turn_id.clone());
        self.interrupted_turns.insert(key.clone());
        let pid = self.active_processes.get(&key).copied();
        claude_code_log_event(
            "turn_interrupt_registered",
            json!({
                "threadId": thread_id,
                "requestedTurnId": requested_turn_id,
                "turnId": turn_id,
                "pid": pid,
            }),
        );
        pid
    }

    fn generated_titles(&self) -> Vec<ClaudeGeneratedTitle> {
        let mut generated_titles = load_claude_generated_titles();
        generated_titles.extend(
            self.threads
                .values()
                .filter_map(claude_generated_title_from_thread),
        );
        generated_titles
    }

    fn finish_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        result: ClaudeRunResult,
        generated_title_hint: Option<ClaudeGeneratedTitle>,
    ) -> Option<FinishTurnNotifications> {
        let key = (thread_id.to_string(), turn_id.to_string());
        self.active_processes.remove(&key);
        let (item, turn_json, generated_title, thread_stream_state, is_title_generation) = {
            let thread = self.threads.get_mut(thread_id)?;
            let turn = thread.turns.iter_mut().find(|turn| turn.id == turn_id)?;
            let interrupted =
                self.interrupted_turns.remove(&key) || turn.status == TurnStatus::Interrupted;
            turn.tool_items = result.tool_items;
            let agent_item_streamed = result.agent_item_streamed;
            if let Some(latest_token_usage_info) = result.latest_token_usage_info {
                thread.latest_token_usage_info = Some(latest_token_usage_info);
            }
            turn.agent_text = result.text;
            turn.duration_ms = Some(result.duration_ms);
            turn.completed_at = Some(now_seconds());
            if interrupted {
                turn.status = TurnStatus::Interrupted;
                turn.error = None;
            } else if let Some(error) = result.error {
                turn.status = TurnStatus::Failed;
                turn.error = Some(error);
            } else {
                turn.status = TurnStatus::Completed;
                turn.error = None;
            }
            thread.updated_at = turn.completed_at.unwrap_or_else(now_seconds);
            let item = (!agent_item_streamed && !turn.agent_text.is_empty())
                .then(|| turn.agent_item_json());
            let turn_json = turn.to_json(false);
            let generated_title = generated_title_hint
                .filter(|generated_title| generated_title.title.is_some())
                .or_else(|| claude_generated_title_from_thread(thread));
            let is_title_generation = generated_title.is_some();
            let thread_stream_state = (!is_title_generation)
                .then(|| claude_thread_stream_state_changed_notification(thread));
            (
                item,
                turn_json,
                generated_title,
                thread_stream_state,
                is_title_generation,
            )
        };
        let mut extra_notifications = Vec::new();
        if let Some(generated_title) = generated_title {
            claude_code_log_event(
                "title_generation_resolved",
                json!({
                    "threadId": thread_id,
                    "title": generated_title.title.as_deref(),
                    "sourcePromptPreview": log_text_preview(&generated_title.source_prompt, 120),
                    "cwd": &generated_title.cwd,
                    "loadedThreadCount": self.threads.len(),
                }),
            );
            if let Some((target_thread_id, name)) = apply_generated_title_to_claude_threads(
                &mut self.threads,
                &generated_title,
                self.workspace_name.as_deref(),
            ) {
                claude_code_log_event(
                    "title_generation_applied",
                    json!({
                        "titleThreadId": thread_id,
                        "targetThreadId": &target_thread_id,
                        "name": &name,
                    }),
                );
                extra_notifications.push(json!({
                    "method": "thread/name/updated",
                    "params": {
                        "threadId": target_thread_id,
                        "name": name,
                    },
                }));
                if let Some(thread) = self.threads.get(&target_thread_id) {
                    extra_notifications.push(claude_thread_started_notification(thread));
                    extra_notifications
                        .push(claude_thread_stream_state_changed_notification(thread));
                }
            } else {
                claude_code_log_event(
                    "title_generation_no_target",
                    json!({
                        "titleThreadId": thread_id,
                        "sourcePromptPreview": log_text_preview(&generated_title.source_prompt, 120),
                        "title": generated_title.title.as_deref(),
                        "candidateThreads": self.threads.values().map(|thread| {
                            json!({
                                "threadId": &thread.id,
                                "cwd": &thread.cwd,
                                "name": &thread.name,
                                "promptPreview": log_text_preview(&thread_initial_prompt(thread), 80),
                                "isTitleGeneration": is_claude_title_generation_thread(thread),
                            })
                        }).collect::<Vec<_>>(),
                    }),
                );
            }
            extra_notifications.push(claude_thread_archived_notification(thread_id));
        }
        Some(FinishTurnNotifications {
            item_completed: (!is_title_generation).then(|| item).flatten().map(|item| {
                json!({
                    "method": "item/completed",
                    "params": {
                        "threadId": thread_id,
                        "turnId": turn_id,
                        "item": item,
                        "completedAtMs": now_millis(),
                    },
                })
            }),
            turn_completed: (!is_title_generation).then(|| {
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": thread_id,
                        "turn": turn_json,
                    },
                })
            }),
            thread_stream_state,
            extra_notifications,
        })
    }
}

struct FinishTurnNotifications {
    item_completed: Option<Value>,
    turn_completed: Option<Value>,
    thread_stream_state: Option<Value>,
    extra_notifications: Vec<Value>,
}

fn claude_thread_archived_notification(thread_id: &str) -> Value {
    json!({
        "method": "thread/archived",
        "params": { "threadId": thread_id },
    })
}

fn claude_thread_started_notification(thread: &ClaudeThread) -> Value {
    json!({
        "method": "thread/started",
        "params": { "thread": thread.to_json(false) },
    })
}

fn claude_thread_stream_state_changed_notification(thread: &ClaudeThread) -> Value {
    json!({
        "type": "ipc-broadcast",
        "method": "thread-stream-state-changed",
        "sourceClientId": "codexl-claude-code-app-server",
        "version": 6,
        "params": {
            "conversationId": thread.id,
            "hostId": "local",
            "version": 6,
            "change": {
                "type": "snapshot",
                "conversationState": claude_conversation_state(thread),
            },
        },
    })
}

fn claude_conversation_state(thread: &ClaudeThread) -> Value {
    json!({
        "id": thread.id,
        "turns": thread
            .turns
            .iter()
            .map(|turn| claude_conversation_turn(thread, turn))
            .collect::<Vec<_>>(),
        "title": thread.name.clone().unwrap_or_default(),
        "source": "cli",
        "modelProvider": PROVIDER_NAME,
        "latestModel": thread.model,
        "latestReasoningEffort": Value::Null,
        "previousTurnModel": Value::Null,
        "latestCollaborationMode": Value::Null,
        "hasUnreadTurn": false,
        "threadGoal": Value::Null,
        "threadGoalResumeConfirmation": Value::Null,
        "completedThreadGoal": Value::Null,
        "threadRuntimeStatus": thread.status_json(),
        "rolloutPath": Value::Null,
        "cwd": thread.cwd,
        "gitInfo": Value::Null,
        "resumeState": "resumed",
        "latestTokenUsageInfo": thread
            .latest_token_usage_info
            .clone()
            .unwrap_or(Value::Null),
        "workspaceKind": "project",
        "workspaceBrowserRoot": Value::Null,
        "turnsPagination": {
            "olderCursor": Value::Null,
            "isLoadingOlder": false,
            "hasLoadedOldest": true,
        },
    })
}

fn claude_conversation_turn(thread: &ClaudeThread, turn: &ClaudeTurn) -> Value {
    json!({
        "params": {
            "threadId": thread.id,
            "input": turn.input,
            "approvalPolicy": "on-request",
            "approvalsReviewer": "user",
            "sandboxPolicy": {
                "type": "workspaceWrite",
                "writableRoots": [&thread.cwd],
            },
            "model": thread.model,
            "cwd": thread.cwd,
            "attachments": [],
            "effort": Value::Null,
            "summary": "none",
            "personality": Value::Null,
            "outputSchema": Value::Null,
            "collaborationMode": Value::Null,
        },
        "turnId": turn.id,
        "turnStartedAtMs": seconds_to_millis_value(turn.started_at),
        "durationMs": turn.duration_ms,
        "finalAssistantStartedAtMs": turn.completed_at.map(seconds_to_millis),
        "status": turn.status.as_protocol_str(),
        "error": turn.error.as_ref().map(|message| {
            json!({
                "message": message,
                "codexErrorInfo": Value::Null,
                "additionalDetails": Value::Null,
            })
        }),
        "diff": Value::Null,
        "items": turn.items_json(),
    })
}

fn seconds_to_millis(value: i64) -> i64 {
    value.saturating_mul(1000)
}

fn seconds_to_millis_value(value: i64) -> Value {
    json!(seconds_to_millis(value))
}

impl ClaudeThread {
    fn to_json(&self, include_turns: bool) -> Value {
        json!({
            "id": self.id,
            "sessionId": self.session_id,
            "forkedFromId": Value::Null,
            "preview": self.preview,
            "ephemeral": false,
            "modelProvider": PROVIDER_NAME,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "status": self.status_json(),
            "path": self
                .path
                .as_ref()
                .map(|path| Value::String(path.clone()))
                .unwrap_or(Value::Null),
            "cwd": self.cwd,
            "cliVersion": env!("CARGO_PKG_VERSION"),
            "source": "cli",
            "threadSource": Value::Null,
            "agentNickname": Value::Null,
            "agentRole": Value::Null,
            "gitInfo": Value::Null,
            "name": self.name,
            "turns": if include_turns {
                Value::Array(self.turns.iter().map(|turn| turn.to_json(true)).collect())
            } else {
                json!([])
            },
            "latestTokenUsageInfo": self
                .latest_token_usage_info
                .clone()
                .unwrap_or(Value::Null),
        })
    }

    fn status_json(&self) -> Value {
        if self
            .turns
            .iter()
            .any(|turn| turn.status == TurnStatus::InProgress)
        {
            json!({ "type": "active", "activeFlags": [] })
        } else {
            json!({ "type": "idle" })
        }
    }
}

impl ClaudeTurn {
    fn to_json(&self, include_items: bool) -> Value {
        json!({
            "id": self.id,
            "items": if include_items { self.items_json() } else { json!([]) },
            "itemsView": if include_items { "full" } else { "notLoaded" },
            "status": self.status.as_protocol_str(),
            "error": self.error.as_ref().map(|message| {
                json!({
                    "message": message,
                    "codexErrorInfo": Value::Null,
                    "additionalDetails": Value::Null,
                })
            }),
            "startedAt": self.started_at,
            "completedAt": self.completed_at,
            "durationMs": self.duration_ms,
        })
    }

    fn items_json(&self) -> Value {
        let mut items = Vec::new();
        items.push(json!({
            "type": "userMessage",
            "id": user_item_id_for_turn(&self.id),
            "content": self.input,
        }));
        items.extend(self.tool_items.iter().cloned());
        if !self.agent_text.is_empty() {
            items.push(self.agent_item_json());
        }
        Value::Array(items)
    }

    fn user_item_json(&self) -> Value {
        json!({
            "type": "userMessage",
            "id": user_item_id_for_turn(&self.id),
            "content": self.input,
        })
    }

    fn agent_item_json(&self) -> Value {
        json!({
            "type": "agentMessage",
            "id": agent_item_id_for_turn(&self.id),
            "text": self.agent_text,
            "phase": Value::Null,
            "memoryCitation": Value::Null,
        })
    }
}

impl TurnStatus {
    fn as_protocol_str(self) -> &'static str {
        match self {
            Self::InProgress => "inProgress",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }
}

fn thread_matches_list_params(thread: &ClaudeThread, params: &Value, archived: bool) -> bool {
    if thread.archived != archived {
        return false;
    }
    if let Some(source_kinds) = params.get("sourceKinds").and_then(Value::as_array) {
        if !source_kinds.is_empty()
            && !source_kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|source| source == "cli")
        {
            return false;
        }
    }
    if let Some(providers) = params.get("modelProviders").and_then(Value::as_array) {
        if !providers.is_empty()
            && !providers
                .iter()
                .filter_map(Value::as_str)
                .any(|provider| provider == PROVIDER_NAME)
        {
            return false;
        }
    }
    if let Some(cwd_filter) = params.get("cwd") {
        let cwd_matches = match cwd_filter {
            Value::String(cwd) => cwd == &thread.cwd,
            Value::Array(cwds) => cwds
                .iter()
                .filter_map(Value::as_str)
                .any(|cwd| cwd == thread.cwd),
            _ => true,
        };
        if !cwd_matches {
            return false;
        }
    }
    if let Some(search) = params
        .get("searchTerm")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    {
        let haystack = format!(
            "{}\n{}\n{}",
            thread.preview,
            thread.name.clone().unwrap_or_default(),
            thread.cwd
        )
        .to_ascii_lowercase();
        if !haystack.contains(&search) {
            return false;
        }
    }
    true
}

fn load_claude_thread_from_params(
    params: &Value,
    workspace_name: Option<String>,
) -> Option<ClaudeThread> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())?;
    if !is_claude_transcript_path(Path::new(path)) {
        return None;
    }
    let mut thread =
        load_claude_thread_from_transcript_path(Path::new(path), workspace_name.clone())?;
    if let Some(cwd) = params
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.trim().is_empty())
    {
        thread.cwd = normalize_cwd(Some(cwd));
    }
    apply_generated_titles_to_claude_thread(&mut thread, workspace_name.as_deref());
    Some(thread)
}

fn load_claude_thread_by_id(
    thread_id: &str,
    workspace_name: Option<String>,
) -> Option<ClaudeThread> {
    let thread_id = strip_local_thread_prefix(thread_id);
    let mut thread = claude_transcript_files()
        .into_iter()
        .filter(|path| path.file_stem().and_then(|value| value.to_str()) == Some(thread_id))
        .filter_map(|path| load_claude_thread_from_transcript_path(&path, workspace_name.clone()))
        .max_by_key(|thread| thread.updated_at)?;
    apply_generated_titles_to_claude_thread(&mut thread, workspace_name.as_deref());
    Some(thread)
}

fn strip_local_thread_prefix(thread_id: &str) -> &str {
    thread_id.strip_prefix("local:").unwrap_or(thread_id)
}

fn load_claude_threads(workspace_name: Option<String>) -> BTreeMap<String, ClaudeThread> {
    let mut threads = BTreeMap::new();
    let mut generated_titles = Vec::new();
    for path in claude_transcript_files() {
        if let Some(generated_title) = load_claude_generated_title_from_transcript_path(&path) {
            generated_titles.push(generated_title);
            continue;
        }
        if let Some(thread) = load_claude_thread_from_transcript_path(&path, workspace_name.clone())
        {
            threads
                .entry(thread.id.clone())
                .and_modify(|existing: &mut ClaudeThread| {
                    if thread.updated_at > existing.updated_at {
                        *existing = thread.clone();
                    }
                })
                .or_insert(thread);
        }
    }
    apply_generated_titles_to_claude_threads(
        &mut threads,
        &generated_titles,
        workspace_name.as_deref(),
    );
    threads
}

fn load_claude_thread_from_transcript_path(
    path: &Path,
    workspace_name: Option<String>,
) -> Option<ClaudeThread> {
    let transcript = std::fs::read_to_string(path).ok()?;
    let (fallback_created_at, fallback_updated_at) = transcript_fallback_times(path);
    if claude_generated_title_from_transcript(
        &transcript,
        path,
        fallback_created_at,
        fallback_updated_at,
    )
    .is_some()
    {
        return None;
    }
    let path_session_id = path.file_stem()?.to_string_lossy().to_string();

    let mut session_id = path_session_id.clone();
    let mut cwd = String::new();
    let mut model = DEFAULT_MODEL.to_string();
    let mut preview = String::new();
    let mut created_at = fallback_created_at;
    let mut updated_at = fallback_updated_at;
    let mut pending_user: Option<(Vec<Value>, i64)> = None;
    let mut turns = Vec::new();
    let mut latest_token_usage_info = None;
    let mut inline_title = None;

    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if let Some(entry_session_id) = value.get("sessionId").and_then(Value::as_str) {
            if !entry_session_id.trim().is_empty() {
                session_id = entry_session_id.to_string();
            }
        }
        if let Some(entry_cwd) = value.get("cwd").and_then(Value::as_str) {
            if !entry_cwd.trim().is_empty() {
                cwd = entry_cwd.to_string();
            }
        }
        if let Some(timestamp) = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_seconds)
        {
            created_at = created_at.min(timestamp);
            updated_at = updated_at.max(timestamp);
        }
        if let Some(info) = claude_token_usage_info_from_message(&value, &model) {
            latest_token_usage_info = Some(info);
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user")
                if !value
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                if let Some(input) = user_input_from_transcript_entry(&value) {
                    if preview.is_empty() {
                        preview = prompt_from_input(&input).chars().take(160).collect();
                    }
                    let started_at = value
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(parse_rfc3339_seconds)
                        .unwrap_or(updated_at);
                    pending_user = Some((input, started_at));
                }
            }
            Some("assistant")
                if !value
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                if let Some(assistant_model) = value
                    .get("message")
                    .and_then(|message| message.get("model"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty() && *value != "<synthetic>")
                {
                    model = assistant_model.to_string();
                }
                if let (Some((input, started_at)), Some(agent_text)) = (
                    pending_user.take(),
                    assistant_text_from_transcript_entry(&value),
                ) {
                    let completed_at = value
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(parse_rfc3339_seconds)
                        .unwrap_or(updated_at);
                    let failed = value.get("error").and_then(Value::as_str).is_some()
                        || value
                            .get("isApiErrorMessage")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                    let turn_suffix = value
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| turns.len().to_string());
                    turns.push(ClaudeTurn {
                        id: format!("turn-{turn_suffix}"),
                        input,
                        tool_items: Vec::new(),
                        agent_text,
                        status: if failed {
                            TurnStatus::Failed
                        } else {
                            TurnStatus::Completed
                        },
                        error: failed.then(|| {
                            value
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("Claude Code turn failed")
                                .to_string()
                        }),
                        started_at,
                        completed_at: Some(completed_at),
                        duration_ms: Some((completed_at - started_at).max(0) * 1000),
                    });
                }
            }
            Some("last-prompt") => {
                if let Some(last_prompt) = value.get("lastPrompt").and_then(Value::as_str) {
                    if !last_prompt.trim().is_empty() {
                        preview = last_prompt.chars().take(160).collect();
                    }
                }
            }
            Some("ai-title") => {
                let title_text = value
                    .get("aiTitle")
                    .or_else(|| value.get("title"))
                    .and_then(Value::as_str);
                if let Some(title_text) = title_text {
                    inline_title = sanitize_generated_thread_title(title_text);
                }
            }
            _ => {}
        }
    }

    if cwd.is_empty() {
        cwd = cwd_from_claude_project_dir(path).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });
    }
    if preview.is_empty() {
        preview = session_id.clone();
    }

    let name = persisted_claude_thread_name(&session_id)
        .or(inline_title)
        .or(workspace_name);

    Some(ClaudeThread {
        id: session_id.clone(),
        session_id: session_id.clone(),
        claude_session_id: session_id,
        path: Some(path.to_string_lossy().to_string()),
        preview,
        cwd,
        model,
        created_at,
        updated_at,
        archived: false,
        name,
        turns,
        latest_token_usage_info,
    })
}

fn load_claude_generated_title_from_transcript_path(path: &Path) -> Option<ClaudeGeneratedTitle> {
    let transcript = std::fs::read_to_string(path).ok()?;
    let (fallback_created_at, fallback_updated_at) = transcript_fallback_times(path);
    claude_generated_title_from_transcript(
        &transcript,
        path,
        fallback_created_at,
        fallback_updated_at,
    )
}

fn load_claude_generated_titles() -> Vec<ClaudeGeneratedTitle> {
    claude_transcript_files()
        .into_iter()
        .filter_map(|path| load_claude_generated_title_from_transcript_path(&path))
        .collect()
}

fn load_claude_inline_thread_titles() -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    for path in claude_transcript_files() {
        let Ok(transcript) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (fallback_created_at, fallback_updated_at) = transcript_fallback_times(&path);
        if claude_generated_title_from_transcript(
            &transcript,
            &path,
            fallback_created_at,
            fallback_updated_at,
        )
        .is_some()
        {
            continue;
        }
        let Some(title) = claude_inline_thread_title_from_transcript(&transcript) else {
            continue;
        };
        let session_id = claude_session_id_from_transcript(&transcript).or_else(|| {
            path.file_stem()
                .map(|value| value.to_string_lossy().to_string())
        });
        if let Some(session_id) = session_id {
            titles.insert(session_id, title);
        }
    }
    titles
}

fn claude_inline_thread_title_from_transcript(transcript: &str) -> Option<String> {
    let mut title = None;
    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if value.get("type").and_then(Value::as_str) != Some("ai-title") {
            continue;
        }
        let title_text = value
            .get("aiTitle")
            .or_else(|| value.get("title"))
            .and_then(Value::as_str);
        if let Some(title_text) = title_text {
            title = sanitize_generated_thread_title(title_text);
        }
    }
    title
}

fn claude_session_id_from_transcript(transcript: &str) -> Option<String> {
    transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|session_id| !session_id.is_empty())
                .map(str::to_string)
        })
        .last()
}

fn claude_generated_title_from_transcript(
    transcript: &str,
    path: &Path,
    fallback_created_at: i64,
    fallback_updated_at: i64,
) -> Option<ClaudeGeneratedTitle> {
    let mut cwd = String::new();
    let mut created_at = fallback_created_at;
    let mut updated_at = fallback_updated_at;
    let mut source_prompt = None;
    let mut assistant_title = None;
    let mut ai_title = None;

    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if let Some(entry_cwd) = value.get("cwd").and_then(Value::as_str) {
            if !entry_cwd.trim().is_empty() {
                cwd = entry_cwd.to_string();
            }
        }
        if let Some(timestamp) = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_seconds)
        {
            created_at = created_at.min(timestamp);
            updated_at = updated_at.max(timestamp);
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user")
                if !value
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && source_prompt.is_none() =>
            {
                if let Some(input) = user_input_from_transcript_entry(&value) {
                    let prompt = prompt_from_input(&input);
                    source_prompt = extract_claude_title_generation_source_prompt(&prompt);
                }
            }
            Some("assistant")
                if !value
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false) =>
            {
                if let Some(text) = assistant_text_from_transcript_entry(&value) {
                    assistant_title = sanitize_generated_thread_title(&text);
                }
            }
            Some("ai-title") => {
                let title_text = value
                    .get("aiTitle")
                    .or_else(|| value.get("title"))
                    .and_then(Value::as_str);
                if let Some(title_text) = title_text {
                    ai_title = sanitize_generated_thread_title(title_text);
                }
            }
            _ => {}
        }
    }

    let source_prompt = source_prompt?;
    if cwd.is_empty() {
        cwd = cwd_from_claude_project_dir(path).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });
    }

    Some(ClaudeGeneratedTitle {
        source_prompt,
        title: ai_title.or(assistant_title),
        cwd,
        created_at,
        updated_at,
    })
}

fn claude_generated_title_from_thread(thread: &ClaudeThread) -> Option<ClaudeGeneratedTitle> {
    let source_prompt =
        extract_claude_title_generation_source_prompt(&thread_initial_prompt(thread))?;
    let title = thread
        .turns
        .iter()
        .rev()
        .find_map(|turn| sanitize_generated_thread_title(&turn.agent_text));
    Some(ClaudeGeneratedTitle {
        source_prompt,
        title,
        cwd: thread.cwd.clone(),
        created_at: thread.created_at,
        updated_at: thread.updated_at,
    })
}

fn is_claude_title_generation_thread(thread: &ClaudeThread) -> bool {
    claude_generated_title_from_thread(thread).is_some()
}

fn is_claude_title_generation_prompt(prompt: &str) -> bool {
    extract_claude_title_generation_source_prompt(prompt).is_some()
}

fn extract_claude_title_generation_source_prompt(prompt: &str) -> Option<String> {
    let normalized = prompt.replace("\r\n", "\n");
    let trimmed = normalized.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if !trimmed.starts_with("You are a helpful assistant. You will be presented with a user prompt")
        || !(lower.contains("generate a concise ui title")
            || lower.contains("generate a clear, informative task title")
            || lower.contains("structured title field")
            || lower.contains("short title for a task"))
    {
        return None;
    }
    let (_, source_prompt) = trimmed.split_once("User prompt:")?;
    let source_prompt = source_prompt.trim();
    (!source_prompt.is_empty()).then(|| source_prompt.to_string())
}

fn sanitize_generated_thread_title(text: &str) -> Option<String> {
    let title = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
        .trim();
    if title.is_empty() {
        return None;
    }
    let mut chars = title.chars();
    let mut truncated = chars.by_ref().take(80).collect::<String>();
    if chars.next().is_some() {
        truncated.push_str("...");
    }
    Some(truncated)
}

fn apply_generated_titles_to_claude_threads(
    threads: &mut BTreeMap<String, ClaudeThread>,
    generated_titles: &[ClaudeGeneratedTitle],
    workspace_name: Option<&str>,
) {
    let mut generated_titles = generated_titles
        .iter()
        .filter(|generated_title| generated_title.title.is_some())
        .collect::<Vec<_>>();
    generated_titles.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

    for generated_title in generated_titles {
        apply_generated_title_to_claude_threads(threads, generated_title, workspace_name);
    }
}

fn apply_generated_title_to_claude_threads(
    threads: &mut BTreeMap<String, ClaudeThread>,
    generated_title: &ClaudeGeneratedTitle,
    workspace_name: Option<&str>,
) -> Option<(String, Option<String>)> {
    let title = generated_title.title.clone()?;
    let thread_id = generated_title_target_thread_id(threads, generated_title)?;
    let thread = threads.get_mut(&thread_id)?;
    if !should_replace_thread_name(thread.name.as_deref(), workspace_name) {
        return None;
    }
    if thread.name.as_deref() == Some(title.as_str()) {
        return None;
    }
    thread.name = Some(title.clone());
    persist_claude_thread_name(&thread_id, Some(&title));
    Some((thread_id, Some(title)))
}

fn apply_inline_claude_thread_title(
    thread: &mut ClaudeThread,
    inline_titles: &BTreeMap<String, String>,
    workspace_name: Option<&str>,
) -> Option<(String, Option<String>)> {
    let title = inline_titles
        .get(&thread.id)
        .or_else(|| inline_titles.get(&thread.session_id))
        .or_else(|| inline_titles.get(&thread.claude_session_id))?
        .clone();
    if !should_replace_thread_name(thread.name.as_deref(), workspace_name) {
        return None;
    }
    if thread.name.as_deref() == Some(title.as_str()) {
        return None;
    }
    thread.name = Some(title.clone());
    persist_claude_thread_name(&thread.id, Some(&title));
    Some((thread.id.clone(), Some(title)))
}

fn apply_generated_titles_to_single_claude_thread(
    thread: &mut ClaudeThread,
    generated_titles: &[ClaudeGeneratedTitle],
    workspace_name: Option<&str>,
) {
    let mut threads = BTreeMap::new();
    threads.insert(thread.id.clone(), thread.clone());
    apply_generated_titles_to_claude_threads(&mut threads, generated_titles, workspace_name);
    if let Some(updated) = threads.remove(&thread.id) {
        *thread = updated;
    }
}

fn apply_generated_titles_to_claude_thread(
    thread: &mut ClaudeThread,
    workspace_name: Option<&str>,
) {
    let generated_titles = load_claude_generated_titles();
    apply_generated_titles_to_single_claude_thread(thread, &generated_titles, workspace_name);
}

fn generated_title_target_thread_id(
    threads: &BTreeMap<String, ClaudeThread>,
    generated_title: &ClaudeGeneratedTitle,
) -> Option<String> {
    threads
        .values()
        .filter(|thread| generated_title_matches_thread(thread, generated_title))
        .min_by_key(|thread| {
            thread
                .created_at
                .abs_diff(generated_title.created_at)
                .min(thread.updated_at.abs_diff(generated_title.created_at))
        })
        .filter(|thread| {
            thread
                .created_at
                .abs_diff(generated_title.created_at)
                .min(thread.updated_at.abs_diff(generated_title.created_at))
                <= CLAUDE_TITLE_MATCH_MAX_DELTA_SECONDS
        })
        .map(|thread| thread.id.clone())
}

fn generated_title_matches_thread(
    thread: &ClaudeThread,
    generated_title: &ClaudeGeneratedTitle,
) -> bool {
    if is_claude_title_generation_thread(thread) {
        return false;
    }
    if thread.cwd != generated_title.cwd {
        return false;
    }
    let source_prompt = compact_cli_text(&generated_title.source_prompt);
    if source_prompt.is_empty() {
        return false;
    }
    let thread_prompt = compact_cli_text(&thread_initial_prompt(thread));
    if thread_prompt.is_empty() {
        return false;
    }
    thread_prompt == source_prompt
        || thread_prompt.contains(&source_prompt)
        || source_prompt.contains(&thread_prompt)
}

fn thread_initial_prompt(thread: &ClaudeThread) -> String {
    thread
        .turns
        .first()
        .map(|turn| prompt_from_input(&turn.input))
        .unwrap_or_else(|| thread.preview.clone())
}

fn should_replace_thread_name(current: Option<&str>, workspace_name: Option<&str>) -> bool {
    let current = current.map(str::trim).filter(|value| !value.is_empty());
    match current {
        None => true,
        Some(current) => {
            workspace_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                == Some(current)
        }
    }
}

fn transcript_fallback_times(path: &Path) -> (i64, i64) {
    let metadata = std::fs::metadata(path).ok();
    let fallback_updated_at = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_to_unix_seconds)
        .unwrap_or_else(now_seconds);
    let fallback_created_at = metadata
        .as_ref()
        .and_then(|metadata| metadata.created().ok())
        .and_then(system_time_to_unix_seconds)
        .unwrap_or(fallback_updated_at);
    (fallback_created_at, fallback_updated_at)
}

fn persisted_claude_thread_name(thread_id: &str) -> Option<String> {
    let thread_id = strip_local_thread_prefix(thread_id);
    load_claude_thread_names().get(thread_id).cloned()
}

fn persist_claude_thread_name(thread_id: &str, name: Option<&str>) {
    let thread_id = strip_local_thread_prefix(thread_id).trim();
    if thread_id.is_empty() {
        return;
    }
    let mut names = load_claude_thread_names();
    if let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) {
        names.insert(thread_id.to_string(), name.to_string());
    } else {
        names.remove(thread_id);
    }
    let Some(path) = claude_thread_names_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(&names) {
        let _ = std::fs::write(path, content);
    }
}

fn load_claude_thread_names() -> BTreeMap<String, String> {
    let Some(path) = claude_thread_names_path() else {
        return BTreeMap::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&content) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(key, value)| {
            let name = value.as_str()?.trim();
            (!key.trim().is_empty() && !name.is_empty()).then(|| (key, name.to_string()))
        })
        .collect()
}

fn claude_thread_names_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_THREAD_NAMES_FILE),
    )
}

fn user_input_from_transcript_entry(value: &Value) -> Option<Vec<Value>> {
    let content = value.get("message")?.get("content")?;
    let mut items = Vec::new();
    collect_user_input_items(content, &mut items);
    (!items.is_empty()).then_some(items)
}

fn collect_user_input_items(value: &Value, items: &mut Vec<Value>) {
    match value {
        Value::String(text) => push_text_user_input(items, text),
        Value::Array(parts) => {
            for part in parts {
                collect_user_input_items(part, items);
            }
        }
        Value::Object(map) => {
            if matches!(map.get("type").and_then(Value::as_str), Some("text")) {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    push_text_user_input(items, text);
                }
            }
        }
        _ => {}
    }
}

fn push_text_user_input(items: &mut Vec<Value>, text: &str) {
    if text.trim().is_empty() || is_synthetic_user_message(text) {
        return;
    }
    items.push(json!({
        "type": "text",
        "text": text,
        "text_elements": [],
    }));
}

fn is_synthetic_user_message(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<local-command-stderr>")
}

fn claude_transcript_files() -> Vec<PathBuf> {
    let Some(projects_dir) = claude_projects_dir() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Ok(project_dirs) = std::fs::read_dir(projects_dir) {
        for project_dir in project_dirs.flatten() {
            let project_path = project_dir.path();
            if !project_path.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(project_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                        paths.push(path);
                    }
                }
            }
        }
    }
    paths
}

fn claude_projects_dir() -> Option<PathBuf> {
    Some(user_home_dir()?.join(".claude").join("projects"))
}

fn user_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_path_without_home_expansion("USERPROFILE")
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                let combined = format!("{}{}", drive.trim(), path.trim());
                if combined.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(combined))
                }
            })
            .or_else(|| env_path_without_home_expansion("HOME"))
    } else {
        env_path_without_home_expansion("HOME")
    }
}

fn env_path_without_home_expansion(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn is_claude_transcript_path(path: &Path) -> bool {
    let Some(projects_dir) = claude_projects_dir() else {
        return false;
    };
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    let Ok(projects_dir) = std::fs::canonicalize(projects_dir) else {
        return false;
    };
    path.starts_with(projects_dir)
        && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}

fn cwd_from_claude_project_dir(path: &Path) -> Option<String> {
    path.parent()?
        .file_name()?
        .to_str()
        .map(|name| name.replace('-', "/"))
        .filter(|cwd| cwd.starts_with('/'))
}

fn system_time_to_unix_seconds(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

fn parse_rfc3339_seconds(value: &str) -> Option<i64> {
    let value = value.trim();
    let date_time = value.get(0..19)?;
    let year = date_time.get(0..4)?.parse::<i32>().ok()?;
    let month = date_time.get(5..7)?.parse::<u32>().ok()?;
    let day = date_time.get(8..10)?.parse::<u32>().ok()?;
    let hour = date_time.get(11..13)?.parse::<i64>().ok()?;
    let minute = date_time.get(14..16)?.parse::<i64>().ok()?;
    let second = date_time.get(17..19)?.parse::<i64>().ok()?;
    if date_time.as_bytes().get(4) != Some(&b'-')
        || date_time.as_bytes().get(7) != Some(&b'-')
        || date_time.as_bytes().get(10) != Some(&b'T')
        || date_time.as_bytes().get(13) != Some(&b':')
        || date_time.as_bytes().get(16) != Some(&b':')
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146_097 + doe - 719_468)
}

fn run_turn_worker<W>(work: TurnWork, state: SharedState, output: SharedOutput<W>)
where
    W: Write + Send + 'static,
{
    claude_code_log_event("turn_worker_start", turn_work_log_fields(&work));
    let result = run_claude_code_turn(&work, Arc::clone(&state), Arc::clone(&output));
    claude_code_log_event(
        "turn_worker_result",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "hasError": result.error.is_some(),
            "textBytes": result.text.len(),
            "toolItemCount": result.tool_items.len(),
            "agentItemStreamed": result.agent_item_streamed,
            "tokenUsageSeen": result.latest_token_usage_info.is_some(),
        }),
    );
    let generated_title_hint = is_claude_title_generation_prompt(&work.prompt)
        .then(|| latest_claude_transcript_generated_title(&work))
        .flatten();
    let notifications = match lock_state(&state).ok().and_then(|mut state| {
        state.finish_turn(&work.thread_id, &work.turn_id, result, generated_title_hint)
    }) {
        Some(notifications) => notifications,
        None => return,
    };
    if let Some(item_completed) = notifications.item_completed {
        let _ = write_notification(&output, item_completed);
    }
    for notification in notifications.extra_notifications {
        let _ = write_notification(&output, notification);
    }
    if let Some(turn_completed) = notifications.turn_completed {
        let _ = write_notification(&output, turn_completed);
    }
    if let Some(thread_stream_state) = notifications.thread_stream_state {
        let _ = write_notification(&output, thread_stream_state);
    }
    claude_code_log_event(
        "turn_worker_notifications_sent",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
        }),
    );
}

fn run_claude_code_turn<W>(
    work: &TurnWork,
    state: SharedState,
    output: SharedOutput<W>,
) -> ClaudeRunResult
where
    W: Write,
{
    let started = Instant::now();
    claude_code_log_event(
        "claude_command_prepare",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "command": log_text_preview(&claude_command_display(work), 4000),
            "mcpConfig": if is_claude_title_generation_prompt(&work.prompt) {
                json!({ "injected": false, "reason": "title_generation" })
            } else {
                claude_code_mcp_config_log_summary(work)
            },
        }),
    );
    let mut command = claude_command(work);
    command.current_dir(&work.cwd);
    run_claude_code_turn_stream_json(command, work, state, output, started)
}

fn emit_current_thread_stream_state<W>(
    state: &SharedState,
    output: &SharedOutput<W>,
    thread_id: &str,
) where
    W: Write,
{
    let notification = lock_state(state).ok().and_then(|state| {
        state
            .threads
            .get(thread_id)
            .map(claude_thread_stream_state_changed_notification)
    });
    if let Some(notification) = notification {
        let _ = write_notification(output, notification);
        claude_code_log_event(
            "thread_stream_state_emit",
            json!({
                "threadId": thread_id,
            }),
        );
    }
}

#[derive(Debug)]
enum ClaudeChildEvent {
    StdoutLine(String),
    StderrLine(String),
    StdoutDone,
    StderrDone,
    StdoutError(String),
    StderrError(String),
}

fn run_claude_code_turn_stream_json<W>(
    mut command: Command,
    work: &TurnWork,
    state: SharedState,
    output: SharedOutput<W>,
    started: Instant,
) -> ClaudeRunResult
where
    W: Write,
{
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            claude_code_log_event(
                "claude_spawn_failed",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "error": err.to_string(),
                }),
            );
            return ClaudeRunResult {
                text: String::new(),
                error: Some(format!("failed to launch Claude Code: {}", err)),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    claude_code_log_event(
        "claude_spawned",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "pid": child.id(),
        }),
    );
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .insert((work.thread_id.clone(), work.turn_id.clone()), child.id());
    }
    let emit_thread_stream_state =
        extract_claude_title_generation_source_prompt(&work.prompt).is_none();
    if emit_thread_stream_state {
        emit_current_thread_stream_state(&state, &output, &work.thread_id);
    }

    let mut child_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            claude_code_log_event(
                "claude_stdio_missing",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "stream": "stdin",
                    "pid": child.id(),
                }),
            );
            terminate_process_group(child.id());
            let _ = child.wait();
            remove_active_process(&state, work);
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to open Claude Code stdin".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            claude_code_log_event(
                "claude_stdio_missing",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "stream": "stdout",
                    "pid": child.id(),
                }),
            );
            terminate_process_group(child.id());
            let _ = child.wait();
            remove_active_process(&state, work);
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to capture Claude Code stdout".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let (event_tx, event_rx) = mpsc::channel();
    let stdout_handle = spawn_claude_child_line_reader(stdout, event_tx.clone(), true);
    let mut stderr_handle = child
        .stderr
        .take()
        .map(|stderr| spawn_claude_child_line_reader(stderr, event_tx.clone(), false));
    drop(event_tx);

    let stdin_payload = claude_stream_json_input(work);
    if let Err(err) = child_stdin
        .write_all(stdin_payload.as_bytes())
        .and_then(|_| child_stdin.flush())
    {
        claude_code_log_event(
            "claude_stdin_write_failed",
            json!({
                "threadId": &work.thread_id,
                "turnId": &work.turn_id,
                "pid": child.id(),
                "error": err.to_string(),
            }),
        );
        terminate_process_group(child.id());
        let _ = child.wait();
        let _ = stdout_handle.join();
        if let Some(handle) = stderr_handle.take() {
            let _ = handle.join();
        }
        remove_active_process(&state, work);
        return ClaudeRunResult {
            text: String::new(),
            error: Some(format!(
                "failed to write prompt to Claude Code stdin: {}",
                err
            )),
            duration_ms: elapsed_millis(started),
            tool_items: Vec::new(),
            agent_item_streamed: false,
            latest_token_usage_info: None,
        };
    }
    claude_code_log_event(
        "claude_stdin_prompt_sent",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "pid": child.id(),
            "bytes": stdin_payload.len(),
        }),
    );

    let mut stream = ClaudeStreamState::default();
    let mut command_output = String::new();
    let mut stderr_output = String::new();
    let mut stdout_done = false;
    let mut stderr_done = stderr_handle.is_none();
    let mut child_status = None;
    let mut last_child_event = Instant::now();
    let idle_timeout = claude_turn_idle_timeout();
    let mut result_seen_at: Option<Instant> = None;
    let mut last_thread_stream_state_heartbeat = Instant::now();

    while child_status.is_none() || !stdout_done || !stderr_done {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                last_child_event = Instant::now();
                match event {
                    ClaudeChildEvent::StdoutLine(line) => {
                        if let Err(err) = handle_claude_stdout_line(
                            &line,
                            work,
                            &state,
                            &output,
                            &mut child_stdin,
                            &mut stream,
                            &mut command_output,
                        ) {
                            command_output.push_str(&format!("[claude-control]\n{}\n", err.trim()));
                            terminate_process_group(child.id());
                            child_status = Some(child.wait());
                        }
                    }
                    ClaudeChildEvent::StderrLine(line) => {
                        stderr_output.push_str(&line);
                        stderr_output.push('\n');
                        claude_code_log_event(
                            "claude_stderr_line",
                            json!({
                                "threadId": &work.thread_id,
                                "turnId": &work.turn_id,
                                "linePreview": log_text_preview(&line, 500),
                            }),
                        );
                    }
                    ClaudeChildEvent::StdoutDone => {
                        claude_code_log_event(
                            "claude_stdout_done",
                            json!({
                                "threadId": &work.thread_id,
                                "turnId": &work.turn_id,
                            }),
                        );
                        stdout_done = true;
                    }
                    ClaudeChildEvent::StderrDone => {
                        claude_code_log_event(
                            "claude_stderr_done",
                            json!({
                                "threadId": &work.thread_id,
                                "turnId": &work.turn_id,
                            }),
                        );
                        stderr_done = true;
                    }
                    ClaudeChildEvent::StdoutError(err) => {
                        claude_code_log_event(
                            "claude_stdout_read_error",
                            json!({
                                "threadId": &work.thread_id,
                                "turnId": &work.turn_id,
                                "error": &err,
                            }),
                        );
                        stdout_done = true;
                        command_output.push_str(&format!(
                            "[stdout]\nfailed to read Claude Code stdout: {}\n",
                            err
                        ));
                    }
                    ClaudeChildEvent::StderrError(err) => {
                        claude_code_log_event(
                            "claude_stderr_read_error",
                            json!({
                                "threadId": &work.thread_id,
                                "turnId": &work.turn_id,
                                "error": &err,
                            }),
                        );
                        stderr_done = true;
                        command_output.push_str(&format!(
                            "[stderr]\nfailed to read Claude Code stderr: {}\n",
                            err
                        ));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stdout_done = true;
                stderr_done = true;
            }
        }

        if result_seen_at.is_none() && claude_stream_result_seen(&stream) {
            result_seen_at = Some(Instant::now());
            claude_code_log_event(
                "claude_result_seen",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "stream": stream_log_summary(&stream),
                }),
            );
        }

        if emit_thread_stream_state
            && child_status.is_none()
            && last_thread_stream_state_heartbeat.elapsed()
                >= Duration::from_millis(CLAUDE_THREAD_STREAM_STATE_HEARTBEAT_MS)
        {
            emit_current_thread_stream_state(&state, &output, &work.thread_id);
            last_thread_stream_state_heartbeat = Instant::now();
        }

        if child_status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    claude_code_log_event(
                        "claude_child_exited",
                        json!({
                            "threadId": &work.thread_id,
                            "turnId": &work.turn_id,
                            "pid": child.id(),
                            "success": status.success(),
                            "status": status.to_string(),
                        }),
                    );
                    child_status = Some(Ok(status));
                }
                Ok(None) => {}
                Err(err) => {
                    claude_code_log_event(
                        "claude_child_wait_error",
                        json!({
                            "threadId": &work.thread_id,
                            "turnId": &work.turn_id,
                            "pid": child.id(),
                            "error": err.to_string(),
                        }),
                    );
                    child_status = Some(Err(err));
                }
            }
        }

        if child_status.is_none() && turn_was_interrupted(&state, work) {
            claude_code_log_event(
                "claude_turn_interrupted",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "pid": child.id(),
                    "stream": stream_log_summary(&stream),
                }),
            );
            terminate_process_group(child.id());
            child_status = Some(child.wait());
        }

        if child_status.is_none()
            && result_seen_at.is_some_and(|seen_at| {
                seen_at.elapsed() >= Duration::from_millis(CLAUDE_RESULT_EXIT_GRACE_MS)
            })
        {
            remove_active_process(&state, work);
            if !stderr_output.trim().is_empty() {
                command_output.push_str("[stderr]\n");
                command_output.push_str(stderr_output.trim());
                command_output.push('\n');
            }
            if stream.saw_tool_call {
                flush_pending_agent_text_as_reasoning(&output, work, &mut stream);
            } else {
                flush_pending_agent_text_as_agent(&output, work, &mut stream);
            }
            emit_reasoning_completed_if_started(&output, work, &stream);

            let success = stream.result_error.is_none();
            finalize_open_tool_calls(&output, work, &mut stream, success);
            let agent_item_streamed = !stream.emitted_text.is_empty();
            let final_text = if stream.emitted_text.is_empty() {
                stream
                    .result_text
                    .clone()
                    .or_else(|| latest_claude_transcript_assistant_text(work))
                    .unwrap_or_default()
            } else {
                stream.emitted_text.clone()
            };
            let duration_ms = elapsed_millis(started);
            let error = stream
                .result_error
                .take()
                .map(|error| non_empty_join(&[error, command_output.clone()], "\n"));
            claude_code_log_event(
                "claude_turn_finish_after_result",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "pid": child.id(),
                    "durationMs": duration_ms,
                    "hasError": error.is_some(),
                    "stream": stream_log_summary(&stream),
                }),
            );
            detach_completed_claude_child(
                child,
                child_stdin,
                stdout_handle,
                stderr_handle.take(),
                event_rx,
                work.thread_id.clone(),
                work.turn_id.clone(),
            );
            let latest_token_usage_info = claude_run_latest_token_usage_info(&stream, work);
            return ClaudeRunResult {
                text: final_text,
                error,
                duration_ms,
                tool_items: stream.completed_tool_items,
                agent_item_streamed,
                latest_token_usage_info,
            };
        }

        if child_status.is_none() && last_child_event.elapsed() >= idle_timeout {
            claude_code_log_event(
                "claude_idle_timeout",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "pid": child.id(),
                    "idleTimeoutMs": idle_timeout.as_millis(),
                    "stdoutDone": stdout_done,
                    "stderrDone": stderr_done,
                    "stream": stream_log_summary(&stream),
                    "stderrPreview": log_text_preview(stderr_output.trim(), 1000),
                    "commandOutputPreview": log_text_preview(&command_output, 1000),
                }),
            );
            terminate_process_group(child.id());
            let _ = child.wait();
            let _ = stdout_handle.join();
            if let Some(handle) = stderr_handle.take() {
                let _ = handle.join();
            }
            remove_active_process(&state, work);
            if !stderr_output.trim().is_empty() {
                command_output.push_str("[stderr]\n");
                command_output.push_str(stderr_output.trim());
                command_output.push('\n');
            }
            finalize_open_tool_calls(&output, work, &mut stream, false);
            let agent_item_streamed = !stream.emitted_text.is_empty();
            let latest_token_usage_info = claude_run_latest_token_usage_info(&stream, work);
            return ClaudeRunResult {
                text: stream.emitted_text,
                error: Some(non_empty_join(
                    &[
                        format!(
                            "Claude Code produced no output for {}ms",
                            idle_timeout.as_millis()
                        ),
                        command_output,
                    ],
                    "\n",
                )),
                duration_ms: elapsed_millis(started),
                tool_items: stream.completed_tool_items,
                agent_item_streamed,
                latest_token_usage_info,
            };
        }
    }

    let status = child_status.unwrap_or_else(|| child.wait());
    let _ = stdout_handle.join();
    if let Some(handle) = stderr_handle.take() {
        let _ = handle.join();
    }
    remove_active_process(&state, work);
    if !stderr_output.trim().is_empty() {
        command_output.push_str("[stderr]\n");
        command_output.push_str(stderr_output.trim());
        command_output.push('\n');
    }
    if stream.saw_tool_call {
        flush_pending_agent_text_as_reasoning(&output, work, &mut stream);
    } else {
        flush_pending_agent_text_as_agent(&output, work, &mut stream);
    }
    emit_reasoning_completed_if_started(&output, work, &stream);

    let duration_ms = elapsed_millis(started);
    match status {
        Ok(status) => {
            let success = status.success() && stream.result_error.is_none();
            finalize_open_tool_calls(&output, work, &mut stream, success);
            let agent_item_streamed = !stream.emitted_text.is_empty();
            let latest_token_usage_info = claude_run_latest_token_usage_info(&stream, work);
            let final_text = if stream.emitted_text.is_empty() {
                stream
                    .result_text
                    .clone()
                    .or_else(|| latest_claude_transcript_assistant_text(work))
                    .unwrap_or_default()
            } else {
                stream.emitted_text.clone()
            };
            claude_code_log_event(
                "claude_turn_finish_after_exit",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "durationMs": duration_ms,
                    "processSuccess": status.success(),
                    "success": success,
                    "status": status.to_string(),
                    "stream": stream_log_summary(&stream),
                    "stderrPreview": log_text_preview(stderr_output.trim(), 1000),
                    "commandOutputPreview": log_text_preview(&command_output, 1000),
                }),
            );
            if success {
                ClaudeRunResult {
                    text: final_text,
                    error: None,
                    duration_ms,
                    tool_items: stream.completed_tool_items,
                    agent_item_streamed,
                    latest_token_usage_info,
                }
            } else {
                ClaudeRunResult {
                    text: final_text,
                    error: Some(non_empty_join(
                        &[
                            (!status.success())
                                .then(|| format!("Claude Code exited with status {}", status))
                                .unwrap_or_default(),
                            stream.result_error.unwrap_or_default(),
                            command_output,
                        ],
                        "\n",
                    )),
                    duration_ms,
                    tool_items: stream.completed_tool_items,
                    agent_item_streamed,
                    latest_token_usage_info,
                }
            }
        }
        Err(err) => {
            finalize_open_tool_calls(&output, work, &mut stream, false);
            let agent_item_streamed = !stream.emitted_text.is_empty();
            let latest_token_usage_info = claude_run_latest_token_usage_info(&stream, work);
            claude_code_log_event(
                "claude_turn_finish_wait_error",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "durationMs": duration_ms,
                    "error": err.to_string(),
                    "stream": stream_log_summary(&stream),
                }),
            );
            ClaudeRunResult {
                text: stream.emitted_text,
                error: Some(format!("failed to wait for Claude Code: {}", err)),
                duration_ms,
                tool_items: stream.completed_tool_items,
                agent_item_streamed,
                latest_token_usage_info,
            }
        }
    }
}

fn spawn_claude_child_line_reader<R>(
    stream: R,
    sender: mpsc::Sender<ClaudeChildEvent>,
    stdout: bool,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let event = match line {
                Ok(line) if stdout => ClaudeChildEvent::StdoutLine(line),
                Ok(line) => ClaudeChildEvent::StderrLine(line),
                Err(err) if stdout => ClaudeChildEvent::StdoutError(err.to_string()),
                Err(err) => ClaudeChildEvent::StderrError(err.to_string()),
            };
            if sender.send(event).is_err() {
                return;
            }
        }
        let _ = sender.send(if stdout {
            ClaudeChildEvent::StdoutDone
        } else {
            ClaudeChildEvent::StderrDone
        });
    })
}

fn claude_stream_result_seen(stream: &ClaudeStreamState) -> bool {
    stream.result_text.is_some() || stream.result_error.is_some()
}

fn claude_run_latest_token_usage_info(
    stream: &ClaudeStreamState,
    work: &TurnWork,
) -> Option<Value> {
    stream
        .latest_token_usage_info
        .clone()
        .or_else(|| latest_claude_transcript_token_usage_info(work))
}

#[cfg(unix)]
fn detach_completed_claude_child(
    mut child: std::process::Child,
    child_stdin: std::process::ChildStdin,
    stdout_handle: thread::JoinHandle<()>,
    stderr_handle: Option<thread::JoinHandle<()>>,
    event_rx: mpsc::Receiver<ClaudeChildEvent>,
    thread_id: String,
    turn_id: String,
) {
    thread::spawn(move || {
        drop(child_stdin);
        let pid = child.id();
        claude_code_log_event(
            "claude_completed_child_terminate",
            json!({
                "threadId": &thread_id,
                "turnId": &turn_id,
                "pid": pid,
            }),
        );
        terminate_process_group(pid);
        loop {
            match event_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(_) => break,
            }
        }
        let _ = child.wait();
        let _ = stdout_handle.join();
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }
    });
}

#[cfg(not(unix))]
fn detach_completed_claude_child(
    mut child: std::process::Child,
    child_stdin: std::process::ChildStdin,
    stdout_handle: thread::JoinHandle<()>,
    stderr_handle: Option<thread::JoinHandle<()>>,
    event_rx: mpsc::Receiver<ClaudeChildEvent>,
    thread_id: String,
    turn_id: String,
) {
    thread::spawn(move || {
        drop(child_stdin);
        let pid = child.id();
        claude_code_log_event(
            "claude_completed_child_terminate",
            json!({
                "threadId": &thread_id,
                "turnId": &turn_id,
                "pid": pid,
            }),
        );
        terminate_process_group(pid);
        while child.try_wait().ok().flatten().is_none() {
            match event_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = child.wait();
        let _ = stdout_handle.join();
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }
    });
}

fn handle_claude_stdout_line<W, S>(
    line: &str,
    work: &TurnWork,
    state: &SharedState,
    output: &SharedOutput<W>,
    child_stdin: &mut S,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) -> Result<(), String>
where
    W: Write,
    S: Write,
{
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let message = match serde_json::from_str::<Value>(trimmed) {
        Ok(message) => message,
        Err(_) => {
            command_output.push_str(line);
            command_output.push('\n');
            claude_code_log_event(
                "claude_stdout_non_json",
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "linePreview": log_text_preview(line, 500),
                }),
            );
            return Ok(());
        }
    };
    claude_code_log_event(
        "claude_stdout_message",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "message": claude_message_log_summary(&message),
        }),
    );
    if is_claude_elicitation_control_request(&message) {
        let request_id = claude_control_request_id(&message);
        let control_response = match request_codex_app_elicitation(&message, work, state, output) {
            Ok(response) => response,
            Err(err) => {
                command_output.push_str(&format!("[elicitation]\n{}\n", err.trim()));
                claude_control_response_for_elicitation(
                    &request_id,
                    &json!({
                        "action": "cancel",
                        "content": Value::Null,
                        "_meta": { "error": err },
                    }),
                )
            }
        };
        claude_code_log_event(
            "elicitation_control_response_send",
            json!({
                "threadId": &work.thread_id,
                "turnId": &work.turn_id,
                "requestId": &request_id,
                "action": control_response
                    .pointer("/response/response/action")
                    .and_then(Value::as_str),
                "responseShape": log_request_params_summary(&control_response),
            }),
        );
        return write_claude_child_json_line(child_stdin, &control_response);
    }
    if is_claude_permission_control_request(&message) {
        let request_id = claude_control_request_id(&message);
        let control_response = match request_codex_app_permissions(&message, work, state, output) {
            Ok(response) => response,
            Err(err) => {
                command_output.push_str(&format!("[permission]\n{}\n", err.trim()));
                claude_control_response_denied(&request_id, &err)
            }
        };
        claude_code_log_event(
            "permission_control_response_send",
            json!({
                "threadId": &work.thread_id,
                "turnId": &work.turn_id,
                "requestId": &request_id,
                "behavior": control_response
                    .pointer("/response/response/behavior")
                    .and_then(Value::as_str),
                "hasUpdatedInput": control_response
                    .pointer("/response/response/updatedInput")
                    .is_some(),
                "toolUseID": control_response
                    .pointer("/response/response/toolUseID")
                    .and_then(Value::as_str),
                "responseShape": log_request_params_summary(&control_response),
            }),
        );
        return write_claude_child_json_line(child_stdin, &control_response);
    }
    handle_claude_stream_message(&message, work, output, stream, command_output);
    Ok(())
}

fn request_codex_app_permissions<W: Write>(
    message: &Value,
    work: &TurnWork,
    state: &SharedState,
    output: &SharedOutput<W>,
) -> Result<Value, String> {
    let request_id = claude_control_request_id(message);
    let _ = take_app_response(state, &request_id);
    claude_code_log_event(
        "permission_request_emit",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "requestId": &request_id,
            "toolName": claude_permission_tool_name(message),
            "serverName": claude_permission_server_name(message),
        }),
    );
    write_json_line(
        output,
        &json!({
            "id": request_id,
            "method": "item/permissions/requestApproval",
            "params": codex_app_permission_request_params(work, &request_id, message),
        }),
    )?;
    let approval = wait_for_codex_app_response(state, work, &request_id)?;
    claude_code_log_event(
        "permission_response_received",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "requestId": &request_id,
            "allows": codex_permission_response_allows(&approval),
            "response": log_request_params_summary(&approval),
        }),
    );
    Ok(claude_control_response_for_permission(
        message,
        &request_id,
        &approval,
    ))
}

fn wait_for_codex_app_response(
    state: &SharedState,
    work: &TurnWork,
    request_id: &str,
) -> Result<Value, String> {
    wait_for_codex_app_response_with_events(state, work, request_id, "permission")
}

fn wait_for_codex_app_response_with_events(
    state: &SharedState,
    work: &TurnWork,
    request_id: &str,
    request_kind: &str,
) -> Result<Value, String> {
    let started = Instant::now();
    let timeout = claude_permission_approval_timeout();
    while started.elapsed() < timeout {
        if let Some(response) = take_app_response(state, request_id) {
            return Ok(response);
        }
        if turn_was_interrupted(state, work) {
            let event = format!("{request_kind}_response_interrupted");
            claude_code_log_event(
                &event,
                json!({
                    "threadId": &work.thread_id,
                    "turnId": &work.turn_id,
                    "requestId": request_id,
                }),
            );
            return Err(format!(
                "{} request {} was interrupted before response",
                request_kind, request_id
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
    let event = format!("{request_kind}_response_timeout");
    claude_code_log_event(
        &event,
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "requestId": request_id,
            "timeoutMs": timeout.as_millis(),
        }),
    );
    Err(format!(
        "timed out waiting for Codex App {} response: {}",
        request_kind, request_id
    ))
}

fn request_codex_app_elicitation<W: Write>(
    message: &Value,
    work: &TurnWork,
    state: &SharedState,
    output: &SharedOutput<W>,
) -> Result<Value, String> {
    let request_id = claude_control_request_id(message);
    let _ = take_app_response(state, &request_id);
    claude_code_log_event(
        "elicitation_request_emit",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "requestId": &request_id,
            "serverName": claude_permission_server_name(message),
            "mode": claude_elicitation_mode(message),
        }),
    );
    write_json_line(
        output,
        &json!({
            "id": request_id,
            "method": "mcpServer/elicitation/request",
            "params": codex_app_elicitation_request_params(work, &request_id, message),
        }),
    )?;
    let response =
        wait_for_codex_app_response_with_events(state, work, &request_id, "elicitation")?;
    claude_code_log_event(
        "elicitation_response_received",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "requestId": &request_id,
            "action": normalized_elicitation_response(&response)
                .get("action")
                .and_then(Value::as_str),
            "response": log_request_params_summary(&response),
        }),
    );
    Ok(claude_control_response_for_elicitation(
        &request_id,
        &response,
    ))
}

fn codex_app_elicitation_request_params(
    work: &TurnWork,
    request_id: &str,
    message: &Value,
) -> Value {
    let mode = claude_elicitation_mode(message);
    let mut params = serde_json::Map::new();
    params.insert("threadId".to_string(), json!(&work.thread_id));
    params.insert("turnId".to_string(), json!(&work.turn_id));
    params.insert("itemId".to_string(), json!(request_id));
    params.insert("mode".to_string(), json!(mode));
    params.insert(
        "message".to_string(),
        json!(claude_elicitation_message(message)),
    );
    if let Some(server_name) = claude_permission_server_name(message) {
        params.insert("serverName".to_string(), json!(server_name));
    }
    if let Some(elicitation_id) = claude_elicitation_id(message) {
        params.insert("elicitationId".to_string(), json!(elicitation_id));
    }
    if let Some(url) = claude_elicitation_url(message) {
        params.insert("url".to_string(), json!(url));
    }
    params.insert(
        "requestedSchema".to_string(),
        claude_elicitation_requested_schema(message),
    );
    if let Some(meta) = claude_elicitation_meta(message) {
        params.insert("_meta".to_string(), meta.clone());
    }
    Value::Object(params)
}

fn codex_app_permission_request_params(
    work: &TurnWork,
    request_id: &str,
    message: &Value,
) -> Value {
    let tool_name = claude_permission_tool_name(message).unwrap_or_else(|| "tool".to_string());
    let server_name = claude_permission_server_name(message);
    let tool_label = server_name
        .as_deref()
        .map(|server| format!("{server}/{tool_name}"))
        .unwrap_or(tool_name);
    json!({
        "threadId": &work.thread_id,
        "turnId": &work.turn_id,
        "itemId": claude_permission_item_id(message).unwrap_or_else(|| request_id.to_string()),
        "cwd": &work.cwd,
        "reason": format!("Claude Code wants to use {tool_label}."),
        "permissions": codex_app_permissions_for_claude_request(work, message),
    })
}

fn codex_app_permissions_for_claude_request(work: &TurnWork, message: &Value) -> Value {
    let tool_name = claude_permission_tool_name(message)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut permissions = serde_json::Map::new();
    permissions.insert("network".to_string(), json!({ "enabled": true }));
    let file_related = tool_name.is_empty()
        || tool_name.contains("bash")
        || tool_name.contains("edit")
        || tool_name.contains("file")
        || tool_name.contains("read")
        || tool_name.contains("write")
        || tool_name.contains("grep")
        || tool_name.contains("glob")
        || tool_name.contains("ls")
        || tool_name.contains("notebook");
    if file_related {
        permissions.insert(
            "fileSystem".to_string(),
            json!({
                "read": [&work.cwd],
                "write": [&work.cwd],
            }),
        );
    }
    Value::Object(permissions)
}

fn is_claude_permission_control_request(message: &Value) -> bool {
    if message.get("type").and_then(Value::as_str) != Some("control_request") {
        return false;
    }
    let subtype = claude_control_request_subtype(message)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if subtype == "initialize" {
        return false;
    }
    subtype.contains("permission")
        || subtype.contains("tool")
        || subtype.contains("can_use")
        || claude_permission_tool_name(message).is_some()
}

fn is_claude_elicitation_control_request(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("control_request")
        && claude_control_request_subtype(message)
            .map(|subtype| subtype.eq_ignore_ascii_case("elicitation"))
            .unwrap_or(false)
}

fn claude_control_request_subtype(message: &Value) -> Option<String> {
    first_non_empty_string_at(
        message,
        &[
            "/request/subtype",
            "/request/type",
            "/subtype",
            "/control_request/subtype",
        ],
    )
}

fn claude_control_request_id(message: &Value) -> String {
    message
        .get("request_id")
        .or_else(|| message.get("id"))
        .and_then(json_rpc_id_key)
        .unwrap_or_else(new_uuid_v4)
}

fn claude_permission_tool_name(message: &Value) -> Option<String> {
    first_non_empty_string_at(
        message,
        &[
            "/request/tool_name",
            "/request/toolName",
            "/request/tool/name",
            "/request/name",
            "/params/tool_name",
            "/params/toolName",
            "/tool_name",
            "/toolName",
            "/name",
        ],
    )
}

fn claude_permission_server_name(message: &Value) -> Option<String> {
    first_non_empty_string_at(
        message,
        &[
            "/request/server_name",
            "/request/serverName",
            "/request/mcp_server_name",
            "/request/mcpServerName",
            "/params/server_name",
            "/params/serverName",
        ],
    )
}

fn claude_permission_item_id(message: &Value) -> Option<String> {
    first_non_empty_string_at(
        message,
        &[
            "/request/tool_use_id",
            "/request/toolUseId",
            "/request/tool/id",
            "/request/itemId",
            "/request/item_id",
            "/params/tool_use_id",
            "/params/toolUseId",
        ],
    )
}

fn claude_permission_request_input(message: &Value) -> Option<&Value> {
    [
        "/request/input",
        "/request/tool_input",
        "/request/toolInput",
        "/request/arguments",
        "/params/input",
        "/params/tool_input",
        "/params/toolInput",
        "/params/arguments",
    ]
    .iter()
    .filter_map(|pointer| message.pointer(pointer))
    .find(|value| !value.is_null())
}

fn claude_elicitation_message(message: &Value) -> String {
    first_non_empty_string_at(
        message,
        &[
            "/request/message",
            "/params/message",
            "/message",
            "/control_request/message",
        ],
    )
    .unwrap_or_else(|| "Codex requests input from an MCP server.".to_string())
}

fn claude_elicitation_mode(message: &Value) -> String {
    first_non_empty_string_at(message, &["/request/mode", "/params/mode"])
        .unwrap_or_else(|| "form".to_string())
}

fn claude_elicitation_requested_schema(message: &Value) -> Value {
    [
        "/request/requestedSchema",
        "/request/requested_schema",
        "/params/requestedSchema",
        "/params/requested_schema",
    ]
    .iter()
    .filter_map(|pointer| message.pointer(pointer))
    .find(|value| value.is_object())
    .cloned()
    .unwrap_or_else(|| {
        json!({
            "type": "object",
            "properties": {},
        })
    })
}

fn claude_elicitation_meta(message: &Value) -> Option<&Value> {
    [
        "/request/_meta",
        "/request/meta",
        "/params/_meta",
        "/params/meta",
    ]
    .iter()
    .filter_map(|pointer| message.pointer(pointer))
    .find(|value| value.is_object())
}

fn claude_elicitation_id(message: &Value) -> Option<String> {
    first_non_empty_string_at(
        message,
        &[
            "/request/elicitationId",
            "/request/elicitation_id",
            "/params/elicitationId",
            "/params/elicitation_id",
        ],
    )
}

fn claude_elicitation_url(message: &Value) -> Option<String> {
    first_non_empty_string_at(message, &["/request/url", "/params/url"])
}

fn first_non_empty_string_at(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn claude_control_response_for_permission(
    message: &Value,
    request_id: &str,
    approval: &Value,
) -> Value {
    let mut permission_response = serde_json::Map::new();
    if let Some(tool_use_id) = claude_permission_item_id(message) {
        permission_response.insert("toolUseID".to_string(), json!(tool_use_id));
    }
    if codex_permission_response_allows(approval) {
        permission_response.insert("behavior".to_string(), json!("allow"));
        let input = claude_permission_request_input(message)
            .cloned()
            .unwrap_or_else(|| json!({}));
        permission_response.insert("updatedInput".to_string(), input);
    } else {
        permission_response.insert("behavior".to_string(), json!("deny"));
        permission_response.insert("message".to_string(), json!("Denied in Codex App"));
    }
    claude_control_response_success(request_id, Value::Object(permission_response))
}

fn claude_control_response_success(request_id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
}

fn claude_control_response_denied(request_id: &str, message: &str) -> Value {
    claude_control_response_success(
        request_id,
        json!({
            "behavior": "deny",
            "message": message,
        }),
    )
}

fn claude_control_response_for_elicitation(request_id: &str, response: &Value) -> Value {
    claude_control_response_success(request_id, normalized_elicitation_response(response))
}

fn normalized_elicitation_response(response: &Value) -> Value {
    let response = response.get("result").unwrap_or(response);
    let action = response
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| matches!(*action, "accept" | "decline" | "cancel"))
        .unwrap_or("cancel");
    let mut result = serde_json::Map::new();
    result.insert("action".to_string(), json!(action));
    let content = if action == "accept" {
        response
            .get("content")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        response.get("content").cloned().unwrap_or(Value::Null)
    };
    result.insert("content".to_string(), content);
    if let Some(meta) = response.get("_meta") {
        result.insert("_meta".to_string(), meta.clone());
    }
    Value::Object(result)
}

fn codex_permission_response_allows(response: &Value) -> bool {
    if response.get("error").is_some() {
        return false;
    }
    if let Some(approved) = response.get("approved").and_then(Value::as_bool) {
        return approved;
    }
    if let Some(permissions) = response.get("permissions") {
        return permission_value_allows(permissions);
    }
    if let Some(result) = response.get("result") {
        return codex_permission_response_allows(result);
    }
    false
}

fn permission_value_allows(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Array(values) => !values.is_empty() && values.iter().any(permission_value_allows),
        Value::Object(values) => {
            if let Some(enabled) = values.get("enabled").and_then(Value::as_bool) {
                return enabled;
            }
            !values.is_empty() && values.values().any(permission_value_allows)
        }
        _ => true,
    }
}

fn write_claude_child_json_line<W: Write>(writer: &mut W, value: &Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    line.push(b'\n');
    writer
        .write_all(&line)
        .and_then(|_| writer.flush())
        .map_err(|err| format!("failed to write Claude Code control response: {}", err))
}

fn claude_stream_json_input(work: &TurnWork) -> String {
    let initialize = json!({
        "type": "control_request",
        "request_id": new_uuid_v4(),
        "request": { "subtype": "initialize" },
    });
    let user_message = json!({
        "type": "user",
        "session_id": "",
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": work.prompt,
                }
            ],
        },
        "parent_tool_use_id": Value::Null,
    });
    format!(
        "{}\n{}\n",
        serde_json::to_string(&initialize).unwrap_or_default(),
        serde_json::to_string(&user_message).unwrap_or_default()
    )
}

fn handle_claude_stream_message<W>(
    message: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    remember_claude_stream_token_usage(message, work, output, stream);
    match message.get("type").and_then(Value::as_str) {
        Some("stream_event") => {
            if let Some(event) = message.get("event") {
                handle_claude_stream_event(event, message, work, output, stream, command_output);
            }
        }
        Some("assistant") => {
            handle_claude_assistant_message(message, work, output, stream, command_output);
        }
        Some("result") => {
            handle_claude_result_message(message, work, output, stream, command_output);
        }
        Some("user") => {
            handle_claude_user_message(message, work, output, stream, command_output);
        }
        Some("tool_progress") => {}
        Some("tool_use_summary") => {}
        Some("system") => {}
        Some("control_response") | Some("keep_alive") => {}
        Some(other) => {
            let _ = other;
        }
        None => {}
    }
}

fn remember_claude_stream_token_usage<W>(
    message: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
) where
    W: Write,
{
    if let Some(model) = claude_model_from_message(message) {
        stream.latest_model = Some(model.to_string());
    }
    let fallback_model = stream.latest_model.as_deref().unwrap_or(DEFAULT_MODEL);
    let Some(info) = claude_token_usage_info_from_message(message, fallback_model) else {
        return;
    };
    if stream.latest_token_usage_info.as_ref() == Some(&info) {
        return;
    }
    stream.latest_token_usage_info = Some(info.clone());
    if is_claude_title_generation_prompt(&work.prompt) {
        return;
    }
    let _ = write_notification(
        output,
        json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": work.thread_id,
                "conversationId": work.thread_id,
                "latestTokenUsageInfo": info,
            },
        }),
    );
}

fn handle_claude_stream_event<W>(
    event: &Value,
    envelope: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    let parent_tool_use_id = envelope.get("parent_tool_use_id").and_then(Value::as_str);
    match event.get("type").and_then(Value::as_str) {
        Some("content_block_start") => {
            let index = event.get("index").and_then(Value::as_i64);
            if let Some(content_block) = event.get("content_block") {
                if let (Some(index), Some(tool_id)) =
                    (index, content_block.get("id").and_then(Value::as_str))
                {
                    stream
                        .tool_block_by_index
                        .insert(index, tool_id.to_string());
                }
                handle_claude_content_block(
                    content_block,
                    parent_tool_use_id,
                    work,
                    output,
                    stream,
                    command_output,
                );
            }
        }
        Some("content_block_delta") => {
            if let Some(delta) = event.get("delta") {
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if parent_tool_use_id.is_none() {
                            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                handle_claude_agent_text_delta(output, work, stream, text);
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            emit_reasoning_delta(output, work, stream, text);
                        }
                    }
                    Some("input_json_delta") => {
                        if let (Some(index), Some(partial_json)) = (
                            event.get("index").and_then(Value::as_i64),
                            delta.get("partial_json").and_then(Value::as_str),
                        ) {
                            if let Some(tool_id) = stream.tool_block_by_index.get(&index) {
                                stream
                                    .tool_input_deltas
                                    .entry(tool_id.clone())
                                    .or_default()
                                    .push_str(partial_json);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("content_block_stop") => {
            if let Some(index) = event.get("index").and_then(Value::as_i64) {
                if let Some(tool_id) = stream.tool_block_by_index.get(&index).cloned() {
                    if let Some(input) = stream.tool_input_deltas.remove(&tool_id) {
                        if !input.trim().is_empty() {
                            update_tool_call_arguments(
                                output,
                                work,
                                stream,
                                &tool_id,
                                parse_tool_arguments(input.trim()),
                            );
                        }
                    }
                }
            }
        }
        Some("message_start") | Some("message_delta") | Some("message_stop") => {}
        _ => {}
    }
}

fn handle_claude_assistant_message<W>(
    message: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    let parent_tool_use_id = message.get("parent_tool_use_id").and_then(Value::as_str);
    let Some(message_body) = message.get("message") else {
        return;
    };
    if let Some(content) = message_body.get("content") {
        if parent_tool_use_id.is_none() {
            if let Some(text) = claude_text_from_content(content) {
                let Some(text) = visible_agent_snapshot_text(stream, &text) else {
                    return;
                };
                if !stream.saw_tool_call && !stream.agent_item_started {
                    if stream.pending_agent_text.trim() != text.trim() {
                        stream.pending_agent_text = text;
                    }
                    return;
                }
                emit_agent_snapshot(
                    output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.agent_item_id,
                    &mut stream.agent_item_started,
                    &mut stream.emitted_text,
                    &text,
                );
            }
        } else if let Some(text) = claude_text_from_content(content) {
            complete_tool_call(
                output,
                work,
                stream,
                parent_tool_use_id.unwrap_or_default(),
                true,
                Some(text),
            );
        }
        if let Value::Array(items) = content {
            for item in items {
                handle_claude_content_block(
                    item,
                    parent_tool_use_id,
                    work,
                    output,
                    stream,
                    command_output,
                );
            }
        }
    }
}

fn handle_claude_user_message<W>(
    message: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    let parent_tool_use_id = message.get("parent_tool_use_id").and_then(Value::as_str);
    let Some(content) = message
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return;
    };
    match content {
        Value::Array(items) => {
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("tool_result") {
                    handle_claude_content_block(
                        item,
                        parent_tool_use_id,
                        work,
                        output,
                        stream,
                        command_output,
                    );
                }
            }
        }
        Value::Object(_) if content.get("type").and_then(Value::as_str) == Some("tool_result") => {
            handle_claude_content_block(
                content,
                parent_tool_use_id,
                work,
                output,
                stream,
                command_output,
            );
        }
        _ => {}
    }
}

fn handle_claude_result_message<W>(
    message: &Value,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    claude_code_log_event(
        "claude_result_message",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "isError": message
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "usage": claude_result_usage_summary(message),
            "resultPreview": message
                .get("result")
                .and_then(Value::as_str)
                .map(|value| log_text_preview(value, 500)),
        }),
    );
    if let Some(result) = message.get("result").and_then(Value::as_str) {
        if !result.trim().is_empty() {
            stream.result_text = Some(result.to_string());
        }
    }
    let is_error = message
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        stream.result_error = message
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                message
                    .get("errors")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
            })
            .filter(|text| !text.trim().is_empty());
    }

    let _ = (work, output, command_output);
}

fn handle_claude_content_block<W>(
    block: &Value,
    parent_tool_use_id: Option<&str>,
    work: &TurnWork,
    output: &SharedOutput<W>,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
) where
    W: Write,
{
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            if parent_tool_use_id.is_none() {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let Some(text) = visible_agent_snapshot_text(stream, text) else {
                        return;
                    };
                    if !stream.saw_tool_call && !stream.agent_item_started {
                        if stream.pending_agent_text.trim() != text.trim() {
                            stream.pending_agent_text = text;
                        }
                        return;
                    }
                    emit_agent_snapshot(
                        output,
                        &work.thread_id,
                        &work.turn_id,
                        &work.agent_item_id,
                        &mut stream.agent_item_started,
                        &mut stream.emitted_text,
                        &text,
                    );
                }
            }
        }
        Some("thinking") => {
            if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                emit_reasoning_delta(output, work, stream, text);
            }
        }
        Some("thinking_delta") => {
            if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                emit_reasoning_delta(output, work, stream, text);
            }
        }
        Some("tool_use") | Some("server_tool_use") | Some("mcp_tool_use") => {
            emit_tool_use_event(output, work, stream, command_output, block);
        }
        Some("tool_result")
        | Some("tool_search_tool_result")
        | Some("web_fetch_tool_result")
        | Some("web_search_tool_result")
        | Some("code_execution_tool_result")
        | Some("bash_code_execution_tool_result")
        | Some("text_editor_code_execution_tool_result")
        | Some("mcp_tool_result") => {
            emit_tool_result_event(output, work, stream, command_output, block);
        }
        _ => {}
    }
}

fn handle_claude_agent_text_delta<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    delta: &str,
) where
    W: Write,
{
    if delta.is_empty() {
        return;
    }
    if !stream.saw_tool_call && !stream.agent_item_started {
        stream.pending_agent_text.push_str(delta);
        return;
    }
    append_agent_delta(output, work, stream, delta);
}

fn append_agent_delta<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    delta: &str,
) where
    W: Write,
{
    if delta.is_empty() {
        return;
    }
    let next_text = format!("{}{}", stream.emitted_text, delta);
    emit_agent_delta(
        output,
        &work.thread_id,
        &work.turn_id,
        &work.agent_item_id,
        &mut stream.agent_item_started,
        &mut stream.emitted_text,
        &next_text,
    );
}

fn flush_pending_agent_text_as_reasoning<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
) where
    W: Write,
{
    if stream.pending_agent_text.trim().is_empty() {
        stream.pending_agent_text.clear();
        return;
    }
    let text = std::mem::take(&mut stream.pending_agent_text);
    stream.suppressed_agent_prefix.push_str(&text);
    emit_reasoning_delta(output, work, stream, &text);
}

fn flush_pending_agent_text_as_agent<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
) where
    W: Write,
{
    if stream.pending_agent_text.trim().is_empty() {
        stream.pending_agent_text.clear();
        return;
    }
    let text = std::mem::take(&mut stream.pending_agent_text);
    append_agent_delta(output, work, stream, &text);
}

fn visible_agent_snapshot_text(stream: &ClaudeStreamState, text: &str) -> Option<String> {
    let mut visible = text;
    if !stream.suppressed_agent_prefix.is_empty()
        && visible.starts_with(stream.suppressed_agent_prefix.as_str())
    {
        visible = &visible[stream.suppressed_agent_prefix.len()..];
    }
    let visible = visible
        .trim_start_matches(|ch: char| ch.is_whitespace())
        .to_string();
    (!visible.trim().is_empty()).then_some(visible)
}

fn emit_agent_snapshot<W>(
    output: &SharedOutput<W>,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    item_started: &mut bool,
    emitted_text: &mut String,
    next_text: &str,
) where
    W: Write,
{
    if next_text.is_empty() {
        return;
    }
    if emitted_text.trim() == next_text.trim() {
        return;
    }
    if emitted_text.is_empty() || next_text.starts_with(emitted_text.as_str()) {
        emit_agent_delta(
            output,
            thread_id,
            turn_id,
            item_id,
            item_started,
            emitted_text,
            next_text,
        );
    }
}

fn emit_tool_use_event<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
    block: &Value,
) where
    W: Write,
{
    let tool_id = block.get("id").and_then(Value::as_str).unwrap_or("unknown");
    let tool_name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
    stream.seen_tool_ids.insert(tool_id.to_string());
    let explicit_arguments = block.get("input").filter(|value| !value.is_null()).cloned();
    let has_explicit_arguments = explicit_arguments.is_some();
    let arguments = explicit_arguments.unwrap_or_else(|| json!({}));
    claude_code_log_event(
        "claude_tool_use",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "toolId": tool_id,
            "toolName": tool_name,
            "arguments": log_request_params_summary(&arguments),
        }),
    );
    emit_tool_call_started(
        output,
        work,
        stream,
        tool_id,
        tool_name,
        arguments,
        has_explicit_arguments,
    );
    let _ = command_output;
}

fn emit_tool_result_event<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    command_output: &mut String,
    block: &Value,
) where
    W: Write,
{
    let tool_id = block
        .get("tool_use_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = if block
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "failed"
    } else {
        "completed"
    };
    let result = block
        .get("content")
        .and_then(claude_text_from_content)
        .unwrap_or_else(|| compact_json(block));
    claude_code_log_event(
        "claude_tool_result",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "toolId": tool_id,
            "status": status,
            "resultPreview": log_text_preview(&result, 500),
        }),
    );
    complete_tool_call(
        output,
        work,
        stream,
        tool_id,
        status == "completed",
        Some(result),
    );
    let _ = command_output;
}

fn emit_tool_call_started<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    tool_id: &str,
    tool_name: &str,
    arguments: Value,
    has_explicit_arguments: bool,
) where
    W: Write,
{
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    let tool_name = non_empty_string(tool_name).unwrap_or_else(|| "tool".to_string());
    stream.saw_tool_call = true;
    flush_pending_agent_text_as_reasoning(output, work, stream);
    {
        let entry =
            stream
                .tool_calls
                .entry(tool_id.clone())
                .or_insert_with(|| ClaudeToolCallState {
                    name: tool_name.clone(),
                    arguments: json!({}),
                    started_at_ms: now_millis(),
                    started_emitted: false,
                    kind: claude_tool_item_kind(&tool_name),
                });
        entry.name = tool_name.clone();
        entry.kind = claude_tool_item_kind(&tool_name);
        if has_explicit_arguments || !is_empty_tool_arguments(&arguments) {
            entry.arguments = arguments;
        }
    }
    maybe_emit_tool_call_started(
        output,
        work,
        stream,
        &tool_id,
        false,
        has_explicit_arguments,
    );
    claude_code_log_event(
        "tool_call_started",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "toolId": &tool_id,
            "toolName": &tool_name,
            "arguments": stream
                .tool_calls
                .get(&tool_id)
                .map(|state| log_request_params_summary(&state.arguments))
                .unwrap_or_else(|| json!({ "kind": "unknown" })),
        }),
    );
}

fn maybe_emit_tool_call_started<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    tool_id: &str,
    force: bool,
    allow_empty_arguments: bool,
) where
    W: Write,
{
    let Some(state) = stream.tool_calls.get_mut(tool_id) else {
        return;
    };
    if state.started_emitted
        || (!force && !allow_empty_arguments && is_empty_tool_arguments(&state.arguments))
    {
        return;
    }
    let item = tool_call_item(
        &work.thread_id,
        &work.cwd,
        tool_id,
        state,
        "inProgress",
        None,
        Value::Null,
    );
    state.started_emitted = true;
    let _ = write_notification(
        output,
        json!({
            "method": "item/started",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "item": item,
                "startedAtMs": state.started_at_ms,
            },
        }),
    );
}

fn update_tool_call_arguments<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    tool_id: &str,
    arguments: Value,
) where
    W: Write,
{
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    if !stream.tool_calls.contains_key(&tool_id) {
        emit_tool_call_started(output, work, stream, &tool_id, "tool", arguments, true);
        return;
    }
    if !is_empty_tool_arguments(&arguments) {
        if let Some(state) = stream.tool_calls.get_mut(&tool_id) {
            state.arguments = arguments;
        }
    }
    maybe_emit_tool_call_started(output, work, stream, &tool_id, false, true);
    claude_code_log_event(
        "tool_call_arguments_updated",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "toolId": &tool_id,
            "arguments": stream
                .tool_calls
                .get(&tool_id)
                .map(|state| log_request_params_summary(&state.arguments))
                .unwrap_or_else(|| json!({ "kind": "unknown" })),
        }),
    );
}

fn complete_tool_call<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    tool_id: &str,
    success: bool,
    result: Option<String>,
) where
    W: Write,
{
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    if stream.completed_tool_ids.contains(&tool_id) {
        return;
    }
    if !stream.tool_calls.contains_key(&tool_id) {
        emit_tool_call_started(output, work, stream, &tool_id, "tool", json!({}), true);
    }
    maybe_emit_tool_call_started(output, work, stream, &tool_id, true, true);
    let Some(state) = stream.tool_calls.get(&tool_id).cloned() else {
        return;
    };
    stream.completed_tool_ids.insert(tool_id.clone());
    let item = tool_call_item(
        &work.thread_id,
        &work.cwd,
        &tool_id,
        &state,
        if success { "completed" } else { "failed" },
        result.as_deref(),
        Value::Null,
    );
    claude_code_log_event(
        "tool_call_completed",
        json!({
            "threadId": &work.thread_id,
            "turnId": &work.turn_id,
            "toolId": &tool_id,
            "toolName": &state.name,
            "success": success,
            "resultPreview": result
                .as_deref()
                .map(|value| log_text_preview(value, 500)),
        }),
    );
    stream.completed_tool_items.push(item.clone());
    let _ = write_notification(
        output,
        json!({
            "method": "item/completed",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "item": item,
                "completedAtMs": now_millis(),
            },
        }),
    );
}

fn finalize_open_tool_calls<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    success: bool,
) where
    W: Write,
{
    let tool_ids = stream.tool_calls.keys().cloned().collect::<Vec<_>>();
    for tool_id in tool_ids {
        complete_tool_call(output, work, stream, &tool_id, success, None);
    }
}

fn tool_call_item(
    thread_id: &str,
    cwd: &str,
    tool_id: &str,
    state: &ClaudeToolCallState,
    status: &str,
    result: Option<&str>,
    duration_ms: Value,
) -> Value {
    match state.kind {
        ClaudeToolItemKind::CommandExecution => {
            command_execution_item_for_tool(tool_id, state, status, result, duration_ms, cwd)
        }
        ClaudeToolItemKind::CollabAgentToolCall => {
            collab_agent_tool_call_item(thread_id, tool_id, state, status, result)
        }
        ClaudeToolItemKind::McpToolCall => mcp_tool_call_item(tool_id, state, status, result),
    }
}

fn claude_tool_item_kind(tool_name: &str) -> ClaudeToolItemKind {
    match tool_name {
        "Agent" | "Task" => ClaudeToolItemKind::CollabAgentToolCall,
        "Bash" => ClaudeToolItemKind::CommandExecution,
        _ => ClaudeToolItemKind::McpToolCall,
    }
}

fn command_execution_item_for_tool(
    tool_id: &str,
    state: &ClaudeToolCallState,
    status: &str,
    result: Option<&str>,
    duration_ms: Value,
    cwd: &str,
) -> Value {
    let command = state
        .arguments
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| compact_json(&state.arguments));
    let aggregated_output = result
        .map(|value| json!(truncate_for_protocol(value, 200_000)))
        .unwrap_or(Value::Null);
    json!({
        "type": "commandExecution",
        "id": tool_item_id(tool_id),
        "command": command,
        "cwd": state.arguments.get("cwd").and_then(Value::as_str).unwrap_or(cwd),
        "processId": Value::Null,
        "source": "agent",
        "status": status,
        "commandActions": [
            {
                "type": "unknown",
                "command": command,
            }
        ],
        "aggregatedOutput": aggregated_output,
        "exitCode": if status == "completed" { json!(0) } else { Value::Null },
        "durationMs": duration_ms,
    })
}

fn collab_agent_tool_call_item(
    thread_id: &str,
    tool_id: &str,
    state: &ClaudeToolCallState,
    status: &str,
    result: Option<&str>,
) -> Value {
    let receiver_thread_ids = collab_agent_receiver_thread_ids(tool_id, &state.arguments);
    let receiver_threads = receiver_thread_ids
        .iter()
        .map(|thread_id| {
            json!({
                "threadId": thread_id,
                "thread": Value::Null,
            })
        })
        .collect::<Vec<_>>();
    let mut agents_states = Map::new();
    for receiver_thread_id in &receiver_thread_ids {
        agents_states.insert(
            receiver_thread_id.clone(),
            json!({ "status": collab_agent_state_status(status) }),
        );
    }
    let failed = status == "failed";
    json!({
        "type": "collabAgentToolCall",
        "id": tool_item_id(tool_id),
        "tool": "spawnAgent",
        "status": status,
        "senderThreadId": thread_id,
        "receiverThreadIds": receiver_thread_ids,
        "receiverThreads": receiver_threads,
        "prompt": collab_agent_prompt(&state.arguments),
        "model": collab_agent_optional_string_argument(&state.arguments, &["model"]),
        "reasoningEffort": collab_agent_optional_string_argument(
            &state.arguments,
            &["reasoningEffort", "reasoning_effort"],
        ),
        "agentsStates": Value::Object(agents_states),
        "result": result.map(|value| truncate_for_protocol(value, 20_000)),
        "error": if failed {
            json!({ "message": result.unwrap_or("Claude Code subagent failed") })
        } else {
            Value::Null
        },
    })
}

fn collab_agent_receiver_thread_ids(tool_id: &str, arguments: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    for key in [
        "receiverThreadId",
        "receiver_thread_id",
        "threadId",
        "thread_id",
    ] {
        push_unique_thread_id(
            &mut ids,
            &mut seen,
            arguments.get(key).and_then(Value::as_str),
        );
    }
    if let Some(values) = arguments.get("receiverThreadIds").and_then(Value::as_array) {
        for value in values {
            push_unique_thread_id(&mut ids, &mut seen, value.as_str());
        }
    }
    if ids.is_empty() {
        ids.push(format!("claude-subagent-{}", sanitize_item_id(tool_id)));
    }
    ids
}

fn push_unique_thread_id(ids: &mut Vec<String>, seen: &mut BTreeSet<String>, value: Option<&str>) {
    let Some(value) = value.and_then(non_empty_string) else {
        return;
    };
    if seen.insert(value.clone()) {
        ids.push(value);
    }
}

fn collab_agent_prompt(arguments: &Value) -> String {
    for key in ["prompt", "description", "task", "message"] {
        if let Some(value) = arguments
            .get(key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            return value;
        }
    }
    if is_empty_tool_arguments(arguments) {
        String::new()
    } else {
        compact_json(arguments)
    }
}

fn collab_agent_optional_string_argument(arguments: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(value) = arguments
            .get(*key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            return json!(value);
        }
    }
    Value::Null
}

fn collab_agent_state_status(status: &str) -> &str {
    match status {
        "inProgress" => "running",
        "completed" => "completed",
        "failed" => "failed",
        _ => status,
    }
}

fn mcp_tool_call_item(
    tool_id: &str,
    state: &ClaudeToolCallState,
    status: &str,
    result: Option<&str>,
) -> Value {
    let failed = status == "failed";
    json!({
        "type": "mcpToolCall",
        "id": tool_item_id(tool_id),
        "server": "claude-code",
        "tool": state.name.clone(),
        "status": status,
        "arguments": state.arguments.clone(),
        "pluginId": Value::Null,
        "result": if failed { Value::Null } else { mcp_tool_result(result) },
        "error": if failed {
            json!({ "message": result.unwrap_or("Claude Code tool failed") })
        } else {
            Value::Null
        },
        "durationMs": Value::Null,
    })
}

fn mcp_tool_result(result: Option<&str>) -> Value {
    match result.map(str::trim).filter(|value| !value.is_empty()) {
        Some(result) => json!({
            "content": [{ "type": "text", "text": truncate_for_protocol(result, 20_000) }],
            "structuredContent": Value::Null,
            "_meta": Value::Null,
        }),
        None => Value::Null,
    }
}

fn parse_tool_arguments(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "raw": raw }))
}

fn is_empty_tool_arguments(value: &Value) -> bool {
    value.is_null()
        || value.as_object().is_some_and(|object| object.is_empty())
        || value.as_array().is_some_and(|array| array.is_empty())
        || value.as_str().is_some_and(str::is_empty)
}

fn tool_item_id(tool_id: &str) -> String {
    format!("claude-tool-{}", sanitize_item_id(tool_id))
}

fn sanitize_item_id(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn claude_text_from_content(content: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_claude_text(content, &mut parts);
    let text = parts.join("");
    (!text.trim().is_empty()).then_some(text)
}

fn collect_claude_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_claude_text(item, parts);
            }
        }
        Value::Object(map) => {
            if matches!(
                map.get("type").and_then(Value::as_str),
                Some("text") | Some("text_delta")
            ) {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            } else if let Some(content) = map.get("content") {
                collect_claude_text(content, parts);
            }
        }
        _ => {}
    }
}

fn claude_result_usage_summary(message: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(num_turns) = message.get("num_turns").and_then(Value::as_i64) {
        parts.push(format!("turns={num_turns}"));
    }
    if let Some(cost) = message.get("total_cost_usd").and_then(Value::as_f64) {
        parts.push(format!("cost=${cost:.6}"));
    }
    if let Some(duration_ms) = message.get("duration_ms").and_then(Value::as_i64) {
        parts.push(format!("duration={}ms", duration_ms));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("result {}", parts.join(" "))
    }
}

fn claude_token_usage_info_from_message(message: &Value, fallback_model: &str) -> Option<Value> {
    let usage = claude_usage_from_message(message)?;
    claude_token_usage_info(
        usage,
        claude_model_from_message(message).unwrap_or(fallback_model),
    )
}

fn claude_usage_from_message(message: &Value) -> Option<&Value> {
    message
        .pointer("/message/usage")
        .or_else(|| message.get("usage"))
        .or_else(|| message.pointer("/event/message/usage"))
        .or_else(|| message.pointer("/event/usage"))
        .filter(|usage| usage.is_object())
}

fn claude_model_from_message(message: &Value) -> Option<&str> {
    message
        .pointer("/message/model")
        .and_then(Value::as_str)
        .or_else(|| message.get("model").and_then(Value::as_str))
        .or_else(|| {
            message
                .pointer("/event/message/model")
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

fn claude_token_usage_info(usage: &Value, model: &str) -> Option<Value> {
    let input_tokens = sum_present_counts(&[
        first_token_count(
            usage,
            &[
                "input_tokens",
                "inputTokens",
                "prompt_tokens",
                "promptTokens",
            ],
        ),
        first_token_count(
            usage,
            &["cache_creation_input_tokens", "cacheCreationInputTokens"],
        ),
        first_token_count(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]),
    ]);
    let output_tokens = first_token_count(
        usage,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    );
    let total_tokens = first_token_count(usage, &["total_tokens", "totalTokens", "total"])
        .or_else(|| sum_present_counts(&[input_tokens, output_tokens]))?;
    if total_tokens <= 0 {
        return None;
    }

    let mut last_token_usage = Map::new();
    if let Some(input_tokens) = input_tokens {
        last_token_usage.insert("input_tokens".to_string(), json!(input_tokens));
    }
    if let Some(output_tokens) = output_tokens {
        last_token_usage.insert("output_tokens".to_string(), json!(output_tokens));
    }
    if let Some(cache_creation_input_tokens) = first_token_count(
        usage,
        &["cache_creation_input_tokens", "cacheCreationInputTokens"],
    ) {
        last_token_usage.insert(
            "cache_creation_input_tokens".to_string(),
            json!(cache_creation_input_tokens),
        );
    }
    if let Some(cache_read_input_tokens) =
        first_token_count(usage, &["cache_read_input_tokens", "cacheReadInputTokens"])
    {
        last_token_usage.insert(
            "cache_read_input_tokens".to_string(),
            json!(cache_read_input_tokens),
        );
    }
    last_token_usage.insert("total_tokens".to_string(), json!(total_tokens));

    let context_window = first_token_count(
        usage,
        &[
            "model_context_window",
            "modelContextWindow",
            "context_window",
            "contextWindow",
            "max_context_window",
            "maxContextWindow",
        ],
    )
    .unwrap_or_else(|| claude_context_window_for_model(model));

    Some(json!({
        "last_token_usage": Value::Object(last_token_usage),
        "model_context_window": context_window,
        "model": model,
    }))
}

fn sum_present_counts(values: &[Option<i64>]) -> Option<i64> {
    let mut total = 0_i64;
    let mut present = false;
    for value in values.iter().flatten() {
        present = true;
        total = total.saturating_add((*value).max(0));
    }
    present.then_some(total)
}

fn first_token_count(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(value_as_positive_i64))
}

fn value_as_positive_i64(value: &Value) -> Option<i64> {
    let parsed = value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value.round() as i64)
        })?;
    (parsed >= 0).then_some(parsed)
}

fn claude_context_window_for_model(model: &str) -> i64 {
    if let Ok(value) = std::env::var(CONTEXT_WINDOW_ENV) {
        if let Some(parsed) = parse_context_window_value(&value) {
            return parsed;
        }
    }
    let normalized = model
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.contains("1mcontext")
        || normalized.contains("1m")
        || normalized.contains("1000000")
        || normalized.contains("million")
    {
        CLAUDE_ONE_M_CONTEXT_WINDOW
    } else {
        DEFAULT_CLAUDE_CONTEXT_WINDOW
    }
}

fn parse_context_window_value(value: &str) -> Option<i64> {
    let normalized = value.trim().replace([',', '_'], "");
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if let Some(number) = lower.strip_suffix('m') {
        return number
            .parse::<f64>()
            .ok()
            .map(|value| (value * 1_000_000.0).round() as i64)
            .filter(|value| *value > 0);
    }
    if let Some(number) = lower.strip_suffix('k') {
        return number
            .parse::<f64>()
            .ok()
            .map(|value| (value * 1_000.0).round() as i64)
            .filter(|value| *value > 0);
    }
    lower.parse::<i64>().ok().filter(|value| *value > 0)
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(not(unix))]
fn run_claude_code_turn_piped<W>(
    mut command: Command,
    work: &TurnWork,
    state: SharedState,
    output: SharedOutput<W>,
    started: Instant,
) -> ClaudeRunResult
where
    W: Write,
{
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ClaudeRunResult {
                text: String::new(),
                error: Some(format!("failed to launch Claude Code: {}", err)),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    emit_command_execution_started(&output, work, Some(child.id()));
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .insert((work.thread_id.clone(), work.turn_id.clone()), child.id());
    }

    let mut child_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            terminate_process_group(child.id());
            let _ = child.wait();
            if let Ok(mut state) = lock_state(&state) {
                state
                    .active_processes
                    .remove(&(work.thread_id.clone(), work.turn_id.clone()));
            }
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to open Claude Code stdin".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_process_group(child.id());
            let _ = child.wait();
            if let Ok(mut state) = lock_state(&state) {
                state
                    .active_processes
                    .remove(&(work.thread_id.clone(), work.turn_id.clone()));
            }
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to capture Claude Code stdout".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let stderr_handle = child.stderr.take().map(|stderr| {
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            text
        })
    });

    if let Err(err) = child_stdin
        .write_all(work.prompt.as_bytes())
        .and_then(|_| child_stdin.write_all(b"\n"))
        .and_then(|_| child_stdin.flush())
    {
        terminate_process_group(child.id());
        let _ = child.wait();
        if let Ok(mut state) = lock_state(&state) {
            state
                .active_processes
                .remove(&(work.thread_id.clone(), work.turn_id.clone()));
        }
        return ClaudeRunResult {
            text: String::new(),
            error: Some(format!(
                "failed to write prompt to Claude Code stdin: {}",
                err
            )),
            duration_ms: elapsed_millis(started),
            tool_items: Vec::new(),
            agent_item_streamed: false,
            latest_token_usage_info: None,
        };
    }
    drop(child_stdin);

    let mut emitted_text = String::new();
    let mut agent_item_started = false;
    let mut command_output = String::new();
    let mut raw_stdout = String::new();
    let mut reader = stdout;
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let chunk = String::from_utf8_lossy(&buffer[..size]).to_string();
                raw_stdout.push_str(&chunk);
                emit_command_execution_output_delta(
                    &output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.cli_item_id,
                    &chunk,
                    &work.prompt,
                    &mut command_output,
                );
                let text = clean_interactive_cli_output(&raw_stdout, &work.prompt);
                emit_agent_delta(
                    &output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.agent_item_id,
                    &mut agent_item_started,
                    &mut emitted_text,
                    &text,
                );
            }
            Err(err) => {
                terminate_process_group(child.id());
                let _ = child.wait();
                if let Ok(mut state) = lock_state(&state) {
                    state
                        .active_processes
                        .remove(&(work.thread_id.clone(), work.turn_id.clone()));
                }
                let agent_item_streamed = !emitted_text.is_empty();
                return ClaudeRunResult {
                    text: emitted_text,
                    error: Some(format!("failed to read Claude Code stdout: {}", err)),
                    duration_ms: elapsed_millis(started),
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                };
            }
        }
    }

    let status = child.wait();
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .remove(&(work.thread_id.clone(), work.turn_id.clone()));
    }
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let cleaned_stdout = clean_interactive_cli_output(&raw_stdout, &work.prompt);
    let cleaned_stderr = clean_interactive_cli_output(&stderr, &work.prompt);
    let final_text = latest_claude_transcript_assistant_text(work).unwrap_or_else(|| {
        if !cleaned_stdout.is_empty() {
            cleaned_stdout.clone()
        } else if !cleaned_stderr.is_empty() {
            cleaned_stderr.clone()
        } else {
            emitted_text.clone()
        }
    });
    if emitted_text.is_empty() && !final_text.is_empty() {
        emit_agent_delta(
            &output,
            &work.thread_id,
            &work.turn_id,
            &work.agent_item_id,
            &mut agent_item_started,
            &mut emitted_text,
            &final_text,
        );
    }

    let agent_item_streamed = !emitted_text.is_empty();
    let duration_ms = elapsed_millis(started);
    match status {
        Ok(status) => {
            emit_command_execution_completed(
                &output,
                work,
                Some(child.id()),
                status.success(),
                &command_output,
                status.code(),
                duration_ms,
            );
            if status.success() {
                ClaudeRunResult {
                    text: if emitted_text.is_empty() {
                        final_text
                    } else {
                        emitted_text
                    },
                    error: None,
                    duration_ms,
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                }
            } else {
                ClaudeRunResult {
                    text: emitted_text,
                    error: Some(non_empty_join(
                        &[
                            format!("Claude Code exited with status {}", status),
                            cleaned_stderr,
                            final_text,
                        ],
                        "\n",
                    )),
                    duration_ms,
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                }
            }
        }
        Err(err) => {
            emit_command_execution_completed(
                &output,
                work,
                Some(child.id()),
                false,
                &command_output,
                None,
                duration_ms,
            );
            ClaudeRunResult {
                text: emitted_text,
                error: Some(format!("failed to wait for Claude Code: {}", err)),
                duration_ms,
                tool_items: Vec::new(),
                agent_item_streamed,
                latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
            }
        }
    }
}

#[cfg(unix)]
fn run_claude_code_turn_pty<W>(
    mut command: Command,
    work: &TurnWork,
    state: SharedState,
    output: SharedOutput<W>,
    started: Instant,
) -> ClaudeRunResult
where
    W: Write,
{
    let (mut master, slave) = match open_unix_pty() {
        Ok(pair) => pair,
        Err(err) => return run_claude_code_turn_piped(command, work, state, output, started, err),
    };

    let stdin = match slave.try_clone() {
        Ok(file) => file,
        Err(err) => return run_claude_code_turn_piped(command, work, state, output, started, err),
    };
    let stdout = match slave.try_clone() {
        Ok(file) => file,
        Err(err) => return run_claude_code_turn_piped(command, work, state, output, started, err),
    };
    let stderr = match slave.try_clone() {
        Ok(file) => file,
        Err(err) => return run_claude_code_turn_piped(command, work, state, output, started, err),
    };

    unsafe {
        command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .pre_exec(|| {
                for signo in [
                    libc::SIGCHLD,
                    libc::SIGHUP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTERM,
                    libc::SIGALRM,
                ] {
                    libc::signal(signo, libc::SIG_DFL);
                }

                let empty_set: libc::sigset_t = std::mem::zeroed();
                libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut());

                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                #[allow(clippy::cast_lossless)]
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ClaudeRunResult {
                text: String::new(),
                error: Some(format!("failed to launch Claude Code: {}", err)),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    drop(slave);

    emit_command_execution_started(&output, work, Some(child.id()));
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .insert((work.thread_id.clone(), work.turn_id.clone()), child.id());
    }

    let mut writer = match master.try_clone() {
        Ok(file) => file,
        Err(err) => {
            terminate_process_group(child.id());
            let _ = child.wait();
            remove_active_process(&state, work);
            return ClaudeRunResult {
                text: String::new(),
                error: Some(format!("failed to clone Claude Code PTY: {}", err)),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };

    if let Err(err) = set_nonblocking(master.as_raw_fd()) {
        terminate_process_group(child.id());
        let _ = child.wait();
        remove_active_process(&state, work);
        return ClaudeRunResult {
            text: String::new(),
            error: Some(format!("failed to configure Claude Code PTY: {}", err)),
            duration_ms: elapsed_millis(started),
            tool_items: Vec::new(),
            agent_item_streamed: false,
            latest_token_usage_info: None,
        };
    }

    let idle_timeout = claude_turn_idle_timeout();
    let mut exit_requested_at: Option<Instant> = None;
    let mut last_meaningful_output_at = Instant::now();
    let mut trust_confirmed = false;
    let mut trust_confirmed_at: Option<Instant> = None;
    let mut prompt_sent = false;
    let mut saw_turn_content = false;
    let mut last_raw_output_at = Instant::now();
    let mut raw_output = String::new();
    let mut command_output = String::new();
    let mut buffer = [0u8; 4096];
    let status = loop {
        match master.read(&mut buffer) {
            Ok(0) => {
                if let Ok(Some(exit_status)) = child.try_wait() {
                    break Some(exit_status);
                }
                thread::sleep(Duration::from_millis(25));
            }
            Ok(size) => {
                let chunk = String::from_utf8_lossy(&buffer[..size]).to_string();
                last_raw_output_at = Instant::now();
                raw_output.push_str(&chunk);
                if !trust_confirmed && looks_like_claude_trust_prompt(&raw_output) {
                    if let Err(err) = writer.write_all(b"\r").and_then(|_| writer.flush()) {
                        terminate_process_group(child.id());
                        let _ = child.wait();
                        remove_active_process(&state, work);
                        return ClaudeRunResult {
                            text: String::new(),
                            error: Some(format!(
                                "failed to write prompt to Claude Code PTY: {}",
                                err
                            )),
                            duration_ms: elapsed_millis(started),
                            tool_items: Vec::new(),
                            agent_item_streamed: false,
                            latest_token_usage_info: None,
                        };
                    }
                    trust_confirmed = true;
                    trust_confirmed_at = Some(Instant::now());
                }
                let emitted = emit_command_execution_output_delta(
                    &output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.cli_item_id,
                    &chunk,
                    &work.prompt,
                    &mut command_output,
                );
                if emitted && prompt_sent {
                    saw_turn_content = true;
                    last_meaningful_output_at = Instant::now();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if let Ok(Some(exit_status)) = child.try_wait() {
                    break Some(exit_status);
                }
                match exit_requested_at {
                    Some(requested_at)
                        if requested_at.elapsed() >= Duration::from_millis(1_200) =>
                    {
                        terminate_process_group(child.id());
                        break child.wait().ok();
                    }
                    Some(_) => {}
                    None if !prompt_sent
                        && should_send_prompt_to_claude(
                            started,
                            trust_confirmed_at,
                            &raw_output,
                            last_raw_output_at,
                        ) =>
                    {
                        match writer
                            .write_all(b"\x1b[200~")
                            .and_then(|_| writer.write_all(work.prompt.as_bytes()))
                            .and_then(|_| writer.write_all(b"\x1b[201~\r"))
                            .and_then(|_| writer.flush())
                        {
                            Ok(()) => {
                                prompt_sent = true;
                                last_meaningful_output_at = Instant::now();
                            }
                            Err(err) => {
                                terminate_process_group(child.id());
                                let _ = child.wait();
                                remove_active_process(&state, work);
                                emit_command_execution_completed(
                                    &output,
                                    work,
                                    Some(child.id()),
                                    false,
                                    &command_output,
                                    None,
                                    elapsed_millis(started),
                                );
                                return ClaudeRunResult {
                                    text: String::new(),
                                    error: Some(format!(
                                        "failed to write prompt to Claude Code PTY: {}",
                                        err
                                    )),
                                    duration_ms: elapsed_millis(started),
                                    tool_items: Vec::new(),
                                    agent_item_streamed: false,
                                    latest_token_usage_info: None,
                                };
                            }
                        }
                    }
                    None if prompt_sent
                        && saw_turn_content
                        && last_meaningful_output_at.elapsed() >= idle_timeout =>
                    {
                        let _ = writer.write_all(b"/exit\r").and_then(|_| writer.flush());
                        exit_requested_at = Some(Instant::now());
                    }
                    None => {}
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                terminate_process_group(child.id());
                let _ = child.wait();
                remove_active_process(&state, work);
                emit_command_execution_completed(
                    &output,
                    work,
                    Some(child.id()),
                    false,
                    &command_output,
                    None,
                    elapsed_millis(started),
                );
                return ClaudeRunResult {
                    text: String::new(),
                    error: Some(format!("failed to read Claude Code PTY: {}", err)),
                    duration_ms: elapsed_millis(started),
                    tool_items: Vec::new(),
                    agent_item_streamed: false,
                    latest_token_usage_info: None,
                };
            }
        }
    };

    remove_active_process(&state, work);
    let duration_ms = elapsed_millis(started);
    let success = status.as_ref().is_some_and(|status| status.success());
    let exit_code = status.as_ref().and_then(|status| status.code());
    emit_command_execution_completed(
        &output,
        work,
        Some(child.id()),
        success,
        &command_output,
        exit_code,
        duration_ms,
    );

    let final_text = latest_claude_transcript_assistant_text(work)
        .unwrap_or_else(|| clean_interactive_cli_output(&raw_output, &work.prompt));
    let mut emitted_text = String::new();
    let mut agent_item_started = false;
    if !final_text.is_empty() {
        emit_agent_delta(
            &output,
            &work.thread_id,
            &work.turn_id,
            &work.agent_item_id,
            &mut agent_item_started,
            &mut emitted_text,
            &final_text,
        );
    }

    if success {
        ClaudeRunResult {
            text: final_text,
            error: None,
            duration_ms,
            tool_items: Vec::new(),
            agent_item_streamed: !emitted_text.is_empty(),
            latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
        }
    } else {
        ClaudeRunResult {
            text: final_text.clone(),
            error: Some(non_empty_join(
                &[
                    status
                        .as_ref()
                        .map(|status| format!("Claude Code exited with status {}", status))
                        .unwrap_or_else(|| "Claude Code did not exit cleanly".to_string()),
                    final_text,
                ],
                "\n",
            )),
            duration_ms,
            tool_items: Vec::new(),
            agent_item_streamed: !emitted_text.is_empty(),
            latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
        }
    }
}

#[cfg(unix)]
fn run_claude_code_turn_piped<W>(
    mut command: Command,
    work: &TurnWork,
    state: SharedState,
    output: SharedOutput<W>,
    started: Instant,
    pty_error: std::io::Error,
) -> ClaudeRunResult
where
    W: Write,
{
    eprintln!(
        "[codexl-claude-code] failed to start PTY, falling back to pipes: {}",
        pty_error
    );
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ClaudeRunResult {
                text: String::new(),
                error: Some(format!("failed to launch Claude Code: {}", err)),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    emit_command_execution_started(&output, work, Some(child.id()));
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .insert((work.thread_id.clone(), work.turn_id.clone()), child.id());
    }

    let mut child_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            terminate_process_group(child.id());
            let _ = child.wait();
            if let Ok(mut state) = lock_state(&state) {
                state
                    .active_processes
                    .remove(&(work.thread_id.clone(), work.turn_id.clone()));
            }
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to open Claude Code stdin".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_process_group(child.id());
            let _ = child.wait();
            if let Ok(mut state) = lock_state(&state) {
                state
                    .active_processes
                    .remove(&(work.thread_id.clone(), work.turn_id.clone()));
            }
            return ClaudeRunResult {
                text: String::new(),
                error: Some("failed to capture Claude Code stdout".to_string()),
                duration_ms: elapsed_millis(started),
                tool_items: Vec::new(),
                agent_item_streamed: false,
                latest_token_usage_info: None,
            };
        }
    };
    let stderr_handle = child.stderr.take().map(|stderr| {
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            text
        })
    });

    if let Err(err) = child_stdin
        .write_all(work.prompt.as_bytes())
        .and_then(|_| child_stdin.write_all(b"\n"))
        .and_then(|_| child_stdin.flush())
    {
        terminate_process_group(child.id());
        let _ = child.wait();
        if let Ok(mut state) = lock_state(&state) {
            state
                .active_processes
                .remove(&(work.thread_id.clone(), work.turn_id.clone()));
        }
        return ClaudeRunResult {
            text: String::new(),
            error: Some(format!(
                "failed to write prompt to Claude Code stdin: {}",
                err
            )),
            duration_ms: elapsed_millis(started),
            tool_items: Vec::new(),
            agent_item_streamed: false,
            latest_token_usage_info: None,
        };
    }
    drop(child_stdin);

    let mut emitted_text = String::new();
    let mut agent_item_started = false;
    let mut command_output = String::new();
    let mut raw_stdout = String::new();
    let mut reader = stdout;
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let chunk = String::from_utf8_lossy(&buffer[..size]).to_string();
                raw_stdout.push_str(&chunk);
                emit_command_execution_output_delta(
                    &output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.cli_item_id,
                    &chunk,
                    &work.prompt,
                    &mut command_output,
                );
                let text = clean_interactive_cli_output(&raw_stdout, &work.prompt);
                emit_agent_delta(
                    &output,
                    &work.thread_id,
                    &work.turn_id,
                    &work.agent_item_id,
                    &mut agent_item_started,
                    &mut emitted_text,
                    &text,
                );
            }
            Err(err) => {
                terminate_process_group(child.id());
                let _ = child.wait();
                if let Ok(mut state) = lock_state(&state) {
                    state
                        .active_processes
                        .remove(&(work.thread_id.clone(), work.turn_id.clone()));
                }
                let agent_item_streamed = !emitted_text.is_empty();
                return ClaudeRunResult {
                    text: emitted_text,
                    error: Some(format!("failed to read Claude Code stdout: {}", err)),
                    duration_ms: elapsed_millis(started),
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                };
            }
        }
    }

    let status = child.wait();
    if let Ok(mut state) = lock_state(&state) {
        state
            .active_processes
            .remove(&(work.thread_id.clone(), work.turn_id.clone()));
    }
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let cleaned_stdout = clean_interactive_cli_output(&raw_stdout, &work.prompt);
    let cleaned_stderr = clean_interactive_cli_output(&stderr, &work.prompt);
    let final_text = latest_claude_transcript_assistant_text(work).unwrap_or_else(|| {
        if !cleaned_stdout.is_empty() {
            cleaned_stdout.clone()
        } else if !cleaned_stderr.is_empty() {
            cleaned_stderr.clone()
        } else {
            emitted_text.clone()
        }
    });
    if emitted_text.is_empty() && !final_text.is_empty() {
        emit_agent_delta(
            &output,
            &work.thread_id,
            &work.turn_id,
            &work.agent_item_id,
            &mut agent_item_started,
            &mut emitted_text,
            &final_text,
        );
    }

    let agent_item_streamed = !emitted_text.is_empty();
    let duration_ms = elapsed_millis(started);
    match status {
        Ok(status) => {
            emit_command_execution_completed(
                &output,
                work,
                Some(child.id()),
                status.success(),
                &command_output,
                status.code(),
                duration_ms,
            );
            if status.success() {
                ClaudeRunResult {
                    text: if emitted_text.is_empty() {
                        final_text
                    } else {
                        emitted_text
                    },
                    error: None,
                    duration_ms,
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                }
            } else {
                ClaudeRunResult {
                    text: emitted_text,
                    error: Some(non_empty_join(
                        &[
                            format!("Claude Code exited with status {}", status),
                            cleaned_stderr,
                            final_text,
                        ],
                        "\n",
                    )),
                    duration_ms,
                    tool_items: Vec::new(),
                    agent_item_streamed,
                    latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
                }
            }
        }
        Err(err) => {
            emit_command_execution_completed(
                &output,
                work,
                Some(child.id()),
                false,
                &command_output,
                None,
                duration_ms,
            );
            ClaudeRunResult {
                text: emitted_text,
                error: Some(format!("failed to wait for Claude Code: {}", err)),
                duration_ms,
                tool_items: Vec::new(),
                agent_item_streamed,
                latest_token_usage_info: latest_claude_transcript_token_usage_info(work),
            }
        }
    }
}

fn claude_command(work: &TurnWork) -> Command {
    let bin = std::env::var(BIN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ccr".to_string());
    let mut command = Command::new(&bin);
    configure_claude_path_env(&mut command, &bin);
    for key in CLAUDE_CHILD_ENV_REMOVALS {
        command.env_remove(key);
    }
    for arg in env_args(BASE_ARGS_ENV, &["code"]) {
        command.arg(arg);
    }
    for arg in CLAUDE_STREAM_JSON_ARGS {
        command.arg(arg);
    }
    if work.resume_existing {
        command.arg("--resume").arg(&work.claude_session_id);
    } else {
        command.arg("--session-id").arg(&work.claude_session_id);
    }
    if let Some(model) = claude_model_arg() {
        command.arg("--model").arg(model);
    }
    if let Some(permission_mode) = std::env::var(PERMISSION_MODE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        command.arg("--permission-mode").arg(permission_mode);
    }
    if let Some(permission_prompt_tool) = claude_permission_prompt_tool_arg() {
        command
            .arg("--permission-prompt-tool")
            .arg(permission_prompt_tool);
    }
    for arg in claude_code_capability_args(work, true) {
        command.arg(arg);
    }
    for arg in env_args(EXTRA_ARGS_ENV, &[]) {
        command.arg(arg);
    }
    command
}

fn remove_active_process(state: &SharedState, work: &TurnWork) {
    if let Ok(mut state) = lock_state(state) {
        state
            .active_processes
            .remove(&(work.thread_id.clone(), work.turn_id.clone()));
    }
}

fn turn_was_interrupted(state: &SharedState, work: &TurnWork) -> bool {
    lock_state(state)
        .map(|state| {
            state
                .interrupted_turns
                .contains(&(work.thread_id.clone(), work.turn_id.clone()))
        })
        .unwrap_or(false)
}

fn claude_turn_idle_timeout() -> Duration {
    std::env::var(TURN_IDLE_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_TURN_IDLE_TIMEOUT_MS))
}

fn claude_permission_approval_timeout() -> Duration {
    std::env::var(PERMISSION_APPROVAL_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_PERMISSION_APPROVAL_TIMEOUT_MS))
}

fn claude_permission_prompt_tool_arg() -> Option<String> {
    let value = std::env::var(PERMISSION_PROMPT_TOOL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_PERMISSION_PROMPT_TOOL.to_string());
    let normalized = value.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "0" | "false" | "off" | "no" | "none" | "disabled"
    ) {
        None
    } else {
        Some(value)
    }
}

#[cfg(unix)]
fn open_unix_pty() -> std::io::Result<(File, File)> {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    let mut size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(size),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    set_cloexec(master)?;
    set_cloexec(slave)?;
    Ok(unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) })
}

#[cfg(unix)]
fn set_cloexec(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn set_nonblocking(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn claude_model_arg() -> Option<String> {
    std::env::var(MODEL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != DEFAULT_MODEL)
}

fn should_send_prompt_to_claude(
    started: Instant,
    trust_confirmed_at: Option<Instant>,
    raw_output: &str,
    last_raw_output_at: Instant,
) -> bool {
    if last_raw_output_at.elapsed() < Duration::from_millis(600) {
        return false;
    }
    let input_ready = looks_like_claude_input_ready(raw_output);
    if let Some(confirmed_at) = trust_confirmed_at {
        return (input_ready && confirmed_at.elapsed() >= Duration::from_millis(500))
            || confirmed_at.elapsed() >= Duration::from_millis(10_000);
    }
    if looks_like_claude_trust_prompt(raw_output) {
        return false;
    }
    (input_ready && started.elapsed() >= Duration::from_millis(500))
        || started.elapsed() >= Duration::from_millis(10_000)
}

fn looks_like_claude_input_ready(raw_output: &str) -> bool {
    let plain = strip_ansi_and_control(raw_output).to_lowercase();
    plain.contains("welcome back")
        || plain.contains("tips for getting started")
        || plain.contains("run /init")
        || plain.contains("/effort")
        || plain.contains("claude code v")
}

fn configure_claude_path_env(command: &mut Command, ccr_bin: &str) {
    if let Some(path) = std::env::var(CLAUDE_PATH_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        command.env(CLAUDE_PATH_ENV, path);
        return;
    }

    if std::env::var(CLAUDE_PATH_ENV)
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return;
    }

    if let Some(path) = resolve_claude_path_for_ccr(ccr_bin) {
        command.env(CLAUDE_PATH_ENV, path);
        command.env("CLAUDE_CODE_INSTALLED_VIA_NPM_WRAPPER", "1");
    }
}

fn resolve_claude_path_for_ccr(ccr_bin: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    candidates.extend(resolve_executable_paths("claude"));
    for ccr_path in resolve_executable_paths(ccr_bin) {
        candidates.extend(claude_candidates_near_ccr(&ccr_path));
    }

    let mut seen = BTreeSet::new();
    candidates.into_iter().find(|candidate| {
        let key = candidate.to_string_lossy().to_string();
        seen.insert(key) && is_probably_usable_claude_path(candidate)
    })
}

fn claude_candidates_near_ccr(ccr_path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(bin_dir) = ccr_path.parent() else {
        return candidates;
    };

    candidates.push(bin_dir.join("claude"));
    candidates.extend(hidden_bin_candidates(bin_dir));

    if let Some(global_modules) = node_global_modules_from_bin_dir(bin_dir) {
        candidates.extend(claude_candidates_from_global_modules(&global_modules));
    }

    candidates
}

fn hidden_bin_candidates(bin_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(bin_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with(".claude") {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    candidates
}

fn node_global_modules_from_bin_dir(bin_dir: &Path) -> Option<PathBuf> {
    (bin_dir.file_name().and_then(|value| value.to_str()) == Some("bin")).then(|| {
        bin_dir
            .parent()
            .unwrap_or(bin_dir)
            .join("lib")
            .join("node_modules")
    })
}

fn claude_candidates_from_global_modules(global_modules: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let anthropic_dir = global_modules.join("@anthropic-ai");
    for package_dir in anthropic_claude_package_dirs(&anthropic_dir) {
        candidates.push(package_dir.join("bin").join("claude.exe"));
        if let Some(native_package) = native_claude_package_name() {
            candidates.push(
                package_dir
                    .join("node_modules")
                    .join("@anthropic-ai")
                    .join(native_package)
                    .join(if cfg!(windows) {
                        "claude.exe"
                    } else {
                        "claude"
                    }),
            );
        }
    }
    candidates
}

fn anthropic_claude_package_dirs(anthropic_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let primary = anthropic_dir.join("claude-code");
    if primary.is_dir() {
        dirs.push(primary);
    }
    if let Ok(entries) = std::fs::read_dir(anthropic_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with(".claude-code-") && path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs.sort();
    dirs
}

fn native_claude_package_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("claude-code-darwin-arm64"),
        ("macos", "x86_64") => Some("claude-code-darwin-x64"),
        ("linux", "x86_64") => Some("claude-code-linux-x64"),
        ("linux", "aarch64") => Some("claude-code-linux-arm64"),
        ("windows", "x86_64") => Some("claude-code-win32-x64"),
        ("windows", "aarch64") => Some("claude-code-win32-arm64"),
        _ => None,
    }
}

fn resolve_executable_paths(program: &str) -> Vec<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return vec![path.to_path_buf()];
    }

    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(program))
                .collect()
        })
        .unwrap_or_default()
}

fn is_probably_usable_claude_path(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || !is_executable_file(&metadata) {
        return false;
    }
    if is_claude_code_placeholder(path) {
        return false;
    }

    let path_text = path.to_string_lossy();
    if path_text.contains("@anthropic-ai/claude-code") && metadata.len() < MIN_NATIVE_CLAUDE_BYTES {
        return false;
    }

    true
}

fn is_claude_code_placeholder(path: &Path) -> bool {
    let Ok(contents) = std::fs::read(path) else {
        return false;
    };
    let prefix = &contents[..contents.len().min(1024)];
    String::from_utf8_lossy(prefix).contains("claude native binary not installed")
}

#[cfg(unix)]
fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    let _ = metadata;
    true
}

fn env_args(name: &str, default: &[&str]) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|value| split_env_args(&value))
        .unwrap_or_else(|| default.iter().map(|value| value.to_string()).collect())
}

fn split_env_args(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn emit_command_execution_started<W>(output: &SharedOutput<W>, work: &TurnWork, pid: Option<u32>)
where
    W: Write,
{
    let _ = write_notification(
        output,
        json!({
            "method": "item/started",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "item": command_execution_item(work, pid, "inProgress", Value::Null, Value::Null, Value::Null),
                "startedAtMs": now_millis(),
            },
        }),
    );
}

fn emit_command_execution_completed<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    pid: Option<u32>,
    success: bool,
    aggregated_output: &str,
    exit_code: Option<i32>,
    duration_ms: i64,
) where
    W: Write,
{
    let status = if success { "completed" } else { "failed" };
    let _ = write_notification(
        output,
        json!({
            "method": "item/completed",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "item": command_execution_item(
                    work,
                    pid,
                    status,
                    json!(truncate_for_protocol(aggregated_output, 200_000)),
                    exit_code.map(Value::from).unwrap_or(Value::Null),
                    json!(duration_ms),
                ),
                "completedAtMs": now_millis(),
            },
        }),
    );
}

fn emit_command_execution_output_delta<W>(
    output: &SharedOutput<W>,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    raw_delta: &str,
    prompt: &str,
    aggregated_output: &mut String,
) -> bool
where
    W: Write,
{
    let delta = clean_command_output_delta(raw_delta, prompt);
    if delta.trim().is_empty() {
        return false;
    }
    aggregated_output.push_str(&delta);
    let _ = write_notification(
        output,
        json!({
            "method": "item/commandExecution/outputDelta",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "itemId": item_id,
                "delta": delta,
            },
        }),
    );
    true
}

fn emit_command_execution_structured_delta<W>(
    output: &SharedOutput<W>,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    delta: &str,
    aggregated_output: &mut String,
) -> bool
where
    W: Write,
{
    if delta.trim().is_empty() {
        return false;
    }
    aggregated_output.push_str(delta);
    let _ = write_notification(
        output,
        json!({
            "method": "item/commandExecution/outputDelta",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "itemId": item_id,
                "delta": delta,
            },
        }),
    );
    true
}

fn command_execution_item(
    work: &TurnWork,
    pid: Option<u32>,
    status: &str,
    aggregated_output: Value,
    exit_code: Value,
    duration_ms: Value,
) -> Value {
    let command = claude_command_display(work);
    json!({
        "type": "commandExecution",
        "id": work.cli_item_id,
        "command": command,
        "cwd": work.cwd,
        "processId": pid.map(|value| value.to_string()),
        "source": "agent",
        "status": status,
        "commandActions": [
            {
                "type": "unknown",
                "command": command,
            }
        ],
        "aggregatedOutput": aggregated_output,
        "exitCode": exit_code,
        "durationMs": duration_ms,
    })
}

fn claude_command_display(work: &TurnWork) -> String {
    let bin = std::env::var(BIN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ccr".to_string());
    let mut parts = vec![bin];
    parts.extend(env_args(BASE_ARGS_ENV, &["code"]));
    parts.extend(CLAUDE_STREAM_JSON_ARGS.iter().map(|arg| arg.to_string()));
    if work.resume_existing {
        parts.push("--resume".to_string());
    } else {
        parts.push("--session-id".to_string());
    }
    parts.push(work.claude_session_id.clone());
    if let Some(model) = claude_model_arg() {
        parts.push("--model".to_string());
        parts.push(model);
    }
    if let Some(permission_mode) = std::env::var(PERMISSION_MODE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push("--permission-mode".to_string());
        parts.push(permission_mode);
    }
    if let Some(permission_prompt_tool) = claude_permission_prompt_tool_arg() {
        parts.push("--permission-prompt-tool".to_string());
        parts.push(permission_prompt_tool);
    }
    parts.extend(claude_code_capability_args(work, false));
    parts.extend(env_args(EXTRA_ARGS_ENV, &[]));
    parts
        .into_iter()
        .map(|part| shell_display_token(&part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_display_token(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn latest_claude_transcript_assistant_text(work: &TurnWork) -> Option<String> {
    let path = claude_transcript_path(work)?;
    let transcript = std::fs::read_to_string(path).ok()?;
    latest_assistant_text_from_transcript(&transcript)
}

fn latest_claude_transcript_generated_title(work: &TurnWork) -> Option<ClaudeGeneratedTitle> {
    let path = claude_transcript_path(work)?;
    load_claude_generated_title_from_transcript_path(&path)
}

fn latest_claude_transcript_token_usage_info(work: &TurnWork) -> Option<Value> {
    let path = claude_transcript_path(work)?;
    let transcript = std::fs::read_to_string(path).ok()?;
    let mut latest = None;
    let mut model = DEFAULT_MODEL.to_string();
    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if let Some(message_model) = claude_model_from_message(&value) {
            model = message_model.to_string();
        }
        if let Some(info) = claude_token_usage_info_from_message(&value, &model) {
            latest = Some(info);
        }
    }
    latest
}

fn claude_transcript_path(work: &TurnWork) -> Option<PathBuf> {
    let projects_dir = claude_projects_dir()?;
    let filename = format!("{}.jsonl", work.claude_session_id);
    for dir_name in claude_project_dir_candidates(&work.cwd) {
        let path = projects_dir.join(dir_name).join(&filename);
        if path.is_file() {
            return Some(path);
        }
    }
    std::fs::read_dir(projects_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&filename))
        .find(|path| path.is_file())
}

fn claude_project_dir_candidates(cwd: &str) -> Vec<String> {
    let mut paths = Vec::new();
    paths.push(PathBuf::from(cwd));
    if let Ok(canonical) = std::fs::canonicalize(cwd) {
        paths.push(canonical);
    }
    if cwd.starts_with("/var/") {
        paths.push(PathBuf::from(format!("/private{}", cwd)));
    }

    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .map(|path| claude_project_dir_name(&path))
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

fn claude_project_dir_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':') {
                '-'
            } else {
                ch
            }
        })
        .collect()
}

fn latest_assistant_text_from_transcript(transcript: &str) -> Option<String> {
    transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| assistant_text_from_transcript_entry(&value))
        .last()
}

fn assistant_text_from_transcript_entry(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let content = message.get("content")?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    item.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn clean_command_output_delta(raw_delta: &str, prompt: &str) -> String {
    let prompt_compact = compact_cli_text(prompt);
    let lines = strip_ansi_and_control(raw_delta)
        .lines()
        .map(normalize_cli_line)
        .filter(|line| !line.is_empty())
        .filter(|line| !is_cli_chrome_line(line))
        .filter(|line| !is_claude_noise_line(line))
        .filter(|line| {
            let compact = compact_plain_text(line);
            !compact.is_empty() && (prompt_compact.is_empty() || !compact.contains(&prompt_compact))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn looks_like_claude_trust_prompt(raw: &str) -> bool {
    let compact = compact_cli_text(raw);
    compact.contains("quicksafetycheck")
        || compact.contains("yes,itrustthisfolder")
        || compact.contains("no,exit")
}

fn compact_cli_text(raw: &str) -> String {
    compact_plain_text(&strip_ansi_and_control(raw))
}

fn compact_plain_text(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_claude_noise_line(line: &str) -> bool {
    let compact = compact_plain_text(line);
    let has_spinner = line.chars().any(|ch| {
        matches!(
            ch,
            '✻' | '✽'
                | '✶'
                | '✳'
                | '✢'
                | '·'
                | '◐'
                | '◓'
                | '◑'
                | '◒'
                | '⠋'
                | '⠙'
                | '⠹'
                | '⠸'
                | '⠼'
                | '⠴'
                | '⠦'
                | '⠧'
                | '⠇'
                | '⠏'
        )
    });
    let has_banner_block = line
        .chars()
        .any(|ch| matches!(ch, '▐' | '▛' | '█' | '▜' | '▌' | '▝' | '▘'));
    let has_status_glyph = line.chars().any(|ch| matches!(ch, '󰉋' | '' | '󰚩' | '⚡'));
    let has_cjk = line.chars().any(is_cjk);
    compact.is_empty()
        || (has_spinner && compact.chars().count() <= 40)
        || (has_spinner && compact.contains("brewing"))
        || (has_spinner && compact.contains("drizzling"))
        || (has_spinner && compact.contains("bakedfor"))
        || (has_banner_block && !has_cjk)
        || (has_status_glyph && !has_cjk)
        || (compact.chars().count() <= 2 && !has_cjk)
        || compact.chars().all(|ch| ch.is_ascii_digit())
        || compact.contains("claudecodev")
        || compact.contains("welcomeback")
        || compact.contains("tipsforgettingstarted")
        || compact.contains("what'snew")
        || compact.contains("opus4")
        || compact.contains("1mcontext")
        || compact.contains("internalinfrastructureimprovements")
        || compact.contains("release-notes")
        || compact.contains("/usage")
        || compact.contains("/diff")
        || compact.contains("apiusagebilling")
        || compact.contains("/effort")
        || compact.contains("tok/s")
        || compact.contains("tokens)")
        || compact.contains("↑")
        || compact.contains("↓")
        || compact.contains("brewing")
        || compact.contains("drizzling")
        || compact.contains("bakedfor")
        || compact.contains("auto-updating")
        || compact.contains("auto-updatefailed")
        || compact.contains("accessingworkspace")
        || compact.contains("quicksafetycheck")
        || compact.contains("securityguide")
        || compact.contains("securityguid")
        || compact.contains("project,orworkfromyourteam")
        || compact.contains("ifnot,takeamomenttoreview")
        || compact.contains("claudecode'llbeabletoread")
        || compact.contains("edit,andexecutefileshere")
        || compact.contains("doyoutrustthefiles")
        || compact.contains("yes,itrustthisfolder")
        || compact.contains("no,exit")
        || compact.contains("entertoconfirm")
        || compact.contains("esctocancel")
        || compact.contains("pressctrl-d")
        || compact.contains("resumethissessionwith:")
        || compact.starts_with("claude--resume")
        || compact.starts_with("ccrcode--resume")
        || compact.contains("codexl-claude-code-")
        || compact.contains("coxl-claude-code-")
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{3000}'..='\u{303F}'
            | '\u{FF00}'..='\u{FFEF}'
    )
}

fn truncate_for_protocol(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut start = value.len().saturating_sub(max_bytes);
    while !value.is_char_boundary(start) {
        start += 1;
    }
    format!("[truncated]\n{}", &value[start..])
}

fn emit_agent_delta<W>(
    output: &SharedOutput<W>,
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    item_started: &mut bool,
    emitted_text: &mut String,
    next_text: &str,
) where
    W: Write,
{
    let delta = if next_text.starts_with(emitted_text.as_str()) {
        next_text[emitted_text.len()..].to_string()
    } else if emitted_text.is_empty() {
        next_text.to_string()
    } else if next_text != emitted_text {
        format!("\n\n{}", next_text)
    } else {
        String::new()
    };
    if delta.is_empty() {
        return;
    }
    if !*item_started {
        let _ = write_notification(
            output,
            json!({
                "method": "item/started",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "item": {
                        "type": "agentMessage",
                        "id": item_id,
                        "text": "",
                        "phase": Value::Null,
                        "memoryCitation": Value::Null,
                    },
                    "startedAtMs": now_millis(),
                },
            }),
        );
        *item_started = true;
    }
    emitted_text.push_str(&delta);
    let _ = write_notification(
        output,
        json!({
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "itemId": item_id,
                "delta": delta,
            },
        }),
    );
}

fn emit_reasoning_delta<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &mut ClaudeStreamState,
    delta: &str,
) where
    W: Write,
{
    if delta.is_empty() {
        return;
    }
    let item_id = reasoning_item_id_for_turn(&work.turn_id);
    if !stream.reasoning_item_started {
        let _ = write_notification(
            output,
            json!({
                "method": "item/started",
                "params": {
                    "threadId": work.thread_id,
                    "turnId": work.turn_id,
                    "item": {
                        "type": "reasoning",
                        "id": item_id,
                        "summary": [],
                        "content": [],
                    },
                    "startedAtMs": now_millis(),
                },
            }),
        );
        stream.reasoning_item_started = true;
    }
    stream.reasoning_text.push_str(delta);
    let _ = write_notification(
        output,
        json!({
            "method": "item/reasoning/textDelta",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "itemId": item_id,
                "delta": delta,
                "contentIndex": 0,
            },
        }),
    );
}

fn emit_reasoning_completed_if_started<W>(
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &ClaudeStreamState,
) where
    W: Write,
{
    if !stream.reasoning_item_started {
        return;
    }
    let item_id = reasoning_item_id_for_turn(&work.turn_id);
    let _ = write_notification(
        output,
        json!({
            "method": "item/completed",
            "params": {
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "item": {
                    "type": "reasoning",
                    "id": item_id,
                    "summary": [],
                    "content": if stream.reasoning_text.is_empty() {
                        json!([])
                    } else {
                        json!([stream.reasoning_text])
                    },
                },
                "completedAtMs": now_millis(),
            },
        }),
    );
}

fn user_item_id_for_turn(turn_id: &str) -> String {
    format!("user-{}", turn_id)
}

fn agent_item_id_for_turn(turn_id: &str) -> String {
    format!("agent-{}", turn_id)
}

fn reasoning_item_id_for_turn(turn_id: &str) -> String {
    format!("reasoning-{}", turn_id)
}

fn cli_item_id_for_turn(turn_id: &str) -> String {
    format!("claude-cli-{}", turn_id)
}

fn clean_interactive_cli_output(raw: &str, prompt: &str) -> String {
    let plain = strip_ansi_and_control(raw);
    let prompt_compact = compact_cli_text(prompt);
    let prompt_lines = prompt
        .lines()
        .map(normalize_cli_line)
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();
    let mut lines = Vec::new();
    for line in plain.lines().map(normalize_cli_line) {
        let compact = compact_plain_text(&line);
        if line.is_empty()
            || prompt_lines.contains(&line)
            || (!prompt_compact.is_empty() && compact.contains(&prompt_compact))
            || is_cli_chrome_line(&line)
            || is_claude_noise_line(&line)
        {
            continue;
        }
        if lines.last().map(String::as_str) != Some(line.as_str()) {
            lines.push(line);
        }
    }
    lines.join("\n")
}

fn normalize_cli_line(line: &str) -> String {
    let normalized = line
        .trim()
        .trim_matches(|ch| matches!(ch, '│' | '┃' | '║' | '╎' | '┆' | '╭' | '╮' | '╰' | '╯'))
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized
        .strip_prefix('⏺')
        .map(str::trim)
        .unwrap_or(&normalized)
        .to_string()
}

fn is_cli_chrome_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    line == ">"
        || line == "❯"
        || line == "..."
        || lower == "claude code"
        || lower.contains("esc to interrupt")
        || lower.contains("ctrl+c")
        || lower.contains("ctrl+d")
        || lower.contains("? for shortcuts")
        || lower.contains("press enter")
        || line.chars().all(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '─' | '━'
                        | '═'
                        | '╭'
                        | '╮'
                        | '╰'
                        | '╯'
                        | '┌'
                        | '┐'
                        | '└'
                        | '┘'
                        | '│'
                        | '┃'
                        | '║'
                        | '╎'
                        | '┆'
                        | '>'
                        | '_'
                        | '-'
                        | ' '
                        | '·'
                        | '•'
                        | '✻'
                        | '✽'
                        | '✶'
                        | '⠋'
                        | '⠙'
                        | '⠹'
                        | '⠸'
                        | '⠼'
                        | '⠴'
                        | '⠦'
                        | '⠧'
                        | '⠇'
                        | '⠏'
                        | '❯'
                        | '�'
                        | '▐'
                        | '▛'
                        | '█'
                        | '▜'
                        | '▌'
                        | '▝'
                        | '▘'
                )
        })
}

fn strip_ansi_and_control(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for seq_ch in chars.by_ref() {
                        if ('@'..='~').contains(&seq_ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut prev_escape = false;
                    for seq_ch in chars.by_ref() {
                        if seq_ch == '\u{7}' || (prev_escape && seq_ch == '\\') {
                            break;
                        }
                        prev_escape = seq_ch == '\x1b';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
        } else if ch == '\r' {
            output.push('\n');
        } else if ch == '\n' || ch == '\t' || !ch.is_control() {
            output.push(ch);
        }
    }
    output
}

fn thread_runtime_response(thread: &ClaudeThread, include_turns: bool) -> Value {
    json!({
        "thread": thread.to_json(include_turns),
        "model": thread.model,
        "modelProvider": PROVIDER_NAME,
        "serviceTier": Value::Null,
        "cwd": thread.cwd,
        "runtimeWorkspaceRoots": [],
        "instructionSources": [],
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "sandbox": { "type": "dangerFullAccess" },
        "activePermissionProfile": Value::Null,
        "reasoningEffort": Value::Null,
    })
}

fn config_read_response(params: &Value) -> Value {
    let layers = params
        .get("includeLayers")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| json!([]))
        .unwrap_or(Value::Null);
    json!({
        "config": {
            "model": Value::Null,
            "review_model": Value::Null,
            "model_context_window": Value::Null,
            "model_auto_compact_token_limit": Value::Null,
            "model_auto_compact_token_limit_scope": Value::Null,
            "model_provider": Value::Null,
            "approval_policy": Value::Null,
            "approvals_reviewer": Value::Null,
            "sandbox_mode": Value::Null,
            "sandbox_workspace_write": Value::Null,
            "forced_chatgpt_workspace_id": Value::Null,
            "forced_login_method": Value::Null,
            "web_search": Value::Null,
            "tools": Value::Null,
            "instructions": Value::Null,
            "developer_instructions": Value::Null,
            "compact_prompt": Value::Null,
            "model_reasoning_effort": Value::Null,
            "model_reasoning_summary": Value::Null,
            "model_verbosity": Value::Null,
            "service_tier": Value::Null,
            "analytics": Value::Null,
            "desktop": Value::Null,
        },
        "origins": {},
        "layers": layers,
    })
}

fn model_from_params(params: &Value) -> String {
    params
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            std::env::var(MODEL_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

fn prompt_from_input(input: &[Value]) -> String {
    let mut parts = Vec::new();
    for item in input {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("localImage") => {
                if let Some(path) = item.get("path").and_then(Value::as_str) {
                    parts.push(format!("[local image: {}]", path));
                }
            }
            Some("image") => {
                if let Some(url) = item.get("url").and_then(Value::as_str) {
                    parts.push(format!("[image: {}]", url));
                }
            }
            Some("mention") | Some("skill") => {
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    parts.push(format!("@{}", name));
                }
            }
            _ => {}
        }
    }
    let prompt = parts.join("\n\n").trim().to_string();
    if prompt.is_empty() {
        "(empty prompt)".to_string()
    } else {
        prompt
    }
}

fn normalize_cwd(value: Option<&str>) -> String {
    let path = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.to_string_lossy().to_string()
}

fn required_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required param: {}", key))
}

fn uuid_from_thread_id(value: &str) -> String {
    if is_uuid_like(value) {
        value.to_string()
    } else {
        new_uuid_v4()
    }
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn new_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    for byte in &mut bytes {
        *byte = rand::random::<u8>();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn trim_json_line(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n")
        .unwrap_or(line)
        .strip_suffix(b"\r")
        .unwrap_or_else(|| line.strip_suffix(b"\n").unwrap_or(line))
}

fn json_rpc_id_key(id: &Value) -> Option<String> {
    match id {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Number(_) | Value::Bool(_) => Some(id.to_string()),
        _ => None,
    }
}

fn take_app_response(state: &SharedState, request_id: &str) -> Option<Value> {
    lock_state(state)
        .ok()
        .and_then(|mut state| state.app_responses.remove(request_id))
}

fn lock_state(
    state: &SharedState,
) -> Result<std::sync::MutexGuard<'_, ClaudeAppServerState>, String> {
    state
        .lock()
        .map_err(|_| "claude-code app-server state mutex poisoned".to_string())
}

fn write_response<W: Write>(
    output: &SharedOutput<W>,
    id: Value,
    result: Value,
) -> Result<(), String> {
    write_json_line(output, &json!({ "id": id, "result": result }))
}

fn write_error<W: Write>(
    output: &SharedOutput<W>,
    id: Value,
    code: i64,
    message: String,
) -> Result<(), String> {
    write_json_line(
        output,
        &json!({
            "id": id,
            "error": {
                "code": code,
                "message": message,
            },
        }),
    )
}

fn write_notification<W: Write>(
    output: &SharedOutput<W>,
    notification: Value,
) -> Result<(), String> {
    write_json_line(output, &notification)
}

fn write_json_line<W: Write>(output: &SharedOutput<W>, value: &Value) -> Result<(), String> {
    let mut line = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    line.push(b'\n');
    let mut output = output
        .lock()
        .map_err(|_| "claude-code app-server stdout mutex poisoned".to_string())?;
    output
        .write_all(&line)
        .map_err(|err| format!("failed to write app-server stdout: {}", err))?;
    output
        .flush()
        .map_err(|err| format!("failed to flush app-server stdout: {}", err))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn elapsed_millis(started: Instant) -> i64 {
    started.elapsed().as_millis() as i64
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn non_empty_join(parts: &[String], separator: &str) -> String {
    let joined = parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(separator);
    if joined.is_empty() {
        "Claude Code failed".to_string()
    } else {
        joined
    }
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{}", pid))
        .status();
}

#[cfg(not(unix))]
fn terminate_process_group(pid: u32) {
    let _ = Command::new("taskkill")
        .arg("/PID")
        .arg(pid.to_string())
        .args(["/T", "/F"])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "codexl-claude-code-{}-{}-{}",
            name,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn claude_code_log_event_writes_configured_log_file() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_log_path = std::env::var_os(APP_SERVER_LOG_PATH_ENV);
        let root = test_dir("app-server-log");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let log_path = root.join("claude-code-app-server.log");
        std::env::set_var(APP_SERVER_LOG_PATH_ENV, &log_path);

        claude_code_log_event(
            "test_event",
            json!({
                "threadId": "thread",
            }),
        );

        let content = std::fs::read_to_string(&log_path).expect("read log file");
        let line = content.lines().last().expect("log line");
        let value = serde_json::from_str::<Value>(line).expect("parse log json");
        assert_eq!(
            value.get("event").and_then(Value::as_str),
            Some("test_event")
        );
        assert_eq!(
            value.get("threadId").and_then(Value::as_str),
            Some("thread")
        );

        restore_env(APP_SERVER_LOG_PATH_ENV, old_log_path);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_metadata_relay_injects_codex_turn_metadata_into_tool_calls() {
        let options = McpMetadataRelayOptions {
            server_name: "codex-computer-use".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            session_id: "session-1".to_string(),
            cwd: "/tmp/work".to_string(),
            command: "computer-use".to_string(),
            args: vec!["mcp".to_string()],
        };
        let output = inject_mcp_codex_turn_metadata(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_apps","arguments":{}}}"#,
            &options,
        );
        let value = serde_json::from_slice::<Value>(&output).expect("parse transformed json");

        assert_eq!(
            value.pointer("/params/_meta/x-codex-turn-metadata/type"),
            Some(&json!("thread-id"))
        );
        assert_eq!(
            value.pointer("/params/_meta/x-codex-turn-metadata/thread-id"),
            Some(&json!("thread-1"))
        );
        assert_eq!(
            value
                .pointer("/params/headers/x-codex-turn-metadata")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|metadata| metadata
                    .get("thread-id")
                    .and_then(Value::as_str)
                    .map(str::to_string)),
            Some("thread-1".to_string())
        );
    }

    #[test]
    fn computer_use_node_relay_routes_tool_calls_through_main_child() {
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT.contains("respondWithFallbackListApps(message"));
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT.contains("sendMainToolCall(transformed, message)"));
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT
            .contains("const DEFAULT_TOOL_CALL_TIMEOUT_MS = 90 * 1000;"));
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT
            .contains("const DEFAULT_GET_APP_STATE_TIMEOUT_MS = 20 * 1000;"));
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT.contains("fallbackGetAppStateResponse"));
        assert!(COMPUTER_USE_NODE_RELAY_SCRIPT.contains("restartMainChild(error)"));
        assert!(!COMPUTER_USE_NODE_RELAY_SCRIPT.contains("runToolCallWithFreshChild(message)"));
        assert!(!COMPUTER_USE_NODE_RELAY_SCRIPT.contains("spawnComputerUseChild(`tool-call-"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn computer_use_service_app_is_derived_from_client_command() {
        let root = test_dir("computer-use-service-path");
        let client = root
            .join("Codex Computer Use.app")
            .join("Contents")
            .join("SharedSupport")
            .join("SkyComputerUseClient.app")
            .join("Contents")
            .join("MacOS")
            .join("SkyComputerUseClient");
        std::fs::create_dir_all(client.parent().expect("client parent"))
            .expect("create client dir");

        let app = computer_use_service_app_from_client_command(&client.to_string_lossy())
            .expect("service app path");

        assert_eq!(app, root.join("Codex Computer Use.app"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn computer_use_plugin_mcp_prefers_global_codex_home_app() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("computer-use-global-home");
        let workspace_home = root.join(".codexl").join("codex-homes").join("Workspace");
        let workspace_mcp = workspace_home
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("computer-use")
            .join("1.0.0")
            .join(".mcp.json");
        let global_client = root
            .join(".codex")
            .join("computer-use")
            .join("Codex Computer Use.app")
            .join("Contents")
            .join("SharedSupport")
            .join("SkyComputerUseClient.app")
            .join("Contents")
            .join("MacOS")
            .join("SkyComputerUseClient");
        std::fs::create_dir_all(workspace_mcp.parent().expect("workspace mcp parent"))
            .expect("create workspace mcp parent");
        std::fs::create_dir_all(global_client.parent().expect("global client parent"))
            .expect("create global client parent");
        std::fs::write(
            &workspace_mcp,
            r#"{
  "mcpServers": {
    "computer-use": {
      "command": "./Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
      "args": ["mcp"],
      "cwd": "."
    }
  }
}"#,
        )
        .expect("write workspace mcp config");
        std::fs::write(&global_client, "").expect("write global computer use client");

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_codexl_home = std::env::var_os("CODEXL_CODEX_HOME");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &workspace_home);
        std::env::remove_var("CODEXL_CODEX_HOME");

        let servers = standalone_mcp_server_status_list();
        let computer_use = servers
            .iter()
            .find(|server| server.get("name").and_then(Value::as_str) == Some("computer-use"))
            .expect("computer-use server");

        assert_eq!(
            computer_use.get("command").and_then(Value::as_str),
            Some(global_client.to_string_lossy().as_ref())
        );
        assert_eq!(
            computer_use.get("cwd").and_then(Value::as_str),
            Some(
                root.join(".codex")
                    .join("computer-use")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(
            computer_use.get("source").and_then(Value::as_str),
            Some("plugin")
        );

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env("CODEXL_CODEX_HOME", old_codexl_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn interrupt_turn_falls_back_to_active_thread_turn() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (thread_response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, stale_processes) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");
        assert!(stale_processes.is_empty());
        state
            .active_processes
            .insert((work.thread_id.clone(), work.turn_id.clone()), 1234);

        let pid = state.interrupt_turn(&json!({
            "threadId": work.thread_id.clone(),
            "turnId": "stale-turn-id",
        }));

        assert_eq!(pid, Some(1234));
        assert!(state.interrupted_turns.contains(&(thread_id, work.turn_id)));
    }

    #[test]
    fn start_turn_interrupts_stale_active_process_for_same_thread() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (thread_response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, old_work, stale_processes) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "old" }],
            }))
            .expect("start old turn");
        assert!(stale_processes.is_empty());
        state
            .active_processes
            .insert((old_work.thread_id.clone(), old_work.turn_id.clone()), 4321);

        let (_, notifications, new_work, stale_processes) = state
            .start_turn(&json!({
                "threadId": old_work.thread_id.clone(),
                "input": [{ "type": "text", "text": "new" }],
            }))
            .expect("start new turn");

        assert_eq!(
            stale_processes,
            vec![StaleActiveProcess {
                thread_id: old_work.thread_id.clone(),
                turn_id: old_work.turn_id.clone(),
                pid: 4321,
            }]
        );
        assert!(!state
            .active_processes
            .contains_key(&(old_work.thread_id.clone(), old_work.turn_id.clone())));
        assert!(state
            .interrupted_turns
            .contains(&(old_work.thread_id.clone(), old_work.turn_id.clone())));
        let thread = state.threads.get(&old_work.thread_id).expect("thread");
        assert_eq!(thread.turns[0].status, TurnStatus::Interrupted);
        assert_eq!(thread.turns[1].id, new_work.turn_id);
        assert_eq!(thread.turns[1].status, TurnStatus::InProgress);
        let snapshot = notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("thread stream snapshot");
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/0/status")
                .and_then(Value::as_str),
            Some("interrupted")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/1/status")
                .and_then(Value::as_str),
            Some("inProgress")
        );
    }

    #[test]
    fn initialize_response_includes_codex_app_required_fields() {
        let root = test_dir("initialize");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let output_path = root.join("out.jsonl");
        let input = b"{\"id\":\"1\",\"method\":\"initialize\",\"params\":{}}\n{\"method\":\"initialized\"}\n{\"id\":\"2\",\"method\":\"config/read\",\"params\":{\"includeLayers\":true}}\n";

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let output = std::fs::read_to_string(&output_path).expect("read output");
        let first_line = output.lines().next().expect("initialize response");
        let response: Value = serde_json::from_str(first_line).expect("json response");
        let result = response.get("result").expect("response result");
        assert_eq!(
            result.get("userAgent").and_then(Value::as_str).is_some(),
            true
        );
        assert_eq!(
            result.get("codexHome").and_then(Value::as_str).is_some(),
            true
        );
        assert_eq!(
            result.get("platformFamily").and_then(Value::as_str),
            Some(std::env::consts::FAMILY)
        );
        assert_eq!(
            result.get("platformOs").and_then(Value::as_str),
            Some(std::env::consts::OS)
        );
        let second_line = output.lines().nth(1).expect("config/read response");
        let config_response: Value = serde_json::from_str(second_line).expect("json response");
        let config_result = config_response.get("result").expect("config/read result");
        assert!(config_result
            .get("config")
            .and_then(Value::as_object)
            .is_some());
        assert_eq!(config_result.get("layers"), Some(&json!([])));
        assert_eq!(output.lines().count(), 2);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_app_capability_methods_work_without_codex_cli() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("standalone-capabilities");
        let codex_home = root.join(".codex");
        let output_path = root.join("out.jsonl");
        let skill_dir = codex_home.join("skills").join("demo-skill");
        let plugin_package_dir = codex_home.join("plugins").join("demo-plugin");
        let plugin_dir = plugin_package_dir.join(".codex-plugin");
        let plugin_skill_dir = plugin_package_dir.join("skills").join("plugin-skill");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        std::fs::create_dir_all(&plugin_skill_dir).expect("create plugin skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: "demo-skill"
description: "Demo skill from CODEX_HOME."
---

# Demo Skill

Use this skill for standalone tests.
"#,
        )
        .expect("write skill");
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
  "id": "demo.plugin",
  "name": "Demo Plugin",
  "description": "Demo plugin from CODEX_HOME",
  "version": "1.2.3",
  "skills": "./skills/",
  "mcpServers": "./.mcp.json"
}"#,
        )
        .expect("write plugin");
        std::fs::write(
            plugin_skill_dir.join("SKILL.md"),
            r#"---
name: "plugin-skill"
description: "Demo plugin skill."
---

# Plugin Skill

Use this skill from the demo plugin.
"#,
        )
        .expect("write plugin skill");
        std::fs::write(
            plugin_package_dir.join(".mcp.json"),
            r#"{
  "mcpServers": {
    "computer-use": {
      "command": "./Computer Use.app/Contents/MacOS/ComputerUse",
      "args": ["mcp"],
      "cwd": "."
    }
  }
}"#,
        )
        .expect("write plugin mcp config");
        std::fs::write(
            codex_home.join("config.toml"),
            r#"
[mcp_servers.demo_mcp]
command = "node"
args = ["server.js", "--stdio"]
"#,
        )
        .expect("write config");

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_proxy = std::env::var_os(CODEX_APP_SERVER_PROXY_ENV);
        let old_computer_use_node = std::env::var_os(COMPUTER_USE_NODE_RELAY_NODE_ENV);
        let fake_node = root.join("node");
        std::fs::write(&fake_node, "").expect("write fake node");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &codex_home);
        std::env::set_var(CODEX_APP_SERVER_PROXY_ENV, "0");
        std::env::set_var(COMPUTER_USE_NODE_RELAY_NODE_ENV, &fake_node);

        let input = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"skills/list","params":{"cwd":root}}),
            json!({"id":"3","method":"mcpServerStatus/list","params":{}}),
            json!({"id":"4","method":"plugin/list","params":{}}),
            json!({"id":"5","method":"plugin/read","params":{"pluginName":"Demo Plugin","marketplacePath":plugin_dir}}),
            json!({"id":"6","method":"plugin/install","params":{"pluginName":"Demo Plugin","marketplacePath":plugin_dir}}),
            json!({"id":"7","method":"account/read","params":{}}),
            json!({"id":"8","method":"getAuthStatus","params":{"includeToken":true}})
        );
        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let responses = json_lines(&std::fs::read_to_string(&output_path).expect("read output"));
        let skills = response_by_id(&responses, "2")
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("skill data");
        assert!(skills
            .iter()
            .any(|skill| skill.get("name").and_then(Value::as_str) == Some("demo-skill")));
        let mcp_servers = response_by_id(&responses, "3")
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("mcp server data");
        assert!(mcp_servers
            .iter()
            .any(|server| server.get("name").and_then(Value::as_str) == Some("demo_mcp")));
        let computer_use = mcp_servers
            .iter()
            .find(|server| server.get("name").and_then(Value::as_str) == Some("computer-use"))
            .expect("computer-use plugin mcp server");
        assert_eq!(
            computer_use.get("source").and_then(Value::as_str),
            Some("plugin")
        );
        assert_eq!(
            computer_use
                .get("command")
                .and_then(Value::as_str)
                .map(|command| command.ends_with("Computer Use.app/Contents/MacOS/ComputerUse")),
            Some(true)
        );
        assert_eq!(
            computer_use.pointer("/args/0").and_then(Value::as_str),
            Some("mcp")
        );
        let plugins = response_by_id(&responses, "4")
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("plugin data");
        assert!(plugins
            .iter()
            .any(|plugin| plugin.get("name").and_then(Value::as_str) == Some("Demo Plugin")));
        let plugin_result = response_by_id(&responses, "4")
            .get("result")
            .expect("plugin result");
        let marketplaces = plugin_result
            .get("marketplaces")
            .and_then(Value::as_array)
            .expect("plugin marketplaces");
        assert!(marketplaces.iter().any(|marketplace| {
            marketplace.get("name").and_then(Value::as_str) == Some("filesystem")
                && marketplace
                    .get("plugins")
                    .and_then(Value::as_array)
                    .is_some_and(|plugins| {
                        plugins.iter().any(|plugin| {
                            plugin.get("name").and_then(Value::as_str) == Some("Demo Plugin")
                                && plugin.pointer("/source/type").and_then(Value::as_str)
                                    == Some("local")
                        })
                    })
        }));
        assert_eq!(
            plugin_result
                .get("marketplaceLoadErrors")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            plugin_result
                .get("featuredPluginIds")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        let plugin_detail = response_by_id(&responses, "5")
            .pointer("/result/plugin")
            .expect("plugin detail");
        assert_eq!(
            plugin_detail
                .pointer("/summary/name")
                .and_then(Value::as_str),
            Some("Demo Plugin")
        );
        assert_eq!(
            plugin_detail.get("marketplaceName").and_then(Value::as_str),
            Some("filesystem")
        );
        assert_eq!(
            plugin_detail
                .pointer("/skills/0/name")
                .and_then(Value::as_str),
            Some("Demo Plugin:plugin-skill")
        );
        assert_eq!(
            plugin_detail
                .pointer("/mcpServers/0")
                .and_then(Value::as_str),
            Some("computer-use")
        );
        assert_eq!(
            response_by_id(&responses, "6")
                .pointer("/result/authPolicy")
                .and_then(Value::as_str),
            Some("ON_INSTALL")
        );
        assert_eq!(
            response_by_id(&responses, "6")
                .pointer("/result/appsNeedingAuth")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        let account = response_by_id(&responses, "7")
            .pointer("/result/account")
            .expect("mock account");
        assert_eq!(account.get("type").and_then(Value::as_str), Some("chatgpt"));
        assert_eq!(
            account.get("email").and_then(Value::as_str),
            Some(PROVIDER_NAME)
        );
        assert_eq!(
            account.get("planType").and_then(Value::as_str),
            Some("unknown")
        );
        assert_eq!(
            response_by_id(&responses, "7").pointer("/result/requiresOpenaiAuth"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            response_by_id(&responses, "8").pointer("/result/authMethod"),
            Some(&json!("chatgpt"))
        );
        assert_eq!(
            response_by_id(&responses, "8").pointer("/result/authToken"),
            Some(&Value::Null)
        );
        assert_eq!(
            response_by_id(&responses, "8").pointer("/result/requiresOpenaiAuth"),
            Some(&Value::Bool(false))
        );

        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: root.to_string_lossy().to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let command = claude_command_display(&work);
        assert!(!command.contains("--plugin-dir"), "{command}");
        assert!(command.contains("--mcp-config"), "{command}");
        let mcp_config = claude_code_mcp_config_json(&work, false).expect("mcp config json");
        let mcp_config: Value = serde_json::from_str(&mcp_config).expect("parse mcp config");
        let servers = mcp_config
            .get("mcpServers")
            .and_then(Value::as_object)
            .expect("mcp servers");
        let computer_use = servers
            .get("codex-computer-use")
            .expect("computer use server");
        assert_eq!(
            computer_use.get("command").and_then(Value::as_str),
            Some(fake_node.to_string_lossy().as_ref())
        );
        let computer_use_args = computer_use
            .get("args")
            .and_then(Value::as_array)
            .expect("computer use relay args");
        assert!(computer_use_args
            .first()
            .and_then(Value::as_str)
            .is_some_and(|arg| arg.ends_with("codexl-computer-use-mcp-relay.cjs")));
        assert!(computer_use_args
            .iter()
            .any(|arg| arg.as_str() == Some("codex-computer-use")));
        assert!(computer_use_args
            .iter()
            .any(|arg| arg.as_str() == Some("--")));
        assert!(computer_use_args.iter().any(|arg| {
            arg.as_str()
                .is_some_and(|arg| arg.ends_with("Computer Use.app/Contents/MacOS/ComputerUse"))
        }));
        assert!(computer_use_args
            .iter()
            .any(|arg| arg.as_str() == Some("mcp")));
        assert_eq!(
            computer_use.pointer("/env/CODEX_SESSION_ID"),
            Some(&json!("11111111-1111-4111-8111-111111111111"))
        );
        assert_eq!(
            computer_use.pointer("/env/CODEX_TURN_ID"),
            Some(&json!("turn"))
        );
        assert_eq!(
            computer_use.pointer("/env/CODEX_THREAD_ID"),
            Some(&json!("thread"))
        );
        assert!(servers.get("computer-use").is_none());
        let title_command = claude_command_display(&TurnWork {
            thread_id: "title-thread".to_string(),
            turn_id: "title-turn".to_string(),
            agent_item_id: "title-agent".to_string(),
            cli_item_id: "title-cli".to_string(),
            claude_session_id: "22222222-2222-4222-8222-222222222222".to_string(),
            cwd: root.to_string_lossy().to_string(),
            prompt: "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nGenerate a concise UI title (up to 36 characters) for this task.\n\nUser prompt:\nhello".to_string(),
            resume_existing: false,
        });
        assert!(!title_command.contains("--mcp-config"), "{title_command}");

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env(CODEX_APP_SERVER_PROXY_ENV, old_proxy);
        restore_env(COMPUTER_USE_NODE_RELAY_NODE_ENV, old_computer_use_node);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claude_command_removes_env_vars_that_break_computer_use_mcp() {
        let _guard = ENV_TEST_LOCK.lock().expect("env lock poisoned");
        let old_env = CLAUDE_CHILD_ENV_REMOVALS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for key in CLAUDE_CHILD_ENV_REMOVALS {
            std::env::set_var(key, "1");
        }

        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "session".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let command = claude_command(&work);
        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for key in CLAUDE_CHILD_ENV_REMOVALS {
            assert_eq!(envs.get(*key), Some(&None), "{key}");
        }

        for (key, value) in old_env {
            restore_env(key, value);
        }
    }

    #[test]
    fn turn_lifecycle_emits_thread_stream_state_snapshots() {
        let root = test_dir("thread-stream-state");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let _ = state.start_thread(&json!({
            "cwd": root.to_string_lossy(),
            "model": "sonnet",
        }));
        let thread_id = state.threads.keys().next().expect("thread id").to_string();

        let (_, notifications, work, stale_processes) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "use computer" }],
            }))
            .expect("start turn");
        assert!(stale_processes.is_empty());
        let started_snapshot = notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("started thread stream snapshot");
        assert_eq!(
            started_snapshot
                .pointer("/params/change/conversationState/threadRuntimeStatus/type")
                .and_then(Value::as_str),
            Some("active")
        );
        assert_eq!(
            started_snapshot
                .pointer("/params/change/conversationState/turns/0/status")
                .and_then(Value::as_str),
            Some("inProgress")
        );

        let finished = state
            .finish_turn(
                &work.thread_id,
                &work.turn_id,
                ClaudeRunResult {
                    text: "done".to_string(),
                    error: None,
                    duration_ms: 12,
                    tool_items: Vec::new(),
                    agent_item_streamed: false,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish turn");
        assert_eq!(
            finished
                .thread_stream_state
                .as_ref()
                .expect("thread stream state")
                .get("method")
                .and_then(Value::as_str),
            Some("thread-stream-state-changed")
        );
        assert_eq!(
            finished
                .thread_stream_state
                .as_ref()
                .expect("thread stream state")
                .pointer("/params/change/conversationState/threadRuntimeStatus/type")
                .and_then(Value::as_str),
            Some("idle")
        );
        assert_eq!(
            finished
                .thread_stream_state
                .as_ref()
                .expect("thread stream state")
                .pointer("/params/change/conversationState/turns/0/status")
                .and_then(Value::as_str),
            Some("completed")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn plugin_methods_proxy_to_bundled_codex_app_server_when_available() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("plugin-proxy");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let fake_codex = root.join("codex");
        write_executable(
            &fake_codex,
            br#"#!/bin/sh
	while IFS= read -r line; do
	  case "$line" in
	    *'"method":"plugin/read"'*)
	      printf '%s\n' '{"id":"__codexl_claude_code_proxy_request__","result":{"plugin":{"summary":{"name":"proxied-plugin"},"skills":[],"hooks":[],"apps":[],"mcpServers":[]}}}'
	      ;;
	    *'"method":"extension/list"'*)
	      printf '%s\n' '{"id":"__codexl_claude_code_proxy_request__","result":{"data":[{"id":"proxied-extension"}],"nextCursor":null}}'
	      ;;
	    *'"method":"plugin/uninstall"'*)
	      printf '%s\n' '{"id":"__codexl_claude_code_proxy_request__","result":{"proxied":true}}'
	      ;;
	  esac
	done
	"#,
        );

        let old_proxy = std::env::var_os(CODEX_APP_SERVER_PROXY_ENV);
        let old_bundled = std::env::var_os("CODEXL_BUNDLED_CODEX_CLI_PATH");
        let old_real = std::env::var_os("CODEXL_REAL_CODEX_CLI_PATH");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        std::env::set_var(CODEX_APP_SERVER_PROXY_ENV, "1");
        std::env::set_var("CODEXL_BUNDLED_CODEX_CLI_PATH", &fake_codex);
        std::env::remove_var("CODEXL_REAL_CODEX_CLI_PATH");
        std::env::set_var("CODEX_HOME", &root);

        let result = standalone_codex_app_result(
            "plugin/read",
            &json!({"pluginName":"computer-use","marketplacePath":"openai-bundled"}),
        )
        .expect("plugin/read result");
        assert_eq!(
            result
                .pointer("/plugin/summary/name")
                .and_then(Value::as_str),
            Some("proxied-plugin")
        );
        let result = standalone_codex_app_result("extension/list", &json!({}))
            .expect("extension/list result");
        assert_eq!(
            result.pointer("/data/0/id").and_then(Value::as_str),
            Some("proxied-extension")
        );
        let result =
            standalone_codex_app_result("plugin/uninstall", &json!({"pluginId":"sample-plugin"}))
                .expect("plugin/uninstall result");
        assert_eq!(result.get("proxied").and_then(Value::as_bool), Some(true));
        for plugin_id in [
            "computer-use@openai-bundled",
            "browser-use@openai-bundled",
            "browser@openai-bundled",
        ] {
            let result =
                standalone_codex_app_result("plugin/uninstall", &json!({"pluginId": plugin_id}))
                    .expect("protected bundled plugin/uninstall result");
            assert!(result.as_object().is_some_and(Map::is_empty));
        }

        restore_env(CODEX_APP_SERVER_PROXY_ENV, old_proxy);
        restore_env("CODEXL_BUNDLED_CODEX_CLI_PATH", old_bundled);
        restore_env("CODEXL_REAL_CODEX_CLI_PATH", old_real);
        restore_env("CODEX_HOME", old_codex_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_goal_methods_have_standalone_empty_result() {
        for method in ["thread/goal/get", "thread/goal/set", "thread/goal/clear"] {
            let result = standalone_codex_app_result(method, &json!({ "threadId": "thread" }))
                .expect("thread goal result");
            assert!(result.get("goal").is_some_and(Value::is_null));
        }
    }

    #[test]
    fn plugin_proxy_uses_codex_home_with_plugin_cache() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("plugin-proxy-home");
        let global_plugin = root
            .join(".codex")
            .join("plugins")
            .join("demo")
            .join(".codex-plugin");
        let workspace_home = root.join(".codexl").join("codex-homes").join("Workspace");
        std::fs::create_dir_all(&global_plugin).expect("create global plugin dir");
        std::fs::create_dir_all(&workspace_home).expect("create workspace home");
        std::fs::write(
            global_plugin.join("plugin.json"),
            r#"{"id":"demo.plugin","name":"Demo Plugin","version":"1.0.0"}"#,
        )
        .expect("write plugin manifest");

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &workspace_home);

        assert_eq!(
            codex_cli_app_server_codex_home("plugin/list"),
            root.join(".codex").to_string_lossy().to_string()
        );
        assert_eq!(
            codex_cli_app_server_codex_home("thread/list"),
            workspace_home.to_string_lossy().to_string()
        );

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn plugin_list_proxy_merges_local_marketplaces_when_proxy_is_empty() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("plugin-list-proxy-merge");
        let global_plugin = root
            .join(".codex")
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("browser")
            .join("1.0.0")
            .join(".codex-plugin");
        let workspace_home = root.join(".codexl").join("codex-homes").join("Workspace");
        std::fs::create_dir_all(&global_plugin).expect("create global plugin dir");
        std::fs::create_dir_all(&workspace_home).expect("create workspace home");
        std::fs::write(
            global_plugin.join("plugin.json"),
            r#"{
  "id": "browser",
  "name": "Browser",
  "version": "1.0.0",
  "description": "Browser plugin"
}"#,
        )
        .expect("write plugin manifest");

        let fake_codex = root.join("codex");
        write_executable(
            &fake_codex,
            br#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"plugin/list"'*)
      printf '%s\n' '{"id":"__codexl_claude_code_proxy_request__","result":{"data":[],"marketplaces":[],"nextCursor":null}}'
      ;;
  esac
done
"#,
        );

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_proxy = std::env::var_os(CODEX_APP_SERVER_PROXY_ENV);
        let old_bundled = std::env::var_os("CODEXL_BUNDLED_CODEX_CLI_PATH");
        let old_real = std::env::var_os("CODEXL_REAL_CODEX_CLI_PATH");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &workspace_home);
        std::env::set_var(CODEX_APP_SERVER_PROXY_ENV, "1");
        std::env::set_var("CODEXL_BUNDLED_CODEX_CLI_PATH", &fake_codex);
        std::env::remove_var("CODEXL_REAL_CODEX_CLI_PATH");

        let result =
            standalone_codex_app_result("plugin/list", &json!({})).expect("plugin/list result");
        let plugins = result
            .get("data")
            .and_then(Value::as_array)
            .expect("plugin data");
        assert!(plugins
            .iter()
            .any(|plugin| plugin.get("name").and_then(Value::as_str) == Some("Browser")));
        let marketplaces = result
            .get("marketplaces")
            .and_then(Value::as_array)
            .expect("marketplaces");
        assert!(marketplaces.iter().any(|marketplace| {
            marketplace.get("name").and_then(Value::as_str) == Some("openai-bundled")
                && marketplace
                    .get("plugins")
                    .and_then(Value::as_array)
                    .is_some_and(|plugins| {
                        plugins.iter().any(|plugin| {
                            plugin.get("id").and_then(Value::as_str) == Some("browser")
                        })
                    })
        }));

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env(CODEX_APP_SERVER_PROXY_ENV, old_proxy);
        restore_env("CODEXL_BUNDLED_CODEX_CLI_PATH", old_bundled);
        restore_env("CODEXL_REAL_CODEX_CLI_PATH", old_real);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn plugin_list_proxy_keeps_local_protected_bundled_plugins_available() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("plugin-list-proxy-protected-bundled");
        let computer_use_plugin = root
            .join(".codex")
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("computer-use")
            .join("1.0.799")
            .join(".codex-plugin");
        let browser_use_plugin = root
            .join(".codex")
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("browser-use")
            .join("2.0.0")
            .join(".codex-plugin");
        let workspace_home = root.join(".codexl").join("codex-homes").join("Workspace");
        std::fs::create_dir_all(&computer_use_plugin).expect("create computer-use plugin dir");
        std::fs::create_dir_all(&browser_use_plugin).expect("create browser-use plugin dir");
        std::fs::create_dir_all(&workspace_home).expect("create workspace home");
        std::fs::write(
            computer_use_plugin.join("plugin.json"),
            r#"{
  "name": "computer-use",
  "version": "1.0.799",
  "description": "Computer Use plugin"
}"#,
        )
        .expect("write computer-use plugin manifest");
        std::fs::write(
            browser_use_plugin.join("plugin.json"),
            r#"{
  "name": "browser-use",
  "version": "2.0.0",
  "description": "Browser Use plugin"
}"#,
        )
        .expect("write browser-use plugin manifest");

        let fake_codex = root.join("codex");
        write_executable(
            &fake_codex,
            br#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"method":"plugin/list"'*)
      printf '%s\n' '{"id":"__codexl_claude_code_proxy_request__","result":{"data":[{"id":"computer-use@openai-bundled","name":"computer-use","installed":false,"enabled":false,"availability":"UNAVAILABLE","installPolicy":"UNAVAILABLE","source":{"type":"local","path":"/tmp/deleted-computer-use"}},{"id":"browser-use@openai-bundled","name":"browser-use","installed":false,"enabled":false,"availability":"UNAVAILABLE","installPolicy":"UNAVAILABLE","source":{"type":"local","path":"/tmp/deleted-browser-use"}}],"marketplaces":[{"name":"openai-bundled","path":"openai-bundled","plugins":[{"id":"computer-use@openai-bundled","name":"computer-use","installed":false,"enabled":false,"availability":"UNAVAILABLE","installPolicy":"UNAVAILABLE","source":{"type":"local","path":"/tmp/deleted-computer-use"}},{"id":"browser-use@openai-bundled","name":"browser-use","installed":false,"enabled":false,"availability":"UNAVAILABLE","installPolicy":"UNAVAILABLE","source":{"type":"local","path":"/tmp/deleted-browser-use"}}]}],"nextCursor":null}}'
      ;;
  esac
done
"#,
        );

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        let old_proxy = std::env::var_os(CODEX_APP_SERVER_PROXY_ENV);
        let old_bundled = std::env::var_os("CODEXL_BUNDLED_CODEX_CLI_PATH");
        let old_real = std::env::var_os("CODEXL_REAL_CODEX_CLI_PATH");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &workspace_home);
        std::env::set_var(CODEX_APP_SERVER_PROXY_ENV, "1");
        std::env::set_var("CODEXL_BUNDLED_CODEX_CLI_PATH", &fake_codex);
        std::env::remove_var("CODEXL_REAL_CODEX_CLI_PATH");

        let result =
            standalone_codex_app_result("plugin/list", &json!({})).expect("plugin/list result");
        let plugins = result
            .get("data")
            .and_then(Value::as_array)
            .expect("plugin data");
        for (plugin_name, local_version, deleted_path) in [
            ("computer-use", "1.0.799", "/tmp/deleted-computer-use"),
            ("browser-use", "2.0.0", "/tmp/deleted-browser-use"),
        ] {
            let plugin = plugins
                .iter()
                .find(|plugin| plugin_matches_name(plugin, plugin_name))
                .expect("protected bundled plugin");
            assert_eq!(plugin.get("installed").and_then(Value::as_bool), Some(true));
            assert_eq!(plugin.get("enabled").and_then(Value::as_bool), Some(true));
            assert_eq!(
                plugin.get("availability").and_then(Value::as_str),
                Some("AVAILABLE")
            );
            assert_eq!(
                plugin.get("localVersion").and_then(Value::as_str),
                Some(local_version)
            );
            assert_ne!(
                plugin.pointer("/source/path").and_then(Value::as_str),
                Some(deleted_path)
            );
        }

        let marketplaces = result
            .get("marketplaces")
            .and_then(Value::as_array)
            .expect("marketplaces");
        let marketplace_plugins = marketplaces
            .iter()
            .find(|marketplace| marketplace_is_openai_bundled(marketplace))
            .and_then(|marketplace| marketplace.get("plugins"))
            .and_then(Value::as_array)
            .expect("openai-bundled marketplace plugins");
        for plugin_name in ["computer-use", "browser-use"] {
            let plugin = marketplace_plugins
                .iter()
                .find(|plugin| plugin_matches_name(plugin, plugin_name))
                .expect("marketplace protected bundled plugin");
            assert_eq!(plugin.get("enabled").and_then(Value::as_bool), Some(true));
            assert_eq!(
                plugin.get("availability").and_then(Value::as_str),
                Some("AVAILABLE")
            );
        }

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env(CODEX_APP_SERVER_PROXY_ENV, old_proxy);
        restore_env("CODEXL_BUNDLED_CODEX_CLI_PATH", old_bundled);
        restore_env("CODEXL_REAL_CODEX_CLI_PATH", old_real);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn filters_claude_code_tui_noise_from_streamed_and_final_text() {
        let prompt = "该项目都有哪些功能";
        let raw = r#"
▐▛███▜▌ClaudeCodev2.1.149
▝▜█████▛▘Opus4.7(1Mcontext)withmediumeffort·APIUsageBilling
▘▘▝▝~/baishan/llm-spec
◐medium·/effort
❯ 该项目都有哪些功能
✽ Brewing…
󰉋 llm-spec  feat/sdk-tester 󰚩 glm-5 ↑0 ↓0 ⚡ 0 tok/s
i…
✻n
25
✻50
✶Brewing…663
Brewing…101 tokens)
↑4
/private/var/folders/9r/example/T/codexl-claude-code-real-ccr-1
project,orworkfromyourteam).Ifnot,takeamomenttoreviewwhat'sinthisfolderfirst.
ClaudeCode'llbeabletoread,edit,andexecutefileshere.
Securityguid
Read 1 file, listed 1 directory (ctrl+o to expand)
⏺让我先了解一下项目结构和功能。
⏺ Reading 1 file, listing 1 directory… (ctrl+o to expand)
⎿ $ cat /Users/jinhuilee/baishan/llm-spec/README.md 2>/dev/null || echo "No README found"
⏺根据README和项目结构，该项目是一个LLM SDK API 兼容性测试工具，主要功能如下：
核心功能
1.多 SDK API 测试—验证三类SDK的API格式/参数/特性支持情况：
-GoogleGemini(@google/genai)
✻Baked for 22s
Resume this session with:
claude --resume acd0d82c-f9f7-4455-95fd-4ab7e0e9130b
"#;

        let streamed = clean_command_output_delta(raw, prompt);
        let final_text = clean_interactive_cli_output(raw, prompt);

        for cleaned in [&streamed, &final_text] {
            assert!(!cleaned.contains("ClaudeCodev"), "{cleaned}");
            assert!(!cleaned.contains("Opus4"), "{cleaned}");
            assert!(!cleaned.contains("Brewing"), "{cleaned}");
            assert!(!cleaned.contains("tok/s"), "{cleaned}");
            assert!(!cleaned.contains("󰉋"), "{cleaned}");
            assert!(!cleaned.contains("claude --resume"), "{cleaned}");
            assert!(!cleaned.contains("project,orworkfromyourteam"), "{cleaned}");
            assert!(!cleaned.contains("ClaudeCode'llbeabletoread"), "{cleaned}");
            assert!(
                !cleaned.contains("codexl-claude-code-real-ccr"),
                "{cleaned}"
            );
            assert!(!cleaned.contains(prompt), "{cleaned}");
            assert!(
                cleaned.contains("让我先了解一下项目结构和功能"),
                "{cleaned}"
            );
            assert!(cleaned.contains("Reading 1 file"), "{cleaned}");
            assert!(
                cleaned.contains("cat /Users/jinhuilee/baishan/llm-spec/README.md"),
                "{cleaned}"
            );
            assert!(cleaned.contains("核心功能"), "{cleaned}");
            assert!(
                cleaned.contains("-GoogleGemini(@google/genai)"),
                "{cleaned}"
            );
        }
    }

    #[test]
    fn extracts_latest_assistant_text_from_claude_transcript() {
        let transcript = r#"
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}
{"type":"system","message":{"role":"system","content":"ignored"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash"},{"type":"text","text":"CODEXL_CCR_APP_SERVER_OK"}]}}
"#;

        assert_eq!(
            latest_assistant_text_from_transcript(transcript),
            Some("CODEXL_CCR_APP_SERVER_OK".to_string())
        );
    }

    #[test]
    fn thread_list_read_and_turns_list_load_claude_transcripts() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-transcripts");
        let cwd = root.join("workspace");
        let session_id = "11111111-1111-4111-8111-222222222222";
        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&cwd));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let transcript_path = projects_dir.join(format!("{session_id}.jsonl"));
        std::fs::write(
            &transcript_path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": {
                        "role": "user",
                        "content": "hello transcript"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:03.000Z",
                    "uuid": "assistant-message",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "hello from claude" }]
                    }
                })
            ),
        )
        .expect("write transcript");
        std::env::set_var("HOME", &root);

        let output_path = root.join("out.jsonl");
        let input = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"thread/list","params":{"limit":10,"sourceKinds":["cli"],"modelProviders":[PROVIDER_NAME]}}),
            json!({"id":"3","method":"thread/read","params":{"threadId":session_id,"includeTurns":true}}),
            json!({"id":"4","method":"thread/turns/list","params":{"threadId":session_id,"limit":1,"sortDirection":"desc"}}),
            json!({"id":"5","method":"thread/resume","params":{"threadId":session_id}}),
            json!({"id":"6","method":"thread/resume","params":{"threadId":session_id,"excludeTurns":true}}),
            json!({"id":"7","method":"thread/read","params":{"threadId":format!("local:{session_id}"),"includeTurns":true}})
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let responses = json_lines(&std::fs::read_to_string(&output_path).expect("read output"));
        let listed = response_by_id(&responses, "2")
            .pointer("/result/data/0")
            .expect("listed thread");
        assert_eq!(listed.get("id").and_then(Value::as_str), Some(session_id));
        assert_eq!(
            listed.get("preview").and_then(Value::as_str),
            Some("hello transcript")
        );
        assert_eq!(
            listed.get("path").and_then(Value::as_str),
            Some(transcript_path.to_string_lossy().as_ref())
        );

        let read = response_by_id(&responses, "3")
            .pointer("/result/thread/turns/0")
            .expect("read turn");
        assert_eq!(
            read.pointer("/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("hello transcript")
        );
        assert_eq!(
            read.pointer("/items/1/text").and_then(Value::as_str),
            Some("hello from claude")
        );

        let turn = response_by_id(&responses, "4")
            .pointer("/result/data/0")
            .expect("listed turn");
        assert_eq!(
            turn.pointer("/items/1/text").and_then(Value::as_str),
            Some("hello from claude")
        );

        let resumed = response_by_id(&responses, "5")
            .pointer("/result/thread/turns/0")
            .expect("resume returns turns by default");
        assert_eq!(
            resumed.pointer("/items/1/text").and_then(Value::as_str),
            Some("hello from claude")
        );

        let cheap_resume_turns = response_by_id(&responses, "6")
            .pointer("/result/thread/turns")
            .and_then(Value::as_array)
            .expect("cheap resume turns");
        assert!(
            cheap_resume_turns.is_empty(),
            "excludeTurns=true should omit turns"
        );

        let local_read = response_by_id(&responses, "7")
            .pointer("/result/thread/turns/0")
            .expect("local-prefixed read turn");
        assert_eq!(
            local_read.pointer("/items/1/text").and_then(Value::as_str),
            Some("hello from claude")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_hides_title_generation_transcripts_and_uses_generated_title() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-title-transcripts");
        let cwd = root.join("workspace");
        let title_session_id = "55555555-5555-4555-8555-555555555555";
        let main_session_id = "66666666-6666-4666-8666-666666666666";
        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&cwd));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let title_transcript_path = projects_dir.join(format!("{title_session_id}.jsonl"));
        let main_transcript_path = projects_dir.join(format!("{main_session_id}.jsonl"));
        let title_prompt = "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nGenerate a concise UI title (up to 36 characters) for this task.\n\nUser prompt:\n你是谁";
        std::fs::write(
            &title_transcript_path,
            format!(
                "{}\n{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": title_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": title_prompt }]
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": title_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:03.000Z",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "Fallback title" }]
                    }
                }),
                json!({
                    "type": "ai-title",
                    "sessionId": title_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:04.000Z",
                    "aiTitle": "Explain who I am"
                })
            ),
        )
        .expect("write title transcript");
        std::fs::write(
            &main_transcript_path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": main_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "message": {
                        "role": "user",
                        "content": "你是谁"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": main_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:05.000Z",
                    "uuid": "assistant-message",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "我是 Claude。" }]
                    }
                })
            ),
        )
        .expect("write main transcript");
        std::env::set_var("HOME", &root);
        assert!(load_claude_thread_by_id(title_session_id, None).is_none());

        let output_path = root.join("out.jsonl");
        let input = format!(
            "{}\n{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"thread/list","params":{"limit":10}}),
            json!({"id":"3","method":"thread/read","params":{"threadId":main_session_id,"includeTurns":true}})
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let responses = json_lines(&std::fs::read_to_string(&output_path).expect("read output"));
        let listed_threads = response_by_id(&responses, "2")
            .pointer("/result/data")
            .and_then(Value::as_array)
            .expect("listed threads");
        assert_eq!(listed_threads.len(), 1, "{listed_threads:#?}");
        let listed = &listed_threads[0];
        assert_eq!(
            listed.get("id").and_then(Value::as_str),
            Some(main_session_id)
        );
        assert_eq!(
            listed.get("name").and_then(Value::as_str),
            Some("Explain who I am")
        );
        assert_eq!(
            response_by_id(&responses, "3")
                .pointer("/result/thread/name")
                .and_then(Value::as_str),
            Some("Explain who I am")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_title_transcript_uses_inline_ai_title_as_name() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-inline-title-load");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let transcript_path = root.join("thread.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type":"user","sessionId":"thread","cwd":"/tmp","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
{"type":"ai-title","aiTitle":"Inline greeting","sessionId":"thread"}
{"type":"assistant","sessionId":"thread","cwd":"/tmp","timestamp":"2026-01-01T00:00:01Z","message":{"role":"assistant","model":"opus","content":[{"type":"text","text":"hello"}]}}
"#,
        )
        .expect("write transcript");

        let thread = load_claude_thread_from_transcript_path(
            &transcript_path,
            Some("workspace".to_string()),
        )
        .expect("thread");
        assert_eq!(thread.name.as_deref(), Some("Inline greeting"));

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_overlays_live_thread_with_inline_transcript_title() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-inline-title-live");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let cwd = root.join("workspace").to_string_lossy().to_string();
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: Some("workspace".to_string()),
        };
        let (response, _) = state.start_thread(&json!({ "cwd": cwd }));
        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, _, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hi" }],
            }))
            .expect("start turn");

        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(Path::new(&cwd)));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        let transcript_path = projects_dir.join(format!("{thread_id}.jsonl"));
        std::fs::write(
            &transcript_path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": thread_id,
                    "cwd": cwd,
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": "hi" }]
                    }
                }),
                json!({
                    "type": "ai-title",
                    "sessionId": thread_id,
                    "cwd": cwd,
                    "aiTitle": "Inline greeting"
                })
            ),
        )
        .expect("write transcript");

        let listed = state.thread_list(&json!({ "limit": 10 }));
        let listed_threads = listed
            .get("data")
            .and_then(Value::as_array)
            .expect("listed threads");
        assert_eq!(listed_threads.len(), 1, "{listed_threads:#?}");
        assert_eq!(
            listed_threads[0].get("name").and_then(Value::as_str),
            Some("Inline greeting")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recognizes_current_claude_title_generation_prompt_template() {
        let prompt = "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nThe tasks typically have to do with coding-related tasks.\nFill the structured title field with plain text.\nGenerate a clear, informative task title.\n\nHow to write a good title:\n- Prefer concrete nouns and verbs.\n- Do not wrap the title in quotes.\n\nUser prompt:\n分析代码说明该项目适配windows还有哪些问题";

        assert_eq!(
            extract_claude_title_generation_source_prompt(prompt).as_deref(),
            Some("分析代码说明该项目适配windows还有哪些问题")
        );
    }

    #[test]
    fn title_generation_finish_uses_transcript_ai_title_immediately() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-title-transcript-hint");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let cwd = root.join("workspace").to_string_lossy().to_string();
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: Some("workspace".to_string()),
        };
        let (main_response, _) = state.start_thread(&json!({ "cwd": cwd }));
        let main_thread_id = main_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("main thread id")
            .to_string();
        let (_, _, main_work, _) = state
            .start_turn(&json!({
                "threadId": main_thread_id,
                "input": [{ "type": "text", "text": "hi" }],
            }))
            .expect("start main turn");
        state
            .finish_turn(
                &main_work.thread_id,
                &main_work.turn_id,
                ClaudeRunResult {
                    text: "Hello".to_string(),
                    error: None,
                    duration_ms: 1,
                    tool_items: Vec::new(),
                    agent_item_streamed: true,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish main turn");

        let (title_response, _) = state.start_thread(&json!({ "cwd": cwd }));
        let title_thread_id = title_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("title thread id")
            .to_string();
        let title_prompt = "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nThe tasks typically have to do with coding-related tasks.\nFill the structured title field with plain text.\nGenerate a clear, informative task title.\n\nUser prompt:\nhi";
        let (_, _, title_work, _) = state
            .start_turn(&json!({
                "threadId": title_thread_id,
                "input": [{ "type": "text", "text": title_prompt }],
            }))
            .expect("start title turn");
        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(Path::new(&cwd)));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        let transcript_path = projects_dir.join(format!("{}.jsonl", title_work.claude_session_id));
        std::fs::write(
            &transcript_path,
            format!(
                "{}\n{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": title_work.claude_session_id,
                    "cwd": cwd,
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": title_prompt }]
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": title_work.claude_session_id,
                    "cwd": cwd,
                    "message": {
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "Fallback title" }]
                    }
                }),
                json!({
                    "type": "ai-title",
                    "sessionId": title_work.claude_session_id,
                    "cwd": cwd,
                    "aiTitle": "Transcript title"
                })
            ),
        )
        .expect("write title transcript");

        let title_hint = latest_claude_transcript_generated_title(&title_work).expect("title hint");
        let title_notifications = state
            .finish_turn(
                &title_work.thread_id,
                &title_work.turn_id,
                ClaudeRunResult {
                    text: String::new(),
                    error: None,
                    duration_ms: 1,
                    tool_items: Vec::new(),
                    agent_item_streamed: true,
                    latest_token_usage_info: None,
                },
                Some(title_hint),
            )
            .expect("finish title turn");

        assert_eq!(
            title_notifications
                .extra_notifications
                .iter()
                .find(|notification| {
                    notification.get("method").and_then(Value::as_str)
                        == Some("thread/name/updated")
                })
                .and_then(|notification| notification.pointer("/params/threadId"))
                .and_then(Value::as_str),
            Some(main_thread_id.as_str())
        );
        assert_eq!(
            title_notifications
                .extra_notifications
                .iter()
                .find(|notification| {
                    notification.get("method").and_then(Value::as_str)
                        == Some("thread/name/updated")
                })
                .and_then(|notification| notification.pointer("/params/name"))
                .and_then(Value::as_str),
            Some("Transcript title")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_start_notification_is_deferred_until_first_non_title_turn() {
        let root = test_dir("claude-deferred-thread-start");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let output_path = root.join("out.jsonl");
        let input = format!(
            "{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"thread/start","params":{"cwd":root}})
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let responses = json_lines(&std::fs::read_to_string(&output_path).expect("read output"));
        assert!(response_by_id(&responses, "2")
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .is_some());
        assert!(
            responses
                .iter()
                .all(|message| message.get("method").and_then(Value::as_str)
                    != Some("thread/started")),
            "thread/start should not expose a sidebar entry before the first prompt is known"
        );

        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: Some("workspace".to_string()),
        };
        let (response, _) = state.start_thread(&json!({ "cwd": root }));
        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, notifications, _, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hi" }],
            }))
            .expect("start turn");
        assert!(
            notifications.iter().any(|notification| {
                notification.get("method").and_then(Value::as_str) == Some("thread/started")
            }),
            "non-title turns should publish the real thread once the prompt is known"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_hides_in_memory_title_generation_thread_and_updates_main_title() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-title-in-memory");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let cwd = root.join("workspace").to_string_lossy().to_string();
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: Some("workspace".to_string()),
        };
        let (main_response, _) = state.start_thread(&json!({ "cwd": cwd }));
        let main_thread_id = main_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("main thread id")
            .to_string();
        let (_, _, main_work, stale_processes) = state
            .start_turn(&json!({
                "threadId": main_thread_id,
                "input": [{ "type": "text", "text": "hi" }],
            }))
            .expect("start main turn");
        assert!(stale_processes.is_empty());
        state
            .finish_turn(
                &main_work.thread_id,
                &main_work.turn_id,
                ClaudeRunResult {
                    text: "Hello".to_string(),
                    error: None,
                    duration_ms: 1,
                    tool_items: Vec::new(),
                    agent_item_streamed: true,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish main turn");

        let (title_response, title_thread_started_notification) =
            state.start_thread(&json!({ "cwd": cwd }));
        let title_thread_id = title_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("title thread id")
            .to_string();
        assert_eq!(
            title_thread_started_notification
                .get("method")
                .and_then(Value::as_str),
            Some("thread/started")
        );
        let title_prompt = "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nGenerate a concise UI title (up to 36 characters) for this task.\n\nUser prompt:\nhi";
        let (_, title_start_notifications, title_work, stale_processes) = state
            .start_turn(&json!({
                "threadId": title_thread_id,
                "input": [{ "type": "text", "text": title_prompt }],
            }))
            .expect("start title turn");
        assert!(stale_processes.is_empty());
        assert!(
            title_start_notifications.iter().any(|notification| {
                notification.get("method").and_then(Value::as_str) == Some("thread/archived")
                    && notification
                        .pointer("/params/threadId")
                        .and_then(Value::as_str)
                        == Some(title_thread_id.as_str())
            }),
            "title generation thread should be hidden after its prompt is recognized"
        );
        let title_notifications = state
            .finish_turn(
                &title_work.thread_id,
                &title_work.turn_id,
                ClaudeRunResult {
                    text: "Greeting".to_string(),
                    error: None,
                    duration_ms: 1,
                    tool_items: Vec::new(),
                    agent_item_streamed: true,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish title turn");

        assert!(title_notifications.item_completed.is_none());
        assert!(title_notifications.turn_completed.is_none());
        assert!(title_notifications.thread_stream_state.is_none());
        assert_eq!(
            title_notifications
                .extra_notifications
                .first()
                .and_then(|notification| notification.pointer("/params/threadId"))
                .and_then(Value::as_str),
            Some(main_thread_id.as_str())
        );
        assert_eq!(
            title_notifications
                .extra_notifications
                .first()
                .and_then(|notification| notification.pointer("/params/name"))
                .and_then(Value::as_str),
            Some("Greeting")
        );
        assert!(
            title_notifications
                .extra_notifications
                .iter()
                .any(|notification| {
                    notification.get("method").and_then(Value::as_str) == Some("thread/archived")
                        && notification
                            .pointer("/params/threadId")
                            .and_then(Value::as_str)
                            == Some(title_thread_id.as_str())
                }),
            "finish should repeat the hide notification for title generation threads"
        );

        let listed = state.thread_list(&json!({ "limit": 10 }));
        let listed_threads = listed
            .get("data")
            .and_then(Value::as_array)
            .expect("listed threads");
        assert_eq!(listed_threads.len(), 1, "{listed_threads:#?}");
        assert_eq!(
            listed_threads[0].get("id").and_then(Value::as_str),
            Some(main_thread_id.as_str())
        );
        assert_eq!(
            listed_threads[0].get("name").and_then(Value::as_str),
            Some("Greeting")
        );
        assert!(state
            .thread_read(&json!({ "threadId": title_thread_id }))
            .is_err());

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_name_set_persists_claude_thread_title() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-title-persistence");
        let cwd = root.join("workspace");
        let session_id = "77777777-7777-4777-8777-777777777777";
        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&cwd));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::write(
            projects_dir.join(format!("{session_id}.jsonl")),
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": {
                        "role": "user",
                        "content": "hello"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "hi" }]
                    }
                })
            ),
        )
        .expect("write transcript");
        std::env::set_var("HOME", &root);

        let output_path = root.join("out.jsonl");
        let input = format!(
            "{}\n{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"thread/name/set","params":{"threadId":session_id,"name":"Custom title"}}),
            json!({"id":"3","method":"thread/list","params":{"limit":10}})
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        let responses = json_lines(&std::fs::read_to_string(&output_path).expect("read output"));
        assert_eq!(
            response_by_id(&responses, "3")
                .pointer("/result/data/0/name")
                .and_then(Value::as_str),
            Some("Custom title")
        );
        assert!(root
            .join(".claude")
            .join(CLAUDE_THREAD_NAMES_FILE)
            .is_file());

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resume_unknown_thread_does_not_create_empty_claude_session() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };

        let err = state
            .resume_thread(&json!({
                "threadId": "22222222-2222-4222-8222-222222222222",
                "cwd": "/tmp",
            }))
            .expect_err("unknown resume should fail");

        assert!(err.contains("thread not found"), "{err}");
        assert!(state.threads.is_empty());
    }

    #[test]
    fn resume_ignores_non_claude_rollout_path() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("non-claude-rollout");
        let cwd = root.join("workspace");
        let rollout_dir = root.join(".codex").join("sessions");
        let thread_id = "33333333-3333-4333-8333-333333333333";
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join(format!("{thread_id}.jsonl"));
        std::fs::write(
            &rollout_path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": thread_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": { "role": "user", "content": "codex rollout prompt" }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": thread_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "message": {
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "codex rollout answer" }]
                    }
                })
            ),
        )
        .expect("write non-claude rollout");
        std::env::set_var("HOME", &root);

        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let err = state
            .resume_thread(&json!({
                "threadId": thread_id,
                "path": rollout_path,
                "cwd": cwd,
            }))
            .expect_err("non-claude path should not resume");

        assert!(err.contains("thread not found"), "{err}");
        assert!(state.threads.is_empty());
        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn started_thread_first_turn_uses_session_id_not_resume() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, stale_processes) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");
        assert!(stale_processes.is_empty());

        let command = claude_command_display(&work);
        assert!(command.contains("--session-id"), "{command}");
        assert!(!command.contains("--resume"), "{command}");
    }

    #[test]
    fn codex_app_model_is_not_forwarded_to_ccr_by_default() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        std::env::remove_var(MODEL_ENV);
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };

        assert_eq!(claude_model_arg(), None);
        assert!(!claude_command_display(&work).contains("--model"));
    }

    #[test]
    fn explicit_claude_code_model_env_is_forwarded_to_ccr() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        std::env::set_var(MODEL_ENV, "sonnet");
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };

        assert_eq!(claude_model_arg(), Some("sonnet".to_string()));
        assert!(claude_command_display(&work).contains("--model sonnet"));
        std::env::remove_var(MODEL_ENV);
    }

    #[test]
    fn claude_command_uses_stream_json_protocol() {
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };

        let command = claude_command_display(&work);
        assert!(command.contains("--output-format stream-json"));
        assert!(command.contains("--verbose"));
        assert!(command.contains("--input-format stream-json"));
        assert!(command.contains("--include-partial-messages"));
        assert!(command.contains("--session-id 11111111-1111-4111-8111-111111111111"));
    }

    #[test]
    fn claude_command_uses_stdio_permission_prompt_by_default() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_prompt_tool = std::env::var_os(PERMISSION_PROMPT_TOOL_ENV);
        std::env::remove_var(PERMISSION_PROMPT_TOOL_ENV);
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };

        assert!(claude_command_display(&work).contains("--permission-prompt-tool stdio"));
        std::env::set_var(PERMISSION_PROMPT_TOOL_ENV, "none");
        assert!(!claude_command_display(&work).contains("--permission-prompt-tool"));

        restore_env(PERMISSION_PROMPT_TOOL_ENV, old_prompt_tool);
    }

    #[test]
    fn json_rpc_response_is_stashed_for_pending_app_request() {
        let state = Arc::new(Mutex::new(ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        }));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));

        let worker = handle_client_line(
            br#"{"id":"perm-1","result":{"permissions":{"network":{"enabled":true}},"scope":"turn"}}"#,
            Arc::clone(&state),
            Arc::clone(&output),
        )
        .expect("handle response");

        assert!(worker.is_none());
        assert_eq!(
            take_app_response(&state, "perm-1")
                .expect("stored response")
                .pointer("/permissions/network/enabled"),
            Some(&Value::Bool(true))
        );
        assert!(output.lock().expect("output lock").is_empty());
    }

    #[test]
    fn claude_permission_request_round_trips_through_codex_app_response() {
        let state = Arc::new(Mutex::new(ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        }));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp/workspace".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let message = json!({
            "type": "control_request",
            "request_id": "perm-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "mcp__computer-use__screenshot",
                "tool_use_id": "toolu_permission_1",
                "input": { "display": 0 }
            }
        });
        let worker_state = Arc::clone(&state);
        let worker_output = Arc::clone(&output);
        let handle = thread::spawn(move || {
            request_codex_app_permissions(&message, &work, &worker_state, &worker_output)
        });

        for _ in 0..50 {
            let current =
                String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8");
            if current.contains(r#""method":"item/permissions/requestApproval""#) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let emitted = String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8");
        assert!(
            emitted.contains(r#""method":"item/permissions/requestApproval""#),
            "{emitted}"
        );
        assert!(
            emitted.contains(r#""network":{"enabled":true}"#),
            "{emitted}"
        );

        handle_client_line(
            br#"{"id":"perm-1","result":{"permissions":{"network":{"enabled":true}},"scope":"turn"}}"#,
            Arc::clone(&state),
            Arc::clone(&output),
        )
        .expect("handle app response");
        let response = handle
            .join()
            .expect("permission worker")
            .expect("permission response");

        assert_eq!(
            response.get("type").and_then(Value::as_str),
            Some("control_response")
        );
        assert_eq!(
            response
                .pointer("/response/request_id")
                .and_then(Value::as_str),
            Some("perm-1")
        );
        assert_eq!(
            response
                .pointer("/response/subtype")
                .and_then(Value::as_str),
            Some("success")
        );
        assert_eq!(
            response
                .pointer("/response/response/behavior")
                .and_then(Value::as_str),
            Some("allow")
        );
        assert_eq!(
            response.pointer("/response/response/updatedInput/display"),
            Some(&json!(0))
        );
        assert_eq!(
            response
                .pointer("/response/response/toolUseID")
                .and_then(Value::as_str),
            Some("toolu_permission_1")
        );
    }

    #[test]
    fn bash_permission_request_uses_codex_app_path_strings() {
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp/workspace".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let message = json!({
            "type": "control_request",
            "request_id": "perm-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "tool_use_id": "toolu_bash_1",
                "input": {
                    "command": "open -a Slack",
                    "description": "Open Slack"
                }
            }
        });

        let params = codex_app_permission_request_params(&work, "perm-1", &message);

        assert_eq!(params.pointer("/cwd"), Some(&json!("/tmp/workspace")));
        assert_eq!(
            params.pointer("/permissions/fileSystem/read/0"),
            Some(&json!("/tmp/workspace"))
        );
        assert_eq!(
            params.pointer("/permissions/fileSystem/write/0"),
            Some(&json!("/tmp/workspace"))
        );
        assert!(!params
            .pointer("/permissions/fileSystem/read/0")
            .is_some_and(Value::is_object));
    }

    #[test]
    fn claude_elicitation_request_round_trips_through_codex_app_response() {
        let state = Arc::new(Mutex::new(ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        }));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp/workspace".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let message = json!({
            "type": "control_request",
            "request_id": "elicitation-1",
            "request": {
                "subtype": "elicitation",
                "mcp_server_name": "codex-computer-use",
                "mode": "form",
                "message": "Computer Use needs confirmation.",
                "requested_schema": {
                    "type": "object",
                    "properties": {}
                },
                "_meta": {
                    "riskLevel": "high"
                }
            }
        });
        assert!(is_claude_elicitation_control_request(&message));
        assert!(!is_claude_permission_control_request(&message));

        let worker_state = Arc::clone(&state);
        let worker_output = Arc::clone(&output);
        let handle = thread::spawn(move || {
            request_codex_app_elicitation(&message, &work, &worker_state, &worker_output)
        });

        for _ in 0..50 {
            let current =
                String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8");
            if current.contains(r#""method":"mcpServer/elicitation/request""#) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let emitted = String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8");
        assert!(
            emitted.contains(r#""method":"mcpServer/elicitation/request""#),
            "{emitted}"
        );
        assert!(
            emitted.contains(r#""serverName":"codex-computer-use""#),
            "{emitted}"
        );
        assert!(emitted.contains(r#""requestedSchema""#), "{emitted}");

        handle_client_line(
            br#"{"id":"elicitation-1","result":{"action":"accept","content":{},"_meta":{"persist":"session"}}}"#,
            Arc::clone(&state),
            Arc::clone(&output),
        )
        .expect("handle app response");
        let response = handle
            .join()
            .expect("elicitation worker")
            .expect("elicitation response");

        assert_eq!(
            response.get("type").and_then(Value::as_str),
            Some("control_response")
        );
        assert_eq!(
            response
                .pointer("/response/request_id")
                .and_then(Value::as_str),
            Some("elicitation-1")
        );
        assert_eq!(
            response
                .pointer("/response/response/action")
                .and_then(Value::as_str),
            Some("accept")
        );
        assert_eq!(
            response
                .pointer("/response/response/_meta/persist")
                .and_then(Value::as_str),
            Some("session")
        );
    }

    #[test]
    fn claude_stream_json_input_matches_sdk_shape() {
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };

        let input = claude_stream_json_input(&work);
        let lines = input
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
            .collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0].get("type").and_then(Value::as_str),
            Some("control_request")
        );
        assert_eq!(
            lines[0]
                .get("request")
                .and_then(|request| request.get("subtype"))
                .and_then(Value::as_str),
            Some("initialize")
        );
        assert_eq!(lines[1].get("type").and_then(Value::as_str), Some("user"));
        assert_eq!(
            lines[1]
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            lines[1]
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str),
            Some("hello")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stream_json_reemits_active_thread_state_after_claude_starts() {
        let root = test_dir("stream-json-thread-state-heartbeat");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let fake_claude = root.join("fake-claude");
        let fake_claude_script = r#"#!/bin/sh
IFS= read -r _line || true
printf '%s\n' '{"type":"result","is_error":false,"result":"done","duration_ms":10}'
"#;
        write_executable(&fake_claude, fake_claude_script.as_bytes());

        let mut initial_state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (thread_response, _) = initial_state.start_thread(&json!({
            "cwd": root.to_string_lossy(),
        }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, stale_processes) = initial_state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "use computer" }],
            }))
            .expect("start turn");
        assert!(stale_processes.is_empty());
        let state = Arc::new(Mutex::new(initial_state));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));

        let result = run_claude_code_turn_stream_json(
            Command::new(&fake_claude),
            &work,
            Arc::clone(&state),
            Arc::clone(&output),
            Instant::now(),
        );

        assert_eq!(result.error, None);
        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let messages = json_lines(&output);
        let active_snapshot = messages
            .iter()
            .find(|message| {
                message.get("method").and_then(Value::as_str) == Some("thread-stream-state-changed")
                    && message
                        .pointer("/params/change/conversationState/threadRuntimeStatus/type")
                        .and_then(Value::as_str)
                        == Some("active")
            })
            .expect("active thread stream state snapshot");
        assert_eq!(
            active_snapshot
                .pointer("/params/change/conversationState/turns/0/status")
                .and_then(Value::as_str),
            Some("inProgress")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn stream_json_turn_finishes_after_result_even_if_child_stays_open() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_timeout = std::env::var_os(TURN_IDLE_TIMEOUT_MS_ENV);
        std::env::set_var(TURN_IDLE_TIMEOUT_MS_ENV, "2000");

        let root = test_dir("persistent-stream-json");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let fake_claude = root.join("fake-claude");
        let pid_path = root.join("fake-claude.pid");
        let killed_path = root.join("fake-claude.killed");
        let fake_claude_script = r#"#!/bin/sh
trap 'printf killed > "__KILLED_PATH__"; exit 143' TERM INT HUP
printf '%s' "$$" > "__PID_PATH__"
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Persistent done"}]}}'
printf '%s\n' '{"type":"result","is_error":false,"result":"Persistent done","duration_ms":10}'
sleep 60
"#
        .replace("__KILLED_PATH__", killed_path.to_string_lossy().as_ref())
        .replace("__PID_PATH__", pid_path.to_string_lossy().as_ref());
        write_executable(&fake_claude, fake_claude_script.as_bytes());

        let state = Arc::new(Mutex::new(ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        }));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: root.to_string_lossy().to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let started = Instant::now();
        let result = run_claude_code_turn_stream_json(
            Command::new(&fake_claude),
            &work,
            Arc::clone(&state),
            Arc::clone(&output),
            started,
        );

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stream-json result should finish the turn promptly"
        );
        assert_eq!(result.text, "Persistent done");
        assert_eq!(result.error, None);
        assert!(result.agent_item_streamed);
        assert!(state
            .lock()
            .expect("state lock")
            .active_processes
            .is_empty());
        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        assert!(output.contains(r#""method":"item/agentMessage/delta""#));
        assert!(output.contains("Persistent done"));
        for _ in 0..20 {
            if killed_path.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            killed_path.exists(),
            "result completion should terminate the lingering Claude Code process group"
        );

        if let Ok(pid) = std::fs::read_to_string(&pid_path)
            .expect("pid file")
            .trim()
            .parse::<u32>()
        {
            terminate_process_group(pid);
        }
        restore_env(TURN_IDLE_TIMEOUT_MS_ENV, old_timeout);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_claude_stream_json_text_tool_and_result_events() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        let messages = [
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "Hel" }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "lo" }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "Read",
                        "input": { "file_path": "/tmp/README.md" }
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 2,
                    "delta": { "type": "thinking_delta", "thinking": "thinking" }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 3,
                    "content_block": {
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": "read result"
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 4,
                    "delta": { "type": "text_delta", "text": "Done" }
                }
            }),
            json!({
                "type": "result",
                "is_error": false,
                "result": "Done",
                "num_turns": 1,
                "duration_ms": 42
            }),
        ];

        for message in messages {
            handle_claude_stream_message(
                &message,
                &work,
                &output,
                &mut stream,
                &mut command_output,
            );
        }
        emit_reasoning_completed_if_started(&output, &work, &stream);

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        assert!(output.contains(r#""method":"item/agentMessage/delta""#));
        assert!(output.contains(r#""delta":"Done""#));
        assert!(!output.contains(r#""type":"commandExecution""#));
        assert!(!output.contains(r#""method":"item/commandExecution/outputDelta""#));
        assert!(output.contains(r#""type":"mcpToolCall""#));
        assert!(!output.contains(r#""type":"dynamicToolCall""#));
        assert!(output.contains(r#""tool":"Read""#));
        assert!(output.contains("/tmp/README.md"));
        assert!(output.contains("read result"));
        assert!(output.contains(r#""method":"item/reasoning/textDelta""#));
        assert!(output.contains(r#""delta":"Hello""#));
        let lines = json_lines(&output);
        let tool_completed_index = lines
            .iter()
            .position(|value| {
                value.get("method").and_then(Value::as_str) == Some("item/completed")
                    && value.pointer("/params/item/type").and_then(Value::as_str)
                        == Some("mcpToolCall")
            })
            .expect("tool completed notification");
        let agent_delta_index = lines
            .iter()
            .position(|value| {
                value.get("method").and_then(Value::as_str) == Some("item/agentMessage/delta")
            })
            .expect("agent delta notification");
        assert!(agent_delta_index > tool_completed_index);
        assert_eq!(stream.emitted_text, "Done");
        assert_eq!(stream.result_text, Some("Done".to_string()));
        assert_eq!(stream.completed_tool_items.len(), 1);
    }

    #[test]
    fn emits_claude_token_usage_updates_from_assistant_messages() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();

        handle_claude_stream_message(
            &json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet-4-20250514",
                    "content": [{ "type": "text", "text": "hello" }],
                    "usage": {
                        "input_tokens": 100,
                        "cache_creation_input_tokens": 20,
                        "cache_read_input_tokens": 30,
                        "output_tokens": 7
                    }
                }
            }),
            &work,
            &output,
            &mut stream,
            &mut command_output,
        );

        let usage = stream.latest_token_usage_info.as_ref().expect("usage info");
        assert_eq!(
            usage
                .pointer("/last_token_usage/total_tokens")
                .and_then(Value::as_i64),
            Some(157)
        );
        assert_eq!(
            usage.get("model_context_window").and_then(Value::as_i64),
            Some(DEFAULT_CLAUDE_CONTEXT_WINDOW)
        );
        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let notification = json_lines(&output)
            .into_iter()
            .find(|message| {
                message.get("method").and_then(Value::as_str) == Some("thread/tokenUsage/updated")
            })
            .expect("token usage notification");
        assert_eq!(
            notification
                .pointer("/params/latestTokenUsageInfo/last_token_usage/total_tokens")
                .and_then(Value::as_i64),
            Some(157)
        );
    }

    #[test]
    fn claude_transcript_threads_include_latest_token_usage_info() {
        let root = test_dir("claude-token-usage");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let transcript_path = root.join("thread.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type":"user","sessionId":"thread","cwd":"/tmp","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
{"type":"assistant","sessionId":"thread","cwd":"/tmp","timestamp":"2026-01-01T00:00:01Z","message":{"role":"assistant","model":"claude-opus-4-1m","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":42,"output_tokens":8}}}
"#,
        )
        .expect("write transcript");

        let thread =
            load_claude_thread_from_transcript_path(&transcript_path, None).expect("thread");
        let state = claude_conversation_state(&thread);

        assert_eq!(
            state
                .pointer("/latestTokenUsageInfo/last_token_usage/total_tokens")
                .and_then(Value::as_i64),
            Some(50)
        );
        assert_eq!(
            state
                .pointer("/latestTokenUsageInfo/model_context_window")
                .and_then(Value::as_i64),
            Some(CLAUDE_ONE_M_CONTEXT_WINDOW)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn streamed_tool_arguments_are_visible_on_started_item() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        let messages = [
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "Read"
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": "{\"file_path\":\"/tmp/README.md\"}"
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": { "type": "content_block_stop", "index": 0 }
            }),
        ];

        for message in messages {
            handle_claude_stream_message(
                &message,
                &work,
                &output,
                &mut stream,
                &mut command_output,
            );
        }

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let started = json_lines(&output)
            .into_iter()
            .find(|value| value.get("method").and_then(Value::as_str) == Some("item/started"))
            .expect("tool started notification");
        assert_eq!(
            started.pointer("/params/item/type").and_then(Value::as_str),
            Some("mcpToolCall")
        );
        assert_eq!(
            started
                .pointer("/params/item/arguments/file_path")
                .and_then(Value::as_str),
            Some("/tmp/README.md")
        );
    }

    #[test]
    fn explicit_empty_tool_arguments_emit_started_item() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        handle_claude_stream_message(
            &json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_list_apps",
                        "name": "mcp__codex-computer-use__list_apps",
                        "input": {}
                    }
                }
            }),
            &work,
            &output,
            &mut stream,
            &mut command_output,
        );

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let started = json_lines(&output)
            .into_iter()
            .find(|value| value.get("method").and_then(Value::as_str) == Some("item/started"))
            .expect("tool started notification");
        assert_eq!(
            started.pointer("/params/item/type").and_then(Value::as_str),
            Some("mcpToolCall")
        );
        assert_eq!(
            started
                .pointer("/params/item/status")
                .and_then(Value::as_str),
            Some("inProgress")
        );
        assert_eq!(
            started.pointer("/params/item/tool").and_then(Value::as_str),
            Some("mcp__codex-computer-use__list_apps")
        );
        assert!(started
            .pointer("/params/item/arguments")
            .and_then(Value::as_object)
            .is_some_and(|arguments| arguments.is_empty()));
    }

    #[test]
    fn maps_agent_tool_to_collab_agent_tool_call() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        let messages = [
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_agent",
                        "name": "Agent",
                        "input": {
                            "description": "Explore repo",
                            "prompt": "Inspect the project structure",
                            "subagent_type": "Explore"
                        }
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "subagent done"
                    }
                }
            }),
        ];

        for message in messages {
            handle_claude_stream_message(
                &message,
                &work,
                &output,
                &mut stream,
                &mut command_output,
            );
        }

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let lines = json_lines(&output);
        let started = lines
            .iter()
            .find(|value| value.get("method").and_then(Value::as_str) == Some("item/started"))
            .expect("agent started notification");
        assert_eq!(
            started.pointer("/params/item/type").and_then(Value::as_str),
            Some("collabAgentToolCall")
        );
        assert_eq!(
            started.pointer("/params/item/tool").and_then(Value::as_str),
            Some("spawnAgent")
        );
        assert_eq!(
            started
                .pointer("/params/item/senderThreadId")
                .and_then(Value::as_str),
            Some("thread")
        );
        assert_eq!(
            started
                .pointer("/params/item/receiverThreadIds/0")
                .and_then(Value::as_str),
            Some("claude-subagent-toolu_agent")
        );
        assert_eq!(
            started
                .pointer("/params/item/prompt")
                .and_then(Value::as_str),
            Some("Inspect the project structure")
        );
        assert_eq!(
            started
                .pointer("/params/item/agentsStates/claude-subagent-toolu_agent/status")
                .and_then(Value::as_str),
            Some("running")
        );

        let completed = lines
            .iter()
            .find(|value| value.get("method").and_then(Value::as_str) == Some("item/completed"))
            .expect("agent completed notification");
        assert_eq!(
            completed
                .pointer("/params/item/type")
                .and_then(Value::as_str),
            Some("collabAgentToolCall")
        );
        assert_eq!(
            completed
                .pointer("/params/item/status")
                .and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            completed
                .pointer("/params/item/result")
                .and_then(Value::as_str),
            Some("subagent done")
        );
        assert_eq!(
            completed
                .pointer("/params/item/agentsStates/claude-subagent-toolu_agent/status")
                .and_then(Value::as_str),
            Some("completed")
        );
        assert!(!output.contains(r#""tool":"Agent""#));
        assert!(!output.contains(r#""type":"mcpToolCall""#));
    }

    #[test]
    fn maps_bash_tool_to_command_execution_item() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        let messages = [
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_bash",
                        "name": "Bash",
                        "input": {
                            "command": "ls -la /tmp",
                            "description": "List temp"
                        }
                    }
                }
            }),
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "tool_result",
                        "tool_use_id": "toolu_bash",
                        "content": "total 0"
                    }
                }
            }),
        ];

        for message in messages {
            handle_claude_stream_message(
                &message,
                &work,
                &output,
                &mut stream,
                &mut command_output,
            );
        }

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        assert!(output.contains(r#""type":"commandExecution""#));
        assert!(output.contains("ls -la /tmp"));
        assert!(output.contains("total 0"));
    }

    #[test]
    fn ignores_matching_final_assistant_snapshot_after_text_stream() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            resume_existing: false,
        };
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();
        let messages = [
            json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "Hello\n" }
                }
            }),
            json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello" }]
                }
            }),
        ];

        for message in messages {
            handle_claude_stream_message(
                &message,
                &work,
                &output,
                &mut stream,
                &mut command_output,
            );
        }
        flush_pending_agent_text_as_agent(&output, &work, &mut stream);

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        assert_eq!(agent_delta_text(&output), "Hello\n");
        assert_eq!(stream.emitted_text, "Hello\n");
    }

    #[test]
    fn finish_turn_skips_final_agent_completed_item_after_streaming() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, stale_processes) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");
        assert!(stale_processes.is_empty());

        let notifications = state
            .finish_turn(
                &work.thread_id,
                &work.turn_id,
                ClaudeRunResult {
                    text: "Hello".to_string(),
                    error: None,
                    duration_ms: 1,
                    tool_items: Vec::new(),
                    agent_item_streamed: true,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish turn");

        assert!(notifications.item_completed.is_none());
        assert_eq!(
            notifications
                .turn_completed
                .as_ref()
                .expect("turn completed")
                .get("method")
                .and_then(Value::as_str),
            Some("turn/completed")
        );
        let turn_items = state
            .threads
            .get(&thread_id)
            .expect("thread")
            .turns
            .first()
            .expect("turn")
            .items_json();
        assert_eq!(
            turn_items.pointer("/1/text").and_then(Value::as_str),
            Some("Hello")
        );
    }

    #[test]
    fn resolves_hidden_native_claude_when_primary_npm_bin_is_placeholder() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_path = std::env::var_os("PATH");
        let root = test_dir("claude-path");
        let bin_dir = root.join("bin");
        let primary_dir = root
            .join("lib")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let hidden_dir = root
            .join("lib")
            .join("node_modules")
            .join("@anthropic-ai")
            .join(".claude-code-good");
        let hidden_bin = hidden_dir.join("bin").join("claude.exe");

        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        std::fs::create_dir_all(primary_dir.join("bin")).expect("create primary dir");
        std::fs::create_dir_all(hidden_bin.parent().expect("hidden bin parent"))
            .expect("create hidden dir");
        write_executable(&bin_dir.join("ccr"), b"#!/bin/sh\n");
        write_executable(
            &primary_dir.join("bin").join("claude.exe"),
            b"echo \"Error: claude native binary not installed.\" >&2\n",
        );
        let file = File::create(&hidden_bin).expect("create hidden native claude");
        file.set_len(MIN_NATIVE_CLAUDE_BYTES + 1)
            .expect("size hidden native claude");
        make_executable(&hidden_bin);
        std::env::set_var("PATH", &bin_dir);

        assert_eq!(resolve_claude_path_for_ccr("ccr"), Some(hidden_bin));

        if let Some(path) = old_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_codex_claude_path_override_sets_claude_path_env() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_override = std::env::var_os(CLAUDE_PATH_OVERRIDE_ENV);
        let old_claude_path = std::env::var_os(CLAUDE_PATH_ENV);
        let override_path = "/tmp/custom-claude";
        std::env::set_var(CLAUDE_PATH_OVERRIDE_ENV, override_path);
        std::env::remove_var(CLAUDE_PATH_ENV);

        let mut command = Command::new("ccr");
        configure_claude_path_env(&mut command, "ccr");
        let claude_path = command
            .get_envs()
            .find_map(|(key, value)| {
                (key == CLAUDE_PATH_ENV).then(|| value.map(|value| value.to_os_string()))
            })
            .flatten();

        assert_eq!(claude_path, Some(OsString::from(override_path)));

        restore_env(CLAUDE_PATH_OVERRIDE_ENV, old_override);
        restore_env(CLAUDE_PATH_ENV, old_claude_path);
    }

    #[test]
    #[ignore = "requires real ccr code service and Claude Code auth"]
    fn real_ccr_code_stream_json_smoke() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("real-ccr");
        std::fs::create_dir_all(&root).expect("create temp dir");
        std::env::set_var(BIN_ENV, "ccr");
        std::env::set_var(BASE_ARGS_ENV, "code");
        std::env::remove_var(EXTRA_ARGS_ENV);
        std::env::remove_var(MODEL_ENV);
        std::env::remove_var(PERMISSION_MODE_ENV);

        let output_path = root.join("out.jsonl");
        let thread_id = new_uuid_v4();
        let claude_project_dir = PathBuf::from(std::env::var_os("HOME").expect("HOME set"))
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&root));
        std::fs::create_dir_all(&claude_project_dir).expect("create claude project dir");
        let transcript_path = claude_project_dir.join(format!("{thread_id}.jsonl"));
        std::fs::write(&transcript_path, "").expect("create empty transcript");
        let token = format!(
            "CODEXL_CCR_APP_SERVER_OK_{}",
            thread_id
                .chars()
                .filter(|ch| ch.is_ascii_hexdigit())
                .take(8)
                .collect::<String>()
        );
        let input = format!(
            "{{\"id\":\"1\",\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\"}}}}\n{{\"method\":\"initialized\",\"params\":{{}}}}\n{{\"id\":\"2\",\"method\":\"thread/resume\",\"params\":{{\"threadId\":\"{}\",\"path\":\"{}\",\"cwd\":\"{}\",\"model\":\"sonnet\"}}}}\n{{\"id\":\"3\",\"method\":\"turn/start\",\"params\":{{\"threadId\":\"{}\",\"input\":[{{\"type\":\"text\",\"text\":\"Reply exactly with this token and nothing else: {}\"}}]}}}}\n",
            thread_id,
            transcript_path.to_string_lossy(),
            root.to_string_lossy(),
            thread_id,
            token
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        std::env::remove_var(BIN_ENV);
        std::env::remove_var(BASE_ARGS_ENV);

        let output = std::fs::read_to_string(&output_path).expect("read output");
        assert!(output.contains(r#""method":"item/started""#));
        assert!(!output.contains("ccr code --output-format"));
        assert!(!output.contains(r#""type":"commandExecution""#));
        assert!(output.contains(r#""method":"item/agentMessage/delta""#));
        assert!(
            !output.contains("claude: command not found"),
            "output was:\n{}",
            output
        );
        assert!(
            !output.contains("claude native binary not installed"),
            "output was:\n{}",
            output
        );
        assert!(
            agent_delta_text(&output).contains(&token),
            "output was:\n{}",
            output
        );
        assert!(!output.contains(r#""method":"item/completed""#));
        assert!(output.contains(r#""method":"turn/completed""#));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(claude_project_dir);
    }

    #[test]
    #[ignore = "requires real ccr code service, Claude Code auth, and tool execution"]
    fn real_ccr_code_stream_json_tool_smoke() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("real-ccr-tool");
        std::fs::create_dir_all(&root).expect("create temp dir");
        std::env::set_var(BIN_ENV, "ccr");
        std::env::set_var(BASE_ARGS_ENV, "code");
        std::env::remove_var(EXTRA_ARGS_ENV);
        std::env::remove_var(MODEL_ENV);
        std::env::remove_var(PERMISSION_MODE_ENV);

        let output_path = root.join("out.jsonl");
        let thread_id = new_uuid_v4();
        let claude_project_dir = PathBuf::from(std::env::var_os("HOME").expect("HOME set"))
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&root));
        std::fs::create_dir_all(&claude_project_dir).expect("create claude project dir");
        let transcript_path = claude_project_dir.join(format!("{thread_id}.jsonl"));
        std::fs::write(&transcript_path, "").expect("create empty transcript");
        let token = format!(
            "CODEXL_CCR_TOOL_OK_{}",
            thread_id
                .chars()
                .filter(|ch| ch.is_ascii_hexdigit())
                .take(8)
                .collect::<String>()
        );
        let marker_path = root.join("marker.txt");
        std::fs::write(&marker_path, &token).expect("write marker");
        let input = format!(
            "{{\"id\":\"1\",\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\"}}}}\n{{\"method\":\"initialized\",\"params\":{{}}}}\n{{\"id\":\"2\",\"method\":\"thread/resume\",\"params\":{{\"threadId\":\"{}\",\"path\":\"{}\",\"cwd\":\"{}\",\"model\":\"sonnet\"}}}}\n{{\"id\":\"3\",\"method\":\"turn/start\",\"params\":{{\"threadId\":\"{}\",\"input\":[{{\"type\":\"text\",\"text\":\"Use the Read tool to read this file, then reply exactly with its full contents and nothing else: {}\"}}]}}}}\n",
            thread_id,
            transcript_path.to_string_lossy(),
            root.to_string_lossy(),
            thread_id,
            marker_path.to_string_lossy()
        );

        run_stdio_app_server_with_io(
            vec![],
            std::io::Cursor::new(input.into_bytes()),
            File::create(&output_path).expect("create output"),
        )
        .expect("run app server");

        std::env::remove_var(BIN_ENV);
        std::env::remove_var(BASE_ARGS_ENV);

        let output = std::fs::read_to_string(&output_path).expect("read output");
        assert!(
            !output.contains("ccr code --output-format"),
            "output was:\n{output}"
        );
        assert!(
            !output.contains(r#""type":"commandExecution""#),
            "output was:\n{output}"
        );
        assert!(
            output.contains(r#""type":"mcpToolCall""#),
            "output was:\n{output}"
        );
        assert!(
            !output.contains(r#""type":"dynamicToolCall""#),
            "output was:\n{output}"
        );
        assert!(output.contains(r#""tool":"Read""#), "output was:\n{output}");
        assert!(
            output.contains(marker_path.to_string_lossy().as_ref()),
            "output was:\n{output}"
        );
        assert!(
            agent_delta_text(&output).contains(&token),
            "output was:\n{}",
            output
        );

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(claude_project_dir);
    }

    fn agent_delta_text(output: &str) -> String {
        output
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| {
                value.get("method").and_then(Value::as_str) == Some("item/agentMessage/delta")
            })
            .filter_map(|value| {
                value
                    .get("params")
                    .and_then(|params| params.get("delta"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    fn json_lines(output: &str) -> Vec<Value> {
        output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
            .collect()
    }

    fn response_by_id<'a>(responses: &'a [Value], id: &str) -> &'a Value {
        responses
            .iter()
            .find(|response| response.get("id").and_then(Value::as_str) == Some(id))
            .unwrap_or_else(|| panic!("missing response id {id}: {responses:#?}"))
    }

    fn write_executable(path: &Path, contents: &[u8]) {
        std::fs::write(path, contents).expect("write executable");
        make_executable(path);
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path)
                .expect("executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("chmod executable");
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
    }

    fn restore_env(name: &str, value: Option<OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
