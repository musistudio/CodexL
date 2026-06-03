#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, posix, relative, resolve } from "node:path";

const repoRoot = resolve(import.meta.dirname, "..");
const defaultAppPath = process.env.CODEX_APP_PATH || "/Applications/Codex.app";
const defaultOutDir = "dist/codex-app-web";
const defaultBridgeScriptPath = "src-tauri/src/remote/cdp_resources/bridge_script.rs";
const defaultPluginRuntimeScriptPath = "src-tauri/src/remote/cdp_resources/plugin_runtime.rs";
const defaultRuntimeBaseUrl = process.env.CODEXL_CODEX_WEB_RUNTIME_BASE_URL || "../codexl-runtime";
const defaultRuntimeDir = "codexl-runtime";
const remotePluginBridgeUrl = "ws://127.0.0.1:0/plugin/_bridge?token=remote-web-bridge";
const registrySchemaVersion = 1;

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  printUsage();
  process.exit(0);
}

const appPath = resolvePath(args.app || defaultAppPath);
const asarPath = resolveAsarPath(args.asar, appPath);
const outDir = resolvePath(args.outDir || defaultOutDir);
const bridgeScriptPath = resolvePath(args.bridgeScript || defaultBridgeScriptPath);
const pluginRuntimeScriptPath = resolvePath(args.pluginRuntimeScript || defaultPluginRuntimeScriptPath);
const runtimeBaseUrl = normalizeRuntimeBaseUrl(args.runtimeBaseUrl || defaultRuntimeBaseUrl);
const runtimeDirName = normalizeRuntimeDir(args.runtimeDir || defaultRuntimeDir);
const clean = Boolean(args.clean);
const writeLatestAlias = args.latest !== false;
const writeHeaders = args.headers !== false;

if (!existsSync(asarPath)) {
  fail(`Codex App ASAR not found: ${asarPath}`);
}

if (!existsSync(bridgeScriptPath)) {
  fail(`Web bridge script source not found: ${bridgeScriptPath}`);
}

if (!existsSync(pluginRuntimeScriptPath)) {
  fail(`CodexL plugin runtime source not found: ${pluginRuntimeScriptPath}`);
}

const asar = readAsar(asarPath);
const packageJson = JSON.parse(readAsarFileText(asar, "package.json"));
const detectedVersion = String(packageJson.version || "").trim();
const version = normalizeVersion(args.version || detectedVersion);
if (!version) {
  fail("Could not determine Codex App version. Pass --version <version>.");
}

const versionDir = join(outDir, version);
if (clean) {
  rmSync(versionDir, { force: true, recursive: true });
}
mkdirSync(versionDir, { recursive: true });

const bridgeScript = readBridgeScript(bridgeScriptPath);
const { script: pluginRuntimeScript, version: pluginRuntimeVersion } =
  readPluginRuntimeScript(pluginRuntimeScriptPath);
const runtimeScripts = runtimeScriptUrls(runtimeBaseUrl);
const extractedAt = new Date().toISOString();
const resources = [];

extractAsarDirectory(asar, "webview", versionDir, (assetPath, content) => {
  let nextContent = content;
  if (assetPath === "index.html") {
    nextContent = Buffer.from(
      prepareIndexHtml(content.toString("utf8"), runtimeScripts),
      "utf8",
    );
  } else if (assetPath.endsWith(".js")) {
    nextContent = Buffer.from(
      patchCodexAppWebJavascript(assetPath, content.toString("utf8")),
      "utf8",
    );
  } else if (assetPath.endsWith(".css")) {
    nextContent = Buffer.from(rewriteCssAssetUrls(assetPath, content.toString("utf8")), "utf8");
  }
  return nextContent;
});

writeRuntimeFiles(outDir, runtimeDirName, bridgeScript, pluginRuntimeScript, pluginRuntimeVersion);

for (const file of listFiles(versionDir)) {
  const content = readFileSync(file);
  const assetPath = toPosixPath(relative(versionDir, file));
  resources.push({
    path: assetPath,
    size: content.length,
    sha256: sha256(content),
    contentType: contentTypeForPath(assetPath),
  });
}
resources.sort((left, right) => left.path.localeCompare(right.path));

const buildId = sha256(
  Buffer.from(resources.map((resource) => `${resource.path}:${resource.sha256}`).join("\n")),
);
const manifest = {
  schemaVersion: registrySchemaVersion,
  product: packageJson.productName || "Codex",
  packageName: packageJson.name || "openai-codex-electron",
  appVersion: version,
  buildId,
  entry: "index.html",
  bridgeScript: "_codexl_bridge.js",
  bridgeScriptUrl: runtimeScripts.bridge,
  pluginRuntimeScript: "_codexl_plugin.js",
  pluginRuntimeScriptUrl: runtimeScripts.plugin,
  pluginRuntimeVersion,
  runtimeBaseUrl,
  runtimeDirectory: runtimeDirName,
  extractedAt,
  source: {
    appPath,
    asarPath,
  },
  resourceCount: resources.length,
  totalBytes: resources.reduce((total, resource) => total + resource.size, 0),
  resources,
};
writeJson(join(versionDir, "manifest.json"), manifest);

mkdirSync(outDir, { recursive: true });
const versionsIndex = updateVersionsIndex(outDir, manifest);
writeJson(join(outDir, "versions.json"), versionsIndex);
writeJson(join(outDir, "latest.json"), {
  schemaVersion: registrySchemaVersion,
  latest: version,
  manifest: `${version}/manifest.json`,
  entry: `${version}/index.html`,
});

if (writeLatestAlias) {
  writeLatestIndex(outDir, version);
}
if (writeHeaders) {
  writeCloudflareHeaders(outDir, runtimeDirName);
}

console.log(`Extracted Codex App web bundle ${version}`);
console.log(`Registry directory: ${outDir}`);
console.log(`Entry: ${join(versionDir, "index.html")}`);

function parseArgs(argv) {
  const parsed = {
    app: "",
    asar: "",
    bridgeScript: "",
    clean: true,
    headers: true,
    help: false,
    latest: true,
    outDir: "",
    pluginRuntimeScript: "",
    runtimeBaseUrl: "",
    runtimeDir: "",
    version: "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--":
        break;
      case "--app":
        parsed.app = readValue(argv, ++index, arg);
        break;
      case "--asar":
        parsed.asar = readValue(argv, ++index, arg);
        break;
      case "--bridge-script":
        parsed.bridgeScript = readValue(argv, ++index, arg);
        break;
      case "--plugin-runtime-script":
        parsed.pluginRuntimeScript = readValue(argv, ++index, arg);
        break;
      case "--runtime-base-url":
        parsed.runtimeBaseUrl = readValue(argv, ++index, arg);
        break;
      case "--runtime-dir":
        parsed.runtimeDir = readValue(argv, ++index, arg);
        break;
      case "--out-dir":
        parsed.outDir = readValue(argv, ++index, arg);
        break;
      case "--version":
        parsed.version = readValue(argv, ++index, arg);
        break;
      case "--clean":
        parsed.clean = true;
        break;
      case "--no-clean":
        parsed.clean = false;
        break;
      case "--no-headers":
        parsed.headers = false;
        break;
      case "--no-latest":
        parsed.latest = false;
        break;
      case "--help":
      case "-h":
        parsed.help = true;
        break;
      default:
        fail(`Unsupported argument: ${arg}`);
    }
  }
  return parsed;
}

function readValue(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    fail(`Missing value for ${flag}`);
  }
  return value;
}

function printUsage() {
  console.log(`Usage:
  pnpm run extract:codex-web -- [options]

Options:
  --app <path>            Codex.app path. Default: ${defaultAppPath}
  --asar <path>           app.asar path. Defaults to <app>/Contents/Resources/app.asar
  --out-dir <path>        Static registry output directory. Default: ${defaultOutDir}
  --version <version>     Override detected Codex App version
  --bridge-script <path>  Rust bridge script source. Default: ${defaultBridgeScriptPath}
  --plugin-runtime-script <path>
                           Rust plugin runtime source. Default: ${defaultPluginRuntimeScriptPath}
  --runtime-base-url <url>
                           Stable URL used by index.html for CodexL runtime scripts.
                           Default: ${defaultRuntimeBaseUrl}
  --runtime-dir <path>    Runtime output directory under --out-dir. Default: ${defaultRuntimeDir}
  --clean                 Remove the target version directory before extracting (default)
  --no-clean              Keep existing files in the target version directory
  --no-latest             Do not write latest/index.html redirect
  --no-headers            Do not write Cloudflare Pages _headers
`);
}

function resolvePath(value) {
  return resolve(repoRoot, value);
}

function resolveAsarPath(asarArg, app) {
  if (asarArg) {
    return resolvePath(asarArg);
  }
  if (app.endsWith(".asar")) {
    return app;
  }
  return join(app, "Contents", "Resources", "app.asar");
}

function readAsar(path) {
  const data = readFileSync(path);
  if (data.length < 16) {
    fail(`Invalid ASAR file: ${path}`);
  }
  const headerSize = data.readUInt32LE(4);
  const headerJsonSize = data.readUInt32LE(12);
  const headerStart = 16;
  const headerEnd = headerStart + headerJsonSize;
  const header = JSON.parse(data.slice(headerStart, headerEnd).toString("utf8"));
  return {
    data,
    dataStart: 8 + headerSize,
    header,
    path,
    unpackedDir: `${path}.unpacked`,
  };
}

function readAsarFileText(asar, path) {
  return readAsarFile(asar, path).toString("utf8");
}

function readAsarFile(asar, path) {
  const entry = findAsarEntry(asar.header, path);
  if (!entry || entry.files) {
    fail(`ASAR file not found: ${path}`);
  }
  if (entry.unpacked) {
    return readFileSync(join(asar.unpackedDir, path));
  }
  const offset = asar.dataStart + Number(entry.offset || 0);
  const size = Number(entry.size || 0);
  return asar.data.slice(offset, offset + size);
}

function findAsarEntry(header, path) {
  return path
    .split("/")
    .filter(Boolean)
    .reduce((node, part) => node?.files?.[part], { files: header.files });
}

function extractAsarDirectory(asar, sourceDir, targetDir, transform) {
  const root = findAsarEntry(asar.header, sourceDir);
  if (!root?.files) {
    fail(`ASAR directory not found: ${sourceDir}`);
  }
  walkAsarDirectory(asar, sourceDir, root, targetDir, transform);
}

function walkAsarDirectory(asar, sourcePath, entry, targetDir, transform) {
  for (const [name, child] of Object.entries(entry.files || {})) {
    const childSourcePath = `${sourcePath}/${name}`;
    const assetPath = childSourcePath.slice(sourcePathRootLength("webview"));
    if (child.files) {
      mkdirSync(join(targetDir, assetPath), { recursive: true });
      walkAsarDirectory(asar, childSourcePath, child, targetDir, transform);
      continue;
    }
    const outputPath = join(targetDir, assetPath);
    mkdirSync(dirname(outputPath), { recursive: true });
    const content = transform(assetPath, readAsarFile(asar, childSourcePath));
    writeFileSync(outputPath, content);
  }
}

function sourcePathRootLength(root) {
  return `${root}/`.length;
}

function readBridgeScript(path) {
  const raw = readFileSync(path, "utf8");
  const match = raw.match(/WEB_BRIDGE_SCRIPT:\s*&str\s*=\s*r#"(.*?)"#;/s);
  if (!match) {
    fail(`Could not find WEB_BRIDGE_SCRIPT raw string in ${path}`);
  }
  return `${match[1].trim()}\n`;
}

function readPluginRuntimeScript(path) {
  const raw = readFileSync(path, "utf8");
  const versionMatch = raw.match(/CODEXL_PLUGIN_RUNTIME_VERSION:\s*&str\s*=\s*"([^"]+)"/);
  const scriptMatch = raw.match(/CODEXL_PLUGIN_BOOTSTRAP:\s*&str\s*=\s*r#"(.*?)"#;/s);
  if (!versionMatch) {
    fail(`Could not find CODEXL_PLUGIN_RUNTIME_VERSION in ${path}`);
  }
  if (!scriptMatch) {
    fail(`Could not find CODEXL_PLUGIN_BOOTSTRAP raw string in ${path}`);
  }
  return {
    script: `${scriptMatch[1]
      .trim()
      .replaceAll('"__CODEXL_PLUGIN_BRIDGE_URL__"', JSON.stringify(remotePluginBridgeUrl))
      .replaceAll("__CODEXL_PLUGIN_RUNTIME_VERSION__", versionMatch[1])}\n`,
    version: versionMatch[1],
  };
}

function runtimeScriptUrls(baseUrl) {
  return {
    bridge: runtimeScriptUrl(baseUrl, "_codexl_bridge.js"),
    plugin: runtimeScriptUrl(baseUrl, "_codexl_plugin.js"),
  };
}

function runtimeScriptUrl(baseUrl, fileName) {
  const trimmedBase = String(baseUrl || "").trim().replace(/\/+$/g, "");
  if (!trimmedBase) {
    return `./${fileName}`;
  }
  return `${trimmedBase}/${fileName}`;
}

function writeRuntimeFiles(root, runtimeDirName, bridgeScript, pluginRuntimeScript, pluginRuntimeVersion) {
  const runtimeDir = join(root, runtimeDirName);
  mkdirSync(runtimeDir, { recursive: true });
  const bridgePath = join(runtimeDir, "_codexl_bridge.js");
  const pluginPath = join(runtimeDir, "_codexl_plugin.js");
  writeFileSync(bridgePath, bridgeScript);
  writeFileSync(pluginPath, pluginRuntimeScript);
  writeJson(join(runtimeDir, "manifest.json"), {
    schemaVersion: registrySchemaVersion,
    bridgeScript: "_codexl_bridge.js",
    bridgeScriptSha256: sha256(bridgeScript),
    pluginRuntimeScript: "_codexl_plugin.js",
    pluginRuntimeScriptSha256: sha256(pluginRuntimeScript),
    pluginRuntimeVersion,
    updatedAt: new Date().toISOString(),
  });
}

function prepareIndexHtml(raw, runtimeScripts) {
  let html = raw
    .replace(/<!--\s*PROD_BASE_TAG_HERE\s*-->/g, "")
    .replace(/<!--\s*PROD_CSP_TAG_HERE\s*-->/g, "")
    .replace(/<meta\b[^>]*http-equiv=["']content-security-policy["'][^>]*>/gi, "")
    .replace(/\b(src|href)=["']\/(?!\/)([^"']+)["']/g, '$1="./$2"');
  html = html.replace(
    /(["'])(?:(?:https?:)?\/\/[^"']+|[^"']*\/)?_codexl_bridge\.js(?:\?[^"']*)?\1/g,
    (_match, quote) => `${quote}${runtimeScripts.bridge}${quote}`,
  );
  html = html.replace(
    /(["'])(?:(?:https?:)?\/\/[^"']+|[^"']*\/)?_codexl_plugin\.js(?:\?[^"']*)?\1/g,
    (_match, quote) => `${quote}${runtimeScripts.plugin}${quote}`,
  );

  const tags = [];
  if (!html.includes("codexl-mobile-touch-fix")) {
    tags.push(codexAppMobileTouchFixTag());
  }
  if (!html.includes("_codexl_bridge.js")) {
    tags.push(`    <script src="${escapeHtml(runtimeScripts.bridge)}"></script>`);
  }
  if (tags.length === 0) {
    return html;
  }

  const tag = `${tags.join("\n")}\n`;
  const firstModuleScript = html.search(/<script\b[^>]*type=["']module["'][^>]*>/i);
  if (firstModuleScript >= 0) {
    return `${html.slice(0, firstModuleScript)}${tag}${html.slice(firstModuleScript)}`;
  }
  if (html.includes("</head>")) {
    return html.replace("</head>", `${tag}</head>`);
  }
  return `${tag}${html}`;
}

function codexAppMobileTouchFixTag() {
  return `    <script id="codexl-mobile-touch-fix">(() => {
      const root = document.documentElement;
      const attr = "data-codexl-touch-device";
      const styleId = "codexl-mobile-touch-style";
      const touchQuery = "(hover: none), (pointer: coarse), (any-pointer: coarse)";
      const selectors = [
        '[data-app-action-sidebar-thread-row]',
        '[role="button"]:has([data-thread-title-trigger])'
      ].join(", ");
      const style = document.createElement("style");
      style.id = styleId;
      style.textContent = \`
        @media \${touchQuery} {
          [data-testid="app-shell-floating-left-panel"],
          div:has(> [data-testid="app-shell-floating-left-panel"]) {
            display: none !important;
            pointer-events: none !important;
          }
          \${selectors} [class*="group-hover:opacity-100"],
          \${selectors} [class*="group-focus-within:opacity-100"],
          \${selectors} [class*="group-hover:opacity-50"] {
            opacity: 1 !important;
          }
          \${selectors} [class*="group-hover:pointer-events-auto"],
          \${selectors} [class*="group-focus-within:pointer-events-auto"] {
            pointer-events: auto !important;
          }
          \${selectors} [class*="group-hover:opacity-0"],
          \${selectors} [class*="group-focus-within:opacity-0"] {
            opacity: 0 !important;
          }
          \${selectors} [class*="group-hover:min-w-5"],
          \${selectors} [class*="group-has-"][class*="min-w-5"] {
            min-width: 1.25rem !important;
          }
          \${selectors} [class*="group-hover:min-w-12"],
          \${selectors} [class*="group-has-"][class*="min-w-12"] {
            min-width: 3rem !important;
          }
          \${selectors} [class*="group-hover:min-w-20"],
          \${selectors} [class*="group-has-"][class*="min-w-20"] {
            min-width: 5rem !important;
          }
          [data-tab-id] [class*="group-hover/tab:flex"] {
            display: flex !important;
            opacity: 1 !important;
            pointer-events: auto !important;
          }
          [role="button"]:has([data-thread-title-trigger]) button,
          [data-tab-id] [role="button"],
          [data-tab-id] button {
            touch-action: manipulation;
          }
        }
        html[\${attr}="1"] [data-testid="app-shell-floating-left-panel"],
        html[\${attr}="1"] div:has(> [data-testid="app-shell-floating-left-panel"]) {
          display: none !important;
          pointer-events: none !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:opacity-100"],
        html[\${attr}="1"] \${selectors} [class*="group-focus-within:opacity-100"],
        html[\${attr}="1"] \${selectors} [class*="group-hover:opacity-50"] {
          opacity: 1 !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:pointer-events-auto"],
        html[\${attr}="1"] \${selectors} [class*="group-focus-within:pointer-events-auto"] {
          pointer-events: auto !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:opacity-0"],
        html[\${attr}="1"] \${selectors} [class*="group-focus-within:opacity-0"] {
          opacity: 0 !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:min-w-5"],
        html[\${attr}="1"] \${selectors} [class*="group-has-"][class*="min-w-5"] {
          min-width: 1.25rem !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:min-w-12"],
        html[\${attr}="1"] \${selectors} [class*="group-has-"][class*="min-w-12"] {
          min-width: 3rem !important;
        }
        html[\${attr}="1"] \${selectors} [class*="group-hover:min-w-20"],
        html[\${attr}="1"] \${selectors} [class*="group-has-"][class*="min-w-20"] {
          min-width: 5rem !important;
        }
        html[\${attr}="1"] [data-tab-id] [class*="group-hover/tab:flex"] {
          display: flex !important;
          opacity: 1 !important;
          pointer-events: auto !important;
        }
        html[\${attr}="1"] [role="button"]:has([data-thread-title-trigger]) button,
        html[\${attr}="1"] [data-tab-id] [role="button"],
        html[\${attr}="1"] [data-tab-id] button {
          touch-action: manipulation;
        }
      \`;
      if (!document.getElementById(styleId)) {
        (document.head || root).appendChild(style);
      }
      const mark = () => {
        root.setAttribute(attr, "1");
      };
      const isTouchDevice = () => {
        try {
          return navigator.maxTouchPoints > 0 || window.matchMedia?.(touchQuery)?.matches === true;
        } catch {
          return false;
        }
      };
      if (isTouchDevice()) {
        mark();
      }
      window.addEventListener("touchstart", mark, { capture: true, passive: true });
      window.addEventListener("pointerdown", (event) => {
        if (event?.pointerType === "touch" || event?.pointerType === "pen") {
          mark();
        }
      }, { capture: true, passive: true });
    })();</script>`;
}

function patchCodexAppWebJavascript(assetPath, raw) {
  if (!assetPath.startsWith("assets/app-shell-") || !assetPath.endsWith(".js")) {
    return raw;
  }

  const floatingPanelMarker = "app-shell-floating-left-panel";
  if (!raw.includes(floatingPanelMarker)) {
    return raw;
  }

  let patched = raw;
  patched = replaceOnce(
    patched,
    "let a=t.watch(({get:a})=>{if(a(Ze)){n=!1,r=void 0,i=void 0,e(!1);return}",
    'let a=t.watch(({get:a})=>{if((()=>{try{return navigator.maxTouchPoints>0||window.matchMedia?.("(hover: none), (pointer: coarse), (any-pointer: coarse)")?.matches===!0}catch{return!1}})()){n=!1,r=void 0,i=void 0,e(!1);return}if(a(Ze)){n=!1,r=void 0,i=void 0,e(!1);return}',
    assetPath,
    "left sidebar floating hover preview",
  );
  patched = replaceOnce(
    patched,
    "R=()=>{s.set(Ne,!0)},z=()=>{s.set(Ne,!1),s.set(Le,!1),s.set(Ae,!1)}",
    "R=e=>{if(e?.pointerType===`touch`||e?.pointerType===`pen`||navigator.maxTouchPoints>0)return;s.set(Ne,!0)},z=()=>{s.set(Ne,!1),s.set(Le,!1),s.set(Ae,!1)}",
    assetPath,
    "left sidebar trigger touch pointer hover",
  );
  return patched;
}

function replaceOnce(raw, search, replacement, assetPath, label) {
  const index = raw.indexOf(search);
  if (index < 0) {
    fail(`Could not apply Codex App web patch for ${label}: ${assetPath}`);
  }
  if (raw.indexOf(search, index + search.length) >= 0) {
    fail(`Codex App web patch for ${label} matched more than once: ${assetPath}`);
  }
  return `${raw.slice(0, index)}${replacement}${raw.slice(index + search.length)}`;
}

function rewriteCssAssetUrls(assetPath, raw) {
  return raw.replace(/url\((["']?)\/assets\/([^)"']+)\1\)/g, (_match, quote, target) => {
    const cssDir = posix.dirname(toPosixPath(assetPath));
    const relativePath = posix.relative(cssDir, `assets/${target}`) || ".";
    const safePath = relativePath.startsWith(".") ? relativePath : `./${relativePath}`;
    return `url(${quote}${safePath}${quote})`;
  });
}

function listFiles(root) {
  const result = [];
  const stack = [root];
  while (stack.length > 0) {
    const dir = stack.pop();
    const entries = readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(path);
      } else if (entry.isFile()) {
        result.push(path);
      }
    }
  }
  return result;
}

function sha256(content) {
  return createHash("sha256").update(content).digest("hex");
}

function contentTypeForPath(path) {
  const extension = path.split(".").pop()?.toLowerCase() || "";
  switch (extension) {
    case "css":
      return "text/css; charset=utf-8";
    case "html":
      return "text/html; charset=utf-8";
    case "js":
    case "mjs":
      return "application/javascript; charset=utf-8";
    case "json":
    case "map":
      return "application/json; charset=utf-8";
    case "svg":
      return "image/svg+xml";
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "webp":
      return "image/webp";
    case "woff":
      return "font/woff";
    case "woff2":
      return "font/woff2";
    default:
      return "application/octet-stream";
  }
}

function updateVersionsIndex(root, manifest) {
  const path = join(root, "versions.json");
  const existing = readJsonIfExists(path) || {};
  const existingVersions = Array.isArray(existing.versions) ? existing.versions : [];
  const current = {
    version: manifest.appVersion,
    appVersion: manifest.appVersion,
    buildId: manifest.buildId,
    entry: `${manifest.appVersion}/${manifest.entry}`,
    manifest: `${manifest.appVersion}/manifest.json`,
    path: `${manifest.appVersion}/`,
    extractedAt: manifest.extractedAt,
    resourceCount: manifest.resourceCount,
    totalBytes: manifest.totalBytes,
  };
  const versions = [
    current,
    ...existingVersions.filter((item) => item?.version !== manifest.appVersion),
  ].sort((left, right) => compareVersionDescending(left.version, right.version));
  return {
    schemaVersion: registrySchemaVersion,
    latest: manifest.appVersion,
    updatedAt: manifest.extractedAt,
    versions,
  };
}

function readJsonIfExists(path) {
  if (!existsSync(path)) {
    return null;
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function writeLatestIndex(root, version) {
  const latestDir = join(root, "latest");
  mkdirSync(latestDir, { recursive: true });
  writeFileSync(
    join(latestDir, "index.html"),
    `<!doctype html>
<meta charset="utf-8">
<title>Codex App Web Bundle</title>
<script>
  const target = new URL("../${escapeHtml(version)}/index.html", location.href);
  target.search = location.search;
  target.hash = location.hash;
  location.replace(target);
</script>
<noscript><a href="../${escapeHtml(version)}/index.html">Open latest Codex App web bundle</a></noscript>
`,
  );
}

function writeCloudflareHeaders(root, runtimeDirName) {
  const runtimeHeaderPath = toPosixPath(runtimeDirName)
    .split("/")
    .map((part) => encodeURIComponent(part))
    .join("/");
  writeFileSync(
    join(root, "_headers"),
    `/*
  Access-Control-Allow-Origin: *
  Cross-Origin-Resource-Policy: cross-origin

/*.html
  Cache-Control: public, max-age=60

/latest/*
  Cache-Control: public, max-age=60

/versions.json
  Cache-Control: public, max-age=60

/latest.json
  Cache-Control: public, max-age=60

/*.js
  Cache-Control: public, max-age=31536000, immutable

/*.css
  Cache-Control: public, max-age=31536000, immutable

/${runtimeHeaderPath}/*
  Cache-Control: public, max-age=60, must-revalidate
`,
  );
}

function compareVersionDescending(left, right) {
  return compareVersion(right, left);
}

function compareVersion(left, right) {
  const leftParts = String(left || "").split(/[.-]/);
  const rightParts = String(right || "").split(/[.-]/);
  const length = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = leftParts[index] || "";
    const rightPart = rightParts[index] || "";
    const leftNumber = /^\d+$/.test(leftPart) ? Number(leftPart) : null;
    const rightNumber = /^\d+$/.test(rightPart) ? Number(rightPart) : null;
    if (leftNumber !== null && rightNumber !== null && leftNumber !== rightNumber) {
      return leftNumber - rightNumber;
    }
    const compared = leftPart.localeCompare(rightPart);
    if (compared !== 0) {
      return compared;
    }
  }
  return 0;
}

function normalizeVersion(value) {
  const version = String(value || "").trim();
  if (!version) {
    return "";
  }
  if (!/^[0-9A-Za-z._-]+$/.test(version)) {
    fail(`Version may only contain letters, digits, dots, underscores, and hyphens: ${version}`);
  }
  return version;
}

function normalizeRuntimeBaseUrl(value) {
  const baseUrl = String(value || "").trim();
  if (!baseUrl) {
    return "";
  }
  if (/^https?:\/\//i.test(baseUrl)) {
    const url = new URL(baseUrl);
    url.hash = "";
    url.search = "";
    return url.toString().replace(/\/+$/g, "");
  }
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(baseUrl)) {
    fail(`Runtime base URL must be http(s), root-relative, or relative: ${baseUrl}`);
  }
  return baseUrl.replace(/\/+$/g, "");
}

function normalizeRuntimeDir(value) {
  const runtimeDir = toPosixPath(String(value || "").trim()).replace(/^\/+|\/+$/g, "");
  if (!runtimeDir) {
    fail("Runtime directory cannot be empty");
  }
  if (
    runtimeDir.includes("..") ||
    runtimeDir.includes("//") ||
    !/^[0-9A-Za-z._/-]+$/.test(runtimeDir)
  ) {
    fail(
      "Runtime directory may only contain letters, digits, dots, underscores, hyphens, and slashes",
    );
  }
  return runtimeDir;
}

function toPosixPath(path) {
  return path.split("\\").join("/");
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
