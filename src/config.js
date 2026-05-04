import crypto from "node:crypto";
import os from "node:os";
import path from "node:path";

const DEFAULT_PORT = 3147;
const DEFAULT_CDP_PORT = 9222;
const DEFAULT_SCREENSHOT_MAX_HEIGHT = 900;
const DEFAULT_SCREENSHOT_MAX_WIDTH = 1440;
const DEFAULT_SCREENSHOT_QUALITY = 74;
const DEFAULT_SCREENCAST_EVERY_NTH_FRAME = 1;
const DEFAULT_REMOTE_ROOM = "default";

export function parseConfig(argv = process.argv.slice(2), env = process.env) {
  const command = commandFromArgv(argv);
  if (command) {
    return command;
  }

  const args = parseArgs(argv);

  if (args.help || args.h) {
    return { help: true };
  }

  const token = stringValue(args.token) || env.REMOTE_TOKEN || makeToken();
  const port = numberValue(args.port, env.PORT, DEFAULT_PORT);
  const remoteWorkerUrl =
    stringValue(args["remote-url"]) ||
    stringValue(args["worker-url"]) ||
    env.REMOTE_WORKER_URL ||
    env.CLOUDFLARE_WORKER_URL ||
    "";
  const rawMode = stringValue(args.mode) || env.REMOTE_MODE || "";
  const explicitMode = normalizeMode(rawMode);
  const mode = explicitMode || (args.remote === true || env.REMOTE === "1" || remoteWorkerUrl ? "remote" : "lan");
  const cdpPortExplicit = args["cdp-port"] !== undefined || env.CDP_PORT !== undefined;
  const cdpPort = numberValue(args["cdp-port"], env.CDP_PORT, DEFAULT_CDP_PORT);
  const screencastEveryNthFrame = clampNumber(
    numberValue(args["screencast-every-nth-frame"], env.SCREENCAST_EVERY_NTH_FRAME, DEFAULT_SCREENCAST_EVERY_NTH_FRAME),
    1,
    10,
  );
  const screenshotQuality = clampNumber(
    numberValue(args["screenshot-quality"], env.SCREENSHOT_QUALITY, DEFAULT_SCREENSHOT_QUALITY),
    30,
    90,
  );
  const screenshotMaxHeight = clampNumber(
    numberValue(args["screenshot-max-height"], env.SCREENSHOT_MAX_HEIGHT, DEFAULT_SCREENSHOT_MAX_HEIGHT),
    320,
    4096,
  );
  const screenshotMaxWidth = clampNumber(
    numberValue(args["screenshot-max-width"], env.SCREENSHOT_MAX_WIDTH, DEFAULT_SCREENSHOT_MAX_WIDTH),
    320,
    4096,
  );

  return {
    appPath: stringValue(args.app) || env.CODEX_APP_PATH || "",
    executablePath: stringValue(args.executable) || env.CODEX_EXECUTABLE || "",
    cdpHost: stringValue(args["cdp-host"]) || env.CDP_HOST || "127.0.0.1",
    cdpPort,
    cdpPortExplicit,
    help: false,
    host: stringValue(args.host) || env.HOST || "0.0.0.0",
    launch: args["no-launch"] !== true && env.NO_LAUNCH !== "1",
    mode,
    modeError: rawMode && !explicitMode ? `unknown mode: ${rawMode}` : "",
    openBrowser: args.open === true || env.OPEN_BROWSER === "1",
    port,
    remoteRoom: stringValue(args["remote-room"]) || stringValue(args.room) || env.REMOTE_ROOM || DEFAULT_REMOTE_ROOM,
    remoteWorkerUrl,
    screencastEveryNthFrame,
    screenshotMaxHeight,
    screenshotMaxWidth,
    screenshotQuality,
    token,
  };
}

export function helpText() {
  return `codex-app-remotely

Starts a local Codex Electron app with Chrome DevTools Protocol enabled, then
serves a mobile web controller on the LAN.

Usage:
  car [options]
  car deploy [wrangler options]
  car remote <worker-url>
  npx codex-app-remotely deploy
  npx codex-app-remotely [options]

Options:
  deploy                         Deploy the bundled Cloudflare Worker relay.
  --app <path>                    Codex .app bundle path on macOS.
  --executable <path>             Electron executable path.
  --no-launch                     Do not launch Codex; connect to existing CDP.
  --cdp-host <host>               CDP host. Default: 127.0.0.1
  --cdp-port <port>               CDP port. Default: first free port from 9222
  --host <host>                   Web server bind host. Default: 0.0.0.0
  --port <port>                   Web server port. Default: 3147
  --mode <lan|remote>             Access mode. Default: lan, or remote when --remote-url is set
  --remote                        Shortcut for --mode remote.
  --remote-url <url>              Cloudflare Worker URL for remote mode.
  --remote-room <room>            Remote session room. Default: default
  --token <token>                 Mobile auth token. Default: random per run
  --screencast-every-nth-frame <n> Minimum screencast frame skip. Default: 1
  --screenshot-max-width <px>     Cap good-profile screenshots to this width. Default: 1440
  --screenshot-max-height <px>    Cap good-profile screenshots to this height. Default: 900
  --screenshot-quality <1-100>    Cap good-profile JPEG quality. Default: 74
  --open                          Open the mobile URL in the default browser.
  -h, --help                      Show this help.

Environment:
  CODEX_APP_PATH, CODEX_EXECUTABLE, CDP_HOST, CDP_PORT, HOST, PORT,
  REMOTE_TOKEN, REMOTE_MODE, REMOTE_WORKER_URL, REMOTE_ROOM,
  SCREENCAST_EVERY_NTH_FRAME, SCREENSHOT_MAX_WIDTH, SCREENSHOT_MAX_HEIGHT, SCREENSHOT_QUALITY,
  NO_LAUNCH=1, OPEN_BROWSER=1
`;
}

export function getLanUrls(host, port, token) {
  const encodedToken = encodeURIComponent(token);
  const urls = [];

  if (host === "127.0.0.1" || host === "localhost") {
    urls.push(`http://${host}:${port}/?token=${encodedToken}`);
    return urls;
  }

  for (const entries of Object.values(os.networkInterfaces())) {
    for (const entry of entries || []) {
      if (entry.family === "IPv4" && !entry.internal) {
        urls.push(`http://${entry.address}:${port}/?token=${encodedToken}`);
      }
    }
  }

  urls.push(`http://127.0.0.1:${port}/?token=${encodedToken}`);
  return [...new Set(urls)];
}

export function resolvePath(input) {
  if (!input) {
    return "";
  }

  if (input.startsWith("~/")) {
    return path.join(os.homedir(), input.slice(2));
  }

  return path.resolve(input);
}

function parseArgs(argv) {
  const args = {};
  const positionals = [];

  for (let i = 0; i < argv.length; i += 1) {
    const raw = argv[i];

    if (!raw.startsWith("-")) {
      positionals.push(raw);
      continue;
    }

    const withoutPrefix = raw.replace(/^--?/, "");
    const [key, inlineValue] = withoutPrefix.split("=", 2);

    if (inlineValue !== undefined) {
      args[key] = inlineValue;
      continue;
    }

    const next = argv[i + 1];
    if (next && !next.startsWith("-")) {
      args[key] = next;
      i += 1;
    } else {
      args[key] = true;
    }
  }

  applyPositionalArgs(args, positionals);
  return args;
}

function commandFromArgv(argv) {
  const deployIndex = argv.findIndex((arg) => arg === "deploy");
  if (deployIndex < 0) {
    return null;
  }

  return {
    command: "deploy",
    deployArgs: argv.slice(deployIndex + 1),
    help: false,
  };
}

function applyPositionalArgs(args, positionals) {
  for (let i = 0; i < positionals.length; i += 1) {
    const value = positionals[i];
    const mode = normalizeMode(value);

    if (mode) {
      if (args.mode === undefined) {
        args.mode = mode;
      }
      if (mode === "remote") {
        args.remote = true;
        const next = positionals[i + 1];
        if (args["remote-url"] === undefined && isHttpUrl(next)) {
          args["remote-url"] = next;
          i += 1;
        }
      }
      continue;
    }

    if (args["remote-url"] === undefined && isHttpUrl(value)) {
      args["remote-url"] = value;
    }
  }
}

function numberValue(primary, secondary, fallback) {
  const raw = primary ?? secondary;
  if (raw === undefined || raw === "") {
    return fallback;
  }

  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function clampNumber(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function stringValue(value) {
  return typeof value === "string" && value.length > 0 ? value : "";
}

function normalizeMode(value) {
  if (value === "local") {
    return "lan";
  }

  if (value === "lan" || value === "remote") {
    return value;
  }

  return "";
}

function isHttpUrl(value) {
  return typeof value === "string" && /^https?:\/\//i.test(value);
}

function makeToken() {
  return crypto.randomBytes(18).toString("base64url");
}
