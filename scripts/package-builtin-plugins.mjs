import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const builtinPluginsDir = join(repoRoot, "extensions", "builtins");
const packageDir = join(repoRoot, "src-tauri", "builtin-plugin-packages");
const nextAiGatewaySourceDir = optionalResolveEnvPath("NEXT_AI_GATEWAY_SOURCE_DIR");
const botGatewaySourceDir = optionalResolveEnvPath("BOT_GATEWAY_SOURCE_DIR");

const plugins = [
  {
    name: "bot-gateway",
    include: ["plugin.json", "package.json", "stdio"],
    beforePackage: syncBotGateway,
  },
  {
    name: "next-ai-gateway",
    include: ["plugin.json", "package.json", "gateway"],
    beforePackage: buildNextAiGateway,
  },
];

mkdirSync(packageDir, { recursive: true });

for (const plugin of plugins) {
  const pluginDir = join(builtinPluginsDir, plugin.name);
  const manifest = readManifest(pluginDir);

  if (plugin.beforePackage) {
    plugin.beforePackage(pluginDir);
  }

  const archivePath = join(packageDir, `${manifest.id}-${manifest.version}.tar.gz`);
  writeTarGz(archivePath, pluginDir, plugin.include);

  console.log(archivePath);
}

function readManifest(pluginDir) {
  const manifestPath = join(pluginDir, "plugin.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

  if (!manifest.id || !manifest.version) {
    throw new Error(`Built-in plugin manifest must include id and version: ${manifestPath}`);
  }

  return manifest;
}

function optionalResolveEnvPath(name) {
  const value = process.env[name];
  return value ? resolve(value) : undefined;
}

function syncBotGateway(pluginDir) {
  const outputFile = join(pluginDir, "stdio", "stdio.js");

  if (!botGatewaySourceDir) {
    reuseExistingBundleOrThrow(outputFile, "BOT_GATEWAY_SOURCE_DIR");
    return;
  }

  const sourceBundle = join(botGatewaySourceDir, "dist-bundle", "stdio", "stdio.js");
  if (!existsSync(sourceBundle)) {
    if (existsSync(outputFile)) {
      console.warn(`Bot Gateway source bundle skipped; reusing existing bundle: ${outputFile}`);
      return;
    }
    throw new Error(
      `Bot Gateway stdio bundle not found: ${sourceBundle}. Run npm run bundle:stdio in ${botGatewaySourceDir}.`,
    );
  }

  mkdirSync(join(pluginDir, "stdio"), { recursive: true });
  copyFileSync(sourceBundle, outputFile);
  patchBotGatewayBundle(outputFile);
}

function patchBotGatewayBundle(outputFile) {
  let content = readFileSync(outputFile, "utf8");
  const originalContent = content;
  content = patchBotGatewayFeishuCardActions(content, outputFile);

  const marker = `#!/usr/bin/env node\n`;
  if (!content.includes("__codexlFileURLToPath")) {
    if (!content.startsWith(marker)) {
      throw new Error(`Bot Gateway stdio bundle has an unexpected header: ${outputFile}`);
    }
    content =
      `${marker}import { fileURLToPath as __codexlFileURLToPath } from "node:url";\n` +
      `import { dirname as __codexlDirname } from "node:path";\n` +
      `const __filename = __codexlFileURLToPath(import.meta.url);\n` +
      `const __dirname = __codexlDirname(__filename);\n` +
      content.slice(marker.length);
  }

  if (content !== originalContent) {
    writeFileSync(outputFile, content);
  }
}

function patchBotGatewayFeishuCardActions(content, outputFile) {
  if (content.includes("disabled: action.disabled === true ? true : void 0")) {
    return content;
  }
  const marker = `        url: action.url,\n        value: action.value ? { value: action.value } : void 0`;
  if (!content.includes(marker)) {
    throw new Error(`Bot Gateway Feishu card action renderer has an unexpected shape: ${outputFile}`);
  }
  return content.replace(
    marker,
    `        url: action.url,\n        disabled: action.disabled === true ? true : void 0,\n        value: action.value ? { value: action.value } : void 0`,
  );
}

function buildNextAiGateway(pluginDir) {
  const outputFile = join(pluginDir, "gateway", "index.cjs");

  if (!nextAiGatewaySourceDir) {
    reuseExistingBundleOrThrow(outputFile, "NEXT_AI_GATEWAY_SOURCE_DIR");
    patchNextAiGatewayBundle(outputFile);
    return;
  }

  const entryPoint = join(nextAiGatewaySourceDir, "src", "index.ts");
  const esbuild = join(
    nextAiGatewaySourceDir,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "esbuild.cmd" : "esbuild",
  );

  if (!existsSync(entryPoint) || !existsSync(esbuild)) {
    if (existsSync(outputFile)) {
      console.warn(`NeXT AI gateway source build skipped; reusing existing bundle: ${outputFile}`);
      return;
    }
    if (!existsSync(entryPoint)) {
      throw new Error(`NeXT AI gateway entry not found: ${entryPoint}`);
    }
    throw new Error(
      `NeXT AI gateway esbuild binary not found: ${esbuild}. Run npm install in ${nextAiGatewaySourceDir}.`,
    );
  }

  mkdirSync(join(pluginDir, "gateway"), { recursive: true });
  rmSync(join(pluginDir, "gateway", "index.js"), { force: true });
  execFileSync(
    esbuild,
    [
      entryPoint,
      "--bundle",
      "--platform=node",
      "--target=node20",
      "--minify",
      "--log-level=warning",
      `--outfile=${outputFile}`,
    ],
    {
      cwd: nextAiGatewaySourceDir,
      stdio: "inherit",
    },
  );
  patchNextAiGatewayBundle(outputFile);
}

function patchNextAiGatewayBundle(outputFile) {
  let content = readFileSync(outputFile, "utf8");
  const enabledPatchedMarker = `if(!Xe(r))continue;if(ko(r.enabled)===!1)continue;let s=r,i=zwt(s.transport)||"stdio"`;
  const enabledPatchedMarkerNew = `if(!Xe(r))continue;let s=r;if(Vi(s.enabled)===!1)continue;let i=evt(s.transport)||"stdio"`;
  if (!content.includes(enabledPatchedMarker) && !content.includes(enabledPatchedMarkerNew)) {
    const marker = `if(!Xe(r))continue;let s=r,i=zwt(s.transport)||"stdio"`;
    const markerNew = `if(!Xe(r))continue;let s=r,i=evt(s.transport)||"stdio"`;
    if (content.includes(marker)) {
      content = content.replace(marker, enabledPatchedMarker);
    } else if (content.includes(markerNew)) {
      content = content.replace(markerNew, enabledPatchedMarkerNew);
    } else {
      throw new Error(`NeXT AI Gateway MCP server parser has an unexpected shape: ${outputFile}`);
    }
  }

  const stdioModePatchedMarker = `Hwt(s.stdioMessageMode)||"newline-json"`;
  const stdioModePatchedMarkerNew = `tvt(s.stdioMessageMode)||"newline-json"`;
  if (!content.includes(stdioModePatchedMarker) && !content.includes(stdioModePatchedMarkerNew)) {
    const marker = `Hwt(s.stdioMessageMode)||"content-length"`;
    const markerNew = `tvt(s.stdioMessageMode)||"content-length"`;
    if (content.includes(marker)) {
      content = content.replace(marker, stdioModePatchedMarker);
    } else if (content.includes(markerNew)) {
      content = content.replace(markerNew, stdioModePatchedMarkerNew);
    } else {
      throw new Error(`NeXT AI Gateway stdio message mode parser has an unexpected shape: ${outputFile}`);
    }
  }

  if (
    !(content.includes(enabledPatchedMarker) || content.includes(enabledPatchedMarkerNew)) ||
    !(content.includes(stdioModePatchedMarker) || content.includes(stdioModePatchedMarkerNew))
  ) {
    throw new Error(`NeXT AI Gateway MCP server parser has an unexpected shape: ${outputFile}`);
  }
  content = patchNextAiGatewayDeepSeekThinking(content, outputFile);
  writeFileSync(outputFile, content);
}

function patchNextAiGatewayDeepSeekThinking(content, outputFile) {
  if (!content.includes("__codexlDeepSeekThinkingModels")) {
    const marker = `function dvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1))return{enabled:!0}}function fvt`;
    const replacement =
      `function dvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1)){let e=__codexlDeepSeekThinkingModels(t);return{enabled:!0,...e.length>0?{models:e}:void 0}}}` +
      `function __codexlDeepSeekThinkingModels(t){let e=t?.models??t?.model,n=Array.isArray(e)?e:typeof e=="string"?e.split(","):[];return n.map(r=>typeof r=="string"?r.trim():"").filter(Boolean)}` +
      `function fvt`;
    if (!content.includes(marker)) {
      throw new Error(`NeXT AI Gateway deepseek thinking parser has an unexpected shape: ${outputFile}`);
    }
    content = content.replace(marker, replacement);
  }

  if (!content.includes("x2e(t.deepseekThinking)")) {
    const marker = `t.deepseekThinking?.enabled?x2e():void 0`;
    if (!content.includes(marker)) {
      throw new Error(`NeXT AI Gateway deepseek thinking config plugin has an unexpected shape: ${outputFile}`);
    }
    content = content.replace(marker, `t.deepseekThinking?.enabled?x2e(t.deepseekThinking):void 0`);
  }

  if (!content.includes("__codexlDeepSeekModelSet")) {
    const marker =
      `function x2e(){return{key:F9t,provider:"openai",transformRequest(t){if(!U9t(t))return{ok:!0,value:t.upstreamRequest};let e=Vk(t.upstreamRequest.body)?{...t.upstreamRequest.body}:void 0;if(!e)return{ok:!0,value:t.upstreamRequest};let n=G9t(t,e),r=V9t(t,e);return!n&&!r?{ok:!0,value:t.upstreamRequest}:n==="disabled"?(e.thinking={type:"disabled"},delete e.reasoning_effort,C2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}}):(e.thinking={type:n||"enabled"},r&&(e.reasoning_effort=r),C2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}})}}}function U9t`;
    const replacement =
      `function x2e(t={}){let e=__codexlDeepSeekModelSet(t.models);return{key:F9t,provider:"openai",transformRequest(n){if(!U9t(n))return{ok:!0,value:n.upstreamRequest};let r=Vk(n.upstreamRequest.body)?{...n.upstreamRequest.body}:void 0;if(!r)return{ok:!0,value:n.upstreamRequest};if(e&&!e.has(__codexlDeepSeekRequestModel(n,r)))return{ok:!0,value:n.upstreamRequest};let s=G9t(n,r),i=V9t(n,r);return!s&&!i?{ok:!0,value:n.upstreamRequest}:s==="disabled"?(r.thinking={type:"disabled"},delete r.reasoning_effort,C2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}}):(r.thinking={type:s||"enabled"},i&&(r.reasoning_effort=i),C2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}})}}}` +
      `function __codexlDeepSeekModelSet(t){let e=Array.isArray(t)?t:typeof t=="string"?t.split(","):[];if(e.length===0)return;let n=new Set;for(let r of e){let s=__codexlDeepSeekModelKey(r);s&&n.add(s)}return n.size>0?n:void 0}` +
      `function __codexlDeepSeekRequestModel(t,e){return __codexlDeepSeekModelKey(t.model||t.standardRequest?.model||e?.model||mK(t).model)}` +
      `function __codexlDeepSeekModelKey(t){if(typeof t!="string")return"";let e=t.trim().replace(/^\\/+/, "").toLowerCase(),n=e.split("/").pop()||e;return n}` +
      `function U9t`;
    if (!content.includes(marker)) {
      throw new Error(`NeXT AI Gateway deepseek thinking plugin has an unexpected shape: ${outputFile}`);
    }
    content = content.replace(marker, replacement);
  }

  return content;
}

function reuseExistingBundleOrThrow(outputFile, envName) {
  if (existsSync(outputFile)) {
    console.warn(`${envName} is not set; reusing existing bundle: ${outputFile}`);
    return;
  }
  throw new Error(`${envName} is not set and no existing bundle was found: ${outputFile}`);
}

function writeTarGz(archivePath, rootDir, includeEntries) {
  const chunks = [];
  for (const entry of includeEntries) {
    addTarPath(chunks, rootDir, normalizeTarPath(entry));
  }
  chunks.push(Buffer.alloc(1024));
  writeFileSync(archivePath, gzipSync(Buffer.concat(chunks), { level: 9 }));
}

function addTarPath(chunks, rootDir, relativePath) {
  const fullPath = join(rootDir, relativePath);
  const stats = statSync(fullPath);
  const tarPath = normalizeTarPath(relativePath);

  if (stats.isDirectory()) {
    const directoryPath = tarPath.endsWith("/") ? tarPath : `${tarPath}/`;
    chunks.push(tarHeader(directoryPath, 0, "5", 0o755, stats.mtimeMs));
    const children = readdirSync(fullPath).sort((left, right) => left.localeCompare(right));
    for (const child of children) {
      addTarPath(chunks, rootDir, `${tarPath}/${child}`);
    }
    return;
  }

  if (!stats.isFile()) {
    throw new Error(`Unsupported built-in plugin package entry: ${fullPath}`);
  }

  const content = readFileSync(fullPath);
  chunks.push(tarHeader(tarPath, content.length, "0", stats.mode & 0o777, stats.mtimeMs));
  chunks.push(content);
  chunks.push(Buffer.alloc(pad512(content.length)));
}

function tarHeader(name, size, typeflag, mode, mtimeMs) {
  const header = Buffer.alloc(512);
  const encodedName = Buffer.from(name, "utf8");
  if (encodedName.length > 100) {
    throw new Error(`Built-in plugin package path is too long for ustar header: ${name}`);
  }

  writeString(header, name, 0, 100);
  writeOctal(header, mode, 100, 8);
  writeOctal(header, 0, 108, 8);
  writeOctal(header, 0, 116, 8);
  writeOctal(header, size, 124, 12);
  writeOctal(header, Math.floor(mtimeMs / 1000), 136, 12);
  header.fill(0x20, 148, 156);
  header[156] = typeflag.charCodeAt(0);
  writeString(header, "ustar", 257, 6);
  writeString(header, "00", 263, 2);
  writeString(header, "codexl", 265, 32);
  writeString(header, "codexl", 297, 32);

  let checksum = 0;
  for (const byte of header) {
    checksum += byte;
  }
  writeChecksum(header, checksum);
  return header;
}

function writeString(buffer, value, offset, length) {
  const bytes = Buffer.from(value, "utf8");
  if (bytes.length > length) {
    throw new Error(`tar header field is too long: ${value}`);
  }
  bytes.copy(buffer, offset);
}

function writeOctal(buffer, value, offset, length) {
  const text = Math.trunc(value).toString(8).padStart(length - 1, "0");
  buffer.write(text.slice(-(length - 1)), offset, length - 1, "ascii");
  buffer[offset + length - 1] = 0;
}

function writeChecksum(buffer, checksum) {
  const text = checksum.toString(8).padStart(6, "0");
  buffer.write(text.slice(-6), 148, 6, "ascii");
  buffer[154] = 0;
  buffer[155] = 0x20;
}

function pad512(size) {
  const remainder = size % 512;
  return remainder === 0 ? 0 : 512 - remainder;
}

function normalizeTarPath(value) {
  const normalized = value
    .split(/[\\/]+/)
    .filter(Boolean)
    .join("/");
  if (!normalized || normalized === "." || normalized.startsWith("../") || normalized.includes("/../")) {
    throw new Error(`Unsafe built-in plugin package path: ${value}`);
  }
  return normalized;
}
