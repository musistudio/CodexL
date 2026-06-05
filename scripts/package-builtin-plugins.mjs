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
  const enabledPatchedMarkerCurrent = `if(!Xe(r))continue;let s=r;if(Vi(s.enabled)===!1)continue;let i=rvt(s.transport)||"stdio"`;
  const enabledPatchedMarkerLatest = `if(!We(r))continue;let s=r;if(_i(s.enabled)===!1)continue;let i=svt(s.transport)||"stdio"`;
  if (
    !content.includes(enabledPatchedMarker) &&
    !content.includes(enabledPatchedMarkerNew) &&
    !content.includes(enabledPatchedMarkerCurrent) &&
    !content.includes(enabledPatchedMarkerLatest)
  ) {
    const marker = `if(!Xe(r))continue;let s=r,i=zwt(s.transport)||"stdio"`;
    const markerNew = `if(!Xe(r))continue;let s=r,i=evt(s.transport)||"stdio"`;
    const markerCurrent = `if(!Xe(r))continue;let s=r,i=rvt(s.transport)||"stdio"`;
    const markerLatest = `if(!We(r))continue;let s=r,i=svt(s.transport)||"stdio"`;
    if (content.includes(marker)) {
      content = content.replace(marker, enabledPatchedMarker);
    } else if (content.includes(markerNew)) {
      content = content.replace(markerNew, enabledPatchedMarkerNew);
    } else if (content.includes(markerCurrent)) {
      content = content.replace(markerCurrent, enabledPatchedMarkerCurrent);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, enabledPatchedMarkerLatest);
    } else {
      throw new Error(`NeXT AI Gateway MCP server parser has an unexpected shape: ${outputFile}`);
    }
  }

  const stdioModePatchedMarker = `Hwt(s.stdioMessageMode)||"newline-json"`;
  const stdioModePatchedMarkerNew = `tvt(s.stdioMessageMode)||"newline-json"`;
  const stdioModePatchedMarkerCurrent = `svt(s.stdioMessageMode)||"newline-json"`;
  const stdioModePatchedMarkerLatest = `ivt(s.stdioMessageMode)||"newline-json"`;
  if (
    !content.includes(stdioModePatchedMarker) &&
    !content.includes(stdioModePatchedMarkerNew) &&
    !content.includes(stdioModePatchedMarkerCurrent) &&
    !content.includes(stdioModePatchedMarkerLatest)
  ) {
    const marker = `Hwt(s.stdioMessageMode)||"content-length"`;
    const markerNew = `tvt(s.stdioMessageMode)||"content-length"`;
    const markerCurrent = `svt(s.stdioMessageMode)||"content-length"`;
    const markerLatest = `ivt(s.stdioMessageMode)||"content-length"`;
    if (content.includes(marker)) {
      content = content.replace(marker, stdioModePatchedMarker);
    } else if (content.includes(markerNew)) {
      content = content.replace(markerNew, stdioModePatchedMarkerNew);
    } else if (content.includes(markerCurrent)) {
      content = content.replace(markerCurrent, stdioModePatchedMarkerCurrent);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, stdioModePatchedMarkerLatest);
    } else {
      throw new Error(`NeXT AI Gateway stdio message mode parser has an unexpected shape: ${outputFile}`);
    }
  }

  if (
    !(content.includes(enabledPatchedMarker) || content.includes(enabledPatchedMarkerNew) || content.includes(enabledPatchedMarkerCurrent) || content.includes(enabledPatchedMarkerLatest)) ||
    !(content.includes(stdioModePatchedMarker) || content.includes(stdioModePatchedMarkerNew) || content.includes(stdioModePatchedMarkerCurrent) || content.includes(stdioModePatchedMarkerLatest))
  ) {
    throw new Error(`NeXT AI Gateway MCP server parser has an unexpected shape: ${outputFile}`);
  }
  content = patchNextAiGatewayDeepSeekThinking(content, outputFile);
  content = patchNextAiGatewayCodexModelRewritePlugin(content, outputFile);
  content = patchNextAiGatewayRawTraceFallback(content, outputFile);
  writeFileSync(outputFile, content);
}

function patchNextAiGatewayDeepSeekThinking(content, outputFile) {
  if (!content.includes("__codexlDeepSeekThinkingModels")) {
    const marker = `function dvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1))return{enabled:!0}}function fvt`;
    const markerCurrent = `function mvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1))return{enabled:!0}}function hvt`;
    const markerLatest = `function hvt(t){if(t===!0)return{enabled:!0};if(!(!We(t)||_i(t.enabled)===!1))return{enabled:!0}}function gvt`;
    const replacement =
      `function dvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1)){let e=__codexlDeepSeekThinkingModels(t);return{enabled:!0,...e.length>0?{models:e}:void 0}}}` +
      `function __codexlDeepSeekThinkingModels(t){let e=t?.models??t?.model,n=Array.isArray(e)?e:typeof e=="string"?e.split(","):[];return n.map(r=>typeof r=="string"?r.trim():"").filter(Boolean)}` +
      `function fvt`;
    const replacementCurrent =
      `function mvt(t){if(t===!0)return{enabled:!0};if(!(!Xe(t)||Vi(t.enabled)===!1)){let e=__codexlDeepSeekThinkingModels(t);return{enabled:!0,...e.length>0?{models:e}:void 0}}}` +
      `function __codexlDeepSeekThinkingModels(t){let e=t?.models??t?.model,n=Array.isArray(e)?e:typeof e=="string"?e.split(","):[];return n.map(r=>typeof r=="string"?r.trim():"").filter(Boolean)}` +
      `function hvt`;
    const replacementLatest =
      `function hvt(t){if(t===!0)return{enabled:!0};if(!(!We(t)||_i(t.enabled)===!1)){let e=__codexlDeepSeekThinkingModels(t);return{enabled:!0,...e.length>0?{models:e}:void 0}}}` +
      `function __codexlDeepSeekThinkingModels(t){let e=t?.models??t?.model,n=Array.isArray(e)?e:typeof e=="string"?e.split(","):[];return n.map(r=>typeof r=="string"?r.trim():"").filter(Boolean)}` +
      `function gvt`;
    if (content.includes(marker)) {
      content = content.replace(marker, replacement);
    } else if (content.includes(markerCurrent)) {
      content = content.replace(markerCurrent, replacementCurrent);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, replacementLatest);
    } else {
      throw new Error(`NeXT AI Gateway deepseek thinking parser has an unexpected shape: ${outputFile}`);
    }
  }

  if (!content.includes("x2e(t.deepseekThinking)") && !content.includes("I2e(t.deepseekThinking)") && !content.includes("P2e(t.deepseekThinking)")) {
    const marker = `t.deepseekThinking?.enabled?x2e():void 0`;
    const markerCurrent = `t.deepseekThinking?.enabled?I2e():void 0`;
    const markerLatest = `t.deepseekThinking?.enabled?P2e():void 0`;
    if (content.includes(marker)) {
      content = content.replace(marker, `t.deepseekThinking?.enabled?x2e(t.deepseekThinking):void 0`);
    } else if (content.includes(markerCurrent)) {
      content = content.replace(markerCurrent, `t.deepseekThinking?.enabled?I2e(t.deepseekThinking):void 0`);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, `t.deepseekThinking?.enabled?P2e(t.deepseekThinking):void 0`);
    } else {
      throw new Error(`NeXT AI Gateway deepseek thinking config plugin has an unexpected shape: ${outputFile}`);
    }
  }

  if (!content.includes("__codexlDeepSeekModelSet")) {
    const marker =
      `function x2e(){return{key:F9t,provider:"openai",transformRequest(t){if(!U9t(t))return{ok:!0,value:t.upstreamRequest};let e=Vk(t.upstreamRequest.body)?{...t.upstreamRequest.body}:void 0;if(!e)return{ok:!0,value:t.upstreamRequest};let n=G9t(t,e),r=V9t(t,e);return!n&&!r?{ok:!0,value:t.upstreamRequest}:n==="disabled"?(e.thinking={type:"disabled"},delete e.reasoning_effort,C2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}}):(e.thinking={type:n||"enabled"},r&&(e.reasoning_effort=r),C2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}})}}}function U9t`;
    const markerCurrent =
      `function I2e(){return{key:z9t,provider:"openai",transformRequest(t){if(!H9t(t))return{ok:!0,value:t.upstreamRequest};let e=zk(t.upstreamRequest.body)?{...t.upstreamRequest.body}:void 0;if(!e)return{ok:!0,value:t.upstreamRequest};let n=J9t(t,e),r=W9t(t,e);return!n&&!r?{ok:!0,value:t.upstreamRequest}:n==="disabled"?(e.thinking={type:"disabled"},delete e.reasoning_effort,A2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}}):(e.thinking={type:n||"enabled"},r&&(e.reasoning_effort=r),A2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}})}}}function H9t`;
    const markerLatest =
      `function P2e(){return{key:nYt,provider:"openai",transformRequest(t){if(!rYt(t))return{ok:!0,value:t.upstreamRequest};let e=Wk(t.upstreamRequest.body)?{...t.upstreamRequest.body}:void 0;if(!e)return{ok:!0,value:t.upstreamRequest};let n=sYt(t,e),r=iYt(t,e);return!n&&!r?{ok:!0,value:t.upstreamRequest}:n==="disabled"?(e.thinking={type:"disabled"},delete e.reasoning_effort,I2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}}):(e.thinking={type:n||"enabled"},r&&(e.reasoning_effort=r),I2e(e),{ok:!0,value:{...t.upstreamRequest,body:e}})}}}function rYt`;
    const replacement =
      `function x2e(t={}){let e=__codexlDeepSeekModelSet(t.models);return{key:F9t,provider:"openai",transformRequest(n){if(!U9t(n))return{ok:!0,value:n.upstreamRequest};let r=Vk(n.upstreamRequest.body)?{...n.upstreamRequest.body}:void 0;if(!r)return{ok:!0,value:n.upstreamRequest};if(e&&!e.has(__codexlDeepSeekRequestModel(n,r)))return{ok:!0,value:n.upstreamRequest};let s=G9t(n,r),i=V9t(n,r);return!s&&!i?{ok:!0,value:n.upstreamRequest}:s==="disabled"?(r.thinking={type:"disabled"},delete r.reasoning_effort,C2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}}):(r.thinking={type:s||"enabled"},i&&(r.reasoning_effort=i),C2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}})}}}` +
      `function __codexlDeepSeekModelSet(t){let e=Array.isArray(t)?t:typeof t=="string"?t.split(","):[];if(e.length===0)return;let n=new Set;for(let r of e){let s=__codexlDeepSeekModelKey(r);s&&n.add(s)}return n.size>0?n:void 0}` +
      `function __codexlDeepSeekRequestModel(t,e){return __codexlDeepSeekModelKey(t.model||t.standardRequest?.model||e?.model||mK(t).model)}` +
      `function __codexlDeepSeekModelKey(t){if(typeof t!="string")return"";let e=t.trim().replace(/^\\/+/, "").toLowerCase(),n=e.split("/").pop()||e;return n}` +
      `function U9t`;
    const replacementCurrent =
      `function I2e(t={}){let e=__codexlDeepSeekModelSet(t.models);return{key:z9t,provider:"openai",transformRequest(n){if(!H9t(n))return{ok:!0,value:n.upstreamRequest};let r=zk(n.upstreamRequest.body)?{...n.upstreamRequest.body}:void 0;if(!r)return{ok:!0,value:n.upstreamRequest};if(e&&!e.has(__codexlDeepSeekRequestModel(n,r)))return{ok:!0,value:n.upstreamRequest};let s=J9t(n,r),i=W9t(n,r);return!s&&!i?{ok:!0,value:n.upstreamRequest}:s==="disabled"?(r.thinking={type:"disabled"},delete r.reasoning_effort,A2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}}):(r.thinking={type:s||"enabled"},i&&(r.reasoning_effort=i),A2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}})}}}` +
      `function __codexlDeepSeekModelSet(t){let e=Array.isArray(t)?t:typeof t=="string"?t.split(","):[];if(e.length===0)return;let n=new Set;for(let r of e){let s=__codexlDeepSeekModelKey(r);s&&n.add(s)}return n.size>0?n:void 0}` +
      `function __codexlDeepSeekRequestModel(t,e){return __codexlDeepSeekModelKey(t.model||t.standardRequest?.model||e?.model)}` +
      `function __codexlDeepSeekModelKey(t){if(typeof t!="string")return"";let e=t.trim().replace(/^\\/+/, "").toLowerCase(),n=e.split("/").pop()||e;return n}` +
      `function H9t`;
    const replacementLatest =
      `function P2e(t={}){let e=__codexlDeepSeekModelSet(t.models);return{key:nYt,provider:"openai",transformRequest(n){if(!rYt(n))return{ok:!0,value:n.upstreamRequest};let r=Wk(n.upstreamRequest.body)?{...n.upstreamRequest.body}:void 0;if(!r)return{ok:!0,value:n.upstreamRequest};if(e&&!e.has(__codexlDeepSeekRequestModel(n,r)))return{ok:!0,value:n.upstreamRequest};let s=sYt(n,r),i=iYt(n,r);return!s&&!i?{ok:!0,value:n.upstreamRequest}:s==="disabled"?(r.thinking={type:"disabled"},delete r.reasoning_effort,I2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}}):(r.thinking={type:s||"enabled"},i&&(r.reasoning_effort=i),I2e(r),{ok:!0,value:{...n.upstreamRequest,body:r}})}}}` +
      `function __codexlDeepSeekModelSet(t){let e=Array.isArray(t)?t:typeof t=="string"?t.split(","):[];if(e.length===0)return;let n=new Set;for(let r of e){let s=__codexlDeepSeekModelKey(r);s&&n.add(s)}return n.size>0?n:void 0}` +
      `function __codexlDeepSeekRequestModel(t,e){return __codexlDeepSeekModelKey(t.model||t.standardRequest?.model||e?.model)}` +
      `function __codexlDeepSeekModelKey(t){if(typeof t!="string")return"";let e=t.trim().replace(/^\\/+/, "").toLowerCase(),n=e.split("/").pop()||e;return n}` +
      `function rYt`;
    if (content.includes(marker)) {
      content = content.replace(marker, replacement);
    } else if (content.includes(markerCurrent)) {
      content = content.replace(markerCurrent, replacementCurrent);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, replacementLatest);
    } else {
      throw new Error(`NeXT AI Gateway deepseek thinking plugin has an unexpected shape: ${outputFile}`);
    }
  }

  return content;
}

function patchNextAiGatewayCodexModelRewritePlugin(content, outputFile) {
  const functionName = "__codexlApplyCodexModelRewrite";
  if (content.includes(functionName)) {
    return content;
  }

  const helper =
    `var __codexlCodexModelRewriteTtlMs=216e5,__codexlCodexModelRewriteSessions=new Map;` +
    `function ${functionName}(t,e,n,r){if(n.adapterKey!=="openai_responses")return e;let s=__codexlCodexModelRewritePlugins(r);if(s.length===0)return e;let i=t7(t,e),o=ze(t.headers["x-target-model"]),a=o||Z$(e,n);if(!a)return e;let c=__codexlCodexModelRewriteTargetModel(s);if(c){if(a.trim()===c)return e;t.log.debug({fromModel:a,toModel:c,sessionId:i?.sessionId,headerModel:o},"Codex model rewrite plugin routed request to selected gateway model.");o&&(t.headers["x-target-model"]=c);return{...e,model:c}}if(i?.agentId!=="codex")return e;let u=Date.now();__codexlPruneCodexModelRewriteSessions(u);let l=__codexlCodexModelRewriteSessionKeys(t,i),d=_p(a,r.providers);if(__codexlIsRememberableCodexGatewayModel(a,d,s))return __codexlRememberCodexModelRewriteSession(l,{model:a,provider:d.provider,providerName:d.providerConfig?.name,expiresAt:u+__codexlCodexModelRewriteTtlMs}),e;if(!__codexlIsCodexInternalGptModel(a))return e;let m=__codexlFindCodexModelRewriteSession(l,u);if(!m||m.model===a)return e;t.log.debug({fromModel:a,toModel:m.model,sessionId:i.sessionId,providerName:m.providerName,headerModel:o},"Codex model rewrite plugin replaced internal GPT model with selected gateway model.");if(o){t.headers["x-target-model"]=m.model;return Z$(e,n)===a?{...e,model:m.model}:e}return{...e,model:m.model}}` +
    `function __codexlCodexModelRewritePlugins(t){return(t.providerPlugins||[]).filter(e=>e.enabled!==!1&&e.key==="codexl-codex-model-rewrite")}` +
    `function __codexlCodexModelRewriteTargetModel(t){for(let e of t){let n=e.codexModelRewrite?.targetModel,r=e.request?.headers?.["x-codexl-codex-model-rewrite-target"],s=typeof r=="string"?r:typeof n=="string"?n:"";if(s.trim())return s.trim()}}` +
    `function __codexlCodexModelRewriteSessionKeys(t,e){let n=ze(t.headers["x-codex-account-id"]),r=[];return e.sessionId&&r.push(["codex",n,e.sessionId].filter(Boolean).join(":")),n&&r.push(["codex",n].join(":")),r.length===0&&r.push("codex"),[...new Set(r)]}` +
    `function __codexlRememberCodexModelRewriteSession(t,e){for(let n of t)__codexlCodexModelRewriteSessions.set(n,e)}` +
    `function __codexlFindCodexModelRewriteSession(t,e){for(let n of t){let r=__codexlCodexModelRewriteSessions.get(n);if(!r)continue;if(r.expiresAt<=e){__codexlCodexModelRewriteSessions.delete(n);continue}return r}}` +
    `function __codexlIsRememberableCodexGatewayModel(t,e,n){return!t.includes("/")||!e?.provider?!1:n.some(r=>__codexlCodexModelRewritePluginMatchesReference(r,e))}` +
    `function __codexlCodexModelRewritePluginMatchesReference(t,e){if(t.provider&&t.provider!==e.provider)return!1;if(t.providerName){let n=t.providerName.trim().toLowerCase(),r=e.providerConfig?.name.trim().toLowerCase();return!!(n&&r===n)}return!0}` +
    `function __codexlIsCodexInternalGptModel(t){let e=t.trim().toLowerCase();return!e.includes("/")&&/^gpt(?:[-_]|$)/.test(e)}` +
    `function __codexlPruneCodexModelRewriteSessions(t){for(let[e,n]of __codexlCodexModelRewriteSessions.entries())n.expiresAt<=t&&__codexlCodexModelRewriteSessions.delete(e)}`;

  const insertionMarker = `async function Uk(t,e,n,r,s){`;
  if (!content.includes(insertionMarker)) {
    throw new Error(`NeXT AI Gateway Codex model rewrite plugin insertion point has an unexpected shape: ${outputFile}`);
  }
  content = content.replace(insertionMarker, `${helper}${insertionMarker}`);

  const handlerMarker =
    'let o=t.body;if(!P(o))return Bk(e,"Request body must be a JSON object.");let a={request:t,body:o,source:n,config:r},c=ze(t.headers["x-target-model"])||Z$(o,n),u=c?b5t(r,c):void 0,l=T5t(t,i.provider,r,u?.targetModelSelector||Z$(o,n));';
  const handlerReplacement =
    'let o=t.body;if(!P(o))return Bk(e,"Request body must be a JSON object.");let __codexlBody=__codexlApplyCodexModelRewrite(t,o,n,r);__codexlBody!==o&&(t.body=__codexlBody);let a={request:t,body:__codexlBody,source:n,config:r},c=ze(t.headers["x-target-model"])||Z$(__codexlBody,n),u=c?b5t(r,c):void 0,l=T5t(t,i.provider,r,u?.targetModelSelector||Z$(__codexlBody,n));';
  if (!content.includes(handlerMarker)) {
    throw new Error(`NeXT AI Gateway Codex model rewrite handler has an unexpected shape: ${outputFile}`);
  }

  content = content.replace(handlerMarker, handlerReplacement);

  const passthroughModelMarker = "let ue=GY(t,v,Z$(o,n),r);";
  const passthroughModelReplacement = "let ue=GY(t,v,Z$(__codexlBody,n),r);";
  if (!content.includes(passthroughModelMarker)) {
    throw new Error(
      `NeXT AI Gateway Codex model rewrite passthrough model resolution has an unexpected shape: ${outputFile}`,
    );
  }

  return content.replace(passthroughModelMarker, passthroughModelReplacement);
}

function patchNextAiGatewayRawTraceFallback(content, outputFile) {
  const functionName = "__codexlRawTraceFallback";
  const helper =
    `function ${functionName}(t,e,n,r){if(!r.rawTrace.enabled)return;let s=typeof t.method=="string"?t.method.toUpperCase():"",i;try{i=new URL(t.url,"http://gateway.local").pathname}catch{i=t.url||""}if(s==="OPTIONS"||!i.startsWith("/v1"))return;let h=typeof e.statusCode=="number"?e.statusCode:200;if(h<400)return;if(!IBe(t))return;let o=r.rawTrace.mode==="body_redacted"?"body_redacted":"none",a=r.rawTrace.mode==="wire_raw"?t.url:aK(t.url),c=typeof n=="string"?n:Buffer.isBuffer(n)?n.toString("utf8"):n,u=typeof e.getHeader=="function"?e.getHeader("content-type"):void 0,l=typeof u=="string"?u:Array.isArray(u)?u.join(", "):"application/json; charset=utf-8",d=typeof e.getHeaders=="function"?BFe(e.getHeaders()):void 0,m=P(t.body)?{model:typeof t.body.model=="string"?t.body.model:void 0}:void 0,f=[{partType:"client_request_metadata",content:H0(r.rawTrace.mode,{method:t.method,url:a,headers:BFe(t.headers)}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"client_request",content:H0(r.rawTrace.mode,PBe(t)??t.body),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response_metadata",content:H0(r.rawTrace.mode,{statusCode:h,headers:d}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response",content:H0(r.rawTrace.mode,c),contentType:l,redactionPolicy:o}];NBe({requestId:t.id,method:t.method,url:a,identity:t.gatewayIdentity,clientContext:P(t.body)?t7(t,t.body):void 0,target:m,parts:f})}`;
  const helperLatest =
    `function ${functionName}(t,e,n,r){if(!r.rawTrace.enabled)return;let s=typeof t.method=="string"?t.method.toUpperCase():"",i;try{i=new URL(t.url,"http://gateway.local").pathname}catch{i=t.url||""}if(s==="OPTIONS"||!i.startsWith("/v1"))return;let h=typeof e.statusCode=="number"?e.statusCode:200;if(h<400)return;if(!PBe(t))return;let o=r.rawTrace.mode==="body_redacted"?"body_redacted":"none",a=r.rawTrace.mode==="wire_raw"?t.url:cK(t.url),c=typeof n=="string"?n:Buffer.isBuffer(n)?n.toString("utf8"):n,u=typeof e.getHeader=="function"?e.getHeader("content-type"):void 0,l=typeof u=="string"?u:Array.isArray(u)?u.join(", "):"application/json; charset=utf-8",d=typeof e.getHeaders=="function"?FFe(e.getHeaders()):void 0,m=O(t.body)?{model:typeof t.body.model=="string"?t.body.model:void 0}:void 0,f=[{partType:"client_request_metadata",content:J0(r.rawTrace.mode,{method:t.method,url:a,headers:FFe(t.headers)}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"client_request",content:J0(r.rawTrace.mode,OBe(t)??t.body),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response_metadata",content:J0(r.rawTrace.mode,{statusCode:h,headers:d}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response",content:J0(r.rawTrace.mode,c),contentType:l,redactionPolicy:o}];MBe({requestId:t.id,method:t.method,url:a,identity:t.gatewayIdentity,clientContext:O(t.body)?lK(t,t.body):void 0,target:m,parts:f})}`;
  const previousHelper =
    `function ${functionName}(t,e,n,r){if(!r.rawTrace.enabled)return;let s=typeof t.method=="string"?t.method.toUpperCase():"",i;try{i=new URL(t.url,"http://gateway.local").pathname}catch{i=t.url||""}if(s==="OPTIONS"||!i.startsWith("/v1"))return;if(!IBe(t))return;let o=r.rawTrace.mode==="body_redacted"?"body_redacted":"none",a=r.rawTrace.mode==="wire_raw"?t.url:aK(t.url),c=typeof n=="string"?n:Buffer.isBuffer(n)?n.toString("utf8"):n,u=typeof e.getHeader=="function"?e.getHeader("content-type"):void 0,l=typeof u=="string"?u:Array.isArray(u)?u.join(", "):"application/json; charset=utf-8",d=typeof e.getHeaders=="function"?BFe(e.getHeaders()):void 0,m=P(t.body)?{model:typeof t.body.model=="string"?t.body.model:void 0}:void 0,f=[{partType:"client_request_metadata",content:H0(r.rawTrace.mode,{method:t.method,url:a,headers:BFe(t.headers)}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"client_request",content:H0(r.rawTrace.mode,PBe(t)??t.body),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response_metadata",content:H0(r.rawTrace.mode,{statusCode:e.statusCode,headers:d}),contentType:"application/json; charset=utf-8",redactionPolicy:o},{partType:"gateway_response",content:H0(r.rawTrace.mode,c),contentType:l,redactionPolicy:o}];NBe({requestId:t.id,method:t.method,url:a,identity:t.gatewayIdentity,clientContext:P(t.body)?t7(t,t.body):void 0,target:m,parts:f})}`;
  if (content.includes(previousHelper)) {
    content = content.replace(previousHelper, helper);
  }

  if (!content.includes(functionName)) {
    const marker = `function W5t(t){return t===429?"rate-limited":t===408||t===504?"timeout":typeof t=="number"&&t>=200&&t<400?"success":"error"}`;
    const markerLatest = `function i8t(t){return t===429?"rate-limited":t===408||t===504?"timeout":typeof t=="number"&&t>=200&&t<400?"success":"error"}`;
    if (content.includes(marker)) {
      content = content.replace(marker, `${helper}${marker}`);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, `${helperLatest}${markerLatest}`);
    } else {
      throw new Error(`NeXT AI Gateway raw trace fallback has an unexpected shape: ${outputFile}`);
    }
  }

  const onSendPatchedMarker = `ln.addHook("onSend",async(t,e,n)=>(LUe(e),${functionName}(t,e,n,rn),n));`;
  const onSendPatchedMarkerLatest = `fn.addHook("onSend",async(t,e,n)=>(qUe(e),${functionName}(t,e,n,nn),n));`;
  if (!content.includes(onSendPatchedMarker) && !content.includes(onSendPatchedMarkerLatest)) {
    const marker = `ln.addHook("onSend",async(t,e,n)=>(LUe(e),n));`;
    const markerLatest = `fn.addHook("onSend",async(t,e,n)=>(qUe(e),n));`;
    if (content.includes(marker)) {
      content = content.replace(marker, onSendPatchedMarker);
    } else if (content.includes(markerLatest)) {
      content = content.replace(markerLatest, onSendPatchedMarkerLatest);
    } else {
      throw new Error(`NeXT AI Gateway onSend hook has an unexpected shape: ${outputFile}`);
    }
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
