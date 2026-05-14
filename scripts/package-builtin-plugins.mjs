import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const builtinPluginsDir = join(repoRoot, "extensions", "builtins");
const packageDir = join(repoRoot, "src-tauri", "builtin-plugin-packages");
const nextAiGatewaySourceDir = resolve(
  process.env.NEXT_AI_GATEWAY_SOURCE_DIR,
);
const botGatewaySourceDir = resolve(
  process.env.BOT_GATEWAY_SOURCE_DIR,
);

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
  execFileSync("tar", ["-czf", archivePath, "-C", pluginDir, ...plugin.include], {
    env: { ...process.env, LANG: "C", LC_ALL: "C" },
    stdio: "inherit",
  });

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

function syncBotGateway(pluginDir) {
  const sourceBundle = join(botGatewaySourceDir, "dist-bundle", "stdio", "stdio.js");
  const outputFile = join(pluginDir, "stdio", "stdio.js");

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
  const entryPoint = join(nextAiGatewaySourceDir, "src", "index.ts");
  const esbuild = join(
    nextAiGatewaySourceDir,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "esbuild.cmd" : "esbuild",
  );
  const outputFile = join(pluginDir, "gateway", "index.cjs");

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
}
