use crate::extensions::builtins::bot_bridge;
use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
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
const SCAN_ALL_CODEX_HOMES_ENV: &str = "CODEXL_CLAUDE_CODE_SCAN_ALL_CODEX_HOMES";
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
const DEFAULT_APPROVAL_POLICY: &str = "on-request";
const DEFAULT_APPROVALS_REVIEWER: &str = "user";
const AUTO_REVIEW_APPROVALS_REVIEWER: &str = "auto_review";
const DEFAULT_PERMISSION_PROMPT_TOOL: &str = "stdio";
const DEFAULT_CLAUDE_CONTEXT_WINDOW: i64 = 200_000;
const CLAUDE_ONE_M_CONTEXT_WINDOW: i64 = 1_000_000;
const DEFAULT_TURN_IDLE_TIMEOUT_MS: u64 = 60 * 60 * 1000;
const DEFAULT_PERMISSION_APPROVAL_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const MIN_NATIVE_CLAUDE_BYTES: u64 = 5 * 1024 * 1024;
const CLAUDE_THREAD_NAMES_FILE: &str = "codex-app-thread-names.json";
const CLAUDE_THREAD_GOALS_FILE: &str = "codex-app-thread-goals.json";
const CLAUDE_THREAD_ARCHIVED_FILE: &str = "codex-app-thread-archived.json";
const CLAUDE_THREAD_PINNED_FILE: &str = "codex-app-thread-pinned.json";
const CLAUDE_THREAD_MEMORY_MODES_FILE: &str = "codex-app-thread-memory-modes.json";
const CLAUDE_PLUGIN_STATE_FILE: &str = "codex-app-plugin-state.json";
const CLAUDE_MCP_SERVER_STATE_FILE: &str = "codex-app-mcp-server-state.json";
const CLAUDE_TITLE_MATCH_MAX_DELTA_SECONDS: u64 = 6 * 60 * 60;
const CLAUDE_THREAD_LIST_CACHE_TTL_MS: i64 = 5_000;
const CLAUDE_THREAD_LIST_MIN_SCAN_LIMIT: usize = 120;
const CLAUDE_THREAD_LIST_MAX_SCAN_LIMIT: usize = 500;
const CLAUDE_THREAD_LIST_LIMIT_MULTIPLIER: usize = 12;
const CLAUDE_THREAD_LIST_MAX_LINES_PER_TRANSCRIPT: usize = 96;
const CLAUDE_THREAD_LIST_TAIL_BYTES: u64 = 256 * 1024;
const CLAUDE_PROJECT_DIR_MAX_LEN: usize = 200;
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
    "CODEXL_CLAUDE_CODE_SCAN_ALL_CODEX_HOMES",
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
    "--print",
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--include-partial-messages",
];

type SharedOutput<W> = Arc<Mutex<W>>;
type SharedState = Arc<Mutex<ClaudeAppServerState>>;

struct ClaudeBotBridgeInput {
    buffer: Vec<u8>,
    tx: mpsc::Sender<Vec<u8>>,
}

struct ClaudeBotBridgeOutput<W> {
    buffer: Vec<u8>,
    inner: W,
    tx: Option<mpsc::Sender<Vec<u8>>>,
}

impl ClaudeBotBridgeInput {
    fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            buffer: Vec::new(),
            tx,
        }
    }
}

impl Write for ClaudeBotBridgeInput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=index).collect::<Vec<_>>();
            self.tx.send(line).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Claude Code bot bridge input channel closed",
                )
            })?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<W> ClaudeBotBridgeOutput<W>
where
    W: Write,
{
    fn new(inner: W, tx: Option<mpsc::Sender<Vec<u8>>>) -> Self {
        Self {
            buffer: Vec::new(),
            inner,
            tx,
        }
    }

    fn write_complete_lines(&mut self) -> std::io::Result<()> {
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=index).collect::<Vec<_>>();
            if let Some(tx) = self.tx.as_ref() {
                let _ = tx.send(line.clone());
            }
            if !bot_bridge::should_intercept_app_server_line(&line) {
                self.inner.write_all(&line)?;
            }
        }
        Ok(())
    }
}

impl<W> Write for ClaudeBotBridgeOutput<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        self.write_complete_lines()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

static CLAUDE_CODE_LOG_LOCK: Mutex<()> = Mutex::new(());
static CLAUDE_THREAD_LIST_CACHE: OnceLock<Mutex<Option<ClaudeThreadListCacheEntry>>> =
    OnceLock::new();
static CLAUDE_ACTIVE_STEER_SENDERS: OnceLock<
    Mutex<BTreeMap<(String, String), mpsc::Sender<Value>>>,
> = OnceLock::new();

#[derive(Debug, Clone)]
struct ClaudeThreadListSnapshot {
    threads: BTreeMap<String, ClaudeThread>,
    generated_titles: Vec<ClaudeGeneratedTitle>,
    inline_titles: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ClaudeThreadListCacheEntry {
    projects_dir: Option<PathBuf>,
    workspace_name: Option<String>,
    scan_limit: Option<usize>,
    loaded_at_ms: i64,
    snapshot: ClaudeThreadListSnapshot,
}

#[derive(Debug, Clone)]
enum ClaudeThreadListTranscriptEntry {
    Thread {
        thread: ClaudeThread,
        inline_title: Option<String>,
    },
    GeneratedTitle(ClaudeGeneratedTitle),
}

#[derive(Debug, Clone)]
struct RunOptions {
    workspace_name: Option<String>,
}

#[derive(Debug)]
struct ClaudeAppServerState {
    active_processes: BTreeMap<(String, String), u32>,
    app_responses: BTreeMap<String, Value>,
    config_values: Map<String, Value>,
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
    git_info: Value,
    workspace_kind: String,
    workspace_roots: Vec<String>,
    workspace_browser_root: Option<String>,
    projectless_output_directory: Option<String>,
    base_instructions: Option<String>,
    developer_instructions: Option<String>,
    personality: Value,
    persist_extended_history: Value,
    model: String,
    reasoning_effort: Value,
    service_tier: Value,
    collaboration_mode: Value,
    created_at: i64,
    updated_at: i64,
    archived: bool,
    name: Option<String>,
    approval_policy: String,
    approvals_reviewer: String,
    turns: Vec<ClaudeTurn>,
    goal: Option<Value>,
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

#[derive(Debug, Clone, Default)]
struct ClaudeResumeTranscriptMetadata {
    session_id: Option<String>,
    cwd: Option<String>,
    first_prompt: Option<String>,
    last_prompt: Option<String>,
    agent_name: Option<String>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    summary: Option<String>,
    saw_message: bool,
    saw_sidechain_message: bool,
    saw_non_sidechain_message: bool,
    head_is_sidechain: bool,
    team_name: Option<String>,
    session_kind: Option<String>,
    entrypoint: Option<String>,
    is_loop_session: bool,
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
    approval_policy: String,
    approvals_reviewer: String,
    reasoning_effort: Value,
    service_tier: Value,
    collaboration_mode: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ThreadMetadataTextUpdate {
    Set(String),
    Clear,
}

#[derive(Debug, Clone)]
struct ThreadWorkspaceMetadata {
    kind: String,
    roots: Vec<String>,
    browser_root: Option<String>,
    projectless_output_directory: Option<String>,
}

#[derive(Debug, Clone)]
struct ThreadInstructionMetadata {
    base: Option<String>,
    developer: Option<String>,
    personality: Value,
    persist_extended_history: Value,
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
    input: Vec<Value>,
    instruction_context: Option<String>,
    resume_existing: bool,
    permission_mode: Option<String>,
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
    reasoning_item_completed: bool,
    reasoning_text: String,
    saw_tool_call: bool,
    seen_tool_ids: BTreeSet<String>,
    tool_block_by_index: BTreeMap<i64, String>,
    tool_input_deltas: BTreeMap<String, String>,
    tool_calls: BTreeMap<String, ClaudeToolCallState>,
    completed_tool_ids: BTreeSet<String>,
    completed_tool_items: Vec<Value>,
    subagent_streams: BTreeMap<String, ClaudeSubagentStreamState>,
}

#[derive(Debug, Default)]
struct ClaudeSubagentStreamState {
    emitted_text: String,
    pending_agent_text: String,
    reasoning_text: String,
    saw_tool_call: bool,
    tool_block_by_index: BTreeMap<i64, String>,
    tool_input_deltas: BTreeMap<String, String>,
    tool_order: Vec<String>,
    tool_calls: BTreeMap<String, ClaudeToolCallState>,
    completed_tools: BTreeMap<String, ClaudeSubagentToolCompletion>,
}

#[derive(Debug, Clone)]
struct ClaudeSubagentToolCompletion {
    success: bool,
    result: Option<String>,
    completed_at_ms: i64,
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
    FileChange,
    McpToolCall,
}

#[derive(Debug, Clone)]
struct TranscriptToolUse {
    id: String,
    name: String,
    input: Value,
    started_at: i64,
}

#[derive(Debug, Clone)]
struct TranscriptToolResult {
    tool_id: String,
    result: String,
    failed: bool,
    completed_at: i64,
}

#[derive(Debug, Clone)]
struct TranscriptToolCall {
    state: ClaudeToolCallState,
    result: Option<String>,
    failed: bool,
    completed_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct TranscriptTurnBuilder {
    input: Vec<Value>,
    started_at: i64,
    turn_suffix: Option<String>,
    completed_at: Option<i64>,
    failed: bool,
    error: Option<String>,
    agent_text: String,
    reasoning_text: String,
    tool_order: Vec<String>,
    tool_calls: BTreeMap<String, TranscriptToolCall>,
}

impl TranscriptTurnBuilder {
    fn new(input: Vec<Value>, started_at: i64) -> Self {
        Self {
            input,
            started_at,
            turn_suffix: None,
            completed_at: None,
            failed: false,
            error: None,
            agent_text: String::new(),
            reasoning_text: String::new(),
            tool_order: Vec::new(),
            tool_calls: BTreeMap::new(),
        }
    }

    fn record_assistant_message(
        &mut self,
        text: Option<String>,
        reasoning: Option<String>,
        tool_uses: Vec<TranscriptToolUse>,
        completed_at: i64,
        turn_suffix: Option<String>,
        failed: bool,
        error: Option<String>,
    ) {
        if let Some(turn_suffix) = turn_suffix {
            self.turn_suffix = Some(turn_suffix);
        }
        self.completed_at = Some(
            self.completed_at
                .map_or(completed_at, |value| value.max(completed_at)),
        );
        if failed {
            self.failed = true;
            self.error = error.or_else(|| Some("Claude Code turn failed".to_string()));
        }
        for tool_use in tool_uses {
            self.record_tool_use(tool_use);
        }
        if let Some(reasoning) = reasoning {
            if !self.reasoning_text.is_empty() {
                self.reasoning_text.push_str("\n\n");
            }
            self.reasoning_text.push_str(&reasoning);
        }
        if let Some(text) = text {
            if !self.agent_text.is_empty() {
                self.agent_text.push_str("\n\n");
            }
            self.agent_text.push_str(&text);
        }
    }

    fn record_tool_use(&mut self, tool_use: TranscriptToolUse) {
        if !self.tool_calls.contains_key(&tool_use.id) {
            self.tool_order.push(tool_use.id.clone());
        }
        let entry = self
            .tool_calls
            .entry(tool_use.id)
            .or_insert_with(|| TranscriptToolCall {
                state: ClaudeToolCallState {
                    name: tool_use.name.clone(),
                    arguments: json!({}),
                    started_at_ms: seconds_to_millis(tool_use.started_at),
                    started_emitted: true,
                    kind: claude_tool_item_kind(&tool_use.name),
                },
                result: None,
                failed: false,
                completed_at: None,
            });
        entry.state.name = tool_use.name;
        entry.state.arguments = tool_use.input;
        entry.state.started_at_ms = seconds_to_millis(tool_use.started_at);
        entry.state.kind = claude_tool_item_kind(&entry.state.name);
    }

    fn record_tool_result(&mut self, result: TranscriptToolResult) {
        if !self.tool_calls.contains_key(&result.tool_id) {
            self.tool_order.push(result.tool_id.clone());
        }
        self.completed_at = Some(
            self.completed_at
                .map_or(result.completed_at, |value| value.max(result.completed_at)),
        );
        let entry = self
            .tool_calls
            .entry(result.tool_id)
            .or_insert_with(|| TranscriptToolCall {
                state: ClaudeToolCallState {
                    name: "tool".to_string(),
                    arguments: json!({}),
                    started_at_ms: seconds_to_millis(result.completed_at),
                    started_emitted: true,
                    kind: ClaudeToolItemKind::McpToolCall,
                },
                result: None,
                failed: false,
                completed_at: None,
            });
        entry.result = Some(result.result);
        entry.failed |= result.failed;
        entry.completed_at = Some(result.completed_at);
    }

    fn into_turn(self, thread_id: &str, cwd: &str, index: usize) -> Option<ClaudeTurn> {
        if self.agent_text.trim().is_empty() && self.tool_calls.is_empty() && !self.failed {
            return None;
        }
        let completed_at = self.completed_at.unwrap_or(self.started_at);
        let turn_suffix = self.turn_suffix.unwrap_or_else(|| index.to_string());
        let turn_id = format!("turn-{turn_suffix}");
        let mut tool_items = Vec::new();
        if !self.reasoning_text.trim().is_empty() {
            tool_items.push(reasoning_item_json(&turn_id, self.reasoning_text.trim()));
        }
        tool_items.extend(
            self.tool_order
                .iter()
                .filter_map(|tool_id| self.tool_calls.get(tool_id).map(|call| (tool_id, call)))
                .map(|(tool_id, call)| {
                    let duration_ms = call
                        .completed_at
                        .map(|completed_at| {
                            (seconds_to_millis(completed_at) - call.state.started_at_ms).max(0)
                        })
                        .map(|duration_ms| json!(duration_ms))
                        .unwrap_or(Value::Null);
                    tool_call_item(
                        thread_id,
                        cwd,
                        tool_id,
                        &call.state,
                        if call.failed { "failed" } else { "completed" },
                        call.result.as_deref(),
                        duration_ms,
                    )
                }),
        );
        Some(ClaudeTurn {
            id: turn_id,
            input: self.input,
            tool_items,
            agent_text: self.agent_text.trim().to_string(),
            status: if self.failed {
                TurnStatus::Failed
            } else {
                TurnStatus::Completed
            },
            error: self.error,
            started_at: self.started_at,
            completed_at: Some(completed_at),
            duration_ms: Some((completed_at - self.started_at).max(0) * 1000),
            approval_policy: DEFAULT_APPROVAL_POLICY.to_string(),
            approvals_reviewer: DEFAULT_APPROVALS_REVIEWER.to_string(),
            reasoning_effort: Value::Null,
            service_tier: Value::Null,
            collaboration_mode: Value::Null,
        })
    }
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
        config_values: Map::new(),
        interrupted_turns: BTreeSet::new(),
        threads: BTreeMap::new(),
        workspace_name: options.workspace_name,
    }));
    let (bot_input_tx, bot_input_rx) = mpsc::channel();
    let bot_bridge_stdout_tx = bot_bridge::spawn_app_stdio_bot_bridge(
        bot_bridge::shared_app_stdin(ClaudeBotBridgeInput::new(bot_input_tx)),
    );
    let output = Arc::new(Mutex::new(ClaudeBotBridgeOutput::new(
        output,
        bot_bridge_stdout_tx.clone(),
    )));
    let _bot_bridge_input_worker = if bot_bridge_stdout_tx.is_some() {
        let bot_state = Arc::clone(&state);
        let bot_output = Arc::clone(&output);
        Some(thread::spawn(move || {
            handle_bot_bridge_input(bot_input_rx, bot_state, bot_output);
        }))
    } else {
        None
    };
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

fn handle_bot_bridge_input<W>(
    input: mpsc::Receiver<Vec<u8>>,
    state: SharedState,
    output: SharedOutput<W>,
) where
    W: Write + Send + 'static,
{
    let mut workers = Vec::new();
    for line in input {
        match handle_client_line(&line, Arc::clone(&state), Arc::clone(&output)) {
            Ok(Some(worker)) => workers.push(worker),
            Ok(None) => {}
            Err(err) => {
                claude_code_log_event(
                    "bot_bridge_input_error",
                    json!({
                        "error": err,
                    }),
                );
            }
        }
    }

    for worker in workers {
        if worker.join().is_err() {
            claude_code_log_event("bot_bridge_worker_panic", json!({}));
        }
    }
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
    let _log_lock = CLAUDE_CODE_LOG_LOCK.lock().ok();
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
        "thread/list" | "thread/search" => {
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
        "thread/turns/list" | "turn/list" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_turns_list(&params)?
            };
            write_response(&output, id, response)?;
        }
        "thread/turns/items/list" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_turns_items_list(&params)?
            };
            write_response(&output, id, response)?;
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
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_metadata_update(&params)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/pin" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_pinned_set(&params, true)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/unpin" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_pinned_set(&params, false)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/pinned/list" | "thread/pins/list" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_pinned_list()
            };
            write_response(&output, id, response)?;
        }
        "thread/memoryMode/get" | "thread/memory/get" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_memory_mode_get(&params)?
            };
            write_response(&output, id, response)?;
        }
        "thread/memoryMode/set" | "thread/memory/set" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_memory_mode_set(&params)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/memoryMode/clear" | "thread/memory/clear" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_memory_mode_clear(&params)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/prewarm" | "thread/prewarm/start" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.prewarm_thread(&params)
            };
            write_response(&output, id, response)?;
            write_notification(&output, notification)?;
        }
        "thread/prewarm/clear" | "thread/prewarm/clearAll" => {
            write_response(&output, id, json!({}))?;
        }
        "thread/goal/get" => {
            let response = {
                let state = lock_state(&state)?;
                state.thread_goal_get(&params)?
            };
            write_response(&output, id, response)?;
        }
        "thread/goal/set" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_goal_set(&params)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
        }
        "thread/goal/clear" => {
            let (response, notification) = {
                let mut state = lock_state(&state)?;
                state.thread_goal_clear(&params)?
            };
            write_response(&output, id, response)?;
            if let Some(notification) = notification {
                write_notification(&output, notification)?;
            }
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
        "turn/steer" => {
            let response = {
                let state = lock_state(&state)?;
                state.steer_turn(&params)?
            };
            write_response(&output, id, response)?;
        }
        "model/list" => {
            write_response(&output, id, claude_code_model_list_response(&params))?;
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
            write_response(&output, id, claude_code_collaboration_mode_list_response())?;
        }
        "config/read" => {
            let response = {
                let state = lock_state(&state)?;
                state.config_read(&params)
            };
            write_response(&output, id, response)?;
        }
        "config/value/write" | "config/batchWrite" => {
            let response = {
                let mut state = lock_state(&state)?;
                state.config_write(method, &params)
            };
            write_response(&output, id, response)?;
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
            | "thread/search"
            | "thread/loaded/list"
            | "thread/turns/list"
            | "turn/list"
            | "thread/turns/items/list"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/unsubscribe"
            | "thread/name/set"
            | "thread/metadata/update"
            | "thread/pin"
            | "thread/unpin"
            | "thread/pinned/list"
            | "thread/pins/list"
            | "thread/memoryMode/get"
            | "thread/memoryMode/set"
            | "thread/memoryMode/clear"
            | "thread/memory/get"
            | "thread/memory/set"
            | "thread/memory/clear"
            | "thread/prewarm"
            | "thread/prewarm/start"
            | "thread/prewarm/clear"
            | "thread/prewarm/clearAll"
            | "thread/goal/get"
            | "thread/goal/set"
            | "thread/goal/clear"
            | "turn/start"
            | "turn/interrupt"
            | "turn/steer"
            | "account/read"
            | "getAuthStatus"
            | "config/read"
            | "config/value/write"
            | "config/batchWrite"
    )
}

fn standalone_codex_app_result(method: &str, params: &Value) -> Option<Value> {
    if method_removes_protected_bundled_plugin(method, params) {
        return Some(json!({}));
    }

    if !codex_cli_app_server_proxy_for_fast_methods_enabled() {
        if let Some(result) = fast_standalone_codex_app_result(method, params) {
            return Some(result);
        }
    }

    if should_proxy_codex_app_method(method) {
        if let Some(result) = codex_cli_app_server_method_result(method, params) {
            return Some(normalize_proxied_codex_app_result(method, params, result));
        }
    }

    if let Some(result) = fast_standalone_codex_app_result(method, params) {
        return Some(result);
    }

    match method {
        "plugin/read" => Some(standalone_plugin_read_result(params)),
        "plugin/install" => Some(standalone_plugin_install_result(params)),
        _ => None,
    }
}

fn fast_standalone_codex_app_result(method: &str, params: &Value) -> Option<Value> {
    match method {
        "config/mcpServer/reload" => Some(standalone_mcp_server_lifecycle_result(method, params)),
        "memory/reset"
        | "experimentalFeature/enablement/set"
        | "marketplace/add"
        | "marketplace/remove"
        | "marketplace/upgrade" => Some(json!({})),
        "externalAgentConfig/detect" => Some(json!({ "items": [] })),
        "externalAgentConfig/import" => Some(json!({})),
        "config/value/write" | "config/batchWrite" => Some(config_write_response(params)),
        "fs/readFile" => Some(fs_read_file_response(params)),
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
        "collaborationMode/list" => Some(claude_code_collaboration_mode_list_response()),
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
        "plugin/uninstall" | "plugin/remove" | "plugin/delete" | "plugin/enable"
        | "plugin/disable" => Some(standalone_plugin_lifecycle_result(method, params)),
        method
            if method.starts_with("plugin/")
                && !matches!(method, "plugin/read" | "plugin/install") =>
        {
            Some(json!({}))
        }
        method if method.starts_with("marketplace/") => Some(json!({})),
        "app/list" => Some(json!({
            "data": standalone_app_list(),
            "nextCursor": Value::Null,
        })),
        "mcpServerStatus/list" => Some(json!({
            "data": standalone_mcp_server_status_list(),
            "nextCursor": Value::Null,
        })),
        method if method.starts_with("mcpServer/") => {
            Some(standalone_mcp_server_lifecycle_result(method, params))
        }
        "model/list" => Some(claude_code_model_list_response(params)),
        "permissionProfile/list" | "experimentalFeature/list" => Some(json!({
            "data": [],
            "nextCursor": Value::Null,
        })),
        _ => None,
    }
}

fn claude_code_collaboration_mode_list_response() -> Value {
    json!({
        "data": [
            {
                "mode": "plan",
                "model": DEFAULT_MODEL,
                "reasoning_effort": Value::Null,
            },
            {
                "mode": "default",
                "model": DEFAULT_MODEL,
                "reasoning_effort": Value::Null,
            },
        ],
    })
}

fn claude_code_model_list_response(params: &Value) -> Value {
    let models = claude_code_model_list();
    let offset = params
        .get("cursor")
        .and_then(Value::as_str)
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .filter(|limit| *limit > 0)
        .unwrap_or(models.len());
    let end = offset.saturating_add(limit).min(models.len());
    let data = if offset < models.len() {
        models[offset..end].to_vec()
    } else {
        Vec::new()
    };
    let next_cursor = if end < models.len() {
        Value::String(end.to_string())
    } else {
        Value::Null
    };
    json!({
        "data": data,
        "nextCursor": next_cursor,
    })
}

fn claude_code_model_list() -> Vec<Value> {
    let selected = model_from_params(&Value::Null);
    let mut ids = Vec::new();
    push_unique_model_id(&mut ids, &selected);
    for model in [DEFAULT_MODEL, "sonnet", "opus", "haiku"] {
        push_unique_model_id(&mut ids, model);
    }
    ids.into_iter()
        .map(|model| claude_code_model_item(&model, &selected))
        .collect()
}

fn push_unique_model_id(models: &mut Vec<String>, model: &str) {
    let model = model.trim();
    if !model.is_empty() && !models.iter().any(|item| item == model) {
        models.push(model.to_string());
    }
}

fn claude_code_model_item(model: &str, selected: &str) -> Value {
    let display_name = if model == DEFAULT_MODEL {
        "Claude Code".to_string()
    } else {
        format!("Claude Code ({model})")
    };
    json!({
        "id": model,
        "model": model,
        "modelProvider": PROVIDER_NAME,
        "displayName": display_name,
        "description": "Claude Code model alias",
        "hidden": false,
        "isDefault": model == selected,
        "contextWindow": claude_context_window_for_model(model),
        "inputModalities": ["text", "image"],
        "supportedReasoningEfforts": [],
        "defaultReasoningEffort": Value::Null,
        "supportsPersonality": false,
        "additionalSpeedTiers": [],
        "serviceTiers": [],
        "defaultServiceTier": Value::Null,
        "upgrade": Value::Null,
        "upgradeInfo": Value::Null,
        "availabilityNux": Value::Null,
    })
}

fn config_write_response(params: &Value) -> Value {
    json!({
        "status": "ok",
        "version": now_millis().to_string(),
        "filePath": config_write_file_path(params),
        "overriddenMetadata": Value::Null,
    })
}

fn config_write_file_path(params: &Value) -> String {
    params
        .get("filePath")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .unwrap_or_else(|| {
            PathBuf::from(crate::config::default_codex_home())
                .join("config.toml")
                .to_string_lossy()
                .to_string()
        })
}

fn fs_read_file_response(params: &Value) -> Value {
    let data = params
        .get("path")
        .and_then(Value::as_str)
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();

    json!({
        "dataBase64": general_purpose::STANDARD.encode(data),
    })
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

fn codex_cli_app_server_proxy_for_fast_methods_enabled() -> bool {
    std::env::var(CODEX_APP_SERVER_PROXY_ENV)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
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
    let mut plugin = json!({
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
    apply_standalone_plugin_state(&mut plugin);
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
    if let Some(entry) = find_standalone_plugin_entry(params) {
        persist_standalone_plugin_lifecycle(&entry.plugin, true, true);
    } else if let Some(plugin_name) = plugin_request_name(params) {
        persist_named_plugin_lifecycle(plugin_name, true, true);
    }
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

fn standalone_plugin_lifecycle_result(method: &str, params: &Value) -> Value {
    let (installed, enabled) = match method {
        "plugin/enable" => (true, true),
        "plugin/disable" => (true, false),
        "plugin/uninstall" | "plugin/remove" | "plugin/delete" => (false, false),
        _ => (true, true),
    };
    if let Some(entry) = find_standalone_plugin_entry(params) {
        persist_standalone_plugin_lifecycle(&entry.plugin, installed, enabled);
        return json!({
            "plugin": apply_standalone_plugin_lifecycle_json(entry.plugin, installed, enabled),
        });
    }
    if let Some(plugin_name) = plugin_request_name(params) {
        persist_named_plugin_lifecycle(plugin_name, installed, enabled);
    }
    json!({})
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

fn apply_standalone_plugin_state(plugin: &mut Value) {
    let Some(state) = standalone_plugin_state_for_plugin(plugin) else {
        return;
    };
    let installed = state
        .get("installed")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            plugin
                .get("installed")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
    let enabled = state
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            plugin
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        })
        && installed;
    if let Some(object) = plugin.as_object_mut() {
        object.insert("installed".to_string(), json!(installed));
        object.insert("enabled".to_string(), json!(enabled));
    }
}

fn apply_standalone_plugin_lifecycle_json(
    mut plugin: Value,
    installed: bool,
    enabled: bool,
) -> Value {
    if let Some(object) = plugin.as_object_mut() {
        object.insert("installed".to_string(), json!(installed));
        object.insert("enabled".to_string(), json!(enabled && installed));
    }
    plugin
}

fn persist_standalone_plugin_lifecycle(plugin: &Value, installed: bool, enabled: bool) {
    for key in plugin_state_keys_for_plugin(plugin) {
        persist_named_plugin_lifecycle(&key, installed, enabled);
    }
}

fn persist_named_plugin_lifecycle(plugin_name: &str, installed: bool, enabled: bool) {
    let plugin_name = plugin_name.trim();
    if plugin_name.is_empty() {
        return;
    }
    let mut state = load_standalone_lifecycle_state(claude_plugin_state_path);
    state.insert(
        plugin_name.to_string(),
        json!({
            "installed": installed,
            "enabled": enabled && installed,
        }),
    );
    persist_standalone_lifecycle_state(claude_plugin_state_path, &state);
}

fn standalone_plugin_state_for_plugin(plugin: &Value) -> Option<Value> {
    let state = load_standalone_lifecycle_state(claude_plugin_state_path);
    plugin_state_keys_for_plugin(plugin)
        .into_iter()
        .find_map(|key| state.get(&key).cloned())
}

fn plugin_state_keys_for_plugin(plugin: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    for pointer in ["/id", "/name", "/source/path", "/path"] {
        if let Some(value) = plugin
            .pointer(pointer)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            if seen.insert(value.clone()) {
                keys.push(value);
            }
        }
    }
    if let Some(path) = plugin.pointer("/source/path").and_then(Value::as_str) {
        if let Some(name) = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(non_empty_string)
        {
            if seen.insert(name.clone()) {
                keys.push(name);
            }
        }
    }
    keys
}

fn plugin_mcp_config_path_enabled(config_path: &Path) -> bool {
    let config_key = canonical_key(config_path);
    for entry in standalone_plugin_entries() {
        let Some(mcp_path) = plugin_manifest_mcp_config_path(&entry.manifest_path) else {
            continue;
        };
        if canonical_key(&mcp_path) != config_key {
            continue;
        }
        let installed = entry
            .plugin
            .get("installed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let enabled = entry
            .plugin
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        return installed && enabled;
    }
    true
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
        for mut server in parse_mcp_servers_from_config(&content, &config_path) {
            apply_standalone_mcp_server_state(&mut server);
            if standalone_mcp_server_removed(&server) {
                continue;
            }
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
        let plugin_enabled = plugin_mcp_config_path_enabled(&config_path);
        for mut server in parse_mcp_servers_from_json(&value, &config_path, base_dir, "plugin") {
            apply_standalone_mcp_server_state(&mut server);
            if !plugin_enabled {
                server.enabled = false;
            }
            if standalone_mcp_server_removed(&server) {
                continue;
            }
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

fn standalone_mcp_server_lifecycle_result(method: &str, params: &Value) -> Value {
    if matches!(method, "config/mcpServer/reload") {
        return json!({
            "data": standalone_mcp_server_status_list(),
            "nextCursor": Value::Null,
        });
    }
    let Some(server_name) = mcp_server_request_name(params) else {
        return json!({});
    };
    let (enabled, removed) = match method {
        "mcpServer/enable" => (true, false),
        "mcpServer/disable" => (false, false),
        "mcpServer/remove" | "mcpServer/delete" | "mcpServer/uninstall" => (false, true),
        _ => (true, false),
    };
    persist_named_mcp_server_lifecycle(&server_name, enabled, removed);
    json!({
        "serverName": server_name,
        "enabled": enabled,
        "removed": removed,
    })
}

fn mcp_server_request_name(params: &Value) -> Option<String> {
    [
        "serverName",
        "server_name",
        "name",
        "id",
        "mcpServerName",
        "mcp_server_name",
    ]
    .into_iter()
    .find_map(|key| {
        params
            .get(key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
    })
}

fn persist_named_mcp_server_lifecycle(server_name: &str, enabled: bool, removed: bool) {
    let server_name = server_name.trim();
    if server_name.is_empty() {
        return;
    }
    let mut state = load_standalone_lifecycle_state(claude_mcp_server_state_path);
    state.insert(
        server_name.to_string(),
        json!({
            "enabled": enabled,
            "removed": removed,
        }),
    );
    persist_standalone_lifecycle_state(claude_mcp_server_state_path, &state);
}

fn apply_standalone_mcp_server_state(server: &mut StandaloneMcpServer) {
    let state = load_standalone_lifecycle_state(claude_mcp_server_state_path);
    if let Some(value) = state
        .get(&server.name)
        .or_else(|| state.get(&server.config_path))
    {
        if let Some(enabled) = value.get("enabled").and_then(Value::as_bool) {
            server.enabled = enabled;
        }
    }
}

fn standalone_mcp_server_removed(server: &StandaloneMcpServer) -> bool {
    let state = load_standalone_lifecycle_state(claude_mcp_server_state_path);
    state
        .get(&server.name)
        .or_else(|| state.get(&server.config_path))
        .and_then(|value| value.get("removed"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
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
    if let Some(active_home) = config.active_codex_home() {
        push_unique_path(
            &mut homes,
            &mut seen,
            PathBuf::from(crate::config::normalize_home_path(active_home)),
        );
    }
    if scan_all_codex_homes_enabled() {
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
    }
    homes
}

fn scan_all_codex_homes_enabled() -> bool {
    std::env::var(SCAN_ALL_CODEX_HOMES_ENV)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no" | "")
        })
        .unwrap_or(false)
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
        let workspace_metadata = thread_workspace_metadata_from_params(params, &cwd);
        let instruction_metadata = thread_instruction_metadata_from_params(params);
        let model = model_from_params(params);
        let reasoning_effort = reasoning_effort_from_params(params, None);
        let service_tier = service_tier_from_params(params, None);
        let collaboration_mode =
            collaboration_mode_from_params(params, &model, &reasoning_effort, None);
        let approval_policy = approval_policy_from_params(params, None);
        let approvals_reviewer = approvals_reviewer_from_params(params, None);
        let now = now_seconds();
        let name = self.workspace_name.clone();
        let git_info = git_info_from_params(params).unwrap_or_else(|| git_info_for_cwd(&cwd));
        let thread = ClaudeThread {
            id: id.clone(),
            session_id: id.clone(),
            claude_session_id: id,
            path: None,
            preview: String::new(),
            cwd,
            git_info,
            workspace_kind: workspace_metadata.kind,
            workspace_roots: workspace_metadata.roots,
            workspace_browser_root: workspace_metadata.browser_root,
            projectless_output_directory: workspace_metadata.projectless_output_directory,
            base_instructions: instruction_metadata.base,
            developer_instructions: instruction_metadata.developer,
            personality: instruction_metadata.personality,
            persist_extended_history: instruction_metadata.persist_extended_history,
            model,
            reasoning_effort,
            service_tier,
            collaboration_mode,
            created_at: now,
            updated_at: now,
            archived: false,
            name,
            approval_policy,
            approvals_reviewer,
            turns: Vec::new(),
            goal: None,
            latest_token_usage_info: None,
        };
        apply_new_thread_persisted_overlays(&thread.id, params);
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
            if let Some(thread) = self.virtual_subagent_thread_for_request(lookup_thread_id) {
                let include_turns = !params
                    .get("excludeTurns")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let response = thread_runtime_response(&thread, include_turns);
                let notification = json!({
                    "method": "thread/started",
                    "params": { "thread": thread.to_json(false) },
                });
                return Ok((response, notification));
            }
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
        apply_thread_workspace_metadata_from_params(&mut response_thread, params);
        apply_thread_runtime_metadata_from_params(&mut response_thread, params);
        apply_thread_instruction_metadata_from_params(&mut response_thread, params);
        apply_thread_git_info_from_params(&mut response_thread, params);
        if let Some(thread) = self.threads.get_mut(&response_thread.id) {
            apply_thread_workspace_metadata_from_params(thread, params);
            apply_thread_runtime_metadata_from_params(thread, params);
            apply_thread_instruction_metadata_from_params(thread, params);
            apply_thread_git_info_from_params(thread, params);
        }
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
        if let Some(thread) = self.virtual_subagent_thread_for_request(lookup_thread_id) {
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
        let snapshot = load_claude_thread_list_snapshot(params, self.workspace_name.clone());
        let mut threads = snapshot.threads;
        let mut generated_titles = snapshot.generated_titles;
        let inline_titles = snapshot.inline_titles;
        for thread in self
            .threads
            .values()
            .filter(|thread| !is_claude_subagent_thread_id(&thread.id))
        {
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
        let thread_id = required_thread_id_param(params)?;
        let thread = self.thread_for_request(thread_id)?;
        if is_claude_title_generation_thread(&thread) {
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

    fn thread_turns_items_list(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_thread_id_param(params)?;
        let thread = self.thread_for_request(thread_id)?;
        if is_claude_title_generation_thread(&thread) {
            return Err(format!("thread not found: {}", thread_id));
        }

        let requested_turn_id = params.get("turnId").and_then(Value::as_str);
        let mut turns = thread
            .turns
            .iter()
            .filter(|turn| requested_turn_id.map_or(true, |turn_id| turn.id == turn_id))
            .collect::<Vec<_>>();
        if !matches!(
            params.get("sortDirection").and_then(Value::as_str),
            Some("asc")
        ) {
            turns.reverse();
        }
        let mut items = turns
            .into_iter()
            .flat_map(|turn| match turn.items_json() {
                Value::Array(items) => items,
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        if let Some(limit) = params.get("limit").and_then(Value::as_u64) {
            items.truncate(limit as usize);
        }
        Ok(json!({
            "data": items,
            "nextCursor": Value::Null,
            "backwardsCursor": Value::Null,
        }))
    }

    fn thread_goal_get(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_param(params, "threadId")?;
        let goal = self
            .threads
            .get(thread_id)
            .or_else(|| self.threads.get(strip_local_thread_prefix(thread_id)))
            .and_then(|thread| thread.goal.clone())
            .or_else(|| persisted_claude_thread_goal(thread_id))
            .unwrap_or(Value::Null);
        Ok(json!({ "goal": goal }))
    }

    fn thread_goal_set(&mut self, params: &Value) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        let goal = thread_goal_from_params(params);
        persist_claude_thread_goal(thread_id, Some(&goal));
        let notification =
            self.set_loaded_thread_goal(thread_id, (!goal.is_null()).then(|| goal.clone()));
        Ok((json!({ "goal": goal }), notification))
    }

    fn thread_goal_clear(&mut self, params: &Value) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        persist_claude_thread_goal(thread_id, None);
        let notification = self.set_loaded_thread_goal(thread_id, None);
        Ok((json!({ "goal": Value::Null }), notification))
    }

    fn config_read(&self, params: &Value) -> Value {
        config_read_response(params, &self.config_values)
    }

    fn config_write(&mut self, method: &str, params: &Value) -> Value {
        apply_config_write_params(method, params, &mut self.config_values);
        config_write_response(params)
    }

    fn thread_for_request(&self, thread_id: &str) -> Result<ClaudeThread, String> {
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        self.threads
            .get(thread_id)
            .or_else(|| self.threads.get(lookup_thread_id))
            .cloned()
            .or_else(|| load_claude_thread_by_id(lookup_thread_id, self.workspace_name.clone()))
            .or_else(|| self.virtual_subagent_thread_for_request(lookup_thread_id))
            .ok_or_else(|| format!("thread not found: {}", thread_id))
    }

    fn virtual_subagent_thread_for_request(&self, thread_id: &str) -> Option<ClaudeThread> {
        let thread_id = strip_local_thread_prefix(thread_id);
        if !is_claude_subagent_thread_id(thread_id) {
            return None;
        }
        for parent_thread in self.threads.values() {
            for turn in &parent_thread.turns {
                for item in &turn.tool_items {
                    if item.get("type").and_then(Value::as_str) != Some("collabAgentToolCall") {
                        continue;
                    }
                    if collab_agent_item_references_thread(item, thread_id) {
                        return Some(virtual_subagent_thread_from_item(
                            thread_id,
                            parent_thread,
                            turn,
                            item,
                        ));
                    }
                }
            }
        }
        Some(fallback_virtual_subagent_thread(
            thread_id,
            self.workspace_name.as_deref(),
        ))
    }

    fn set_loaded_thread_goal(&mut self, thread_id: &str, goal: Option<Value>) -> Option<Value> {
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        let thread = if let Some(thread) = self.threads.get_mut(thread_id) {
            thread
        } else {
            self.threads.get_mut(lookup_thread_id)?
        };
        thread.goal = goal;
        thread.updated_at = now_seconds();
        Some(claude_thread_stream_state_changed_notification(thread))
    }

    fn set_archived(&mut self, params: &Value, archived: bool) -> Option<Value> {
        let thread_id = params.get("threadId").and_then(Value::as_str)?;
        persist_claude_thread_archived(thread_id, archived);
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        let thread = if let Some(thread) = self.threads.get_mut(thread_id) {
            thread
        } else {
            self.threads.get_mut(lookup_thread_id)?
        };
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

    fn thread_metadata_update(&mut self, params: &Value) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        if !self.threads.contains_key(thread_id) && !self.threads.contains_key(lookup_thread_id) {
            let thread = load_claude_thread_by_id(lookup_thread_id, self.workspace_name.clone())
                .ok_or_else(|| format!("thread not found: {}", thread_id))?;
            self.threads.insert(thread.id.clone(), thread);
        }

        let thread = if let Some(thread) = self.threads.get_mut(thread_id) {
            thread
        } else {
            self.threads
                .get_mut(lookup_thread_id)
                .ok_or_else(|| format!("thread not loaded: {}", thread_id))?
        };
        if is_claude_title_generation_thread(thread) {
            return Err(format!("thread not found: {}", thread_id));
        }

        let mut changed = false;
        if let Some(name) = thread_metadata_text_update(params, &["name", "title"]) {
            let thread_name_id = thread.id.clone();
            match name {
                ThreadMetadataTextUpdate::Set(name) => {
                    if thread.name.as_deref() != Some(name.as_str()) {
                        thread.name = Some(name.clone());
                        changed = true;
                    }
                    persist_claude_thread_name(&thread_name_id, Some(&name));
                }
                ThreadMetadataTextUpdate::Clear => {
                    if thread.name.is_some() {
                        thread.name = None;
                        changed = true;
                    }
                    persist_claude_thread_name(&thread_name_id, None);
                }
            }
        }
        if let Some(cwd) = thread_metadata_string(params, &["cwd"]) {
            let cwd = normalize_cwd(Some(&cwd));
            if thread.cwd != cwd {
                update_thread_cwd(thread, cwd);
                changed = true;
            }
        }
        if let Some(git_info) = git_info_update_from_params(params) {
            if thread.git_info != git_info {
                thread.git_info = git_info;
                changed = true;
            }
        }
        if let Some(workspace_kind) =
            thread_metadata_string(params, &["workspaceKind", "workspace_kind"])
        {
            if thread.workspace_kind != workspace_kind {
                thread.workspace_kind = workspace_kind;
                changed = true;
            }
        }
        if let Some(workspace_roots) =
            thread_metadata_string_array(params, &["workspaceRoots", "workspace_roots"])
        {
            let workspace_roots = normalize_workspace_roots(workspace_roots, &thread.cwd);
            if thread.workspace_roots != workspace_roots {
                thread.workspace_roots = workspace_roots;
                changed = true;
            }
        }
        if let Some(browser_root) = thread_metadata_optional_string_update(
            params,
            &[
                "workspaceBrowserRoot",
                "workspace_browser_root",
                "workspaceRoot",
                "workspace_root",
            ],
        ) {
            if thread.workspace_browser_root != browser_root {
                thread.workspace_browser_root = browser_root;
                changed = true;
            }
        }
        if let Some(output_directory) = thread_metadata_optional_string_update(
            params,
            &[
                "projectlessOutputDirectory",
                "projectless_output_directory",
                "outputDirectory",
                "output_directory",
            ],
        ) {
            if thread.projectless_output_directory != output_directory {
                thread.projectless_output_directory = output_directory;
                changed = true;
            }
        }
        if let Some(model) = thread_metadata_string(params, &["model"]) {
            if thread.model != model {
                thread.model = model;
                changed = true;
            }
        }
        if let Some(reasoning_effort) = thread_runtime_metadata_value(
            params,
            &["reasoningEffort", "reasoning_effort", "effort"],
        ) {
            if thread.reasoning_effort != reasoning_effort {
                thread.reasoning_effort = reasoning_effort;
                changed = true;
            }
        }
        if let Some(service_tier) =
            thread_runtime_metadata_value(params, &["serviceTier", "service_tier"])
        {
            if thread.service_tier != service_tier {
                thread.service_tier = service_tier;
                changed = true;
            }
        }
        if let Some(collaboration_mode) = thread_metadata_value(params, &["collaborationMode"])
            .filter(|value| !value.is_null())
            .cloned()
        {
            if thread.collaboration_mode != collaboration_mode {
                thread.collaboration_mode = normalized_collaboration_mode(
                    collaboration_mode,
                    &thread.model,
                    &thread.reasoning_effort,
                );
                changed = true;
            }
        }
        if let Some(base_instructions) = thread_metadata_optional_string_update(
            params,
            &["baseInstructions", "base_instructions"],
        ) {
            if thread.base_instructions != base_instructions {
                thread.base_instructions = base_instructions;
                changed = true;
            }
        }
        if let Some(developer_instructions) =
            optional_combined_developer_instructions_from_params(params)
        {
            if thread.developer_instructions != developer_instructions {
                thread.developer_instructions = developer_instructions;
                changed = true;
            }
        }
        if let Some(personality) = thread_runtime_metadata_value(params, &["personality"]) {
            if thread.personality != personality {
                thread.personality = personality;
                changed = true;
            }
        }
        if let Some(persist_extended_history) = thread_runtime_metadata_value(
            params,
            &["persistExtendedHistory", "persist_extended_history"],
        ) {
            if thread.persist_extended_history != persist_extended_history {
                thread.persist_extended_history = persist_extended_history;
                changed = true;
            }
        }
        if let Some(preview) = thread_metadata_string(params, &["preview"]) {
            if thread.preview != preview {
                thread.preview = preview;
                changed = true;
            }
        }
        if let Some(approval_policy) =
            thread_metadata_string(params, &["approvalPolicy", "approval_policy"])
        {
            if thread.approval_policy != approval_policy {
                thread.approval_policy = approval_policy;
                changed = true;
            }
        }
        if let Some(approvals_reviewer) =
            thread_metadata_string(params, &["approvalsReviewer", "approvals_reviewer"])
        {
            if thread.approvals_reviewer != approvals_reviewer {
                thread.approvals_reviewer = approvals_reviewer;
                changed = true;
            }
        }
        if let Some(archived) = thread_metadata_bool(params, &["archived"]) {
            let thread_archive_id = thread.id.clone();
            if thread.archived != archived {
                thread.archived = archived;
                changed = true;
            }
            persist_claude_thread_archived(&thread_archive_id, archived);
        }
        if let Some(pinned) = thread_metadata_bool(params, &["pinned"]) {
            let thread_pin_id = thread.id.clone();
            persist_claude_thread_pinned(&thread_pin_id, pinned);
            changed = true;
        }
        if let Some(memory_mode) =
            thread_metadata_value(params, &["memoryMode", "memory_mode"]).cloned()
        {
            let thread_memory_id = thread.id.clone();
            if memory_mode.is_null() {
                persist_claude_thread_memory_mode(&thread_memory_id, None);
            } else {
                persist_claude_thread_memory_mode(&thread_memory_id, Some(&memory_mode));
            }
            changed = true;
        }
        if changed {
            thread.updated_at = now_seconds();
        }

        let include_turns = params
            .get("includeTurns")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let response = json!({ "thread": thread.to_json(include_turns) });
        let notification = changed.then(|| claude_thread_stream_state_changed_notification(thread));
        Ok((response, notification))
    }

    fn thread_pinned_list(&self) -> Value {
        let mut ids = load_claude_thread_pinned();
        for thread in self.threads.values() {
            if persisted_claude_thread_pinned(&thread.id) {
                ids.insert(strip_local_thread_prefix(&thread.id).to_string());
            }
        }
        let thread_ids = ids.into_iter().collect::<Vec<_>>();
        json!({
            "threadIds": thread_ids,
            "data": thread_ids,
            "nextCursor": Value::Null,
        })
    }

    fn thread_pinned_set(
        &mut self,
        params: &Value,
        pinned: bool,
    ) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        persist_claude_thread_pinned(thread_id, pinned);
        let notification = self.thread_stream_state_notification_for_id(thread_id);
        Ok((
            json!({
                "threadId": thread_id,
                "pinned": pinned,
            }),
            notification,
        ))
    }

    fn thread_memory_mode_get(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_param(params, "threadId")?;
        let memory_mode = persisted_claude_thread_memory_mode(thread_id).unwrap_or(Value::Null);
        Ok(json!({
            "threadId": thread_id,
            "memoryMode": memory_mode,
        }))
    }

    fn thread_memory_mode_set(&mut self, params: &Value) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        let memory_mode = thread_memory_mode_from_params(params);
        persist_claude_thread_memory_mode(thread_id, Some(&memory_mode));
        let notification = self.thread_stream_state_notification_for_id(thread_id);
        Ok((
            json!({
                "threadId": thread_id,
                "memoryMode": memory_mode,
            }),
            notification,
        ))
    }

    fn thread_memory_mode_clear(
        &mut self,
        params: &Value,
    ) -> Result<(Value, Option<Value>), String> {
        let thread_id = required_param(params, "threadId")?;
        persist_claude_thread_memory_mode(thread_id, None);
        let notification = self.thread_stream_state_notification_for_id(thread_id);
        Ok((
            json!({
                "threadId": thread_id,
                "memoryMode": Value::Null,
            }),
            notification,
        ))
    }

    fn prewarm_thread(&mut self, params: &Value) -> (Value, Value) {
        let (mut response, notification) = self.start_thread(params);
        if let Some(object) = response.as_object_mut() {
            object.insert("prewarmed".to_string(), json!(true));
        }
        (response, notification)
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
                update_thread_cwd(thread, normalize_cwd(Some(cwd)));
            }
            apply_thread_workspace_metadata_from_params(thread, params);
            apply_thread_instruction_metadata_from_params(thread, params);
            apply_thread_git_info_from_params(thread, params);
            if let Some(model) = params.get("model").and_then(Value::as_str) {
                thread.model = model.to_string();
            }
            thread.reasoning_effort =
                reasoning_effort_from_params(params, Some(&thread.reasoning_effort));
            thread.service_tier = service_tier_from_params(params, Some(&thread.service_tier));
            thread.collaboration_mode = collaboration_mode_from_params(
                params,
                &thread.model,
                &thread.reasoning_effort,
                Some(&thread.collaboration_mode),
            );
            thread.approval_policy =
                approval_policy_from_params(params, Some(&thread.approval_policy));
            thread.approvals_reviewer =
                approvals_reviewer_from_params(params, Some(&thread.approvals_reviewer));
        }
        let mut input = params
            .get("input")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        append_turn_attachments_to_input(&mut input, params);
        let prompt = prompt_from_input(&input);
        let work_input = input.clone();
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
            approval_policy: thread.approval_policy.clone(),
            approvals_reviewer: thread.approvals_reviewer.clone(),
            reasoning_effort: thread.reasoning_effort.clone(),
            service_tier: thread.service_tier.clone(),
            collaboration_mode: thread.collaboration_mode.clone(),
        };
        let turn_id = turn.id.clone();
        let user_item = turn.user_item_json();
        let agent_item_id = agent_item_id_for_turn(&turn_id);
        let cli_item_id = cli_item_id_for_turn(&turn_id);
        let response_turn = turn.to_json(false);
        let instruction_context = claude_thread_instruction_context(thread);
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
            input: work_input,
            instruction_context,
            resume_existing,
            permission_mode: claude_permission_mode_for_approvals_reviewer(
                &thread.approvals_reviewer,
            ),
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
                unregister_active_steer_sender(&key.0, &key.1);
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
        unregister_active_steer_sender(thread_id, &turn_id);
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

    fn steer_turn(&self, params: &Value) -> Result<Value, String> {
        let thread_id = required_param(params, "threadId")?;
        let turn_id = params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                self.thread_for_request(thread_id).ok().and_then(|thread| {
                    thread
                        .turns
                        .iter()
                        .rev()
                        .find(|turn| turn.status == TurnStatus::InProgress)
                        .map(|turn| turn.id.clone())
                })
            })
            .ok_or_else(|| steer_turn_inactive_error(thread_id))?;
        let input = steer_turn_input_from_params(params);
        let key = (thread_id.to_string(), turn_id.clone());
        let sender = active_steer_senders()
            .lock()
            .ok()
            .and_then(|senders| senders.get(&key).cloned())
            .or_else(|| {
                let lookup_thread_id = strip_local_thread_prefix(thread_id).to_string();
                active_steer_senders()
                    .lock()
                    .ok()
                    .and_then(|senders| senders.get(&(lookup_thread_id, turn_id.clone())).cloned())
            })
            .ok_or_else(|| steer_turn_inactive_error(thread_id))?;
        sender
            .send(input)
            .map_err(|_| steer_turn_inactive_error(thread_id))?;
        Ok(json!({
            "threadId": thread_id,
            "turnId": turn_id,
            "status": "sent",
        }))
    }

    fn thread_stream_state_notification_for_id(&self, thread_id: &str) -> Option<Value> {
        let lookup_thread_id = strip_local_thread_prefix(thread_id);
        self.threads
            .get(thread_id)
            .or_else(|| self.threads.get(lookup_thread_id))
            .map(claude_thread_stream_state_changed_notification)
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

    fn sync_subagent_threads_for_parent_turn(
        &mut self,
        parent_thread: &ClaudeThread,
        parent_turn: &ClaudeTurn,
    ) -> Vec<ClaudeThread> {
        let subagent_threads = parent_turn
            .tool_items
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("collabAgentToolCall"))
            .flat_map(|item| {
                collab_agent_item_receiver_thread_ids(item)
                    .into_iter()
                    .map(move |thread_id| (thread_id, item))
            })
            .map(|(thread_id, item)| {
                let generated =
                    virtual_subagent_thread_from_item(&thread_id, parent_thread, parent_turn, item);
                merge_completed_subagent_thread(self.threads.remove(&generated.id), generated, item)
            })
            .collect::<Vec<_>>();

        for thread in &subagent_threads {
            self.threads.insert(thread.id.clone(), thread.clone());
        }
        subagent_threads
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
        unregister_active_steer_sender(thread_id, turn_id);
        let (
            item,
            turn_json,
            generated_title,
            thread_stream_state,
            is_title_generation,
            parent_thread_snapshot,
            parent_turn_snapshot,
        ) = {
            let thread = self.threads.get_mut(thread_id)?;
            let Some(turn_index) = thread.turns.iter().position(|turn| turn.id == turn_id) else {
                return None;
            };
            let agent_item_streamed = result.agent_item_streamed;
            if let Some(latest_token_usage_info) = result.latest_token_usage_info {
                thread.latest_token_usage_info = Some(latest_token_usage_info);
            }
            let (item, turn_json, turn_snapshot, completed_at) = {
                let turn = &mut thread.turns[turn_index];
                let interrupted =
                    self.interrupted_turns.remove(&key) || turn.status == TurnStatus::Interrupted;
                turn.tool_items = result.tool_items;
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
                let item = (!agent_item_streamed && !turn.agent_text.is_empty())
                    .then(|| turn.agent_item_json());
                (
                    item,
                    turn.to_json(false),
                    turn.clone(),
                    turn.completed_at.unwrap_or_else(now_seconds),
                )
            };
            thread.updated_at = completed_at;
            let generated_title = generated_title_hint
                .filter(|generated_title| generated_title.title.is_some())
                .or_else(|| claude_generated_title_from_thread(thread));
            let is_title_generation = generated_title.is_some();
            let thread_stream_state = (!is_title_generation)
                .then(|| claude_thread_stream_state_changed_notification(thread));
            let parent_thread_snapshot = (!is_title_generation).then(|| thread.clone());
            let parent_turn_snapshot = (!is_title_generation).then_some(turn_snapshot);
            (
                item,
                turn_json,
                generated_title,
                thread_stream_state,
                is_title_generation,
                parent_thread_snapshot,
                parent_turn_snapshot,
            )
        };
        let subagent_thread_snapshots = if let (Some(parent_thread), Some(parent_turn)) = (
            parent_thread_snapshot.as_ref(),
            parent_turn_snapshot.as_ref(),
        ) {
            self.sync_subagent_threads_for_parent_turn(parent_thread, parent_turn)
        } else {
            Vec::new()
        };
        let mut extra_notifications = subagent_thread_snapshots
            .iter()
            .map(claude_thread_stream_state_changed_notification)
            .collect::<Vec<_>>();
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

fn is_claude_subagent_thread_id(thread_id: &str) -> bool {
    let thread_id = strip_local_thread_prefix(thread_id);
    thread_id.starts_with("claude-subagent-") || thread_id.starts_with("claude-subagent_")
}

fn collab_agent_item_references_thread(item: &Value, thread_id: &str) -> bool {
    let thread_id = strip_local_thread_prefix(thread_id);
    collab_agent_item_receiver_thread_ids(item)
        .iter()
        .map(|candidate| strip_local_thread_prefix(candidate))
        .any(|candidate| candidate == thread_id)
        || item
            .get("receiverThreads")
            .and_then(Value::as_array)
            .is_some_and(|threads| {
                threads.iter().any(|thread| {
                    thread
                        .get("threadId")
                        .and_then(Value::as_str)
                        .map(strip_local_thread_prefix)
                        == Some(thread_id)
                })
            })
        || item
            .get("agentsStates")
            .and_then(Value::as_object)
            .is_some_and(|states| {
                states
                    .keys()
                    .map(|key| strip_local_thread_prefix(key))
                    .any(|candidate| candidate == thread_id)
            })
}

fn collab_agent_item_receiver_thread_ids(item: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(values) = item.get("receiverThreadIds").and_then(Value::as_array) {
        for value in values {
            push_unique_thread_id(&mut ids, &mut seen, value.as_str());
        }
    }
    if let Some(receiver_threads) = item.get("receiverThreads").and_then(Value::as_array) {
        for receiver_thread in receiver_threads {
            push_unique_thread_id(
                &mut ids,
                &mut seen,
                receiver_thread.get("threadId").and_then(Value::as_str),
            );
        }
    }
    if let Some(states) = item.get("agentsStates").and_then(Value::as_object) {
        for key in states.keys() {
            push_unique_thread_id(&mut ids, &mut seen, Some(key));
        }
    }
    ids
}

fn collab_agent_item_parent_tool_id(item: &Value) -> Option<String> {
    item.get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix("claude-tool-"))
        .and_then(non_empty_string)
        .or_else(|| {
            collab_agent_item_receiver_thread_ids(item)
                .into_iter()
                .find_map(|thread_id| {
                    let thread_id = strip_local_thread_prefix(&thread_id);
                    thread_id
                        .strip_prefix("claude-subagent-")
                        .or_else(|| thread_id.strip_prefix("claude-subagent_"))
                        .and_then(non_empty_string)
                })
        })
}

fn virtual_subagent_thread_from_item(
    thread_id: &str,
    parent_thread: &ClaudeThread,
    parent_turn: &ClaudeTurn,
    item: &Value,
) -> ClaudeThread {
    let fallback =
        virtual_subagent_thread_from_item_summary(thread_id, parent_thread, parent_turn, item);
    load_virtual_subagent_thread_from_parent_transcript(
        thread_id,
        parent_thread,
        parent_turn,
        item,
        &fallback,
    )
    .unwrap_or(fallback)
}

fn merge_completed_subagent_thread(
    existing: Option<ClaudeThread>,
    generated: ClaudeThread,
    item: &Value,
) -> ClaudeThread {
    let Some(mut existing) = existing else {
        return generated;
    };
    if existing.turns.is_empty() || subagent_thread_has_richer_history(&generated, &existing) {
        return generated;
    }

    if !generated.preview.trim().is_empty() {
        existing.preview = generated.preview.clone();
    }
    existing.cwd = generated.cwd.clone();
    existing.git_info = generated.git_info.clone();
    existing.workspace_kind = generated.workspace_kind.clone();
    existing.workspace_roots = generated.workspace_roots.clone();
    existing.workspace_browser_root = generated.workspace_browser_root.clone();
    existing.projectless_output_directory = generated.projectless_output_directory.clone();
    existing.model = generated.model.clone();
    existing.reasoning_effort = generated.reasoning_effort.clone();
    existing.service_tier = generated.service_tier.clone();
    existing.collaboration_mode = generated.collaboration_mode.clone();
    existing.approval_policy = generated.approval_policy.clone();
    existing.approvals_reviewer = generated.approvals_reviewer.clone();
    existing.updated_at = existing.updated_at.max(generated.updated_at);
    apply_collab_agent_item_status_to_subagent_turns(item, &mut existing.turns);
    apply_collab_agent_item_result_to_subagent_turns(item, &mut existing.turns);
    existing
}

fn subagent_thread_has_richer_history(candidate: &ClaudeThread, existing: &ClaudeThread) -> bool {
    let candidate_score = subagent_thread_history_score(candidate);
    let existing_score = subagent_thread_history_score(existing);
    candidate_score > existing_score
}

fn subagent_thread_history_score(thread: &ClaudeThread) -> usize {
    thread.turns.len().saturating_mul(10)
        + thread
            .turns
            .iter()
            .map(|turn| {
                turn.tool_items.len().saturating_mul(3)
                    + usize::from(!turn.agent_text.trim().is_empty())
                    + usize::from(!turn.input.is_empty())
            })
            .sum::<usize>()
}

fn virtual_subagent_thread_from_item_summary(
    thread_id: &str,
    parent_thread: &ClaudeThread,
    parent_turn: &ClaudeTurn,
    item: &Value,
) -> ClaudeThread {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("inProgress");
    let turn_status = collab_agent_item_turn_status(status);
    let prompt = item
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let result = item
        .get("result")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let mut preview = prompt.trim().chars().take(160).collect::<String>();
    if preview.is_empty() {
        preview = result.trim().chars().take(160).collect();
    }
    let error = (turn_status == TurnStatus::Failed).then(|| {
        item.pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if result.trim().is_empty() {
                    "Claude Code subagent failed"
                } else {
                    result.trim()
                }
            })
            .to_string()
    });
    let completed_at = matches!(turn_status, TurnStatus::Completed | TurnStatus::Failed)
        .then(|| parent_turn.completed_at.unwrap_or_else(now_seconds));
    let input = if prompt.trim().is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "type": "text",
            "text": prompt,
            "text_elements": [],
        })]
    };
    let turn = ClaudeTurn {
        id: format!("turn-{}", sanitize_item_id(thread_id)),
        input,
        agent_text: result,
        tool_items: Vec::new(),
        status: turn_status,
        error,
        started_at: parent_turn.started_at,
        completed_at,
        duration_ms: completed_at.map(|completed_at| {
            seconds_to_millis(completed_at.saturating_sub(parent_turn.started_at))
        }),
        approval_policy: parent_turn.approval_policy.clone(),
        approvals_reviewer: parent_turn.approvals_reviewer.clone(),
        reasoning_effort: item
            .get("reasoningEffort")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| parent_turn.reasoning_effort.clone()),
        service_tier: parent_turn.service_tier.clone(),
        collaboration_mode: parent_turn.collaboration_mode.clone(),
    };
    let mut thread = fallback_virtual_subagent_thread(thread_id, parent_thread.name.as_deref());
    thread.preview = preview;
    thread.cwd = parent_thread.cwd.clone();
    thread.git_info = parent_thread.git_info.clone();
    thread.workspace_kind = parent_thread.workspace_kind.clone();
    thread.workspace_roots = parent_thread.workspace_roots.clone();
    thread.workspace_browser_root = parent_thread.workspace_browser_root.clone();
    thread.projectless_output_directory = parent_thread.projectless_output_directory.clone();
    thread.model = item
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| parent_thread.model.clone());
    thread.reasoning_effort = item
        .get("reasoningEffort")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| parent_thread.reasoning_effort.clone());
    thread.service_tier = parent_thread.service_tier.clone();
    thread.collaboration_mode = parent_thread.collaboration_mode.clone();
    thread.created_at = parent_turn.started_at;
    thread.updated_at = parent_turn.completed_at.unwrap_or(parent_thread.updated_at);
    thread.approval_policy = parent_turn.approval_policy.clone();
    thread.approvals_reviewer = parent_turn.approvals_reviewer.clone();
    thread.turns = vec![turn];
    thread
}

fn load_virtual_subagent_thread_from_parent_transcript(
    thread_id: &str,
    parent_thread: &ClaudeThread,
    parent_turn: &ClaudeTurn,
    item: &Value,
    fallback: &ClaudeThread,
) -> Option<ClaudeThread> {
    let transcript_path = parent_thread
        .path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            claude_transcript_path_for_session(&parent_thread.cwd, &parent_thread.claude_session_id)
        })?;
    let transcript = std::fs::read_to_string(transcript_path).ok()?;
    let entries = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    let tool_ids = collab_agent_tool_id_candidates(thread_id, item);
    if tool_ids.is_empty() {
        return None;
    }
    let sidechain_entries = subagent_sidechain_entries_for_tool(&entries, &tool_ids);
    if sidechain_entries.is_empty() {
        return None;
    }
    virtual_subagent_thread_from_sidechain_entries(
        thread_id,
        parent_thread,
        parent_turn,
        item,
        fallback,
        &sidechain_entries,
    )
}

fn collab_agent_tool_id_candidates(thread_id: &str, item: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if let Some(tool_id) = item
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix("claude-tool-"))
        .and_then(non_empty_string)
    {
        ids.insert(tool_id);
    }
    for value in collab_agent_item_receiver_thread_ids(item)
        .into_iter()
        .chain(std::iter::once(thread_id.to_string()))
    {
        let value = strip_local_thread_prefix(&value);
        if let Some(tool_id) = value
            .strip_prefix("claude-subagent-")
            .or_else(|| value.strip_prefix("claude-subagent_"))
            .and_then(non_empty_string)
        {
            ids.insert(tool_id);
        }
    }
    ids
}

fn subagent_sidechain_entries_for_tool<'a>(
    entries: &'a [Value],
    tool_ids: &BTreeSet<String>,
) -> Vec<&'a Value> {
    let mut root_parent_uuids = BTreeSet::new();
    for entry in entries {
        if transcript_assistant_tool_ids(entry)
            .into_iter()
            .any(|tool_id| tool_ids.contains(&tool_id))
        {
            if let Some(uuid) = transcript_entry_uuid(entry) {
                root_parent_uuids.insert(uuid);
            }
        }
    }

    let mut selected_indices = BTreeSet::new();
    let mut selected_uuids = root_parent_uuids;
    let mut changed = true;
    while changed {
        changed = false;
        for (index, entry) in entries.iter().enumerate() {
            if selected_indices.contains(&index) {
                continue;
            }
            let explicit_match = transcript_entry_parent_tool_id(entry)
                .is_some_and(|tool_id| tool_ids.contains(&tool_id));
            let sidechain_child = entry
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && transcript_entry_parent_uuid(entry)
                    .is_some_and(|parent_uuid| selected_uuids.contains(&parent_uuid));
            if !explicit_match && !sidechain_child {
                continue;
            }
            selected_indices.insert(index);
            if let Some(uuid) = transcript_entry_uuid(entry) {
                selected_uuids.insert(uuid);
            }
            changed = true;
        }
    }

    selected_indices
        .into_iter()
        .filter_map(|index| entries.get(index))
        .collect()
}

fn virtual_subagent_thread_from_sidechain_entries(
    thread_id: &str,
    parent_thread: &ClaudeThread,
    parent_turn: &ClaudeTurn,
    item: &Value,
    fallback: &ClaudeThread,
    entries: &[&Value],
) -> Option<ClaudeThread> {
    let mut cwd = parent_thread.cwd.clone();
    let mut model = parent_thread.model.clone();
    let mut preview = fallback.preview.clone();
    let mut created_at = fallback.created_at;
    let mut updated_at = fallback.updated_at;
    let mut pending_turn: Option<TranscriptTurnBuilder> = None;
    let mut turn_builders = Vec::new();
    let mut latest_token_usage_info = None;

    for value in entries {
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
        if let Some(message_model) = claude_model_from_message(value) {
            model = message_model.to_string();
        }
        if let Some(info) = claude_token_usage_info_from_message(value, &model) {
            latest_token_usage_info = Some(info);
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                let started_at = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_seconds)
                    .unwrap_or(updated_at);
                let tool_results = transcript_tool_results_from_user_entry(value, started_at);
                if !tool_results.is_empty() {
                    if let Some(turn) = pending_turn.as_mut() {
                        for result in tool_results {
                            turn.record_tool_result(result);
                        }
                    }
                    continue;
                }
                if let Some(input) = user_input_from_transcript_entry(value) {
                    if let Some(turn) = pending_turn.take() {
                        turn_builders.push(turn);
                    }
                    if preview.trim().is_empty() {
                        preview = prompt_from_input(&input).chars().take(160).collect();
                    }
                    pending_turn = Some(TranscriptTurnBuilder::new(input, started_at));
                }
            }
            Some("assistant") => {
                if let Some(turn) = pending_turn.as_mut() {
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
                    let error = failed.then(|| {
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Claude Code subagent failed")
                            .to_string()
                    });
                    turn.record_assistant_message(
                        assistant_text_from_transcript_entry(value),
                        transcript_reasoning_from_assistant_entry(value),
                        transcript_tool_uses_from_assistant_entry(value, completed_at),
                        completed_at,
                        value
                            .get("uuid")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        failed,
                        error,
                    );
                }
            }
            _ => {}
        }
    }
    if let Some(turn) = pending_turn.take() {
        turn_builders.push(turn);
    }

    let mut turns = turn_builders
        .into_iter()
        .enumerate()
        .filter_map(|(index, turn)| turn.into_turn(thread_id, &cwd, index))
        .collect::<Vec<_>>();
    if turns.is_empty() {
        return None;
    }

    for turn in &mut turns {
        turn.approval_policy = parent_turn.approval_policy.clone();
        turn.approvals_reviewer = parent_turn.approvals_reviewer.clone();
        turn.reasoning_effort = item
            .get("reasoningEffort")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| parent_turn.reasoning_effort.clone());
        turn.service_tier = parent_turn.service_tier.clone();
        turn.collaboration_mode = parent_turn.collaboration_mode.clone();
    }
    apply_collab_agent_item_status_to_subagent_turns(item, &mut turns);

    if preview.trim().is_empty() {
        preview = turns
            .first()
            .map(|turn| prompt_from_input(&turn.input).chars().take(160).collect())
            .unwrap_or_default();
    }

    let mut thread = fallback.clone();
    thread.preview = preview;
    thread.cwd = cwd.clone();
    thread.git_info = parent_thread.git_info.clone();
    thread.workspace_kind = parent_thread.workspace_kind.clone();
    thread.workspace_roots = parent_thread.workspace_roots.clone();
    thread.workspace_browser_root = parent_thread.workspace_browser_root.clone();
    thread.projectless_output_directory = parent_thread.projectless_output_directory.clone();
    thread.model = model;
    thread.created_at = created_at;
    thread.updated_at = updated_at.max(parent_thread.updated_at);
    thread.turns = turns;
    thread.latest_token_usage_info =
        latest_token_usage_info.or_else(|| fallback.latest_token_usage_info.clone());
    Some(thread)
}

fn apply_collab_agent_item_status_to_subagent_turns(item: &Value, turns: &mut [ClaudeTurn]) {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .map(collab_agent_item_turn_status)
        .unwrap_or(TurnStatus::InProgress);
    let Some(last_turn) = turns.last_mut() else {
        return;
    };
    match status {
        TurnStatus::InProgress => {
            last_turn.status = TurnStatus::InProgress;
            last_turn.completed_at = None;
            last_turn.duration_ms = None;
        }
        TurnStatus::Failed => {
            complete_subagent_turn_status(last_turn, TurnStatus::Failed);
            last_turn.status = TurnStatus::Failed;
            last_turn.error = Some(
                item.pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Claude Code subagent failed")
                    .to_string(),
            );
        }
        TurnStatus::Interrupted => {
            last_turn.status = TurnStatus::Interrupted;
            last_turn.completed_at = None;
            last_turn.duration_ms = None;
        }
        TurnStatus::Completed => {
            complete_subagent_turn_status(last_turn, TurnStatus::Completed);
            last_turn.error = None;
        }
    }
}

fn apply_collab_agent_item_result_to_subagent_turns(item: &Value, turns: &mut [ClaudeTurn]) {
    let Some(result) = collab_agent_item_result_text(item) else {
        return;
    };
    let Some(last_turn) = turns.last_mut() else {
        return;
    };
    merge_subagent_result_text(last_turn, &result);
}

fn collab_agent_item_result_text(item: &Value) -> Option<String> {
    item.get("result").and_then(|value| match value {
        Value::String(text) => non_empty_string(text),
        Value::Null => None,
        value => claude_text_from_content(value).or_else(|| non_empty_string(&compact_json(value))),
    })
}

fn merge_subagent_result_text(turn: &mut ClaudeTurn, result: &str) {
    let Some(result) = non_empty_string(result) else {
        return;
    };
    if turn.agent_text.trim().is_empty() {
        turn.agent_text = result;
        return;
    }
    let existing_key = compact_cli_text(&turn.agent_text);
    let result_key = compact_cli_text(&result);
    if existing_key == result_key || existing_key.contains(&result_key) {
        return;
    }
    if result_key.contains(&existing_key) {
        turn.agent_text = result;
        return;
    }
    turn.agent_text.push_str("\n\n");
    turn.agent_text.push_str(&result);
}

fn complete_subagent_turn_status(turn: &mut ClaudeTurn, status: TurnStatus) {
    turn.status = status;
    let completed_at = turn.completed_at.unwrap_or_else(now_seconds);
    turn.completed_at = Some(completed_at);
    if turn.duration_ms.is_none() {
        turn.duration_ms = Some(seconds_to_millis(
            completed_at.saturating_sub(turn.started_at),
        ));
    }
}

fn transcript_assistant_tool_ids(value: &Value) -> Vec<String> {
    let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    collect_transcript_assistant_tool_ids(content, &mut ids);
    ids
}

fn collect_transcript_assistant_tool_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_transcript_assistant_tool_ids(item, ids);
            }
        }
        Value::Object(map) => {
            if matches!(map.get("type").and_then(Value::as_str), Some("tool_use")) {
                if let Some(id) = map
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                {
                    ids.push(id);
                }
            } else if let Some(content) = map.get("content") {
                collect_transcript_assistant_tool_ids(content, ids);
            }
        }
        _ => {}
    }
}

fn transcript_entry_uuid(value: &Value) -> Option<String> {
    value
        .get("uuid")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| {
            value
                .pointer("/message/id")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
        })
}

fn transcript_entry_parent_uuid(value: &Value) -> Option<String> {
    value
        .get("parentUuid")
        .or_else(|| value.get("parent_uuid"))
        .and_then(Value::as_str)
        .and_then(non_empty_string)
}

fn transcript_entry_parent_tool_id(value: &Value) -> Option<String> {
    for pointer in [
        "/parent_tool_use_id",
        "/parentToolUseId",
        "/parentToolUseID",
        "/message/parent_tool_use_id",
        "/message/parentToolUseId",
        "/message/parentToolUseID",
    ] {
        if let Some(value) = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            return Some(value);
        }
    }
    None
}

fn fallback_virtual_subagent_thread(thread_id: &str, workspace_name: Option<&str>) -> ClaudeThread {
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string();
    let workspace_metadata = default_thread_workspace_metadata(&cwd);
    let now = now_seconds();
    ClaudeThread {
        id: thread_id.to_string(),
        session_id: thread_id.to_string(),
        claude_session_id: thread_id.to_string(),
        path: None,
        preview: String::new(),
        cwd,
        git_info: Value::Null,
        workspace_kind: workspace_metadata.kind,
        workspace_roots: workspace_metadata.roots,
        workspace_browser_root: workspace_metadata.browser_root,
        projectless_output_directory: workspace_metadata.projectless_output_directory,
        base_instructions: None,
        developer_instructions: None,
        personality: Value::Null,
        persist_extended_history: Value::Null,
        model: DEFAULT_MODEL.to_string(),
        reasoning_effort: Value::Null,
        service_tier: Value::Null,
        collaboration_mode: Value::Null,
        created_at: now,
        updated_at: now,
        archived: false,
        name: Some(
            workspace_name
                .map(str::to_string)
                .unwrap_or_else(|| "Claude Code subagent".to_string()),
        ),
        approval_policy: DEFAULT_APPROVAL_POLICY.to_string(),
        approvals_reviewer: DEFAULT_APPROVALS_REVIEWER.to_string(),
        turns: Vec::new(),
        goal: None,
        latest_token_usage_info: None,
    }
}

fn collab_agent_item_turn_status(status: &str) -> TurnStatus {
    match status {
        "completed" => TurnStatus::Completed,
        "failed" => TurnStatus::Failed,
        "interrupted" => TurnStatus::Interrupted,
        _ => TurnStatus::InProgress,
    }
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
    let memory_mode = persisted_claude_thread_memory_mode(&thread.id).unwrap_or(Value::Null);
    json!({
        "id": thread.id,
        "requests": [],
        "turns": thread
            .turns
            .iter()
            .map(|turn| claude_conversation_turn(thread, turn))
            .collect::<Vec<_>>(),
        "title": thread.display_title().unwrap_or_default(),
        "source": "cli",
        "modelProvider": PROVIDER_NAME,
        "latestModel": thread.model,
        "latestReasoningEffort": thread.reasoning_effort,
        "previousTurnModel": Value::Null,
        "latestCollaborationMode": thread.collaboration_mode,
        "baseInstructions": thread
            .base_instructions
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "developerInstructions": thread
            .developer_instructions
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "personality": thread.personality.clone(),
        "persistExtendedHistory": thread.persist_extended_history.clone(),
        "hasUnreadTurn": false,
        "pinned": persisted_claude_thread_pinned(&thread.id),
        "memoryMode": memory_mode,
        "threadGoal": thread.goal.clone().unwrap_or(Value::Null),
        "threadGoalResumeConfirmation": Value::Null,
        "completedThreadGoal": Value::Null,
        "threadRuntimeStatus": thread.status_json(),
        "rolloutPath": Value::Null,
        "cwd": thread.cwd,
        "gitInfo": thread.git_info.clone(),
        "resumeState": "resumed",
        "latestTokenUsageInfo": thread
            .latest_token_usage_info
            .clone()
            .unwrap_or(Value::Null),
        "workspaceKind": thread.workspace_kind,
        "workspaceRoots": thread.workspace_roots,
        "workspaceBrowserRoot": thread
            .workspace_browser_root
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "projectlessOutputDirectory": thread
            .projectless_output_directory
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
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
            "approvalPolicy": turn.approval_policy,
            "approvalsReviewer": turn.approvals_reviewer,
            "sandboxPolicy": claude_workspace_write_sandbox_policy(&thread.workspace_roots),
            "model": thread.model,
            "cwd": thread.cwd,
            "attachments": [],
            "effort": turn.reasoning_effort,
            "serviceTier": turn.service_tier,
            "summary": "none",
            "personality": thread.personality.clone(),
            "baseInstructions": thread
                .base_instructions
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "developerInstructions": thread
                .developer_instructions
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "outputSchema": Value::Null,
            "collaborationMode": turn.collaboration_mode,
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
    fn display_title(&self) -> Option<String> {
        self.raw_non_fallback_name()
            .or_else(|| self.display_title_from_first_turn())
            .or_else(|| display_safe_thread_title(&self.preview))
    }

    fn raw_non_fallback_name(&self) -> Option<String> {
        self.name.as_deref().and_then(display_safe_thread_title)
    }

    fn display_title_from_first_turn(&self) -> Option<String> {
        self.turns
            .first()
            .map(|turn| prompt_from_input(&turn.input))
            .and_then(|prompt| display_safe_thread_title(&prompt))
    }

    fn serialized_name(&self) -> Option<String> {
        let raw_name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())?;
        if is_claude_resume_fallback_thread_name(raw_name) {
            return self.display_title();
        }
        Some(raw_name.to_string())
    }

    fn to_json(&self, include_turns: bool) -> Value {
        let display_title = self.display_title();
        let serialized_name = self.serialized_name();
        let memory_mode = persisted_claude_thread_memory_mode(&self.id).unwrap_or(Value::Null);
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
            "gitInfo": self.git_info.clone(),
            "workspaceKind": self.workspace_kind,
            "workspaceRoots": self.workspace_roots,
            "workspaceBrowserRoot": self
                .workspace_browser_root
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "projectlessOutputDirectory": self
                .projectless_output_directory
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "reasoningEffort": self.reasoning_effort.clone(),
            "serviceTier": self.service_tier.clone(),
            "collaborationMode": self.collaboration_mode.clone(),
            "baseInstructions": self
                .base_instructions
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "developerInstructions": self
                .developer_instructions
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "personality": self.personality.clone(),
            "persistExtendedHistory": self.persist_extended_history.clone(),
            "approvalPolicy": self.approval_policy,
            "approvalsReviewer": self.approvals_reviewer,
            "pinned": persisted_claude_thread_pinned(&self.id),
            "memoryMode": memory_mode,
            "name": serialized_name.map(Value::String).unwrap_or(Value::Null),
            "title": display_title.map(Value::String).unwrap_or(Value::Null),
            "threadGoal": self.goal.clone().unwrap_or(Value::Null),
            "requests": [],
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
    if let Some(search) = thread_list_search_term(params) {
        let haystack = format!(
            "{}\n{}\n{}\n{}",
            thread.id,
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

fn thread_list_search_term(params: &Value) -> Option<String> {
    ["searchTerm", "query", "q", "text"].iter().find_map(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
    })
}

impl ClaudeResumeTranscriptMetadata {
    fn record_entry(&mut self, value: &Value) {
        if let Some(session_id) = transcript_metadata_string(value, &["sessionId", "session_id"]) {
            self.session_id = Some(session_id);
        }
        if let Some(cwd) = transcript_metadata_string(value, &["cwd"]) {
            self.cwd = Some(cwd);
        }
        if let Some(team_name) = transcript_metadata_string(value, &["teamName", "team_name"]) {
            self.team_name = Some(team_name);
        }
        if let Some(session_kind) =
            transcript_metadata_string(value, &["sessionKind", "session_kind"])
        {
            self.session_kind = Some(session_kind);
        }
        if let Some(entrypoint) = transcript_metadata_string(value, &["entrypoint"]) {
            self.entrypoint = Some(entrypoint);
        }
        if value
            .get("isLoopSession")
            .or_else(|| value.get("is_loop_session"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.is_loop_session = true;
        }

        match value.get("type").and_then(Value::as_str) {
            Some("last-prompt") => {
                if let Some(last_prompt) = transcript_metadata_string(value, &["lastPrompt"]) {
                    self.last_prompt = Some(last_prompt);
                }
            }
            Some("agent-name") => {
                if let Some(agent_name) = transcript_metadata_string(value, &["agentName", "name"])
                {
                    self.agent_name = Some(agent_name);
                }
            }
            Some("custom-title") => {
                if let Some(title) = transcript_metadata_string(value, &["customTitle", "title"]) {
                    self.custom_title = Some(title);
                }
            }
            Some("ai-title") => {
                if let Some(title) = transcript_metadata_string(value, &["aiTitle", "title"]) {
                    self.ai_title = Some(title);
                }
            }
            Some("summary") => {
                if let Some(summary) = transcript_metadata_string(value, &["summary", "content"]) {
                    self.summary = Some(summary);
                }
            }
            Some("user") | Some("assistant") => {
                self.record_message_entry(value);
            }
            _ => {}
        }

        if let Some(first_prompt) =
            transcript_metadata_string(value, &["firstPrompt", "first_prompt"])
        {
            self.first_prompt = Some(first_prompt);
        }
        if let Some(agent_name) = transcript_metadata_string(value, &["agentName", "agent_name"]) {
            self.agent_name = Some(agent_name);
        }
        if let Some(custom_title) = transcript_metadata_string(value, &["customTitle"]) {
            self.custom_title = Some(custom_title);
        }
        if let Some(ai_title) = transcript_metadata_string(value, &["aiTitle"]) {
            self.ai_title = Some(ai_title);
        }
        if let Some(last_prompt) = transcript_metadata_string(value, &["lastPrompt"]) {
            self.last_prompt = Some(last_prompt);
        }
        if let Some(summary) = transcript_metadata_string(value, &["summary", "summaryHint"]) {
            self.summary = Some(summary);
        }
    }

    fn record_message_entry(&mut self, value: &Value) {
        let is_sidechain = transcript_entry_is_sidechain(value);
        if !self.saw_message {
            self.head_is_sidechain = is_sidechain;
        }
        self.saw_message = true;
        if is_sidechain {
            self.saw_sidechain_message = true;
        } else {
            self.saw_non_sidechain_message = true;
        }

        if value.get("type").and_then(Value::as_str) == Some("user")
            && !is_sidechain
            && !value
                .get("isMeta")
                .or_else(|| value.get("is_meta"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            && transcript_tool_results_from_user_entry(value, 0).is_empty()
        {
            if let Some(input) = user_input_from_transcript_entry(value) {
                let prompt = prompt_from_input(&input);
                if !prompt.trim().is_empty() {
                    if self.first_prompt.is_none() {
                        self.first_prompt = Some(prompt.clone());
                    }
                    self.last_prompt = Some(prompt);
                }
            }
        }
    }

    fn hidden_from_resume(&self) -> bool {
        self.head_is_sidechain
            || (self.saw_sidechain_message && !self.saw_non_sidechain_message)
            || self.team_name.is_some()
            || self
                .session_kind
                .as_deref()
                .is_some_and(|kind| matches!(kind, "daemon" | "daemon-worker"))
            || self
                .entrypoint
                .as_deref()
                .is_some_and(claude_resume_entrypoint_is_hidden)
            || self.is_loop_session
    }

    fn display_title(&self) -> Option<String> {
        [
            self.agent_name.as_deref(),
            self.custom_title.as_deref(),
            self.ai_title.as_deref(),
            self.summary.as_deref(),
            self.first_prompt.as_deref(),
            self.last_prompt.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(sanitize_resume_thread_title)
    }

    fn preview(&self) -> Option<String> {
        self.last_prompt
            .as_deref()
            .or(self.first_prompt.as_deref())
            .map(|prompt| prompt.chars().take(160).collect())
    }
}

fn claude_resume_entrypoint_is_hidden(entrypoint: &str) -> bool {
    let entrypoint = entrypoint.trim();
    !entrypoint.is_empty() && entrypoint.starts_with("command-name/")
}

fn transcript_entry_is_sidechain(value: &Value) -> bool {
    value
        .get("isSidechain")
        .or_else(|| value.get("is_sidechain"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn transcript_metadata_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            return Some(text);
        }
    }
    None
}

fn sanitize_resume_thread_title(text: &str) -> Option<String> {
    let stripped = strip_claude_resume_context_tags(text);
    let source = if stripped.trim().is_empty() {
        text
    } else {
        stripped.as_str()
    };
    let title = source
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

fn strip_claude_resume_context_tags(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(open_offset) = text[index..].find('<') {
        let open = index + open_offset;
        output.push_str(&text[index..open]);
        let tag_start = open + 1;
        let Some(first) = text[tag_start..].chars().next() else {
            output.push('<');
            index = tag_start;
            continue;
        };
        if !first.is_ascii_lowercase() {
            output.push('<');
            index = tag_start;
            continue;
        }
        let mut tag_end = tag_start + first.len_utf8();
        for ch in text[tag_end..].chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                tag_end += ch.len_utf8();
            } else {
                break;
            }
        }
        let Some(close_bracket_offset) = text[tag_end..].find('>') else {
            output.push('<');
            index = tag_start;
            continue;
        };
        let close_bracket = tag_end + close_bracket_offset;
        let tag_name = &text[tag_start..tag_end];
        let close_tag = format!("</{tag_name}>");
        let content_start = close_bracket + 1;
        let Some(close_tag_offset) = text[content_start..].find(&close_tag) else {
            output.push('<');
            index = tag_start;
            continue;
        };
        index = content_start + close_tag_offset + close_tag.len();
        if text[index..].starts_with('\n') {
            index += 1;
        }
    }
    output.push_str(&text[index..]);
    output
}

fn display_safe_thread_title(value: &str) -> Option<String> {
    sanitize_resume_thread_title(value)
        .filter(|title| !is_claude_resume_fallback_thread_name(title))
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
        update_thread_cwd(&mut thread, normalize_cwd(Some(cwd)));
    }
    apply_thread_workspace_metadata_from_params(&mut thread, params);
    apply_thread_instruction_metadata_from_params(&mut thread, params);
    apply_thread_git_info_from_params(&mut thread, params);
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

fn load_claude_thread_list_snapshot(
    params: &Value,
    workspace_name: Option<String>,
) -> ClaudeThreadListSnapshot {
    let projects_dir = claude_projects_dir();
    let scan_limit = claude_thread_list_scan_limit(params);
    let now = now_millis();
    let cache = CLAUDE_THREAD_LIST_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(entry) = guard.as_ref() {
            if entry.projects_dir == projects_dir
                && entry.workspace_name == workspace_name
                && entry.scan_limit == scan_limit
                && now.saturating_sub(entry.loaded_at_ms) <= CLAUDE_THREAD_LIST_CACHE_TTL_MS
            {
                return entry.snapshot.clone();
            }
        }
    }

    let paths = claude_transcript_files_for_thread_list(scan_limit);
    let snapshot = load_claude_thread_list_snapshot_from_paths(&paths, workspace_name.clone());
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(ClaudeThreadListCacheEntry {
            projects_dir,
            workspace_name,
            scan_limit,
            loaded_at_ms: now,
            snapshot: snapshot.clone(),
        });
    }
    snapshot
}

fn clear_claude_thread_list_cache() {
    if let Some(cache) = CLAUDE_THREAD_LIST_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

fn claude_thread_list_scan_limit(params: &Value) -> Option<usize> {
    if thread_list_search_term(params).is_some() {
        return None;
    }
    let limit = params.get("limit").and_then(Value::as_u64)? as usize;
    Some(
        limit
            .saturating_mul(CLAUDE_THREAD_LIST_LIMIT_MULTIPLIER)
            .clamp(
                CLAUDE_THREAD_LIST_MIN_SCAN_LIMIT,
                CLAUDE_THREAD_LIST_MAX_SCAN_LIMIT,
            ),
    )
}

fn load_claude_thread_list_snapshot_from_paths(
    paths: &[PathBuf],
    workspace_name: Option<String>,
) -> ClaudeThreadListSnapshot {
    let mut threads = BTreeMap::new();
    let mut generated_titles = Vec::new();
    let mut inline_titles = BTreeMap::new();
    let thread_names = load_claude_thread_names();
    let thread_goals = load_claude_thread_goals();
    let archived_threads = load_claude_thread_archived();
    for path in paths {
        match load_claude_thread_list_entry_from_transcript_path(
            path,
            workspace_name.as_deref(),
            &thread_names,
            &thread_goals,
            &archived_threads,
        ) {
            Some(ClaudeThreadListTranscriptEntry::Thread {
                thread,
                inline_title,
            }) => {
                if let Some(inline_title) = inline_title {
                    inline_titles.insert(thread.id.clone(), inline_title);
                }
                threads
                    .entry(thread.id.clone())
                    .and_modify(|existing: &mut ClaudeThread| {
                        if thread.updated_at > existing.updated_at {
                            *existing = thread.clone();
                        }
                    })
                    .or_insert(thread);
            }
            Some(ClaudeThreadListTranscriptEntry::GeneratedTitle(generated_title)) => {
                generated_titles.push(generated_title);
            }
            None => {}
        }
    }
    apply_generated_titles_to_claude_threads(
        &mut threads,
        &generated_titles,
        workspace_name.as_deref(),
    );
    ClaudeThreadListSnapshot {
        threads,
        generated_titles,
        inline_titles,
    }
}

fn load_claude_thread_list_entry_from_transcript_path(
    path: &Path,
    workspace_name: Option<&str>,
    thread_names: &BTreeMap<String, String>,
    thread_goals: &BTreeMap<String, Value>,
    archived_threads: &BTreeSet<String>,
) -> Option<ClaudeThreadListTranscriptEntry> {
    let file = File::open(path).ok()?;
    let path_session_id = path.file_stem()?.to_string_lossy().to_string();
    let (fallback_created_at, fallback_updated_at) = transcript_fallback_times(path);
    let mut session_id = path_session_id.clone();
    let mut cwd = String::new();
    let mut model = DEFAULT_MODEL.to_string();
    let mut preview = String::new();
    let mut created_at = fallback_created_at;
    let mut updated_at = fallback_updated_at;
    let mut inline_title = None;
    let mut source_prompt = None;
    let mut assistant_title = None;
    let mut ai_title = None;
    let mut resume_metadata = ClaudeResumeTranscriptMetadata::default();

    for (index, line) in BufReader::new(file).lines().enumerate() {
        if index >= CLAUDE_THREAD_LIST_MAX_LINES_PER_TRANSCRIPT {
            break;
        }
        let Ok(line) = line else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        resume_metadata.record_entry(&value);
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
        if let Some(entry_model) = claude_model_from_message(&value) {
            if entry_model != "<synthetic>" {
                model = entry_model.to_string();
            }
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user") if !transcript_entry_is_sidechain(&value) => {
                if let Some(input) = user_input_from_transcript_entry(&value) {
                    let prompt = prompt_from_input(&input);
                    if preview.is_empty() {
                        preview = prompt.chars().take(160).collect();
                    }
                    if source_prompt.is_none() {
                        source_prompt = extract_claude_title_generation_source_prompt(&prompt);
                    }
                }
            }
            Some("assistant") if !transcript_entry_is_sidechain(&value) => {
                if source_prompt.is_some() {
                    assistant_title = assistant_text_from_transcript_entry(&value)
                        .and_then(|text| sanitize_generated_thread_title(&text));
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
                    if source_prompt.is_some() {
                        ai_title = inline_title.clone();
                    }
                }
            }
            _ => {}
        }
        if source_prompt.is_some() && ai_title.is_some() {
            break;
        }
    }

    record_claude_thread_list_tail_metadata(
        path,
        &mut resume_metadata,
        &mut inline_title,
        &mut ai_title,
        &mut preview,
    );
    if let Some(metadata_session_id) = resume_metadata.session_id.clone() {
        session_id = metadata_session_id;
    }
    if let Some(metadata_cwd) = resume_metadata.cwd.clone() {
        cwd = metadata_cwd;
    }
    if ai_title.is_none() {
        ai_title = resume_metadata
            .ai_title
            .as_deref()
            .and_then(sanitize_generated_thread_title);
    }
    let resume_display_title = resume_metadata.display_title();
    if resume_display_title.is_some() {
        inline_title = resume_display_title.clone();
    }

    if cwd.is_empty() {
        cwd = cwd_from_claude_project_dir(path).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });
    }
    if let Some(source_prompt) = source_prompt {
        return Some(ClaudeThreadListTranscriptEntry::GeneratedTitle(
            ClaudeGeneratedTitle {
                source_prompt,
                title: ai_title.or(assistant_title),
                cwd,
                created_at,
                updated_at,
            },
        ));
    }

    if resume_metadata.hidden_from_resume() {
        return None;
    }
    let resume_display_title = resume_metadata.display_title();
    if preview.is_empty() {
        preview = resume_metadata
            .preview()
            .or_else(|| resume_display_title.clone())
            .unwrap_or_else(|| session_id.clone());
    }
    let name = thread_names
        .get(&session_id)
        .cloned()
        .filter(|name| !is_claude_resume_fallback_thread_name(name))
        .or_else(|| inline_title.clone())
        .or_else(|| resume_display_title.clone())
        .or_else(|| workspace_name.map(str::to_string));
    let goal = thread_goals.get(&session_id).cloned();
    let archived = archived_threads.contains(&session_id);
    let workspace_metadata = default_thread_workspace_metadata(&cwd);
    let thread = ClaudeThread {
        id: session_id.clone(),
        session_id: session_id.clone(),
        claude_session_id: session_id,
        path: Some(path.to_string_lossy().to_string()),
        preview,
        cwd,
        git_info: Value::Null,
        workspace_kind: workspace_metadata.kind,
        workspace_roots: workspace_metadata.roots,
        workspace_browser_root: workspace_metadata.browser_root,
        projectless_output_directory: workspace_metadata.projectless_output_directory,
        base_instructions: None,
        developer_instructions: None,
        personality: Value::Null,
        persist_extended_history: Value::Null,
        model,
        reasoning_effort: Value::Null,
        service_tier: Value::Null,
        collaboration_mode: Value::Null,
        created_at,
        updated_at,
        archived,
        name,
        approval_policy: DEFAULT_APPROVAL_POLICY.to_string(),
        approvals_reviewer: DEFAULT_APPROVALS_REVIEWER.to_string(),
        turns: Vec::new(),
        goal,
        latest_token_usage_info: None,
    };
    Some(ClaudeThreadListTranscriptEntry::Thread {
        thread,
        inline_title,
    })
}

fn record_claude_thread_list_tail_metadata(
    path: &Path,
    resume_metadata: &mut ClaudeResumeTranscriptMetadata,
    inline_title: &mut Option<String>,
    ai_title: &mut Option<String>,
    preview: &mut String,
) {
    let Some(tail) = read_claude_transcript_tail(path, CLAUDE_THREAD_LIST_TAIL_BYTES) else {
        return;
    };
    for line in tail.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        resume_metadata.record_entry(&value);
        match value.get("type").and_then(Value::as_str) {
            Some("last-prompt") => {
                if let Some(last_prompt) = value.get("lastPrompt").and_then(Value::as_str) {
                    if !last_prompt.trim().is_empty() {
                        *preview = last_prompt.chars().take(160).collect();
                    }
                }
            }
            Some("custom-title") => {
                if let Some(title) = value
                    .get("customTitle")
                    .or_else(|| value.get("title"))
                    .and_then(Value::as_str)
                    .and_then(sanitize_resume_thread_title)
                {
                    *inline_title = Some(title);
                }
            }
            Some("ai-title") => {
                if let Some(title) = value
                    .get("aiTitle")
                    .or_else(|| value.get("title"))
                    .and_then(Value::as_str)
                    .and_then(sanitize_generated_thread_title)
                {
                    *inline_title = Some(title.clone());
                    *ai_title = Some(title);
                }
            }
            Some("agent-name") => {
                if let Some(title) = value
                    .get("agentName")
                    .or_else(|| value.get("name"))
                    .and_then(Value::as_str)
                    .and_then(sanitize_resume_thread_title)
                {
                    *inline_title = Some(title);
                }
            }
            _ => {}
        }
    }
}

fn read_claude_transcript_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut content = String::new();
    file.read_to_string(&mut content).ok()?;
    if start == 0 {
        return Some(content);
    }
    let newline = content.find('\n')?;
    Some(content[newline + 1..].to_string())
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
    let mut pending_turn: Option<TranscriptTurnBuilder> = None;
    let mut turn_builders = Vec::new();
    let mut latest_token_usage_info = None;
    let mut inline_title = None;
    let mut resume_metadata = ClaudeResumeTranscriptMetadata::default();

    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        resume_metadata.record_entry(&value);
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
            Some("user") if !transcript_entry_is_sidechain(&value) => {
                let started_at = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_seconds)
                    .unwrap_or(updated_at);
                let tool_results = transcript_tool_results_from_user_entry(&value, started_at);
                if !tool_results.is_empty() {
                    if let Some(turn) = pending_turn.as_mut() {
                        for result in tool_results {
                            turn.record_tool_result(result);
                        }
                    }
                    continue;
                }
                if let Some(input) = user_input_from_transcript_entry(&value) {
                    if let Some(turn) = pending_turn.take() {
                        turn_builders.push(turn);
                    }
                    if preview.is_empty() {
                        preview = prompt_from_input(&input).chars().take(160).collect();
                    }
                    pending_turn = Some(TranscriptTurnBuilder::new(input, started_at));
                }
            }
            Some("assistant") if !transcript_entry_is_sidechain(&value) => {
                if let Some(assistant_model) = value
                    .get("message")
                    .and_then(|message| message.get("model"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty() && *value != "<synthetic>")
                {
                    model = assistant_model.to_string();
                }
                if let Some(turn) = pending_turn.as_mut() {
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
                        .map(str::to_string);
                    let error = failed.then(|| {
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Claude Code turn failed")
                            .to_string()
                    });
                    turn.record_assistant_message(
                        assistant_text_from_transcript_entry(&value),
                        transcript_reasoning_from_assistant_entry(&value),
                        transcript_tool_uses_from_assistant_entry(&value, completed_at),
                        completed_at,
                        turn_suffix,
                        failed,
                        error,
                    );
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
    if let Some(turn) = pending_turn.take() {
        turn_builders.push(turn);
    }

    if cwd.is_empty() {
        cwd = cwd_from_claude_project_dir(path).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .to_string_lossy()
                .to_string()
        });
    }
    if resume_metadata.hidden_from_resume() {
        return None;
    }
    let resume_display_title = resume_metadata.display_title();
    if preview.is_empty() {
        preview = resume_metadata
            .preview()
            .or_else(|| resume_display_title.clone())
            .unwrap_or_else(|| session_id.clone());
    }
    let turns = turn_builders
        .into_iter()
        .enumerate()
        .filter_map(|(index, turn)| turn.into_turn(&session_id, &cwd, index))
        .collect::<Vec<_>>();

    let name = persisted_claude_thread_name(&session_id)
        .or_else(|| resume_display_title.clone())
        .or(inline_title)
        .or(workspace_name);
    let goal = persisted_claude_thread_goal(&session_id);
    let archived = persisted_claude_thread_archived(&session_id);
    let workspace_metadata = default_thread_workspace_metadata(&cwd);

    let git_info = git_info_for_cwd(&cwd);
    Some(ClaudeThread {
        id: session_id.clone(),
        session_id: session_id.clone(),
        claude_session_id: session_id,
        path: Some(path.to_string_lossy().to_string()),
        preview,
        cwd,
        git_info,
        workspace_kind: workspace_metadata.kind,
        workspace_roots: workspace_metadata.roots,
        workspace_browser_root: workspace_metadata.browser_root,
        projectless_output_directory: workspace_metadata.projectless_output_directory,
        base_instructions: None,
        developer_instructions: None,
        personality: Value::Null,
        persist_extended_history: Value::Null,
        model,
        reasoning_effort: Value::Null,
        service_tier: Value::Null,
        collaboration_mode: Value::Null,
        created_at,
        updated_at,
        archived,
        name,
        approval_policy: DEFAULT_APPROVAL_POLICY.to_string(),
        approvals_reviewer: DEFAULT_APPROVALS_REVIEWER.to_string(),
        turns,
        goal,
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
    let paths = claude_transcript_files();
    load_claude_inline_thread_titles_from_paths(&paths)
}

fn load_claude_inline_thread_titles_from_paths(paths: &[PathBuf]) -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    for path in paths {
        let Ok(transcript) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (fallback_created_at, fallback_updated_at) = transcript_fallback_times(path);
        if claude_generated_title_from_transcript(
            &transcript,
            path,
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
    let mut metadata = ClaudeResumeTranscriptMetadata::default();
    for value in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        metadata.record_entry(&value);
    }
    metadata.display_title()
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
    let prompt_title = sanitize_resume_thread_title(&generated_title.source_prompt);
    if !should_replace_thread_name(thread.name.as_deref(), workspace_name)
        && thread.name.as_deref() != prompt_title.as_deref()
    {
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
    let prompt_title = sanitize_resume_thread_title(&thread_initial_prompt(thread));
    if !should_replace_thread_name(thread.name.as_deref(), workspace_name)
        && thread.name.as_deref() != prompt_title.as_deref()
    {
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
                || is_claude_resume_fallback_thread_name(current)
        }
    }
}

fn is_claude_resume_fallback_thread_name(value: &str) -> bool {
    let value = strip_local_thread_prefix(value.trim());
    is_uuid_like(value) || (value.len() == 8 && value.as_bytes().iter().all(u8::is_ascii_hexdigit))
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
    load_claude_thread_names()
        .get(thread_id)
        .cloned()
        .filter(|name| !is_claude_resume_fallback_thread_name(name))
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
    clear_claude_thread_list_cache();
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

fn persisted_claude_thread_goal(thread_id: &str) -> Option<Value> {
    let thread_id = strip_local_thread_prefix(thread_id);
    load_claude_thread_goals().get(thread_id).cloned()
}

fn persist_claude_thread_goal(thread_id: &str, goal: Option<&Value>) {
    let thread_id = strip_local_thread_prefix(thread_id).trim();
    if thread_id.is_empty() {
        return;
    }
    let mut goals = load_claude_thread_goals();
    if let Some(goal) = goal.filter(|goal| !goal.is_null()) {
        goals.insert(thread_id.to_string(), goal.clone());
    } else {
        goals.remove(thread_id);
    }
    let Some(path) = claude_thread_goals_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(&goals) {
        let _ = std::fs::write(path, content);
    }
    clear_claude_thread_list_cache();
}

fn load_claude_thread_goals() -> BTreeMap<String, Value> {
    let Some(path) = claude_thread_goals_path() else {
        return BTreeMap::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&content) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(key, value)| !key.trim().is_empty() && !value.is_null())
        .collect()
}

fn claude_thread_goals_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_THREAD_GOALS_FILE),
    )
}

fn persisted_claude_thread_archived(thread_id: &str) -> bool {
    let thread_id = strip_local_thread_prefix(thread_id);
    load_claude_thread_archived().contains(thread_id)
}

fn persist_claude_thread_archived(thread_id: &str, archived: bool) {
    let thread_id = strip_local_thread_prefix(thread_id).trim();
    if thread_id.is_empty() {
        return;
    }
    let mut archived_ids = load_claude_thread_archived();
    if archived {
        archived_ids.insert(thread_id.to_string());
    } else {
        archived_ids.remove(thread_id);
    }
    let Some(path) = claude_thread_archived_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let values = archived_ids.into_iter().collect::<Vec<_>>();
    if let Ok(content) = serde_json::to_string_pretty(&values) {
        let _ = std::fs::write(path, content);
    }
    clear_claude_thread_list_cache();
}

fn load_claude_thread_archived() -> BTreeSet<String> {
    let Some(path) = claude_thread_archived_path() else {
        return BTreeSet::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return BTreeSet::new();
    };
    match value {
        Value::Array(values) => values
            .into_iter()
            .filter_map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .collect(),
        Value::Object(map) => map
            .into_iter()
            .filter_map(|(key, value)| {
                (!key.trim().is_empty() && value.as_bool().unwrap_or(false)).then_some(key)
            })
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn claude_thread_archived_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_THREAD_ARCHIVED_FILE),
    )
}

fn persisted_claude_thread_pinned(thread_id: &str) -> bool {
    let thread_id = strip_local_thread_prefix(thread_id);
    load_claude_thread_pinned().contains(thread_id)
}

fn persist_claude_thread_pinned(thread_id: &str, pinned: bool) {
    let thread_id = strip_local_thread_prefix(thread_id).trim();
    if thread_id.is_empty() {
        return;
    }
    let mut pinned_ids = load_claude_thread_pinned();
    if pinned {
        pinned_ids.insert(thread_id.to_string());
    } else {
        pinned_ids.remove(thread_id);
    }
    let Some(path) = claude_thread_pinned_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let values = pinned_ids.into_iter().collect::<Vec<_>>();
    if let Ok(content) = serde_json::to_string_pretty(&values) {
        let _ = std::fs::write(path, content);
    }
    clear_claude_thread_list_cache();
}

fn load_claude_thread_pinned() -> BTreeSet<String> {
    let Some(path) = claude_thread_pinned_path() else {
        return BTreeSet::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    load_string_set_json(&content)
}

fn claude_thread_pinned_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_THREAD_PINNED_FILE),
    )
}

fn persisted_claude_thread_memory_mode(thread_id: &str) -> Option<Value> {
    let thread_id = strip_local_thread_prefix(thread_id);
    load_claude_thread_memory_modes().get(thread_id).cloned()
}

fn persist_claude_thread_memory_mode(thread_id: &str, memory_mode: Option<&Value>) {
    let thread_id = strip_local_thread_prefix(thread_id).trim();
    if thread_id.is_empty() {
        return;
    }
    let mut modes = load_claude_thread_memory_modes();
    if let Some(memory_mode) = memory_mode.filter(|value| !value.is_null()) {
        modes.insert(thread_id.to_string(), memory_mode.clone());
    } else {
        modes.remove(thread_id);
    }
    let Some(path) = claude_thread_memory_modes_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(&modes) {
        let _ = std::fs::write(path, content);
    }
    clear_claude_thread_list_cache();
}

fn load_claude_thread_memory_modes() -> BTreeMap<String, Value> {
    let Some(path) = claude_thread_memory_modes_path() else {
        return BTreeMap::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&content) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(key, value)| !key.trim().is_empty() && !value.is_null())
        .collect()
}

fn claude_thread_memory_modes_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_THREAD_MEMORY_MODES_FILE),
    )
}

fn load_string_set_json(content: &str) -> BTreeSet<String> {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return BTreeSet::new();
    };
    match value {
        Value::Array(values) => values
            .into_iter()
            .filter_map(|value| value.as_str().and_then(non_empty_string))
            .collect(),
        Value::Object(map) => map
            .into_iter()
            .filter_map(|(key, value)| {
                (!key.trim().is_empty() && value.as_bool().unwrap_or(false)).then_some(key)
            })
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn load_standalone_lifecycle_state(path_fn: fn() -> Option<PathBuf>) -> BTreeMap<String, Value> {
    let Some(path) = path_fn() else {
        return BTreeMap::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&content) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(key, value)| !key.trim().is_empty() && value.is_object())
        .collect()
}

fn persist_standalone_lifecycle_state(
    path_fn: fn() -> Option<PathBuf>,
    state: &BTreeMap<String, Value>,
) {
    let Some(path) = path_fn() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, content);
    }
}

fn claude_plugin_state_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_PLUGIN_STATE_FILE),
    )
}

fn claude_mcp_server_state_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(".claude")
            .join(CLAUDE_MCP_SERVER_STATE_FILE),
    )
}

fn thread_memory_mode_from_params(params: &Value) -> Value {
    for key in ["memoryMode", "memory_mode", "mode", "value"] {
        if let Some(value) = params.get(key).filter(|value| !value.is_null()) {
            return value.clone();
        }
    }
    json!("auto")
}

fn apply_new_thread_persisted_overlays(thread_id: &str, params: &Value) {
    if let Some(pinned) = params.get("pinned").and_then(Value::as_bool) {
        persist_claude_thread_pinned(thread_id, pinned);
    }
    for key in ["memoryMode", "memory_mode"] {
        if let Some(memory_mode) = params.get(key).filter(|value| !value.is_null()) {
            persist_claude_thread_memory_mode(thread_id, Some(memory_mode));
            break;
        }
    }
}

fn thread_goal_from_params(params: &Value) -> Value {
    if let Some(goal) = params.get("goal").filter(|goal| !goal.is_null()) {
        return goal.clone();
    }
    let mut goal = Map::new();
    for key in [
        "objective",
        "status",
        "tokenBudget",
        "token_budget",
        "summary",
        "createdAt",
        "updatedAt",
    ] {
        if let Some(value) = params.get(key).filter(|value| !value.is_null()) {
            goal.insert(key.to_string(), value.clone());
        }
    }
    if goal.is_empty() {
        Value::Null
    } else {
        Value::Object(goal)
    }
}

fn user_input_from_transcript_entry(value: &Value) -> Option<Vec<Value>> {
    let content = value.get("message")?.get("content")?;
    let mut items = Vec::new();
    collect_user_input_items(content, &mut items);
    (!items.is_empty()).then_some(items)
}

fn transcript_reasoning_from_assistant_entry(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let content = message.get("content")?;
    let mut parts = Vec::new();
    collect_transcript_reasoning(content, &mut parts);
    let text = parts.join("");
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

fn collect_transcript_reasoning(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_transcript_reasoning(item, parts);
            }
        }
        Value::Object(map) => {
            if matches!(
                map.get("type").and_then(Value::as_str),
                Some("thinking") | Some("thinking_delta")
            ) {
                if let Some(text) = map
                    .get("thinking")
                    .or_else(|| map.get("text"))
                    .and_then(Value::as_str)
                {
                    parts.push(text.to_string());
                }
            } else if let Some(content) = map.get("content") {
                collect_transcript_reasoning(content, parts);
            }
        }
        _ => {}
    }
}

fn transcript_tool_uses_from_assistant_entry(
    value: &Value,
    started_at: i64,
) -> Vec<TranscriptToolUse> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(message) = value.get("message") else {
        return Vec::new();
    };
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(content) = message.get("content") else {
        return Vec::new();
    };
    let mut tool_uses = Vec::new();
    collect_transcript_tool_uses(content, started_at, &mut tool_uses);
    tool_uses
}

fn collect_transcript_tool_uses(
    value: &Value,
    started_at: i64,
    tool_uses: &mut Vec<TranscriptToolUse>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_transcript_tool_uses(item, started_at, tool_uses);
            }
        }
        Value::Object(map) => {
            if matches!(map.get("type").and_then(Value::as_str), Some("tool_use")) {
                let id = map
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                    .unwrap_or_else(|| format!("tool-{}", tool_uses.len()));
                let name = map
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                    .unwrap_or_else(|| "tool".to_string());
                let input = map.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_uses.push(TranscriptToolUse {
                    id,
                    name,
                    input,
                    started_at,
                });
            } else if let Some(content) = map.get("content") {
                collect_transcript_tool_uses(content, started_at, tool_uses);
            }
        }
        _ => {}
    }
}

fn transcript_tool_results_from_user_entry(
    value: &Value,
    completed_at: i64,
) -> Vec<TranscriptToolResult> {
    if value.get("type").and_then(Value::as_str) != Some("user") {
        return Vec::new();
    }
    let Some(message) = value.get("message") else {
        return Vec::new();
    };
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return Vec::new();
    }
    let Some(content) = message.get("content") else {
        return Vec::new();
    };
    let mut results = Vec::new();
    collect_transcript_tool_results(content, completed_at, &mut results);
    results
}

fn collect_transcript_tool_results(
    value: &Value,
    completed_at: i64,
    results: &mut Vec<TranscriptToolResult>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_transcript_tool_results(item, completed_at, results);
            }
        }
        Value::Object(map) => {
            if matches!(map.get("type").and_then(Value::as_str), Some("tool_result")) {
                let Some(tool_id) = map
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                else {
                    return;
                };
                let result = map
                    .get("content")
                    .and_then(claude_text_from_content)
                    .unwrap_or_else(|| compact_json(value));
                results.push(TranscriptToolResult {
                    tool_id,
                    result,
                    failed: map
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    completed_at,
                });
            } else if let Some(content) = map.get("content") {
                collect_transcript_tool_results(content, completed_at, results);
            }
        }
        _ => {}
    }
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

fn claude_transcript_files_for_thread_list(scan_limit: Option<usize>) -> Vec<PathBuf> {
    let mut files = claude_transcript_files()
        .into_iter()
        .map(|path| {
            let modified_at = std::fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_to_unix_seconds)
                .unwrap_or_default();
            (path, modified_at)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    if let Some(limit) = scan_limit {
        files.truncate(limit);
    }
    files.into_iter().map(|(path, _)| path).collect()
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
    let (steer_tx, steer_rx) = mpsc::channel();
    register_active_steer_sender(work, steer_tx);

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
        if child_status.is_none() {
            match drain_steer_messages(work, &mut child_stdin, &steer_rx) {
                Ok(sent_count) if sent_count > 0 => {
                    last_child_event = Instant::now();
                }
                Ok(_) => {}
                Err(err) => {
                    command_output.push_str(&format!("[steer]\n{}\n", err.trim()));
                    terminate_process_group(child.id());
                    child_status = Some(child.wait());
                }
            }
        }

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
            emit_reasoning_completed_if_started(&output, work, &mut stream);

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
    emit_reasoning_completed_if_started(&output, work, &mut stream);

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
    sync_claude_stream_state_to_thread(state, output, work, stream);
    Ok(())
}

fn sync_claude_stream_state_to_thread<W>(
    state: &SharedState,
    output: &SharedOutput<W>,
    work: &TurnWork,
    stream: &ClaudeStreamState,
) where
    W: Write,
{
    if is_claude_title_generation_prompt(&work.prompt) {
        return;
    }
    let notifications = lock_state(state).ok().map(|mut state| {
        let mut notifications = Vec::new();
        let (thread_notification, subagent_threads) = {
            let thread = state.threads.get_mut(&work.thread_id)?;
            let turn = thread
                .turns
                .iter_mut()
                .find(|turn| turn.id == work.turn_id)?;
            apply_live_claude_stream_state_to_turn(work, stream, turn);
            if let Some(info) = stream.latest_token_usage_info.clone() {
                thread.latest_token_usage_info = Some(info);
            }
            thread.updated_at = now_seconds();
            let thread_snapshot = thread.clone();
            let turn_snapshot = thread_snapshot
                .turns
                .iter()
                .find(|turn| turn.id == work.turn_id)?;
            (
                claude_thread_stream_state_changed_notification(&thread_snapshot),
                live_subagent_threads_for_stream(work, stream, &thread_snapshot, turn_snapshot),
            )
        };
        notifications.push(thread_notification);
        for subagent_thread in subagent_threads {
            state
                .threads
                .insert(subagent_thread.id.clone(), subagent_thread);
        }
        Some(notifications)
    });
    if let Some(Some(notifications)) = notifications {
        for notification in notifications {
            let _ = write_notification(output, notification);
        }
    }
}

fn apply_live_claude_stream_state_to_turn(
    work: &TurnWork,
    stream: &ClaudeStreamState,
    turn: &mut ClaudeTurn,
) {
    turn.agent_text = live_agent_text_for_stream(stream);
    turn.tool_items = live_tool_items_for_stream(work, stream);
}

fn live_agent_text_for_stream(stream: &ClaudeStreamState) -> String {
    if !stream.emitted_text.is_empty() {
        return stream.emitted_text.clone();
    }
    if !stream.saw_tool_call && !stream.pending_agent_text.trim().is_empty() {
        return stream.pending_agent_text.clone();
    }
    String::new()
}

fn live_tool_items_for_stream(work: &TurnWork, stream: &ClaudeStreamState) -> Vec<Value> {
    let mut items = Vec::new();
    if !stream.reasoning_item_completed {
        let mut reasoning_text = stream.reasoning_text.clone();
        if stream.saw_tool_call && !stream.pending_agent_text.trim().is_empty() {
            reasoning_text.push_str(&stream.pending_agent_text);
        }
        if !reasoning_text.trim().is_empty() {
            items.push(reasoning_item_json(&work.turn_id, &reasoning_text));
        }
    }
    items.extend(stream.completed_tool_items.iter().cloned());
    for (tool_id, state) in &stream.tool_calls {
        if stream.completed_tool_ids.contains(tool_id) {
            continue;
        }
        if !state.started_emitted && is_empty_tool_arguments(&state.arguments) {
            continue;
        }
        items.push(tool_call_item(
            &work.thread_id,
            &work.cwd,
            tool_id,
            state,
            "inProgress",
            None,
            Value::Null,
        ));
    }
    items
}

fn live_subagent_threads_for_stream(
    work: &TurnWork,
    stream: &ClaudeStreamState,
    parent_thread: &ClaudeThread,
    parent_turn: &ClaudeTurn,
) -> Vec<ClaudeThread> {
    parent_turn
        .tool_items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("collabAgentToolCall"))
        .flat_map(|item| {
            let tool_id = collab_agent_item_parent_tool_id(item);
            collab_agent_item_receiver_thread_ids(item)
                .into_iter()
                .map(move |thread_id| (thread_id, tool_id.clone(), item))
        })
        .map(|(thread_id, tool_id, item)| {
            let mut thread =
                virtual_subagent_thread_from_item(&thread_id, parent_thread, parent_turn, item);
            if let Some(tool_id) = tool_id.as_deref() {
                if let Some(subagent_stream) = stream.subagent_streams.get(tool_id) {
                    apply_live_subagent_stream_to_thread(work, subagent_stream, &mut thread);
                }
            }
            thread
        })
        .collect()
}

fn apply_live_subagent_stream_to_thread(
    work: &TurnWork,
    stream: &ClaudeSubagentStreamState,
    thread: &mut ClaudeThread,
) {
    let Some(turn) = thread.turns.last_mut() else {
        return;
    };
    turn.agent_text = live_agent_text_for_subagent_stream(stream);
    turn.tool_items = live_tool_items_for_subagent_stream(&thread.id, &work.cwd, &turn.id, stream);
    turn.status = TurnStatus::InProgress;
    turn.completed_at = None;
    turn.duration_ms = None;
    thread.updated_at = now_seconds();
}

fn live_agent_text_for_subagent_stream(stream: &ClaudeSubagentStreamState) -> String {
    if !stream.emitted_text.is_empty() {
        return stream.emitted_text.clone();
    }
    if !stream.saw_tool_call && !stream.pending_agent_text.trim().is_empty() {
        return stream.pending_agent_text.clone();
    }
    String::new()
}

fn live_tool_items_for_subagent_stream(
    thread_id: &str,
    cwd: &str,
    turn_id: &str,
    stream: &ClaudeSubagentStreamState,
) -> Vec<Value> {
    let mut items = Vec::new();
    let mut reasoning_text = stream.reasoning_text.clone();
    if stream.saw_tool_call && !stream.pending_agent_text.trim().is_empty() {
        reasoning_text.push_str(&stream.pending_agent_text);
    }
    if !reasoning_text.trim().is_empty() {
        items.push(reasoning_item_json(turn_id, &reasoning_text));
    }
    for tool_id in &stream.tool_order {
        let Some(state) = stream.tool_calls.get(tool_id) else {
            continue;
        };
        if let Some(completion) = stream.completed_tools.get(tool_id) {
            let duration_ms = json!((completion.completed_at_ms - state.started_at_ms).max(0));
            items.push(tool_call_item(
                thread_id,
                cwd,
                tool_id,
                state,
                if completion.success {
                    "completed"
                } else {
                    "failed"
                },
                completion.result.as_deref(),
                duration_ms,
            ));
        } else if state.started_emitted || !is_empty_tool_arguments(&state.arguments) {
            items.push(tool_call_item(
                thread_id,
                cwd,
                tool_id,
                state,
                "inProgress",
                None,
                Value::Null,
            ));
        }
    }
    items
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
    let content = claude_stream_json_user_content(work);
    let user_message = json!({
        "type": "user",
        "session_id": "",
        "message": {
            "role": "user",
            "content": content,
        },
        "parent_tool_use_id": Value::Null,
    });
    format!(
        "{}\n{}\n",
        serde_json::to_string(&initialize).unwrap_or_default(),
        serde_json::to_string(&user_message).unwrap_or_default()
    )
}

fn claude_stream_json_steer_message(input: &Value) -> Value {
    json!({
        "type": "user",
        "session_id": "",
        "message": {
            "role": "user",
            "content": claude_stream_json_content_from_input_value(input),
        },
        "parent_tool_use_id": Value::Null,
    })
}

fn claude_stream_json_content_from_input_value(input: &Value) -> Vec<Value> {
    let mut content = Vec::new();
    let values = input
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![input.clone()]);
    let mut has_user_content = false;
    for item in values {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            Some("localImage") | Some("image") => {
                if let Some(image) = claude_stream_json_image_content(&item) {
                    content.push(image);
                    has_user_content = true;
                } else if let Some(text) = prompt_text_for_single_input_item(&item) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            Some("mention") | Some("skill") => {
                if let Some(text) = prompt_text_for_single_input_item(&item) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            _ => {
                if let Some(text) = item.as_str().and_then(non_empty_string).or_else(|| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .and_then(non_empty_string)
                }) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
        }
    }
    if !has_user_content {
        content.push(json!({ "type": "text", "text": compact_json(input) }));
    }
    content
}

fn drain_steer_messages<W>(
    work: &TurnWork,
    child_stdin: &mut W,
    steer_rx: &mpsc::Receiver<Value>,
) -> Result<usize, String>
where
    W: Write,
{
    let mut sent_count = 0;
    loop {
        match steer_rx.try_recv() {
            Ok(input) => {
                let message = claude_stream_json_steer_message(&input);
                write_claude_child_json_line(child_stdin, &message)?;
                sent_count += 1;
                claude_code_log_event(
                    "claude_steer_sent",
                    json!({
                        "threadId": &work.thread_id,
                        "turnId": &work.turn_id,
                        "input": log_request_params_summary(&input),
                    }),
                );
            }
            Err(mpsc::TryRecvError::Empty) => return Ok(sent_count),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(sent_count),
        }
    }
}

fn claude_stream_json_user_content(work: &TurnWork) -> Vec<Value> {
    let mut content = Vec::new();
    if let Some(instruction_context) = work
        .instruction_context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        content.push(json!({ "type": "text", "text": instruction_context }));
    }
    if work.input.is_empty() {
        content.push(json!({ "type": "text", "text": work.prompt }));
        return content;
    }
    let mut has_user_content = false;
    for item in &work.input {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            Some("localImage") | Some("image") => {
                if let Some(image) = claude_stream_json_image_content(item) {
                    content.push(image);
                    has_user_content = true;
                } else if let Some(text) = prompt_text_for_single_input_item(item) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            Some("mention") | Some("skill") => {
                if let Some(text) = prompt_text_for_single_input_item(item) {
                    content.push(json!({ "type": "text", "text": text }));
                    has_user_content = true;
                }
            }
            _ => {}
        }
    }
    if !has_user_content {
        content.push(json!({ "type": "text", "text": work.prompt }));
    }
    content
}

fn claude_stream_json_image_content(item: &Value) -> Option<Value> {
    if let Some(url) = first_input_item_string(item, &["url", "uri", "href", "src"]) {
        return Some(json!({
            "type": "image",
            "source": {
                "type": "url",
                "url": url,
            },
        }));
    }
    let mime_type = first_input_item_string(item, &["mimeType", "mediaType", "mime_type"])
        .unwrap_or_else(|| {
            first_input_item_string(item, &["path", "filePath", "file_path"])
                .as_deref()
                .map(mime_type_for_image_path)
                .unwrap_or("image/png")
                .to_string()
        });
    let data = first_input_item_string(item, &["data", "dataBase64", "base64"]).or_else(|| {
        first_input_item_string(item, &["path", "filePath", "file_path"])
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| general_purpose::STANDARD.encode(bytes))
    })?;
    Some(json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": mime_type,
            "data": data,
        },
    }))
}

fn prompt_text_for_single_input_item(item: &Value) -> Option<String> {
    let prompt = prompt_from_input(std::slice::from_ref(item));
    (prompt != "(empty prompt)").then_some(prompt)
}

fn first_input_item_string(item: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        item.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn mime_type_for_image_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        _ => "image/png",
    }
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
    if let Some(parent_tool_use_id) = parent_tool_use_id {
        handle_claude_subagent_stream_event(parent_tool_use_id, event, stream);
        return;
    }
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
        if let Some(parent_tool_use_id) = parent_tool_use_id {
            handle_claude_subagent_assistant_content(parent_tool_use_id, content, stream);
            if !content_contains_tool_use(content) {
                if let Some(text) = claude_text_from_content(content) {
                    complete_tool_call(output, work, stream, parent_tool_use_id, true, Some(text));
                }
            }
            return;
        }
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
    if let Some(parent_tool_use_id) = parent_tool_use_id {
        handle_claude_subagent_user_content(parent_tool_use_id, content, stream);
        return;
    }
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

fn handle_claude_subagent_stream_event(
    parent_tool_use_id: &str,
    event: &Value,
    stream: &mut ClaudeStreamState,
) {
    match event.get("type").and_then(Value::as_str) {
        Some("content_block_start") => {
            let index = event.get("index").and_then(Value::as_i64);
            if let Some(content_block) = event.get("content_block") {
                let subagent = stream
                    .subagent_streams
                    .entry(parent_tool_use_id.to_string())
                    .or_default();
                if let (Some(index), Some(tool_id)) =
                    (index, content_block.get("id").and_then(Value::as_str))
                {
                    subagent
                        .tool_block_by_index
                        .insert(index, tool_id.to_string());
                }
                handle_claude_subagent_content_block(content_block, subagent);
            }
        }
        Some("content_block_delta") => {
            let Some(delta) = event.get("delta") else {
                return;
            };
            let subagent = stream
                .subagent_streams
                .entry(parent_tool_use_id.to_string())
                .or_default();
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        append_subagent_agent_text_delta(subagent, text);
                    }
                }
                Some("thinking_delta") => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        append_subagent_reasoning(subagent, text);
                    }
                }
                Some("input_json_delta") => {
                    if let (Some(index), Some(partial_json)) = (
                        event.get("index").and_then(Value::as_i64),
                        delta.get("partial_json").and_then(Value::as_str),
                    ) {
                        if let Some(tool_id) = subagent.tool_block_by_index.get(&index) {
                            subagent
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
        Some("content_block_stop") => {
            let Some(index) = event.get("index").and_then(Value::as_i64) else {
                return;
            };
            let Some(subagent) = stream.subagent_streams.get_mut(parent_tool_use_id) else {
                return;
            };
            if let Some(tool_id) = subagent.tool_block_by_index.get(&index).cloned() {
                if let Some(input) = subagent.tool_input_deltas.remove(&tool_id) {
                    if !input.trim().is_empty() {
                        update_subagent_tool_call_arguments(
                            subagent,
                            &tool_id,
                            parse_tool_arguments(input.trim()),
                        );
                    }
                }
            }
        }
        Some("message_start") | Some("message_delta") | Some("message_stop") => {}
        _ => {}
    }
}

fn handle_claude_subagent_assistant_content(
    parent_tool_use_id: &str,
    content: &Value,
    stream: &mut ClaudeStreamState,
) {
    let subagent = stream
        .subagent_streams
        .entry(parent_tool_use_id.to_string())
        .or_default();
    if let Some(text) = claude_text_from_content(content) {
        set_subagent_agent_text(subagent, &text);
    }
    collect_claude_subagent_content_blocks(content, subagent);
}

fn handle_claude_subagent_user_content(
    parent_tool_use_id: &str,
    content: &Value,
    stream: &mut ClaudeStreamState,
) {
    let subagent = stream
        .subagent_streams
        .entry(parent_tool_use_id.to_string())
        .or_default();
    collect_claude_subagent_content_blocks(content, subagent);
}

fn collect_claude_subagent_content_blocks(
    content: &Value,
    subagent: &mut ClaudeSubagentStreamState,
) {
    match content {
        Value::Array(items) => {
            for item in items {
                handle_claude_subagent_content_block(item, subagent);
            }
        }
        Value::Object(_) => handle_claude_subagent_content_block(content, subagent),
        _ => {}
    }
}

fn handle_claude_subagent_content_block(block: &Value, subagent: &mut ClaudeSubagentStreamState) {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                set_subagent_agent_text(subagent, text);
            }
        }
        Some("thinking") | Some("thinking_delta") => {
            if let Some(text) = block
                .get("thinking")
                .or_else(|| block.get("text"))
                .and_then(Value::as_str)
            {
                append_subagent_reasoning(subagent, text);
            }
        }
        Some("tool_use") | Some("server_tool_use") | Some("mcp_tool_use") => {
            let tool_id = block.get("id").and_then(Value::as_str).unwrap_or("unknown");
            let tool_name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
            let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
            start_subagent_tool_call(subagent, tool_id, tool_name, arguments);
        }
        Some("tool_result")
        | Some("tool_search_tool_result")
        | Some("web_fetch_tool_result")
        | Some("web_search_tool_result")
        | Some("code_execution_tool_result")
        | Some("bash_code_execution_tool_result")
        | Some("text_editor_code_execution_tool_result")
        | Some("mcp_tool_result") => {
            let tool_id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let success = !block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let result = block
                .get("content")
                .and_then(claude_text_from_content)
                .unwrap_or_else(|| compact_json(block));
            complete_subagent_tool_call(subagent, tool_id, success, Some(result));
        }
        _ => {}
    }
}

fn content_contains_tool_use(content: &Value) -> bool {
    match content {
        Value::Array(items) => items.iter().any(content_contains_tool_use),
        Value::Object(map) => {
            matches!(
                map.get("type").and_then(Value::as_str),
                Some("tool_use") | Some("server_tool_use") | Some("mcp_tool_use")
            ) || map.get("content").is_some_and(content_contains_tool_use)
        }
        _ => false,
    }
}

fn append_subagent_agent_text_delta(subagent: &mut ClaudeSubagentStreamState, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if !subagent.saw_tool_call && subagent.emitted_text.is_empty() {
        subagent.pending_agent_text.push_str(delta);
    } else {
        subagent.emitted_text.push_str(delta);
    }
}

fn set_subagent_agent_text(subagent: &mut ClaudeSubagentStreamState, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !subagent.saw_tool_call && subagent.emitted_text.is_empty() {
        subagent.pending_agent_text = text.to_string();
    } else {
        subagent.emitted_text = text.to_string();
    }
}

fn append_subagent_reasoning(subagent: &mut ClaudeSubagentStreamState, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    subagent.reasoning_text.push_str(text);
}

fn flush_subagent_pending_agent_text_as_reasoning(subagent: &mut ClaudeSubagentStreamState) {
    if subagent.pending_agent_text.trim().is_empty() {
        subagent.pending_agent_text.clear();
        return;
    }
    let text = std::mem::take(&mut subagent.pending_agent_text);
    subagent.reasoning_text.push_str(&text);
}

fn start_subagent_tool_call(
    subagent: &mut ClaudeSubagentStreamState,
    tool_id: &str,
    tool_name: &str,
    arguments: Value,
) {
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    let tool_name = non_empty_string(tool_name).unwrap_or_else(|| "tool".to_string());
    subagent.saw_tool_call = true;
    flush_subagent_pending_agent_text_as_reasoning(subagent);
    if !subagent.tool_calls.contains_key(&tool_id) {
        subagent.tool_order.push(tool_id.clone());
    }
    let started_emitted = !is_empty_tool_arguments(&arguments);
    let entry = subagent
        .tool_calls
        .entry(tool_id)
        .or_insert_with(|| ClaudeToolCallState {
            name: tool_name.clone(),
            arguments: json!({}),
            started_at_ms: now_millis(),
            started_emitted,
            kind: claude_tool_item_kind(&tool_name),
        });
    entry.name = tool_name;
    entry.kind = claude_tool_item_kind(&entry.name);
    if !is_empty_tool_arguments(&arguments) {
        entry.arguments = arguments;
        entry.started_emitted = true;
    }
}

fn update_subagent_tool_call_arguments(
    subagent: &mut ClaudeSubagentStreamState,
    tool_id: &str,
    arguments: Value,
) {
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    if !subagent.tool_calls.contains_key(&tool_id) {
        start_subagent_tool_call(subagent, &tool_id, "tool", arguments);
        return;
    }
    if !is_empty_tool_arguments(&arguments) {
        if let Some(state) = subagent.tool_calls.get_mut(&tool_id) {
            state.arguments = arguments;
            state.started_emitted = true;
        }
    }
}

fn complete_subagent_tool_call(
    subagent: &mut ClaudeSubagentStreamState,
    tool_id: &str,
    success: bool,
    result: Option<String>,
) {
    let tool_id = non_empty_string(tool_id).unwrap_or_else(|| "unknown".to_string());
    if !subagent.tool_calls.contains_key(&tool_id) {
        start_subagent_tool_call(subagent, &tool_id, "tool", json!({}));
    }
    if let Some(state) = subagent.tool_calls.get_mut(&tool_id) {
        state.started_emitted = true;
    }
    subagent.completed_tools.insert(
        tool_id,
        ClaudeSubagentToolCompletion {
            success,
            result,
            completed_at_ms: now_millis(),
        },
    );
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
        ClaudeToolItemKind::FileChange => {
            file_change_item_for_tool(tool_id, cwd, state, status, result)
                .unwrap_or_else(|| mcp_tool_call_item(tool_id, state, status, result))
        }
        ClaudeToolItemKind::McpToolCall => mcp_tool_call_item(tool_id, state, status, result),
    }
}

fn claude_tool_item_kind(tool_name: &str) -> ClaudeToolItemKind {
    match tool_name {
        "Agent" | "Task" => ClaudeToolItemKind::CollabAgentToolCall,
        "Bash" => ClaudeToolItemKind::CommandExecution,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "NotebookWrite" => {
            ClaudeToolItemKind::FileChange
        }
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

fn file_change_item_for_tool(
    tool_id: &str,
    cwd: &str,
    state: &ClaudeToolCallState,
    status: &str,
    result: Option<&str>,
) -> Option<Value> {
    let path = file_change_path_from_arguments(&state.arguments)?;
    let (kind, diff) = file_change_diff_for_tool(cwd, &path, &state.name, &state.arguments)?;
    let failed = status == "failed";
    Some(json!({
        "type": "fileChange",
        "id": tool_item_id(tool_id),
        "status": status,
        "changes": [
            {
                "path": path,
                "kind": kind,
                "diff": diff,
            }
        ],
        "error": if failed {
            json!({ "message": result.unwrap_or("Claude Code file edit failed") })
        } else {
            Value::Null
        },
        "durationMs": Value::Null,
    }))
}

fn file_change_path_from_arguments(arguments: &Value) -> Option<String> {
    ["file_path", "path", "notebook_path", "notebookPath"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .and_then(non_empty_string)
}

fn file_change_diff_for_tool(
    cwd: &str,
    path: &str,
    tool_name: &str,
    arguments: &Value,
) -> Option<(Value, String)> {
    match tool_name {
        "Write" | "NotebookWrite" => {
            let new_content = arguments
                .get("content")
                .or_else(|| arguments.get("new_content"))
                .or_else(|| arguments.get("text"))
                .and_then(Value::as_str)?;
            let file_path = resolve_tool_file_path(cwd, path);
            let old_content = std::fs::read_to_string(&file_path).unwrap_or_default();
            let kind = if file_path.is_file() {
                file_change_kind("update")
            } else {
                file_change_kind("create")
            };
            Some((kind, unified_diff_for_content(&old_content, new_content)))
        }
        "MultiEdit" => {
            let edits = arguments.get("edits").and_then(Value::as_array)?;
            let file_path = resolve_tool_file_path(cwd, path);
            let original = std::fs::read_to_string(&file_path).unwrap_or_default();
            let updated = apply_multi_edit_preview(&original, edits)?;
            Some((
                file_change_kind("update"),
                unified_diff_for_content(&original, &updated),
            ))
        }
        "Edit" | "NotebookEdit" => {
            let old_string = arguments.get("old_string").and_then(Value::as_str)?;
            let new_string = arguments.get("new_string").and_then(Value::as_str)?;
            Some((
                file_change_kind("update"),
                unified_diff_for_edit(cwd, path, old_string, new_string),
            ))
        }
        _ => None,
    }
}

fn file_change_kind(kind: &str) -> Value {
    json!({
        "type": kind,
        "move_path": Value::Null,
    })
}

fn unified_diff_for_edit(cwd: &str, path: &str, old_string: &str, new_string: &str) -> String {
    let start_line = edit_hunk_start_line(cwd, path, old_string, new_string).unwrap_or(1);
    let old_lines = diff_lines(old_string);
    let new_lines = diff_lines(new_string);
    let mut diff = format!(
        "@@ -{},{} +{},{} @@\n",
        start_line,
        old_lines.len(),
        start_line,
        new_lines.len()
    );
    for line in &old_lines {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &new_lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    truncate_for_protocol(&diff, 200_000)
}

fn unified_diff_for_content(old_content: &str, new_content: &str) -> String {
    let old_lines = diff_lines(old_content);
    let new_lines = diff_lines(new_content);
    let mut diff = format!("@@ -1,{} +1,{} @@\n", old_lines.len(), new_lines.len());
    for line in &old_lines {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &new_lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    truncate_for_protocol(&diff, 200_000)
}

fn apply_multi_edit_preview(original: &str, edits: &[Value]) -> Option<String> {
    let mut content = original.to_string();
    for edit in edits {
        let old_string = edit.get("old_string").and_then(Value::as_str)?;
        let new_string = edit.get("new_string").and_then(Value::as_str)?;
        let replace_all = edit
            .get("replace_all")
            .or_else(|| edit.get("replaceAll"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if replace_all {
            content = content.replace(old_string, new_string);
        } else if let Some(index) = content.find(old_string) {
            content.replace_range(index..index + old_string.len(), new_string);
        } else {
            content.push_str("\n");
            content.push_str(new_string);
        }
    }
    Some(content)
}

fn diff_lines(value: &str) -> Vec<&str> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.trim_end_matches('\n').split('\n').collect()
    }
}

fn edit_hunk_start_line(
    cwd: &str,
    path: &str,
    old_string: &str,
    new_string: &str,
) -> Option<usize> {
    let file_path = resolve_tool_file_path(cwd, path);
    let content = std::fs::read_to_string(file_path).ok()?;
    find_text_start_line(&content, old_string)
        .or_else(|| find_text_start_line(&content, new_string))
}

fn resolve_tool_file_path(cwd: &str, path: &str) -> PathBuf {
    let file_path = PathBuf::from(path);
    if file_path.is_absolute() {
        file_path
    } else {
        PathBuf::from(cwd).join(file_path)
    }
}

fn find_text_start_line(content: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let index = content.find(needle)?;
    Some(
        content[..index]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1,
    )
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
    if let Some(permission_mode) = claude_permission_mode_arg(work) {
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
    unregister_active_steer_sender(&work.thread_id, &work.turn_id);
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

fn active_steer_senders() -> &'static Mutex<BTreeMap<(String, String), mpsc::Sender<Value>>> {
    CLAUDE_ACTIVE_STEER_SENDERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn register_active_steer_sender(work: &TurnWork, sender: mpsc::Sender<Value>) {
    if let Ok(mut senders) = active_steer_senders().lock() {
        senders.insert((work.thread_id.clone(), work.turn_id.clone()), sender);
    }
}

fn unregister_active_steer_sender(thread_id: &str, turn_id: &str) {
    if let Ok(mut senders) = active_steer_senders().lock() {
        senders.remove(&(thread_id.to_string(), turn_id.to_string()));
        senders.remove(&(
            strip_local_thread_prefix(thread_id).to_string(),
            turn_id.to_string(),
        ));
    }
}

fn steer_turn_input_from_params(params: &Value) -> Value {
    for key in ["input", "message", "content", "delta"] {
        if let Some(value) = params.get(key).filter(|value| !value.is_null()) {
            return value.clone();
        }
    }
    for key in ["text", "prompt"] {
        if let Some(text) = params
            .get(key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            return json!([{ "type": "text", "text": text }]);
        }
    }
    json!([{ "type": "text", "text": "" }])
}

fn steer_turn_inactive_error(thread_id: &str) -> String {
    if thread_id.trim().is_empty() {
        "SteerTurnInactiveError: cannot steer inactive local turn".to_string()
    } else {
        format!(
            "SteerTurnInactiveError: cannot steer inactive local turn for conversation {}",
            thread_id
        )
    }
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

fn claude_permission_mode_arg(work: &TurnWork) -> Option<String> {
    std::env::var(PERMISSION_MODE_ENV)
        .ok()
        .and_then(|value| non_empty_string(&value))
        .or_else(|| work.permission_mode.clone())
}

fn claude_permission_mode_for_approvals_reviewer(approvals_reviewer: &str) -> Option<String> {
    is_auto_review_approvals_reviewer(approvals_reviewer).then(|| "auto".to_string())
}

fn is_auto_review_approvals_reviewer(approvals_reviewer: &str) -> bool {
    let normalized = approvals_reviewer
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    matches!(
        normalized.as_str(),
        AUTO_REVIEW_APPROVALS_REVIEWER | "autoreview" | "auto" | "guardian_subagent"
    )
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
    if let Some(permission_mode) = claude_permission_mode_arg(work) {
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
    claude_transcript_path_for_session(&work.cwd, &work.claude_session_id)
}

fn claude_transcript_path_for_session(cwd: &str, session_id: &str) -> Option<PathBuf> {
    let projects_dir = claude_projects_dir()?;
    let filename = format!("{session_id}.jsonl");
    for dir_name in claude_project_dir_candidates(cwd) {
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
    let path = path.to_string_lossy();
    let sanitized = path
        .encode_utf16()
        .map(|unit| {
            if (b'0' as u16..=b'9' as u16).contains(&unit)
                || (b'A' as u16..=b'Z' as u16).contains(&unit)
                || (b'a' as u16..=b'z' as u16).contains(&unit)
            {
                char::from_u32(unit as u32).unwrap_or('-')
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.len() <= CLAUDE_PROJECT_DIR_MAX_LEN {
        return sanitized;
    }
    format!(
        "{}-{}",
        &sanitized[..CLAUDE_PROJECT_DIR_MAX_LEN],
        base36_u64(claude_js_string_hash_abs(&path))
    )
}

fn claude_js_string_hash_abs(value: &str) -> u64 {
    let mut hash = 0i32;
    for unit in value.encode_utf16() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(unit as i32);
    }
    (hash as i64).abs() as u64
}

fn base36_u64(mut value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        let ch = if digit < 10 {
            (b'0' + digit) as char
        } else {
            (b'a' + digit - 10) as char
        };
        digits.push(ch);
        value /= 36;
    }
    digits.iter().rev().collect()
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
    stream: &mut ClaudeStreamState,
) where
    W: Write,
{
    if !stream.reasoning_item_started || stream.reasoning_item_completed {
        return;
    }
    let item = reasoning_item_json(&work.turn_id, &stream.reasoning_text);
    stream.completed_tool_items.push(item.clone());
    stream.reasoning_item_completed = true;
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

fn reasoning_item_json(turn_id: &str, text: &str) -> Value {
    json!({
        "type": "reasoning",
        "id": reasoning_item_id_for_turn(turn_id),
        "summary": [],
        "content": if text.is_empty() {
            json!([])
        } else {
            json!([text])
        },
    })
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
        "serviceTier": thread.service_tier,
        "cwd": thread.cwd,
        "runtimeWorkspaceRoots": thread.workspace_roots,
        "instructionSources": [],
        "approvalPolicy": thread.approval_policy,
        "approvalsReviewer": thread.approvals_reviewer,
        "sandbox": claude_workspace_write_sandbox_policy(&thread.workspace_roots),
        "activePermissionProfile": Value::Null,
        "reasoningEffort": thread.reasoning_effort,
        "baseInstructions": thread
            .base_instructions
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "developerInstructions": thread
            .developer_instructions
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "personality": thread.personality.clone(),
        "persistExtendedHistory": thread.persist_extended_history.clone(),
    })
}

fn claude_workspace_write_sandbox_policy(workspace_roots: &[String]) -> Value {
    json!({
        "type": "workspaceWrite",
        "writableRoots": workspace_roots,
        "networkAccess": false,
        "excludeTmpdirEnvVar": false,
        "excludeSlashTmp": false,
    })
}

fn config_read_response(params: &Value, overrides: &Map<String, Value>) -> Value {
    let layers = params
        .get("includeLayers")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| json!([]))
        .unwrap_or(Value::Null);
    let mut config = default_config_read_config();
    merge_config_values(&mut config, overrides);
    json!({
        "config": Value::Object(config),
        "origins": {},
        "layers": layers,
    })
}

fn default_config_read_config() -> Map<String, Value> {
    let mut config = Map::new();
    for key in [
        "model",
        "review_model",
        "model_context_window",
        "model_auto_compact_token_limit",
        "model_auto_compact_token_limit_scope",
        "model_provider",
        "approval_policy",
        "approvals_reviewer",
        "sandbox_mode",
        "sandbox_workspace_write",
        "forced_chatgpt_workspace_id",
        "forced_login_method",
        "web_search",
        "tools",
        "instructions",
        "developer_instructions",
        "compact_prompt",
        "model_reasoning_effort",
        "model_reasoning_summary",
        "model_verbosity",
        "service_tier",
        "analytics",
        "desktop",
    ] {
        config.insert(key.to_string(), Value::Null);
    }
    config.insert("projects".to_string(), json!({}));
    config
}

fn apply_config_write_params(method: &str, params: &Value, config: &mut Map<String, Value>) {
    if method == "config/value/write" {
        if let Some((key, value)) = config_write_entry(params) {
            insert_config_value(config, &key, value);
        }
        return;
    }

    if let Some(values) = params.get("values").and_then(Value::as_object) {
        merge_config_values(config, values);
    }
    if let Some(values) = params.get("config").and_then(Value::as_object) {
        merge_config_values(config, values);
    }
    if let Some(edits) = params.get("edits").and_then(Value::as_array) {
        for edit in edits {
            if let Some((key, value)) = config_write_entry(edit) {
                insert_config_value(config, &key, value);
            }
        }
    }
}

fn config_write_entry(value: &Value) -> Option<(String, Value)> {
    let key = [
        "keyPath",
        "key_path",
        "key",
        "name",
        "path",
        "configKey",
        "config_key",
        "field",
    ]
    .into_iter()
    .find_map(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })?;
    let value = value
        .get("value")
        .or_else(|| value.get("configValue"))
        .or_else(|| value.get("config_value"))
        .cloned()
        .unwrap_or(Value::Null);
    Some((key, value))
}

fn merge_config_values(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (key, value) in source {
        if let Some(source_object) = value.as_object() {
            if let Some(target_object) = target.get_mut(key).and_then(Value::as_object_mut) {
                merge_config_values(target_object, source_object);
                continue;
            }
        }
        target.insert(key.clone(), value.clone());
    }
}

fn insert_config_value(config: &mut Map<String, Value>, key: &str, value: Value) {
    let parts = key
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return;
    }
    insert_nested_config_value(config, &parts, value);
}

fn insert_nested_config_value(config: &mut Map<String, Value>, parts: &[&str], value: Value) {
    if parts.len() == 1 {
        config.insert(parts[0].to_string(), value);
        return;
    }
    let child = config
        .entry(parts[0].to_string())
        .or_insert_with(|| json!({}));
    if !child.is_object() {
        *child = json!({});
    }
    if let Some(child) = child.as_object_mut() {
        insert_nested_config_value(child, &parts[1..], value);
    }
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

fn approval_policy_from_params(params: &Value, fallback: Option<&str>) -> String {
    json_non_empty_string(params.get("approvalPolicy"))
        .or_else(|| json_non_empty_string(params.pointer("/config/approvalPolicy")))
        .or_else(|| json_non_empty_string(params.pointer("/config/approval_policy")))
        .or_else(|| fallback.and_then(non_empty_string))
        .unwrap_or_else(|| DEFAULT_APPROVAL_POLICY.to_string())
}

fn approvals_reviewer_from_params(params: &Value, fallback: Option<&str>) -> String {
    json_non_empty_string(params.get("approvalsReviewer"))
        .or_else(|| json_non_empty_string(params.pointer("/config/approvalsReviewer")))
        .or_else(|| json_non_empty_string(params.pointer("/config/approvals_reviewer")))
        .or_else(|| fallback.and_then(non_empty_string))
        .unwrap_or_else(|| DEFAULT_APPROVALS_REVIEWER.to_string())
}

fn reasoning_effort_from_params(params: &Value, fallback: Option<&Value>) -> Value {
    thread_runtime_metadata_value(params, &["reasoningEffort", "reasoning_effort", "effort"])
        .or_else(|| {
            params
                .pointer("/collaborationMode/settings/reasoning_effort")
                .or_else(|| params.pointer("/collaborationMode/settings/reasoningEffort"))
                .filter(|value| !value.is_null())
                .cloned()
        })
        .or_else(|| {
            params
                .pointer("/config/model_reasoning_effort")
                .filter(|value| !value.is_null())
                .cloned()
        })
        .or_else(|| fallback.filter(|value| !value.is_null()).cloned())
        .unwrap_or(Value::Null)
}

fn service_tier_from_params(params: &Value, fallback: Option<&Value>) -> Value {
    thread_runtime_metadata_value(params, &["serviceTier", "service_tier"])
        .or_else(|| {
            params
                .pointer("/config/service_tier")
                .filter(|value| !value.is_null())
                .cloned()
        })
        .or_else(|| fallback.filter(|value| !value.is_null()).cloned())
        .unwrap_or(Value::Null)
}

fn collaboration_mode_from_params(
    params: &Value,
    model: &str,
    reasoning_effort: &Value,
    fallback: Option<&Value>,
) -> Value {
    thread_metadata_value(params, &["collaborationMode"])
        .filter(|value| !value.is_null())
        .cloned()
        .map(|value| normalized_collaboration_mode(value, model, reasoning_effort))
        .or_else(|| fallback.filter(|value| !value.is_null()).cloned())
        .unwrap_or(Value::Null)
}

fn apply_thread_runtime_metadata_from_params(thread: &mut ClaudeThread, params: &Value) {
    if let Some(model) = params.get("model").and_then(Value::as_str) {
        thread.model = model.to_string();
    }
    thread.reasoning_effort = reasoning_effort_from_params(params, Some(&thread.reasoning_effort));
    thread.service_tier = service_tier_from_params(params, Some(&thread.service_tier));
    thread.collaboration_mode = collaboration_mode_from_params(
        params,
        &thread.model,
        &thread.reasoning_effort,
        Some(&thread.collaboration_mode),
    );
}

fn thread_instruction_metadata_from_params(params: &Value) -> ThreadInstructionMetadata {
    ThreadInstructionMetadata {
        base: instruction_string_from_params(params, &["baseInstructions", "base_instructions"]),
        developer: combined_developer_instructions_from_params(params),
        personality: thread_runtime_metadata_value(params, &["personality"]).unwrap_or(Value::Null),
        persist_extended_history: thread_runtime_metadata_value(
            params,
            &["persistExtendedHistory", "persist_extended_history"],
        )
        .unwrap_or(Value::Null),
    }
}

fn apply_thread_instruction_metadata_from_params(thread: &mut ClaudeThread, params: &Value) {
    if let Some(base) =
        thread_metadata_optional_string_update(params, &["baseInstructions", "base_instructions"])
    {
        thread.base_instructions = base;
    }
    if let Some(developer) = optional_combined_developer_instructions_from_params(params) {
        thread.developer_instructions = developer;
    }
    if let Some(personality) = thread_runtime_metadata_value(params, &["personality"]) {
        thread.personality = personality;
    }
    if let Some(persist_extended_history) = thread_runtime_metadata_value(
        params,
        &["persistExtendedHistory", "persist_extended_history"],
    ) {
        thread.persist_extended_history = persist_extended_history;
    }
}

fn instruction_string_from_params(params: &Value, keys: &[&str]) -> Option<String> {
    thread_metadata_string(params, keys)
}

fn optional_combined_developer_instructions_from_params(params: &Value) -> Option<Option<String>> {
    let developer = thread_metadata_optional_string_update(
        params,
        &["developerInstructions", "developer_instructions"],
    );
    let additional = thread_metadata_string(
        params,
        &[
            "additionalDeveloperInstructions",
            "additional_developer_instructions",
        ],
    );
    match (developer, additional) {
        (None, None) => None,
        (Some(None), None) => Some(None),
        (Some(None), Some(additional)) => Some(Some(additional)),
        (Some(Some(developer)), None) => Some(Some(developer)),
        (None, Some(additional)) => Some(Some(additional)),
        (Some(Some(developer)), Some(additional)) => {
            Some(Some(format!("{}\n\n{}", developer, additional)))
        }
    }
}

fn combined_developer_instructions_from_params(params: &Value) -> Option<String> {
    optional_combined_developer_instructions_from_params(params).flatten()
}

fn claude_thread_instruction_context(thread: &ClaudeThread) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(base) = thread
        .base_instructions
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("Base instructions:\n{base}"));
    }
    if let Some(developer) = thread
        .developer_instructions
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("Developer instructions:\n{developer}"));
    }
    (!sections.is_empty()).then(|| {
        format!(
            "Follow these instructions for this Codex app turn:\n\n{}",
            sections.join("\n\n")
        )
    })
}

fn thread_runtime_metadata_value(params: &Value, keys: &[&str]) -> Option<Value> {
    thread_metadata_value(params, keys)
        .filter(|value| !value.is_null())
        .cloned()
}

fn git_info_from_params(params: &Value) -> Option<Value> {
    thread_metadata_value(params, &["gitInfo", "git_info"])
        .filter(|value| !value.is_null())
        .cloned()
}

fn git_info_update_from_params(params: &Value) -> Option<Value> {
    thread_metadata_value(params, &["gitInfo", "git_info"]).cloned()
}

fn apply_thread_git_info_from_params(thread: &mut ClaudeThread, params: &Value) {
    if let Some(git_info) = git_info_update_from_params(params) {
        thread.git_info = git_info;
    }
}

fn git_info_for_cwd(cwd: &str) -> Value {
    let inside_work_tree = git_command_stdout(cwd, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == "true");
    if !inside_work_tree {
        return Value::Null;
    }

    let branch = git_command_stdout(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|value| value != "HEAD");
    let sha = git_command_stdout(cwd, &["rev-parse", "HEAD"]);
    let origin_url = git_command_stdout(cwd, &["config", "--get", "remote.origin.url"]);
    if branch.is_none() && sha.is_none() && origin_url.is_none() {
        return Value::Null;
    }

    json!({
        "branch": branch.map(Value::String).unwrap_or(Value::Null),
        "sha": sha.map(Value::String).unwrap_or(Value::Null),
        "originUrl": origin_url.map(Value::String).unwrap_or(Value::Null),
    })
}

fn git_command_stdout(cwd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn normalized_collaboration_mode(
    mut collaboration_mode: Value,
    model: &str,
    reasoning_effort: &Value,
) -> Value {
    let Value::Object(map) = &mut collaboration_mode else {
        return collaboration_mode;
    };
    map.entry("mode".to_string())
        .or_insert_with(|| json!("default"));
    let settings = map
        .entry("settings".to_string())
        .or_insert_with(|| json!({}));
    if let Value::Object(settings) = settings {
        settings.insert("model".to_string(), json!(model));
        settings.insert("reasoning_effort".to_string(), reasoning_effort.clone());
        settings
            .entry("developer_instructions".to_string())
            .or_insert(Value::Null);
    }
    collaboration_mode
}

fn json_non_empty_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).and_then(non_empty_string)
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

fn append_turn_attachments_to_input(input: &mut Vec<Value>, params: &Value) {
    let mut lines = Vec::new();
    collect_turn_attachment_lines(params.get("attachments"), "attachment", &mut lines);
    collect_turn_attachment_lines(
        params.get("commentAttachments"),
        "comment attachment",
        &mut lines,
    );
    if lines.is_empty() {
        return;
    }
    input.push(json!({
        "type": "text",
        "text": format!(
            "Attached context:\n{}",
            lines
                .into_iter()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }));
}

fn collect_turn_attachment_lines(value: Option<&Value>, label: &str, lines: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(line) = turn_attachment_line(item, label) {
                    lines.push(line);
                }
            }
        }
        Some(value) => {
            if let Some(line) = turn_attachment_line(value, label) {
                lines.push(line);
            }
        }
        None => {}
    }
}

fn turn_attachment_line(value: &Value, label: &str) -> Option<String> {
    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(format!("{label}: {text}"));
    }
    let object = value.as_object()?;
    let title =
        first_attachment_string(object, &["name", "fileName", "filename", "title", "label"]);
    let location = first_attachment_string(
        object,
        &["path", "filePath", "url", "uri", "href", "src", "id"],
    );
    let kind = first_attachment_string(object, &["type", "kind", "mimeType", "mediaType"]);
    let text = first_attachment_string(object, &["text", "content", "description"]);
    let mut parts = Vec::new();
    if let Some(title) = title {
        parts.push(title);
    }
    if let Some(location) = location {
        parts.push(location);
    }
    if let Some(kind) = kind {
        parts.push(format!("type={kind}"));
    }
    if let Some(text) = text {
        parts.push(format!("text={}", truncate_for_protocol(&text, 2_000)));
    }
    (!parts.is_empty()).then(|| format!("{label}: {}", parts.join(" | ")))
}

fn first_attachment_string(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn thread_metadata_string(params: &Value, keys: &[&str]) -> Option<String> {
    thread_metadata_value(params, keys)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn thread_metadata_string_array(params: &Value, keys: &[&str]) -> Option<Vec<String>> {
    let value = thread_metadata_value(params, keys)?;
    match value {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                Some(Vec::new())
            } else {
                Some(vec![value.to_string()])
            }
        }
        _ => None,
    }
}

fn thread_metadata_text_update(params: &Value, keys: &[&str]) -> Option<ThreadMetadataTextUpdate> {
    let value = thread_metadata_value(params, keys)?;
    if value.is_null() {
        return Some(ThreadMetadataTextUpdate::Clear);
    }
    let text = value.as_str()?.trim();
    if text.is_empty() {
        Some(ThreadMetadataTextUpdate::Clear)
    } else {
        Some(ThreadMetadataTextUpdate::Set(text.to_string()))
    }
}

fn thread_metadata_optional_string_update(params: &Value, keys: &[&str]) -> Option<Option<String>> {
    let value = thread_metadata_value(params, keys)?;
    if value.is_null() {
        return Some(None);
    }
    Some(
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    )
}

fn thread_metadata_bool(params: &Value, keys: &[&str]) -> Option<bool> {
    thread_metadata_value(params, keys).and_then(Value::as_bool)
}

fn thread_metadata_value<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    if let Some(value) = value_at_any_key(params, keys) {
        return Some(value);
    }
    for container_key in ["metadata", "updates", "thread"] {
        if let Some(container) = params.get(container_key) {
            if let Some(value) = value_at_any_key(container, keys) {
                return Some(value);
            }
        }
    }
    None
}

fn value_at_any_key<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn default_thread_workspace_metadata(cwd: &str) -> ThreadWorkspaceMetadata {
    ThreadWorkspaceMetadata {
        kind: "project".to_string(),
        roots: vec![cwd.to_string()],
        browser_root: None,
        projectless_output_directory: None,
    }
}

fn thread_workspace_metadata_from_params(params: &Value, cwd: &str) -> ThreadWorkspaceMetadata {
    let kind = thread_metadata_string(params, &["workspaceKind", "workspace_kind"])
        .unwrap_or_else(|| "project".to_string());
    let roots = thread_metadata_string_array(params, &["workspaceRoots", "workspace_roots"])
        .map(|roots| normalize_workspace_roots(roots, cwd))
        .unwrap_or_else(|| vec![cwd.to_string()]);
    let browser_root = thread_metadata_string(
        params,
        &[
            "workspaceBrowserRoot",
            "workspace_browser_root",
            "workspaceRoot",
            "workspace_root",
        ],
    )
    .or_else(|| {
        (kind == "projectless")
            .then(|| roots.first().cloned())
            .flatten()
    });
    let projectless_output_directory = thread_metadata_string(
        params,
        &[
            "projectlessOutputDirectory",
            "projectless_output_directory",
            "outputDirectory",
            "output_directory",
        ],
    )
    .or_else(|| (kind == "projectless").then(|| cwd.to_string()));
    ThreadWorkspaceMetadata {
        kind,
        roots,
        browser_root,
        projectless_output_directory,
    }
}

fn apply_thread_workspace_metadata_from_params(thread: &mut ClaudeThread, params: &Value) {
    if let Some(kind) = thread_metadata_string(params, &["workspaceKind", "workspace_kind"]) {
        thread.workspace_kind = kind;
    }
    if let Some(roots) =
        thread_metadata_string_array(params, &["workspaceRoots", "workspace_roots"])
    {
        thread.workspace_roots = normalize_workspace_roots(roots, &thread.cwd);
    }
    if let Some(browser_root) = thread_metadata_optional_string_update(
        params,
        &[
            "workspaceBrowserRoot",
            "workspace_browser_root",
            "workspaceRoot",
            "workspace_root",
        ],
    ) {
        thread.workspace_browser_root = browser_root;
    } else if thread.workspace_kind == "projectless" && thread.workspace_browser_root.is_none() {
        thread.workspace_browser_root = thread.workspace_roots.first().cloned();
    }
    if let Some(output_directory) = thread_metadata_optional_string_update(
        params,
        &[
            "projectlessOutputDirectory",
            "projectless_output_directory",
            "outputDirectory",
            "output_directory",
        ],
    ) {
        thread.projectless_output_directory = output_directory;
    } else if thread.workspace_kind == "projectless"
        && thread.projectless_output_directory.is_none()
    {
        thread.projectless_output_directory = Some(thread.cwd.clone());
    }
}

fn update_thread_cwd(thread: &mut ClaudeThread, cwd: String) {
    if thread.cwd == cwd {
        return;
    }
    let old_cwd = std::mem::replace(&mut thread.cwd, cwd.clone());
    if thread.workspace_roots.len() == 1
        && thread
            .workspace_roots
            .first()
            .is_some_and(|root| root == &old_cwd)
    {
        thread.workspace_roots = vec![cwd.clone()];
    }
    if thread.workspace_kind == "projectless"
        && thread
            .projectless_output_directory
            .as_deref()
            .is_none_or(|output_directory| output_directory == old_cwd)
    {
        thread.projectless_output_directory = Some(cwd);
    }
    thread.git_info = git_info_for_cwd(&thread.cwd);
}

fn normalize_workspace_roots(roots: Vec<String>, cwd: &str) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        let root = root.trim();
        if !root.is_empty() && seen.insert(root.to_string()) {
            normalized.push(root.to_string());
        }
    }
    if normalized.is_empty() {
        normalized.push(cwd.to_string());
    }
    normalized
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

fn required_thread_id_param(params: &Value) -> Result<&str, String> {
    params
        .get("threadId")
        .or_else(|| params.get("conversationId"))
        .and_then(Value::as_str)
        .ok_or_else(|| "missing required param: threadId".to_string())
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

    fn test_state(workspace_name: Option<&str>) -> ClaudeAppServerState {
        ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            config_values: Map::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: workspace_name.map(str::to_string),
        }
    }

    #[test]
    fn bot_bridge_input_buffers_until_complete_json_lines() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut input = ClaudeBotBridgeInput::new(tx);

        input.write_all(br#"{"id":"1""#).expect("write partial");
        assert!(rx.try_recv().is_err());

        input
            .write_all(
                br#","method":"thread/list","params":{}}
{"id":"2","method":"config/read","params":{}}
"#,
            )
            .expect("write complete lines");

        let first = String::from_utf8(rx.recv().expect("first line")).expect("utf8");
        let second = String::from_utf8(rx.recv().expect("second line")).expect("utf8");
        assert_eq!(
            first,
            r#"{"id":"1","method":"thread/list","params":{}}
"#
        );
        assert_eq!(
            second,
            r#"{"id":"2","method":"config/read","params":{}}
"#
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn bot_bridge_output_tees_events_and_hides_internal_responses() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut output = ClaudeBotBridgeOutput::new(Vec::<u8>::new(), Some(tx));

        output
            .write_all(
                br#"{"method":"item/completed","params":{"threadId":"thread","turnId":"turn"}}
"#,
            )
            .expect("write event");
        output
            .write_all(
                br#"{"id":"codexl-bot-1","result":{"ok":true}}
"#,
            )
            .expect("write internal response");
        output.flush().expect("flush");

        let visible = String::from_utf8(output.inner).expect("visible utf8");
        assert!(
            visible.contains(r#""method":"item/completed""#),
            "{visible}"
        );
        assert!(!visible.contains("codexl-bot-1"), "{visible}");

        let first = String::from_utf8(rx.recv().expect("first tee line")).expect("utf8");
        let second = String::from_utf8(rx.recv().expect("second tee line")).expect("utf8");
        assert!(first.contains(r#""method":"item/completed""#), "{first}");
        assert!(second.contains(r#""id":"codexl-bot-1""#), "{second}");
        assert!(rx.try_recv().is_err());
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            cwd.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_test_git_repo(root: &Path) {
        std::fs::create_dir_all(root).expect("create git repo dir");
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "feature/test"]);
        std::fs::write(root.join("README.md"), "hello\n").expect("write git fixture");
        run_git(root, &["add", "README.md"]);
        run_git(
            root,
            &[
                "-c",
                "user.name=Codex Test",
                "-c",
                "user.email=codex@example.test",
                "commit",
                "-m",
                "init",
            ],
        );
        run_git(
            root,
            &["remote", "add", "origin", "https://example.test/repo.git"],
        );
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
        let found = content.lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|value| {
                value.get("event").and_then(Value::as_str) == Some("test_event")
                    && value.get("threadId").and_then(Value::as_str) == Some("thread")
            })
        });
        assert!(found, "test log event not found in {content}");

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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
    fn thread_runtime_response_uses_workspace_write_sandbox() {
        let root = test_dir("workspace-write-sandbox");
        let cwd = root.join("workspace");
        std::fs::create_dir_all(&cwd).expect("create workspace");
        let mut state = test_state(None);

        let (response, _) = state.start_thread(&json!({
            "cwd": cwd.to_string_lossy(),
            "reasoningEffort": "medium",
            "serviceTier": "flex",
            "collaborationMode": {
                "mode": "default",
                "settings": {
                    "developer_instructions": "extra"
                }
            }
        }));
        assert_eq!(
            response.pointer("/sandbox/type").and_then(Value::as_str),
            Some("workspaceWrite")
        );
        assert_eq!(
            response
                .pointer("/sandbox/writableRoots/0")
                .and_then(Value::as_str),
            Some(cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/sandbox/networkAccess")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            response
                .pointer("/runtimeWorkspaceRoots/0")
                .and_then(Value::as_str),
            Some(cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/thread/workspaceRoots/0")
                .and_then(Value::as_str),
            Some(cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/thread/workspaceKind")
                .and_then(Value::as_str),
            Some("project")
        );
        assert_eq!(
            response.get("reasoningEffort").and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            response.get("serviceTier").and_then(Value::as_str),
            Some("flex")
        );
        assert_eq!(
            response
                .pointer("/thread/workspaceKind")
                .and_then(Value::as_str),
            Some("project")
        );

        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, notifications, _, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");
        let snapshot = notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("snapshot");
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/0/params/sandboxPolicy/type")
                .and_then(Value::as_str),
            Some("workspaceWrite")
        );
        assert_eq!(
            snapshot
                .pointer(
                    "/params/change/conversationState/turns/0/params/sandboxPolicy/writableRoots/0"
                )
                .and_then(Value::as_str),
            Some(cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/latestReasoningEffort")
                .and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/latestCollaborationMode/settings/model")
                .and_then(Value::as_str),
            Some(DEFAULT_MODEL)
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/latestCollaborationMode/settings/reasoning_effort")
                .and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/0/params/effort")
                .and_then(Value::as_str),
            Some("medium")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/0/params/serviceTier")
                .and_then(Value::as_str),
            Some("flex")
        );

        let projectless_root = root.join("Codex");
        let output_dir = projectless_root.join("2026-06-03").join("chat");
        std::fs::create_dir_all(&output_dir).expect("create projectless output");
        let (projectless_response, _) = state.start_thread(&json!({
            "cwd": output_dir.to_string_lossy(),
            "workspaceKind": "projectless",
            "workspaceRoots": [projectless_root.to_string_lossy()],
            "projectlessOutputDirectory": output_dir.to_string_lossy(),
        }));
        assert_eq!(
            projectless_response
                .pointer("/thread/workspaceKind")
                .and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            projectless_response
                .pointer("/thread/workspaceRoots/0")
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            projectless_response
                .pointer("/thread/workspaceBrowserRoot")
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            projectless_response
                .pointer("/thread/projectlessOutputDirectory")
                .and_then(Value::as_str),
            Some(output_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            projectless_response
                .pointer("/runtimeWorkspaceRoots/0")
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            projectless_response
                .pointer("/sandbox/writableRoots/0")
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );
        let projectless_thread_id = projectless_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("projectless thread id")
            .to_string();
        let (_, projectless_notifications, _, _) = state
            .start_turn(&json!({
                "threadId": projectless_thread_id,
                "input": [{ "type": "text", "text": "hello projectless" }],
            }))
            .expect("start projectless turn");
        let projectless_snapshot = projectless_notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("projectless snapshot");
        assert_eq!(
            projectless_snapshot
                .pointer("/params/change/conversationState/workspaceKind")
                .and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            projectless_snapshot
                .pointer("/params/change/conversationState/workspaceBrowserRoot")
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            projectless_snapshot
                .pointer(
                    "/params/change/conversationState/turns/0/params/sandboxPolicy/writableRoots/0"
                )
                .and_then(Value::as_str),
            Some(projectless_root.to_string_lossy().as_ref())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn start_thread_includes_git_info_for_git_workspace() {
        let root = test_dir("thread-git-info");
        init_test_git_repo(&root);
        let mut state = test_state(None);

        let (response, _) = state.start_thread(&json!({
            "cwd": root.to_string_lossy(),
        }));

        assert_eq!(
            response
                .pointer("/thread/gitInfo/branch")
                .and_then(Value::as_str),
            Some("feature/test")
        );
        let sha = response
            .pointer("/thread/gitInfo/sha")
            .and_then(Value::as_str)
            .expect("git sha");
        assert_eq!(sha.len(), 40);
        assert_eq!(
            response
                .pointer("/thread/gitInfo/originUrl")
                .and_then(Value::as_str),
            Some("https://example.test/repo.git")
        );

        let thread_id = response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, notifications, _, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");
        let snapshot = notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("snapshot");
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/gitInfo/branch")
                .and_then(Value::as_str),
            Some("feature/test")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/gitInfo/originUrl")
                .and_then(Value::as_str),
            Some("https://example.test/repo.git")
        );

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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
        });
        assert!(!title_command.contains("--mcp-config"), "{title_command}");

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
        restore_env(CODEX_APP_SERVER_PROXY_ENV, old_proxy);
        restore_env(COMPUTER_USE_NODE_RELAY_NODE_ENV, old_computer_use_node);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_and_mcp_lifecycle_overlay_updates_standalone_lists() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let root = test_dir("plugin-mcp-lifecycle");
        let codex_home = root.join(".codex");
        let plugin_package_dir = codex_home.join("plugins").join("demo-plugin");
        let plugin_dir = plugin_package_dir.join(".codex-plugin");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
  "id": "demo-plugin",
  "name": "Demo Plugin",
  "version": "1.0.0",
  "mcpServers": "./.mcp.json"
}"#,
        )
        .expect("write plugin");
        std::fs::write(
            plugin_package_dir.join(".mcp.json"),
            r#"{
  "mcpServers": {
    "demo-server": {
      "command": "node",
      "args": ["server.js"]
    }
  }
}"#,
        )
        .expect("write mcp config");

        let old_home = std::env::var_os("HOME");
        let old_codex_home = std::env::var_os("CODEX_HOME");
        std::env::set_var("HOME", &root);
        std::env::set_var("CODEX_HOME", &codex_home);

        standalone_plugin_lifecycle_result("plugin/disable", &json!({ "pluginId": "demo-plugin" }));
        let plugins = standalone_plugin_list_result();
        let demo_plugin = plugins
            .get("data")
            .and_then(Value::as_array)
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin.get("id").and_then(Value::as_str) == Some("demo-plugin"))
            })
            .expect("demo plugin");
        assert_eq!(
            demo_plugin.get("enabled").and_then(Value::as_bool),
            Some(false)
        );
        let mcp_servers = standalone_mcp_server_status_list();
        let demo_server = mcp_servers
            .iter()
            .find(|server| server.get("name").and_then(Value::as_str) == Some("demo-server"))
            .expect("demo mcp server");
        assert_eq!(
            demo_server.get("enabled").and_then(Value::as_bool),
            Some(false)
        );

        standalone_plugin_lifecycle_result("plugin/enable", &json!({ "pluginId": "demo-plugin" }));
        standalone_mcp_server_lifecycle_result(
            "mcpServer/disable",
            &json!({ "serverName": "demo-server" }),
        );
        let demo_server = standalone_mcp_server_status_list()
            .into_iter()
            .find(|server| server.get("name").and_then(Value::as_str) == Some("demo-server"))
            .expect("demo mcp server after disable");
        assert_eq!(
            demo_server.get("enabled").and_then(Value::as_bool),
            Some(false)
        );

        standalone_plugin_lifecycle_result(
            "plugin/uninstall",
            &json!({ "pluginId": "demo-plugin" }),
        );
        let plugins = standalone_plugin_list_result();
        let demo_plugin = plugins
            .get("data")
            .and_then(Value::as_array)
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin.get("id").and_then(Value::as_str) == Some("demo-plugin"))
            })
            .expect("demo plugin after uninstall");
        assert_eq!(
            demo_plugin.get("installed").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            demo_plugin.get("enabled").and_then(Value::as_bool),
            Some(false)
        );

        restore_env("HOME", old_home);
        restore_env("CODEX_HOME", old_codex_home);
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            config_values: Map::new(),
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
        assert!(started_snapshot
            .pointer("/params/change/conversationState/requests")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
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

    #[test]
    fn thread_and_conversation_state_include_requests_array() {
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            config_values: Map::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };

        let (thread_response, notification) = state.start_thread(&json!({ "cwd": "/tmp" }));
        assert!(thread_response
            .pointer("/thread/requests")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        assert!(notification
            .pointer("/params/thread/requests")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));

        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id");
        let thread = state.threads.get(thread_id).expect("thread");
        assert!(claude_conversation_state(thread)
            .get("requests")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
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
    fn thread_metadata_update_applies_fields_and_emits_snapshot() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("thread-metadata-update");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let mut state = test_state(Some("workspace"));
        let initial_cwd = root.join("old");
        let new_cwd = root.join("new");
        let workspace_root = root.join("workspace-root");
        let (thread_response, _) = state.start_thread(&json!({
            "cwd": initial_cwd.to_string_lossy(),
            "model": "claude-code",
        }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();

        let (response, notification) = state
            .thread_metadata_update(&json!({
                "threadId": format!("local:{thread_id}"),
                "metadata": {
                    "title": "Manual thread title",
                    "cwd": new_cwd.to_string_lossy(),
                    "model": "opus",
                    "preview": "manual preview",
                    "workspaceKind": "projectless",
                    "workspaceRoots": [workspace_root.to_string_lossy()],
                    "workspaceBrowserRoot": workspace_root.to_string_lossy(),
                    "projectlessOutputDirectory": new_cwd.to_string_lossy(),
                    "reasoningEffort": "high",
                    "serviceTier": "priority",
                    "collaborationMode": {
                        "mode": "default",
                        "settings": {}
                    },
                    "baseInstructions": "base update",
                    "developerInstructions": "developer update",
                    "additionalDeveloperInstructions": "additional update",
                    "personality": "concise",
                    "persistExtendedHistory": true,
                    "gitInfo": {
                        "branch": "manual-branch",
                        "sha": "abc123",
                        "originUrl": "https://example.test/manual.git"
                    },
                    "approvalPolicy": "never",
                    "approvalsReviewer": "auto_review",
                    "archived": true,
                },
            }))
            .expect("metadata update");
        assert_eq!(
            response.pointer("/thread/name").and_then(Value::as_str),
            Some("Manual thread title")
        );
        assert_eq!(
            response.pointer("/thread/cwd").and_then(Value::as_str),
            Some(new_cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/thread/modelProvider")
                .and_then(Value::as_str),
            Some(PROVIDER_NAME)
        );
        assert_eq!(
            response
                .pointer("/thread/approvalPolicy")
                .and_then(Value::as_str),
            Some("never")
        );
        assert_eq!(
            response
                .pointer("/thread/workspaceKind")
                .and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            response
                .pointer("/thread/workspaceRoots/0")
                .and_then(Value::as_str),
            Some(workspace_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/thread/projectlessOutputDirectory")
                .and_then(Value::as_str),
            Some(new_cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            response
                .pointer("/thread/reasoningEffort")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            response
                .pointer("/thread/serviceTier")
                .and_then(Value::as_str),
            Some("priority")
        );
        assert_eq!(
            response
                .pointer("/thread/baseInstructions")
                .and_then(Value::as_str),
            Some("base update")
        );
        assert_eq!(
            response
                .pointer("/thread/developerInstructions")
                .and_then(Value::as_str),
            Some("developer update\n\nadditional update")
        );
        assert_eq!(
            response
                .pointer("/thread/personality")
                .and_then(Value::as_str),
            Some("concise")
        );
        assert_eq!(
            response
                .pointer("/thread/persistExtendedHistory")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            response
                .pointer("/thread/gitInfo/branch")
                .and_then(Value::as_str),
            Some("manual-branch")
        );
        assert_eq!(
            response
                .pointer("/thread/gitInfo/originUrl")
                .and_then(Value::as_str),
            Some("https://example.test/manual.git")
        );
        let notification = notification.expect("snapshot notification");
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/title")
                .and_then(Value::as_str),
            Some("Manual thread title")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/latestModel")
                .and_then(Value::as_str),
            Some("opus")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/cwd")
                .and_then(Value::as_str),
            Some(new_cwd.to_string_lossy().as_ref())
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/workspaceKind")
                .and_then(Value::as_str),
            Some("projectless")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/workspaceBrowserRoot")
                .and_then(Value::as_str),
            Some(workspace_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/latestReasoningEffort")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/latestCollaborationMode/settings/model")
                .and_then(Value::as_str),
            Some("opus")
        );
        assert_eq!(
            notification
                .pointer(
                    "/params/change/conversationState/latestCollaborationMode/settings/reasoning_effort"
                )
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/baseInstructions")
                .and_then(Value::as_str),
            Some("base update")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/developerInstructions")
                .and_then(Value::as_str),
            Some("developer update\n\nadditional update")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/personality")
                .and_then(Value::as_str),
            Some("concise")
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/persistExtendedHistory")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            notification
                .pointer("/params/change/conversationState/gitInfo/branch")
                .and_then(Value::as_str),
            Some("manual-branch")
        );
        assert!(state.threads.get(&thread_id).expect("thread").archived);

        let read = state
            .thread_read(&json!({ "threadId": thread_id }))
            .expect("thread read");
        assert_eq!(
            read.pointer("/thread/name").and_then(Value::as_str),
            Some("Manual thread title")
        );
        assert_eq!(
            read.pointer("/thread/workspaceRoots/0")
                .and_then(Value::as_str),
            Some(workspace_root.to_string_lossy().as_ref())
        );
        assert!(root
            .join(".claude")
            .join(CLAUDE_THREAD_NAMES_FILE)
            .is_file());

        let (clear_response, _) = state
            .thread_metadata_update(&json!({
                "threadId": read.pointer("/thread/id").and_then(Value::as_str),
                "name": Value::Null,
            }))
            .expect("clear metadata name");
        assert!(clear_response
            .pointer("/thread/name")
            .is_some_and(Value::is_null));

        let (clear_git_response, _) = state
            .thread_metadata_update(&json!({
                "threadId": read.pointer("/thread/id").and_then(Value::as_str),
                "gitInfo": Value::Null,
            }))
            .expect("clear metadata git info");
        assert!(clear_git_response
            .pointer("/thread/gitInfo")
            .is_some_and(Value::is_null));

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_goal_methods_store_clear_and_emit_snapshot() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("thread-goal-state");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let mut state = test_state(Some("workspace"));
        let (thread_response, _) = state.start_thread(&json!({ "cwd": root }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();

        let (set_response, notification) = state
            .thread_goal_set(&json!({
                "threadId": thread_id,
                "goal": {
                    "objective": "finish claude-code adapter",
                    "status": "active",
                },
            }))
            .expect("set goal");
        assert_eq!(
            set_response
                .pointer("/goal/objective")
                .and_then(Value::as_str),
            Some("finish claude-code adapter")
        );
        assert_eq!(
            notification
                .as_ref()
                .and_then(
                    |value| value.pointer("/params/change/conversationState/threadGoal/objective")
                )
                .and_then(Value::as_str),
            Some("finish claude-code adapter")
        );
        assert_eq!(
            state
                .thread_goal_get(&json!({ "threadId": thread_id }))
                .expect("get goal")
                .pointer("/goal/status")
                .and_then(Value::as_str),
            Some("active")
        );
        assert!(root
            .join(".claude")
            .join(CLAUDE_THREAD_GOALS_FILE)
            .is_file());

        let (clear_response, _) = state
            .thread_goal_clear(&json!({ "threadId": thread_id }))
            .expect("clear goal");
        assert!(clear_response.get("goal").is_some_and(Value::is_null));
        assert!(state
            .thread_goal_get(&json!({ "threadId": thread_id }))
            .expect("get cleared goal")
            .get("goal")
            .is_some_and(Value::is_null));

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_pinned_memory_prewarm_and_steer_adapters() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("thread-pinned-memory");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);

        let mut state = test_state(Some("workspace"));
        let (prewarm_response, _) =
            state.prewarm_thread(&json!({ "cwd": root, "pinned": true, "memoryMode": "off" }));
        let thread_id = prewarm_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        assert_eq!(
            prewarm_response.get("prewarmed").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            prewarm_response
                .pointer("/thread/pinned")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            prewarm_response
                .pointer("/thread/memoryMode")
                .and_then(Value::as_str),
            Some("off")
        );

        let list = state.thread_pinned_list();
        assert!(list
            .get("threadIds")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&thread_id))));

        state
            .thread_pinned_set(&json!({ "threadId": thread_id }), false)
            .expect("unpin");
        assert_eq!(
            state
                .thread_read(&json!({ "threadId": thread_id }))
                .expect("thread read")
                .pointer("/thread/pinned")
                .and_then(Value::as_bool),
            Some(false)
        );

        state
            .thread_memory_mode_set(&json!({ "threadId": thread_id, "memoryMode": "auto" }))
            .expect("set memory mode");
        assert_eq!(
            state
                .thread_memory_mode_get(&json!({ "threadId": thread_id }))
                .expect("get memory mode")
                .get("memoryMode")
                .and_then(Value::as_str),
            Some("auto")
        );
        let err = state
            .steer_turn(&json!({ "threadId": thread_id, "text": "continue" }))
            .expect_err("inactive steer fails");
        assert!(err.contains("SteerTurnInactiveError"));

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_write_updates_later_config_read() {
        let mut state = test_state(None);

        let write = state.config_write(
            "config/value/write",
            &json!({ "key": "model", "value": "sonnet" }),
        );
        assert_eq!(write.get("status").and_then(Value::as_str), Some("ok"));
        state.config_write(
            "config/batchWrite",
            &json!({
                "edits": [
                    { "key": "projects.demo.trust_level", "value": "trusted" },
                    { "keyPath": "profiles.claude-code.model", "value": "opus" },
                    { "key_path": "profiles.claude-code.model_reasoning_effort", "value": "high" }
                ],
                "values": {
                    "approval_policy": "on-request"
                }
            }),
        );

        let read = state.config_read(&json!({ "includeLayers": true }));
        assert_eq!(
            read.pointer("/config/model").and_then(Value::as_str),
            Some("sonnet")
        );
        assert_eq!(
            read.pointer("/config/approval_policy")
                .and_then(Value::as_str),
            Some("on-request")
        );
        assert_eq!(
            read.pointer("/config/projects/demo/trust_level")
                .and_then(Value::as_str),
            Some("trusted")
        );
        assert_eq!(
            read.pointer("/config/profiles/claude-code/model")
                .and_then(Value::as_str),
            Some("opus")
        );
        assert_eq!(
            read.pointer("/config/profiles/claude-code/model_reasoning_effort")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(read.get("layers"), Some(&json!([])));
    }

    #[test]
    fn model_list_exposes_claude_code_model_aliases() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_model = std::env::var_os(MODEL_ENV);
        std::env::set_var(MODEL_ENV, "sonnet");

        let result =
            standalone_codex_app_result("model/list", &json!({ "limit": 2 })).expect("model list");
        let models = result
            .get("data")
            .and_then(Value::as_array)
            .expect("models");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].get("id").and_then(Value::as_str), Some("sonnet"));
        assert_eq!(
            models[0].get("isDefault").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            models[0]
                .get("inputModalities")
                .and_then(Value::as_array)
                .and_then(|items| items.get(1))
                .and_then(Value::as_str),
            Some("image")
        );
        assert_eq!(result.get("nextCursor").and_then(Value::as_str), Some("2"));

        restore_env(MODEL_ENV, old_model);
    }

    #[test]
    fn collaboration_mode_list_exposes_plan_and_default_modes() {
        let result = standalone_codex_app_result("collaborationMode/list", &json!({}))
            .expect("collaboration mode list");
        let modes = result.get("data").and_then(Value::as_array).expect("modes");

        assert_eq!(
            modes
                .iter()
                .filter_map(|mode| mode.get("mode").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["plan", "default"]
        );
        assert_eq!(
            modes[0].get("model").and_then(Value::as_str),
            Some(DEFAULT_MODEL)
        );
        assert!(modes[0].get("reasoning_effort").is_some_and(Value::is_null));
    }

    #[test]
    fn thread_turns_items_list_returns_materialized_items() {
        let mut state = test_state(None);
        let (thread_response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "run a tool" }],
            }))
            .expect("start turn");
        state
            .finish_turn(
                &work.thread_id,
                &work.turn_id,
                ClaudeRunResult {
                    text: "done".to_string(),
                    error: None,
                    duration_ms: 5,
                    tool_items: vec![json!({
                        "type": "mcpToolCall",
                        "id": "tool-1",
                        "server": "claude-code",
                        "tool": "Read",
                        "status": "completed",
                    })],
                    agent_item_streamed: false,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish turn");

        let response = state
            .thread_turns_items_list(&json!({
                "threadId": work.thread_id,
                "turnId": work.turn_id,
                "sortDirection": "asc",
            }))
            .expect("items list");
        let item_types = response
            .get("data")
            .and_then(Value::as_array)
            .expect("items")
            .iter()
            .filter_map(|item| item.get("type").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            item_types,
            vec!["userMessage", "mcpToolCall", "agentMessage"]
        );
    }

    #[test]
    fn subagent_receiver_thread_id_returns_virtual_thread() {
        let mut state = test_state(Some("workspace"));
        let (thread_response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "spawn a subagent" }],
            }))
            .expect("start turn");
        let receiver_thread_id = "claude-subagent-call_e375199e78234d0eb05ed702";
        state
            .finish_turn(
                &work.thread_id,
                &work.turn_id,
                ClaudeRunResult {
                    text: "parent done".to_string(),
                    error: None,
                    duration_ms: 5,
                    tool_items: vec![json!({
                        "type": "collabAgentToolCall",
                        "id": "tool-agent",
                        "tool": "spawnAgent",
                        "status": "completed",
                        "senderThreadId": work.thread_id,
                        "receiverThreadIds": [receiver_thread_id],
                        "receiverThreads": [
                            {
                                "threadId": receiver_thread_id,
                                "thread": Value::Null,
                            }
                        ],
                        "prompt": "Inspect the project structure",
                        "model": Value::Null,
                        "reasoningEffort": Value::Null,
                        "agentsStates": {
                            receiver_thread_id: { "status": "completed" }
                        },
                        "result": "subagent done",
                        "error": Value::Null,
                    })],
                    agent_item_streamed: false,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish turn");

        let read = state
            .thread_read(&json!({
                "threadId": receiver_thread_id,
                "includeTurns": true,
            }))
            .expect("subagent thread read");
        assert_eq!(
            read.pointer("/thread/id").and_then(Value::as_str),
            Some(receiver_thread_id)
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("Inspect the project structure")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/1/text")
                .and_then(Value::as_str),
            Some("subagent done")
        );

        let turns = state
            .thread_turns_list(&json!({
                "threadId": receiver_thread_id,
                "sortDirection": "asc",
            }))
            .expect("subagent turns list");
        assert_eq!(
            turns
                .pointer("/data/0/items/1/text")
                .and_then(Value::as_str),
            Some("subagent done")
        );

        let items = state
            .thread_turns_items_list(&json!({
                "threadId": receiver_thread_id,
                "sortDirection": "asc",
            }))
            .expect("subagent items list");
        assert_eq!(
            items.pointer("/data/1/text").and_then(Value::as_str),
            Some("subagent done")
        );

        let (resumed, _) = state
            .resume_thread(&json!({
                "threadId": receiver_thread_id,
                "excludeTurns": false,
            }))
            .expect("subagent resume");
        assert_eq!(
            resumed.pointer("/thread/id").and_then(Value::as_str),
            Some(receiver_thread_id)
        );
    }

    #[test]
    fn subagent_receiver_thread_loads_sidechain_transcript_history() {
        let _guard = ENV_TEST_LOCK.lock().expect("env lock poisoned");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("subagent-sidechain");
        let cwd = root.join("workspace");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::env::set_var("HOME", &root);

        let mut state = test_state(Some("workspace"));
        let (thread_response, _) = state.start_thread(&json!({ "cwd": cwd.to_string_lossy() }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "spawn a subagent" }],
            }))
            .expect("start turn");

        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&cwd));
        std::fs::create_dir_all(&projects_dir).expect("create projects dir");
        let transcript_path = projects_dir.join(format!("{}.jsonl", work.claude_session_id));
        let transcript_lines = [
            json!({
                "type": "user",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:00Z",
                "uuid": "parent-user",
                "isSidechain": false,
                "message": { "role": "user", "content": "spawn a subagent" }
            }),
            json!({
                "type": "assistant",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:01Z",
                "uuid": "parent-assistant",
                "parentUuid": "parent-user",
                "isSidechain": false,
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet-4-20250514",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_agent",
                        "name": "Task",
                        "input": {
                            "description": "Explore repo",
                            "prompt": "Inspect the project structure",
                            "subagent_type": "Explore"
                        }
                    }]
                }
            }),
            json!({
                "type": "user",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:02Z",
                "uuid": "side-user",
                "parentUuid": "parent-assistant",
                "isSidechain": true,
                "message": { "role": "user", "content": "Inspect the project structure" }
            }),
            json!({
                "type": "assistant",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:03Z",
                "uuid": "side-assistant-tool",
                "parentUuid": "side-user",
                "isSidechain": true,
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet-4-20250514",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "Read",
                        "input": { "file_path": "/tmp/README.md" }
                    }]
                }
            }),
            json!({
                "type": "user",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:04Z",
                "uuid": "side-tool-result",
                "parentUuid": "side-assistant-tool",
                "isSidechain": true,
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_read",
                        "content": "README contents"
                    }]
                }
            }),
            json!({
                "type": "assistant",
                "sessionId": work.claude_session_id,
                "cwd": cwd,
                "timestamp": "2026-01-01T00:00:05Z",
                "uuid": "side-final",
                "parentUuid": "side-tool-result",
                "isSidechain": true,
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet-4-20250514",
                    "content": [{ "type": "text", "text": "sidechain done" }]
                }
            }),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&transcript_path, transcript_lines).expect("write transcript");

        let receiver_thread_id = "claude-subagent-toolu_agent";
        state
            .finish_turn(
                &work.thread_id,
                &work.turn_id,
                ClaudeRunResult {
                    text: "parent done".to_string(),
                    error: None,
                    duration_ms: 5,
                    tool_items: vec![json!({
                        "type": "collabAgentToolCall",
                        "id": "claude-tool-toolu_agent",
                        "tool": "spawnAgent",
                        "status": "completed",
                        "senderThreadId": work.thread_id,
                        "receiverThreadIds": [receiver_thread_id],
                        "receiverThreads": [{ "threadId": receiver_thread_id, "thread": Value::Null }],
                        "prompt": "Inspect the project structure",
                        "model": Value::Null,
                        "reasoningEffort": Value::Null,
                        "agentsStates": { receiver_thread_id: { "status": "completed" } },
                        "result": "parent summary",
                        "error": Value::Null,
                    })],
                    agent_item_streamed: false,
                    latest_token_usage_info: None,
                },
                None,
            )
            .expect("finish turn");

        let read = state
            .thread_read(&json!({
                "threadId": receiver_thread_id,
                "includeTurns": true,
            }))
            .expect("subagent thread read");
        assert_eq!(
            read.pointer("/thread/turns/0/items/0/content/0/text")
                .and_then(Value::as_str),
            Some("Inspect the project structure")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/1/type")
                .and_then(Value::as_str),
            Some("mcpToolCall")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/1/tool")
                .and_then(Value::as_str),
            Some("Read")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/1/result/content/0/text")
                .and_then(Value::as_str),
            Some("README contents")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/2/text")
                .and_then(Value::as_str),
            Some("sidechain done")
        );
        assert_ne!(
            read.pointer("/thread/turns/0/items/2/text")
                .and_then(Value::as_str),
            Some("parent summary")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn subagent_live_stream_finishes_receiver_thread_and_broadcasts_snapshot() {
        let mut initial_state = test_state(None);
        let (thread_response, _) = initial_state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, _) = initial_state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "spawn a subagent" }],
            }))
            .expect("start turn");
        let state = Arc::new(Mutex::new(initial_state));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut child_stdin = Vec::<u8>::new();
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();

        handle_claude_stdout_line(
            &json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_agent",
                        "name": "Task",
                        "input": {
                            "description": "Explore repo",
                            "prompt": "Inspect the project structure"
                        }
                    }
                }
            })
            .to_string(),
            &work,
            &state,
            &output,
            &mut child_stdin,
            &mut stream,
            &mut command_output,
        )
        .expect("handle agent tool");
        handle_claude_stdout_line(
            &json!({
                "type": "stream_event",
                "parent_tool_use_id": "toolu_agent",
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "subagent live" }
                }
            })
            .to_string(),
            &work,
            &state,
            &output,
            &mut child_stdin,
            &mut stream,
            &mut command_output,
        )
        .expect("handle subagent text");

        let receiver_thread_id = "claude-subagent-toolu_agent";
        let read = state
            .lock()
            .expect("state lock")
            .thread_read(&json!({
                "threadId": receiver_thread_id,
                "includeTurns": true,
            }))
            .expect("subagent thread read");
        assert_eq!(
            read.pointer("/thread/status/type").and_then(Value::as_str),
            Some("active")
        );
        assert_eq!(
            read.pointer("/thread/turns/0/items/1/text")
                .and_then(Value::as_str),
            Some("subagent live")
        );

        let finish_notifications = {
            let mut state = state.lock().expect("state lock");
            state
                .finish_turn(
                    &work.thread_id,
                    &work.turn_id,
                    ClaudeRunResult {
                        text: "parent done".to_string(),
                        error: None,
                        duration_ms: 10,
                        tool_items: vec![json!({
                            "type": "collabAgentToolCall",
                            "id": "claude-tool-toolu_agent",
                            "tool": "spawnAgent",
                            "status": "completed",
                            "senderThreadId": work.thread_id,
                            "receiverThreadIds": [receiver_thread_id],
                            "receiverThreads": [{ "threadId": receiver_thread_id, "thread": Value::Null }],
                            "prompt": "Inspect the project structure",
                            "model": Value::Null,
                            "reasoningEffort": Value::Null,
                            "agentsStates": {
                                receiver_thread_id: { "status": "completed" }
                            },
                            "result": "subagent done",
                            "error": Value::Null,
                        })],
                        agent_item_streamed: false,
                        latest_token_usage_info: None,
                    },
                    None,
                )
                .expect("finish parent turn")
        };
        let completed_read = state
            .lock()
            .expect("state lock")
            .thread_read(&json!({
                "threadId": receiver_thread_id,
                "includeTurns": true,
            }))
            .expect("completed subagent thread read");
        assert_eq!(
            completed_read
                .pointer("/thread/status/type")
                .and_then(Value::as_str),
            Some("idle")
        );
        assert_eq!(
            completed_read
                .pointer("/thread/turns/0/status")
                .and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            completed_read
                .pointer("/thread/turns/0/items/1/text")
                .and_then(Value::as_str),
            Some("subagent live\n\nsubagent done")
        );
        assert!(
            finish_notifications
                .extra_notifications
                .iter()
                .any(|message| {
                    message.get("method").and_then(Value::as_str)
                        == Some("thread-stream-state-changed")
                        && message
                            .pointer("/params/conversationId")
                            .and_then(Value::as_str)
                            == Some(receiver_thread_id)
                        && message
                            .pointer("/params/change/conversationState/threadRuntimeStatus/type")
                            .and_then(Value::as_str)
                            == Some("idle")
                        && message
                            .pointer("/params/change/conversationState/turns/0/status")
                            .and_then(Value::as_str)
                            == Some("completed")
                        && message
                            .pointer("/params/change/conversationState/turns/0/items/1/text")
                            .and_then(Value::as_str)
                            == Some("subagent live\n\nsubagent done")
                }),
            "{:#?}",
            finish_notifications.extra_notifications
        );
    }

    #[test]
    fn unknown_subagent_receiver_thread_id_uses_empty_virtual_thread() {
        let state = test_state(None);
        let response = state
            .thread_read(&json!({
                "threadId": "claude-subagent-call_missing",
                "includeTurns": true,
            }))
            .expect("unknown subagent thread read");
        assert_eq!(
            response.pointer("/thread/id").and_then(Value::as_str),
            Some("claude-subagent-call_missing")
        );
        assert!(response
            .pointer("/thread/turns")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        assert!(state
            .thread_read(&json!({ "threadId": "missing-real-thread" }))
            .is_err());
    }

    #[test]
    fn start_turn_appends_attachments_to_prompt_and_history() {
        let mut state = test_state(None);
        let (thread_response, _) = state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();

        let (_, _, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "inspect this" }],
                "attachments": [
                    { "name": "report", "path": "/tmp/report.txt", "mimeType": "text/plain" }
                ],
                "commentAttachments": [
                    { "url": "https://example.test/design.png", "type": "image" }
                ]
            }))
            .expect("start turn with attachments");

        assert!(work.prompt.contains("Attached context:"), "{}", work.prompt);
        assert!(work.prompt.contains("/tmp/report.txt"), "{}", work.prompt);
        assert!(
            work.prompt.contains("https://example.test/design.png"),
            "{}",
            work.prompt
        );
        let thread = state.threads.get(&work.thread_id).expect("thread");
        let turn_input = thread
            .turns
            .first()
            .expect("turn")
            .input
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(turn_input.contains("Attached context:"), "{turn_input}");
    }

    #[test]
    fn start_turn_injects_thread_instructions_without_polluting_visible_input() {
        let mut state = test_state(None);
        let (thread_response, _) = state.start_thread(&json!({
            "cwd": "/tmp",
            "baseInstructions": "base policy",
            "developerInstructions": "developer policy",
            "additionalDeveloperInstructions": "additional policy",
            "personality": "pragmatic",
            "persistExtendedHistory": true,
        }));
        assert_eq!(
            thread_response
                .pointer("/baseInstructions")
                .and_then(Value::as_str),
            Some("base policy")
        );
        assert_eq!(
            thread_response
                .pointer("/developerInstructions")
                .and_then(Value::as_str),
            Some("developer policy\n\nadditional policy")
        );
        assert_eq!(
            thread_response
                .pointer("/thread/personality")
                .and_then(Value::as_str),
            Some("pragmatic")
        );
        assert_eq!(
            thread_response
                .pointer("/persistExtendedHistory")
                .and_then(Value::as_bool),
            Some(true)
        );
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();

        let (_, notifications, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "visible user request" }],
            }))
            .expect("start turn");

        let thread = state.threads.get(&work.thread_id).expect("thread");
        let turn = thread.turns.first().expect("turn");
        assert_eq!(
            turn.input
                .first()
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str),
            Some("visible user request")
        );
        assert!(!format!("{:?}", turn.input).contains("base policy"));
        assert!(!format!("{:?}", turn.input).contains("developer policy"));

        let input = claude_stream_json_input(&work);
        let lines = input
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
            .collect::<Vec<_>>();
        let content = lines[1]
            .pointer("/message/content")
            .and_then(Value::as_array)
            .expect("content");
        let instruction_text = content[0]
            .get("text")
            .and_then(Value::as_str)
            .expect("instruction text");
        assert!(instruction_text.contains("Base instructions:\nbase policy"));
        assert!(instruction_text.contains("Developer instructions:\ndeveloper policy"));
        assert!(instruction_text.contains("additional policy"));
        assert_eq!(
            content[1].get("text").and_then(Value::as_str),
            Some("visible user request")
        );

        let snapshot = notifications
            .iter()
            .find(|notification| {
                notification.get("method").and_then(Value::as_str)
                    == Some("thread-stream-state-changed")
            })
            .expect("snapshot");
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/baseInstructions")
                .and_then(Value::as_str),
            Some("base policy")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/turns/0/params/developerInstructions")
                .and_then(Value::as_str),
            Some("developer policy\n\nadditional policy")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/personality")
                .and_then(Value::as_str),
            Some("pragmatic")
        );
        assert_eq!(
            snapshot
                .pointer("/params/change/conversationState/persistExtendedHistory")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn new_frontend_init_methods_have_standalone_results() {
        let root = test_dir("new-frontend-init-methods");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let config_path = root.join("config.toml");
        let read_path = root.join("AGENTS.md");
        std::fs::write(&read_path, "hello").expect("write readable file");

        let detect = standalone_codex_app_result("externalAgentConfig/detect", &json!({}))
            .expect("external agent detect result");
        assert!(detect
            .get("items")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        let import = standalone_codex_app_result(
            "externalAgentConfig/import",
            &json!({ "migrationItems": [] }),
        )
        .expect("external agent import result");
        assert!(import.as_object().is_some_and(Map::is_empty));

        let write = standalone_codex_app_result(
            "config/batchWrite",
            &json!({ "edits": [], "filePath": config_path.to_string_lossy() }),
        )
        .expect("config batch write result");
        assert_eq!(write.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(
            write.get("filePath").and_then(Value::as_str),
            Some(config_path.to_string_lossy().as_ref())
        );
        assert!(write.get("overriddenMetadata").is_some_and(Value::is_null));

        let read = standalone_codex_app_result(
            "fs/readFile",
            &json!({ "path": read_path.to_string_lossy() }),
        )
        .expect("fs read file result");
        let expected_data = general_purpose::STANDARD.encode("hello");
        assert_eq!(
            read.get("dataBase64").and_then(Value::as_str),
            Some(expected_data.as_str())
        );

        let _ = std::fs::remove_dir_all(root);
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
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            json!({"id":"1","method":"initialize","params":{}}),
            json!({"id":"2","method":"thread/list","params":{"limit":10,"sourceKinds":["cli"],"modelProviders":[PROVIDER_NAME]}}),
            json!({"id":"3","method":"thread/read","params":{"threadId":session_id,"includeTurns":true}}),
            json!({"id":"4","method":"thread/turns/list","params":{"threadId":session_id,"limit":1,"sortDirection":"desc"}}),
            json!({"id":"5","method":"thread/resume","params":{"threadId":session_id}}),
            json!({"id":"6","method":"thread/resume","params":{"threadId":session_id,"excludeTurns":true}}),
            json!({"id":"7","method":"thread/read","params":{"threadId":format!("local:{session_id}"),"includeTurns":true}}),
            json!({"id":"8","method":"thread/search","params":{"query":"transcript","limit":10}}),
            json!({"id":"9","method":"turn/list","params":{"conversationId":session_id,"limit":1,"sortDirection":"desc"}})
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

        let searched = response_by_id(&responses, "8")
            .pointer("/result/data/0")
            .expect("searched thread");
        assert_eq!(searched.get("id").and_then(Value::as_str), Some(session_id));

        let turn_alias = response_by_id(&responses, "9")
            .pointer("/result/data/0")
            .expect("turn/list alias");
        assert_eq!(
            turn_alias.pointer("/items/1/text").and_then(Value::as_str),
            Some("hello from claude")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_list_matches_claude_resume_filters_and_uses_prompt_title() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-resume-metadata");
        let cwd = root.join("workspace");
        let main_session_id = "11111111-1111-4111-8111-333333333333";
        let subagent_session_id = "22222222-2222-4222-8222-333333333333";
        let daemon_session_id = "33333333-3333-4333-8333-333333333333";
        let entrypoint_session_id = "44444444-4444-4444-8444-333333333333";
        let loop_session_id = "55555555-5555-4555-8555-333333333333";
        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&cwd));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::write(
            projects_dir.join(format!("{main_session_id}.jsonl")),
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": main_session_id,
                    "entrypoint": "cli",
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": {
                        "role": "user",
                        "content": "Build the main feature"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": main_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "done" }]
                    }
                })
            ),
        )
        .expect("write main transcript");
        std::fs::write(
            projects_dir.join(format!("{subagent_session_id}.jsonl")),
            format!(
                "{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": subagent_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:02.000Z",
                    "isSidechain": true,
                    "parentUuid": "parent-assistant",
                    "message": {
                        "role": "user",
                        "content": "Inspect files for the parent agent"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": subagent_session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:03.000Z",
                    "isSidechain": true,
                    "parentUuid": "side-user",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "subagent done" }]
                    }
                })
            ),
        )
        .expect("write subagent transcript");
        std::fs::write(
            projects_dir.join(format!("{daemon_session_id}.jsonl")),
            format!(
                "{}\n",
                json!({
                    "type": "user",
                    "sessionId": daemon_session_id,
                    "sessionKind": "daemon",
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:04.000Z",
                    "message": { "role": "user", "content": "daemon task" }
                })
            ),
        )
        .expect("write daemon transcript");
        std::fs::write(
            projects_dir.join(format!("{entrypoint_session_id}.jsonl")),
            format!(
                "{}\n",
                json!({
                    "type": "user",
                    "sessionId": entrypoint_session_id,
                    "entrypoint": "command-name/loop",
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:05.000Z",
                    "message": { "role": "user", "content": "entrypoint task" }
                })
            ),
        )
        .expect("write entrypoint transcript");
        std::fs::write(
            projects_dir.join(format!("{loop_session_id}.jsonl")),
            format!(
                "{}\n",
                json!({
                    "type": "user",
                    "sessionId": loop_session_id,
                    "isLoopSession": true,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:06.000Z",
                    "message": { "role": "user", "content": "loop task" }
                })
            ),
        )
        .expect("write loop transcript");
        std::env::set_var("HOME", &root);
        clear_claude_thread_list_cache();

        let state = test_state(None);
        let listed = state.thread_list(&json!({ "limit": 10 }));
        let listed_threads = listed
            .get("data")
            .and_then(Value::as_array)
            .expect("listed threads");
        assert_eq!(listed_threads.len(), 1, "{listed_threads:#?}");
        assert_eq!(
            listed_threads[0].get("id").and_then(Value::as_str),
            Some(main_session_id)
        );
        assert_eq!(
            listed_threads[0].get("name").and_then(Value::as_str),
            Some("Build the main feature")
        );
        assert_eq!(
            listed_threads[0].get("preview").and_then(Value::as_str),
            Some("Build the main feature")
        );
        for hidden_id in [
            subagent_session_id,
            daemon_session_id,
            entrypoint_session_id,
            loop_session_id,
        ] {
            assert!(
                load_claude_thread_by_id(hidden_id, None).is_none(),
                "{hidden_id} should follow Claude /resume hidden-session filters"
            );
        }

        restore_env("HOME", old_home);
        clear_claude_thread_list_cache();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claude_transcript_restores_tool_items_from_tool_use_results() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-transcript-tool-items");
        let cwd = root.join("workspace");
        let session_id = "22222222-2222-4222-8222-333333333333";
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
                "{}\n{}\n{}\n{}\n",
                json!({
                    "type": "user",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:00.000Z",
                    "message": {
                        "role": "user",
                        "content": "list files"
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "uuid": "assistant-tool-message",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [
                            { "type": "thinking", "thinking": "I should inspect the directory." },
                            {
                                "type": "tool_use",
                                "id": "toolu_bash",
                                "name": "Bash",
                                "input": { "command": "ls -la" }
                            }
                        ]
                    }
                }),
                json!({
                    "type": "user",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:02.000Z",
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "toolu_bash",
                            "content": "total 8\n-rw-r--r-- README.md\n"
                        }]
                    }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:03.000Z",
                    "uuid": "assistant-final-message",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "done" }]
                    }
                })
            ),
        )
        .expect("write transcript");
        std::env::set_var("HOME", &root);

        let thread =
            load_claude_thread_from_transcript_path(&transcript_path, None).expect("thread");
        assert_eq!(thread.turns.len(), 1);
        let items = thread.turns[0]
            .items_json()
            .as_array()
            .cloned()
            .expect("turn items");
        assert_eq!(
            items
                .iter()
                .filter_map(|item| item.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec![
                "userMessage",
                "reasoning",
                "commandExecution",
                "agentMessage"
            ]
        );
        assert_eq!(items[1].get("command").and_then(Value::as_str), None);
        assert_eq!(
            items[1]
                .get("content")
                .and_then(Value::as_array)
                .and_then(|content| content.first())
                .and_then(Value::as_str),
            Some("I should inspect the directory.")
        );
        assert_eq!(
            items[2].get("command").and_then(Value::as_str),
            Some("ls -la")
        );
        assert_eq!(
            items[2].get("status").and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            items[2].get("aggregatedOutput").and_then(Value::as_str),
            Some("total 8\n-rw-r--r-- README.md\n")
        );
        assert_eq!(
            items[2].get("durationMs").and_then(Value::as_i64),
            Some(1000)
        );
        assert_eq!(items[3].get("text").and_then(Value::as_str), Some("done"));

        let state = test_state(None);
        let listed_items = state
            .thread_turns_items_list(&json!({
                "threadId": session_id,
                "sortDirection": "asc",
            }))
            .expect("items list");
        assert_eq!(
            listed_items.pointer("/data/1/type").and_then(Value::as_str),
            Some("reasoning")
        );
        assert_eq!(
            listed_items.pointer("/data/2/type").and_then(Value::as_str),
            Some("commandExecution")
        );
        assert_eq!(
            listed_items
                .pointer("/data/2/aggregatedOutput")
                .and_then(Value::as_str),
            Some("total 8\n-rw-r--r-- README.md\n")
        );

        restore_env("HOME", old_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn archive_persists_for_unloaded_claude_transcript_threads() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-archive-persist");
        let cwd = root.join("workspace");
        let session_id = "33333333-3333-4333-8333-444444444444";
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
                    "message": { "role": "user", "content": "archive me" }
                }),
                json!({
                    "type": "assistant",
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "timestamp": "2026-05-25T07:00:01.000Z",
                    "message": {
                        "role": "assistant",
                        "model": "opus",
                        "content": [{ "type": "text", "text": "ok" }]
                    }
                })
            ),
        )
        .expect("write transcript");
        std::env::set_var("HOME", &root);

        let mut state = test_state(None);
        assert!(state
            .set_archived(&json!({ "threadId": session_id }), true)
            .is_none());
        assert!(root
            .join(".claude")
            .join(CLAUDE_THREAD_ARCHIVED_FILE)
            .is_file());

        let active = state.thread_list(&json!({ "limit": 10, "archived": false }));
        assert!(active
            .get("data")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        let archived = state.thread_list(&json!({ "limit": 10, "archived": true }));
        assert_eq!(
            archived.pointer("/data/0/id").and_then(Value::as_str),
            Some(session_id)
        );

        assert!(state
            .set_archived(&json!({ "threadId": format!("local:{session_id}") }), false)
            .is_none());
        let active = state.thread_list(&json!({ "limit": 10, "archived": false }));
        assert_eq!(
            active.pointer("/data/0/id").and_then(Value::as_str),
            Some(session_id)
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
            config_values: Map::new(),
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
    fn thread_list_uses_tail_resume_title_over_live_uuid_name() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_home = std::env::var_os("HOME");
        let root = test_dir("claude-tail-title-live");
        std::fs::create_dir_all(&root).expect("create temp home");
        std::env::set_var("HOME", &root);
        clear_claude_thread_list_cache();

        let cwd = root.join("workspace").to_string_lossy().to_string();
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            config_values: Map::new(),
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
        state.threads.get_mut(&thread_id).expect("live thread").name = Some(thread_id.clone());

        let projects_dir = root
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(Path::new(&cwd)));
        std::fs::create_dir_all(&projects_dir).expect("create claude projects dir");
        let transcript_path = projects_dir.join(format!("{thread_id}.jsonl"));
        let mut transcript = format!(
            "{}\n",
            json!({
                "type": "user",
                "sessionId": thread_id,
                "cwd": cwd,
                "timestamp": "2026-05-25T07:00:00.000Z",
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "initial prompt title" }]
                }
            })
        );
        for index in 0..120 {
            transcript.push_str(
                &json!({
                    "type": "progress",
                    "sessionId": thread_id,
                    "timestamp": format!("2026-05-25T07:{:02}:00.000Z", index % 60),
                    "message": format!("filler {index}")
                })
                .to_string(),
            );
            transcript.push('\n');
        }
        transcript.push_str(
            &json!({
                "type": "custom-title",
                "sessionId": thread_id,
                "cwd": cwd,
                "timestamp": "2026-05-25T08:00:00.000Z",
                "customTitle": "Tail custom title"
            })
            .to_string(),
        );
        transcript.push('\n');
        std::fs::write(&transcript_path, transcript).expect("write transcript");

        let listed = state.thread_list(&json!({ "limit": 10 }));
        let listed_threads = listed
            .get("data")
            .and_then(Value::as_array)
            .expect("listed threads");
        assert_eq!(listed_threads.len(), 1, "{listed_threads:#?}");
        assert_eq!(
            listed_threads[0].get("name").and_then(Value::as_str),
            Some("Tail custom title")
        );

        restore_env("HOME", old_home);
        clear_claude_thread_list_cache();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thread_json_uses_display_title_when_name_is_uuid() {
        let root = test_dir("claude-display-title");
        let cwd = root.join("workspace").to_string_lossy().to_string();
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            config_values: Map::new(),
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
                "input": [{ "type": "text", "text": "Analyze Claude resume titles" }],
            }))
            .expect("start turn");

        let thread = state.threads.get_mut(&thread_id).expect("live thread");
        thread.name = Some(thread_id.clone());
        let thread_json = thread.to_json(false);
        assert_eq!(
            thread_json.get("name").and_then(Value::as_str),
            Some("Analyze Claude resume titles")
        );
        assert_eq!(
            thread_json.get("title").and_then(Value::as_str),
            Some("Analyze Claude resume titles")
        );
        assert_eq!(
            claude_conversation_state(thread)
                .get("title")
                .and_then(Value::as_str),
            Some("Analyze Claude resume titles")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claude_project_dir_name_matches_resume_truncation_rule() {
        assert_eq!(claude_project_dir_name(Path::new("/tmp/项目")), "-tmp---");
        let long_path = format!("/Users/jinhuilee/{}", "a".repeat(260));
        let expected_prefix = format!(
            "-Users-jinhuilee-{}",
            "a".repeat(CLAUDE_PROJECT_DIR_MAX_LEN - "-Users-jinhuilee-".len())
        );
        assert_eq!(
            claude_project_dir_name(Path::new(&long_path)),
            format!("{expected_prefix}-1gt486")
        );
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
        };

        let command = claude_command_display(&work);
        assert!(command.contains("--print"));
        assert!(command.contains("--output-format stream-json"));
        assert!(command.contains("--verbose"));
        assert!(command.contains("--input-format stream-json"));
        assert!(command.contains("--include-partial-messages"));
        assert!(command.contains("--session-id 11111111-1111-4111-8111-111111111111"));
    }

    #[test]
    fn auto_review_maps_to_claude_auto_permission_mode() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_permission_mode = std::env::var_os(PERMISSION_MODE_ENV);
        std::env::remove_var(PERMISSION_MODE_ENV);
        let mut state = ClaudeAppServerState {
            active_processes: BTreeMap::new(),
            app_responses: BTreeMap::new(),
            config_values: Map::new(),
            interrupted_turns: BTreeSet::new(),
            threads: BTreeMap::new(),
            workspace_name: None,
        };
        let (thread_response, _) = state.start_thread(&json!({
            "cwd": "/tmp",
            "approvalsReviewer": "auto_review",
        }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        assert_eq!(
            thread_response
                .get("approvalsReviewer")
                .and_then(Value::as_str),
            Some("auto_review")
        );

        let (turn_response, _, work, _) = state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }],
            }))
            .expect("start turn");

        assert_eq!(work.permission_mode.as_deref(), Some("auto"));
        assert_eq!(
            turn_response
                .pointer("/turn/status")
                .and_then(Value::as_str),
            Some("inProgress")
        );
        let thread = state.threads.get(&thread_id).expect("thread state");
        assert_eq!(
            thread
                .turns
                .first()
                .map(|turn| turn.approvals_reviewer.as_str()),
            Some("auto_review")
        );
        let command = claude_command_display(&work);
        assert!(command.contains("--permission-mode auto"), "{command}");

        restore_env(PERMISSION_MODE_ENV, old_permission_mode);
    }

    #[test]
    fn permission_mode_env_overrides_auto_review_mode() {
        let _env_lock = ENV_TEST_LOCK.lock().expect("env test lock");
        let old_permission_mode = std::env::var_os(PERMISSION_MODE_ENV);
        std::env::set_var(PERMISSION_MODE_ENV, "plan");
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "hello".to_string(),
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: Some("auto".to_string()),
        };

        let command = claude_command_display(&work);
        assert!(command.contains("--permission-mode plan"), "{command}");
        assert!(!command.contains("--permission-mode auto"), "{command}");

        restore_env(PERMISSION_MODE_ENV, old_permission_mode);
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            config_values: Map::new(),
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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

    #[test]
    fn claude_stream_json_input_preserves_image_blocks() {
        let root = test_dir("stream-json-image-input");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let image_path = root.join("sample.png");
        std::fs::write(&image_path, [1_u8, 2, 3]).expect("write image");
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "inspect images".to_string(),
            input: vec![
                json!({ "type": "text", "text": "inspect images" }),
                json!({ "type": "localImage", "path": image_path.to_string_lossy() }),
                json!({ "type": "image", "url": "https://example.test/image.png" }),
            ],
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
        };

        let input = claude_stream_json_input(&work);
        let lines = input
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
            .collect::<Vec<_>>();
        let content = lines[1]
            .pointer("/message/content")
            .and_then(Value::as_array)
            .expect("content");
        assert_eq!(content.len(), 3);
        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1].pointer("/source/type").and_then(Value::as_str),
            Some("base64")
        );
        assert_eq!(
            content[1]
                .pointer("/source/media_type")
                .and_then(Value::as_str),
            Some("image/png")
        );
        assert_eq!(
            content[1].pointer("/source/data").and_then(Value::as_str),
            Some("AQID")
        );
        assert_eq!(
            content[2].pointer("/source/type").and_then(Value::as_str),
            Some("url")
        );
        assert_eq!(
            content[2].pointer("/source/url").and_then(Value::as_str),
            Some("https://example.test/image.png")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claude_stream_json_input_keeps_prompt_fallback_with_instruction_context() {
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: "/tmp".to_string(),
            prompt: "visible prompt".to_string(),
            input: vec![json!({ "type": "unsupported" })],
            instruction_context: Some("hidden instructions".to_string()),
            resume_existing: false,
            permission_mode: None,
        };

        let input = claude_stream_json_input(&work);
        let lines = input
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
            .collect::<Vec<_>>();
        let content = lines[1]
            .pointer("/message/content")
            .and_then(Value::as_array)
            .expect("content");

        assert_eq!(
            content[0].get("text").and_then(Value::as_str),
            Some("hidden instructions")
        );
        assert_eq!(
            content[1].get("text").and_then(Value::as_str),
            Some("visible prompt")
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
            config_values: Map::new(),
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
            config_values: Map::new(),
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
        emit_reasoning_completed_if_started(&output, &work, &mut stream);

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
        assert_eq!(stream.completed_tool_items.len(), 2);
        assert_eq!(
            stream.completed_tool_items[1]
                .get("type")
                .and_then(Value::as_str),
            Some("reasoning")
        );
    }

    #[test]
    fn stdout_stream_updates_loaded_thread_snapshots_realtime() {
        let mut initial_state = test_state(None);
        let (thread_response, _) = initial_state.start_thread(&json!({ "cwd": "/tmp" }));
        let thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .expect("thread id")
            .to_string();
        let (_, _, work, _) = initial_state
            .start_turn(&json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "inspect" }],
            }))
            .expect("start turn");
        let state = Arc::new(Mutex::new(initial_state));
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let mut child_stdin = Vec::<u8>::new();
        let mut stream = ClaudeStreamState::default();
        let mut command_output = String::new();

        handle_claude_stdout_line(
            &json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": "Thinking live" }
                }
            })
            .to_string(),
            &work,
            &state,
            &output,
            &mut child_stdin,
            &mut stream,
            &mut command_output,
        )
        .expect("handle text delta");
        {
            let state = state.lock().expect("state lock");
            let turn = &state.threads.get(&work.thread_id).expect("thread").turns[0];
            assert_eq!(turn.agent_text, "Thinking live");
        }

        handle_claude_stdout_line(
            &json!({
                "type": "stream_event",
                "parent_tool_use_id": Value::Null,
                "event": {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "Read",
                        "input": { "file_path": "/tmp/README.md" }
                    }
                }
            })
            .to_string(),
            &work,
            &state,
            &output,
            &mut child_stdin,
            &mut stream,
            &mut command_output,
        )
        .expect("handle tool use");
        {
            let state = state.lock().expect("state lock");
            let turn = &state.threads.get(&work.thread_id).expect("thread").turns[0];
            assert_eq!(turn.agent_text, "");
            let item_types = turn
                .tool_items
                .iter()
                .filter_map(|item| item.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>();
            assert_eq!(item_types, vec!["reasoning", "mcpToolCall"]);
            assert_eq!(
                turn.tool_items[0]
                    .get("content")
                    .and_then(Value::as_array)
                    .and_then(|content| content.first())
                    .and_then(Value::as_str),
                Some("Thinking live")
            );
            assert_eq!(
                turn.tool_items[1].get("status").and_then(Value::as_str),
                Some("inProgress")
            );
        }

        let output =
            String::from_utf8(output.lock().expect("output lock").clone()).expect("utf8 output");
        let snapshots = json_lines(&output)
            .into_iter()
            .filter(|message| {
                message.get("method").and_then(Value::as_str) == Some("thread-stream-state-changed")
            })
            .collect::<Vec<_>>();
        assert!(snapshots.len() >= 2, "{output}");
        assert!(snapshots.iter().any(|snapshot| {
            snapshot
                .pointer("/params/change/conversationState/turns/0/items")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("type").and_then(Value::as_str) == Some("agentMessage")
                            && item.get("text").and_then(Value::as_str) == Some("Thinking live")
                    })
                })
        }));
        assert!(snapshots.iter().any(|snapshot| {
            snapshot
                .pointer("/params/change/conversationState/turns/0/items")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item.get("type").and_then(Value::as_str) == Some("mcpToolCall")
                            && item.get("status").and_then(Value::as_str) == Some("inProgress")
                    })
                })
        }));
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
    fn maps_edit_tool_to_file_change_item() {
        let root = test_dir("edit-tool-file-change");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let file_path = root.join("notes.txt");
        std::fs::write(&file_path, "alpha\nold line\nomega\n").expect("write file");
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let work = TurnWork {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            agent_item_id: "agent".to_string(),
            cli_item_id: "cli".to_string(),
            claude_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            cwd: root.to_string_lossy().to_string(),
            prompt: "edit a file".to_string(),
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
                        "id": "toolu_edit",
                        "name": "Edit",
                        "input": {
                            "file_path": "notes.txt",
                            "old_string": "old line",
                            "new_string": "new line"
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
                        "tool_use_id": "toolu_edit",
                        "content": "Updated notes.txt"
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
        assert!(output.contains(r#""type":"fileChange""#));
        assert!(!output.contains(r#""type":"mcpToolCall""#));
        let completed = json_lines(&output)
            .into_iter()
            .find(|value| {
                value.get("method").and_then(Value::as_str) == Some("item/completed")
                    && value.pointer("/params/item/type").and_then(Value::as_str)
                        == Some("fileChange")
            })
            .expect("file change completed notification");
        assert_eq!(
            completed
                .pointer("/params/item/changes/0/path")
                .and_then(Value::as_str),
            Some("notes.txt")
        );
        assert_eq!(
            completed
                .pointer("/params/item/changes/0/kind/type")
                .and_then(Value::as_str),
            Some("update")
        );
        let diff = completed
            .pointer("/params/item/changes/0/diff")
            .and_then(Value::as_str)
            .expect("diff");
        assert!(diff.contains("@@ -2,1 +2,1 @@"), "{diff}");
        assert!(diff.contains("-old line"), "{diff}");
        assert!(diff.contains("+new line"), "{diff}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn maps_write_and_multiedit_tools_to_file_change_items() {
        let root = test_dir("write-multiedit-file-change");
        std::fs::create_dir_all(&root).expect("create temp dir");
        std::fs::write(root.join("notes.txt"), "alpha\nbeta\n").expect("write fixture");
        let cwd = root.to_string_lossy().to_string();

        let write_state = ClaudeToolCallState {
            name: "Write".to_string(),
            arguments: json!({
                "file_path": "new.txt",
                "content": "hello\nworld\n",
            }),
            started_at_ms: 0,
            started_emitted: true,
            kind: claude_tool_item_kind("Write"),
        };
        let write_item = file_change_item_for_tool("write", &cwd, &write_state, "completed", None)
            .expect("write file change item");
        assert_eq!(
            write_item
                .pointer("/changes/0/kind/type")
                .and_then(Value::as_str),
            Some("create")
        );
        let write_diff = write_item
            .pointer("/changes/0/diff")
            .and_then(Value::as_str)
            .expect("write diff");
        assert!(write_diff.contains("+hello"), "{write_diff}");
        assert!(write_diff.contains("+world"), "{write_diff}");

        let multiedit_state = ClaudeToolCallState {
            name: "MultiEdit".to_string(),
            arguments: json!({
                "file_path": "notes.txt",
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" },
                    { "old_string": "beta", "new_string": "BETA" }
                ],
            }),
            started_at_ms: 0,
            started_emitted: true,
            kind: claude_tool_item_kind("MultiEdit"),
        };
        let multiedit_item =
            file_change_item_for_tool("multiedit", &cwd, &multiedit_state, "completed", None)
                .expect("multiedit file change item");
        assert_eq!(
            multiedit_item
                .pointer("/changes/0/kind/type")
                .and_then(Value::as_str),
            Some("update")
        );
        let multiedit_diff = multiedit_item
            .pointer("/changes/0/diff")
            .and_then(Value::as_str)
            .expect("multiedit diff");
        assert!(multiedit_diff.contains("-alpha"), "{multiedit_diff}");
        assert!(multiedit_diff.contains("+ALPHA"), "{multiedit_diff}");
        assert!(multiedit_diff.contains("-beta"), "{multiedit_diff}");
        assert!(multiedit_diff.contains("+BETA"), "{multiedit_diff}");

        let _ = std::fs::remove_dir_all(root);
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
            input: Vec::new(),
            instruction_context: None,
            resume_existing: false,
            permission_mode: None,
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
            config_values: Map::new(),
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
