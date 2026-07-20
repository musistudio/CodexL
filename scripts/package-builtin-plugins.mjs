import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const require = createRequire(import.meta.url);
const { buildSync: esbuildSync } = require("esbuild");
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const builtinPluginsDir = join(repoRoot, "extensions", "builtins");
const packageDir = join(repoRoot, "src-tauri", "builtin-plugin-packages");

const nextAiGatewayPackage = resolvePackage("@the-next-ai/ai-gateway", "1.0.6");
const botGatewayPackage = resolvePackage("@the-next-ai/bot-gateway", "1.0.0");

const plugins = [
  {
    name: "bot-gateway",
    include: ["plugin.json", "package.json", "stdio"],
    beforePackage: buildBotGateway,
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

function buildBotGateway(pluginDir) {
  bundleNodeEntry({
    entryPoint: join(botGatewayPackage.dir, "dist", "src", "stdio.js"),
    outputFile: join(pluginDir, "stdio", "stdio.js"),
    format: "esm",
    target: "node22",
    packageName: botGatewayPackage.name,
  });
}

function buildNextAiGateway(pluginDir) {
  bundleNodeEntry({
    entryPoint: join(nextAiGatewayPackage.dir, "dist", "index.js"),
    outputFile: join(pluginDir, "gateway", "index.cjs"),
    format: "cjs",
    target: "node20",
    packageName: nextAiGatewayPackage.name,
  });
}

function bundleNodeEntry({ entryPoint, outputFile, format, target, packageName }) {
  if (!existsSync(entryPoint)) {
    throw new Error(`${packageName} entry not found: ${entryPoint}`);
  }

  mkdirSync(dirname(outputFile), { recursive: true });
  rmSync(outputFile, { force: true });
  esbuildSync({
    entryPoints: [entryPoint],
    bundle: true,
    platform: "node",
    target,
    format,
    minify: true,
    legalComments: "none",
    logLevel: "warning",
    outfile: outputFile,
    absWorkingDir: repoRoot,
    ...(format === "esm"
      ? {
          banner: {
            js: 'import { createRequire as __codexlCreateRequire } from "node:module";import { fileURLToPath as __codexlFileURLToPath } from "node:url";import { dirname as __codexlDirname } from "node:path";const require = __codexlCreateRequire(import.meta.url);const __filename = __codexlFileURLToPath(import.meta.url);const __dirname = __codexlDirname(__filename);',
          },
        }
      : {}),
  });
}

function resolvePackage(packageName, expectedVersion) {
  const packageJsonPath = require.resolve(`${packageName}/package.json`);
  const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
  if (packageJson.version !== expectedVersion) {
    throw new Error(
      `${packageName} version mismatch: expected ${expectedVersion}, found ${packageJson.version}`,
    );
  }
  return {
    name: packageName,
    version: packageJson.version,
    dir: dirname(packageJsonPath),
  };
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
