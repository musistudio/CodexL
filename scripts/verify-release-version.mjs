import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const repoRoot = resolve(import.meta.dirname, "..");
const rawTag = process.argv[2] || process.env.GITHUB_REF_NAME || "";
const tag = rawTag.replace(/^refs\/tags\//, "");
const versionFromTag = parseReleaseTag(tag);

if (!versionFromTag) {
  fail(`Release tag must be a semantic version tag like v0.1.0 or 0.1.0. Received: ${rawTag || "(empty)"}`);
}

const versions = {
  "package.json": readPackageVersion("package.json"),
  "src-tauri/tauri.conf.json": readPackageVersion("src-tauri/tauri.conf.json"),
  "src-tauri/Cargo.toml": readCargoVersion("src-tauri/Cargo.toml"),
  "src-tauri/Cargo.lock": readCargoLockPackageVersion("src-tauri/Cargo.lock", "codexl"),
};

const mismatches = Object.entries(versions).filter(([, version]) => version !== versionFromTag);

if (mismatches.length > 0) {
  const details = Object.entries(versions)
    .map(([file, version]) => `  - ${file}: ${version}`)
    .join("\n");
  fail(`Release tag ${tag} resolves to version ${versionFromTag}, but app versions do not match:\n${details}`);
}

console.log(`Release version verified: ${versionFromTag}`);

function parseReleaseTag(value) {
  const match = value.match(/^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$/);
  return match?.[1] ?? null;
}

function readPackageVersion(path) {
  const json = JSON.parse(readFileSync(resolve(repoRoot, path), "utf8"));
  if (typeof json.version !== "string" || json.version.trim() === "") {
    fail(`${path} must contain a non-empty version field.`);
  }
  return json.version.trim();
}

function readCargoVersion(path) {
  const content = readFileSync(resolve(repoRoot, path), "utf8");
  const match = content.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) {
    fail(`${path} must contain a package version field.`);
  }
  return match[1].trim();
}

function readCargoLockPackageVersion(path, packageName) {
  const content = readFileSync(resolve(repoRoot, path), "utf8");
  const packageBlock = findCargoLockPackageBlock(content, packageName);
  if (!packageBlock) {
    fail(`${path} must contain package ${packageName}.`);
  }

  const version = readCargoLockBlockField(packageBlock, "version");
  if (!version) {
    fail(`${path} package ${packageName} must contain a version field.`);
  }

  return version;
}

function findCargoLockPackageBlock(content, packageName) {
  const blocks = content.split(/\r?\n(?=\[\[package\]\]\r?\n)/);
  return blocks.find((block) => readCargoLockBlockField(block, "name") === packageName);
}

function readCargoLockBlockField(block, fieldName) {
  for (const line of block.split(/\r?\n/)) {
    const match = line.match(new RegExp(`^\\s*${fieldName}\\s*=\\s*"([^"]+)"\\s*$`));
    if (match) {
      return match[1].trim();
    }
  }
  return null;
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
