#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const repoRoot = resolve(import.meta.dirname, "..");
const versionFiles = [
  "package.json",
  "src-tauri/tauri.conf.json",
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
];

const args = process.argv.slice(2);
const positionalArgs = args.filter((arg) => !arg.startsWith("--"));
const versionArg = positionalArgs[0];
const flags = new Set(args.filter((arg) => arg.startsWith("--")));
const supportedFlags = new Set(["--dry-run", "--no-push", "--allow-dirty"]);
const originalFileContents = new Map();

for (const flag of flags) {
  if (!supportedFlags.has(flag)) {
    fail(`Unsupported flag: ${flag}`);
  }
}

if (!versionArg) {
  fail("Usage: pnpm release v1.0.1 [--dry-run] [--no-push] [--allow-dirty]");
}

if (positionalArgs.length > 1) {
  fail(`Expected one release version, received: ${positionalArgs.join(", ")}`);
}

const release = parseReleaseTag(versionArg);

if (!release) {
  fail(`Release version must be a semantic version tag like v1.0.1 or 1.0.1. Received: ${versionArg}`);
}

const dryRun = flags.has("--dry-run");
const noPush = flags.has("--no-push");
const allowDirty = flags.has("--allow-dirty");

if (dryRun) {
  process.on("exit", () => revertVersionChanges());
}

ensureGitRepository();
ensureCleanWorktree();
ensureTagDoesNotExist(release.tag);
updateVersions(release.version);
run(["node", "scripts/verify-release-version.mjs", release.tag]);

const changedFiles = git(["diff", "--name-only", "--", ...versionFiles], { capture: true })
  .stdout.trim()
  .split("\n")
  .filter(Boolean);

if (dryRun) {
  console.log(`Dry run complete. ${release.tag} would update: ${changedFiles.length > 0 ? changedFiles.join(", ") : "(no file changes)"}`);
  revertVersionChanges();
  process.exit(0);
}

if (changedFiles.length > 0) {
  git(["add", ...versionFiles]);
  git(["commit", "-m", `chore: release ${release.tag}`]);
} else {
  console.log(`Versions already match ${release.version}; creating an empty release commit.`);
  git(["commit", "--allow-empty", "-m", `chore: release ${release.tag}`]);
}

git(["tag", "-a", release.tag, "-m", `Release ${release.tag}`]);

if (noPush) {
  console.log(`Release commit and tag are ready locally: ${release.tag}`);
  process.exit(0);
}

const branch = git(["branch", "--show-current"], { capture: true }).stdout.trim();

if (!branch) {
  fail("Cannot push from a detached HEAD. Run this release command from a branch.");
}

git(["push", "origin", branch]);
git(["push", "origin", release.tag]);

console.log(`Release pushed: ${release.tag}`);

function parseReleaseTag(value) {
  const match = value.match(/^v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$/);
  if (!match) {
    return null;
  }
  return {
    version: match[1],
    tag: `v${match[1]}`,
  };
}

function ensureGitRepository() {
  const topLevel = git(["rev-parse", "--show-toplevel"], { capture: true }).stdout.trim();
  if (topLevel !== repoRoot) {
    fail(`Release script must run from repo root ${repoRoot}. Git top-level is ${topLevel}.`);
  }
}

function ensureCleanWorktree() {
  if (allowDirty) {
    return;
  }

  const status = git(["status", "--porcelain"], { capture: true }).stdout.trim();

  if (status) {
    fail("Working tree must be clean before creating a release. Commit or stash local changes, or pass --allow-dirty.");
  }
}

function ensureTagDoesNotExist(tag) {
  const localTag = git(["rev-parse", "--verify", "--quiet", `refs/tags/${tag}`], { allowFailure: true });
  if (localTag.status === 0) {
    fail(`Local tag already exists: ${tag}`);
  }

  if (dryRun) {
    return;
  }

  const remoteTag = git(["ls-remote", "--exit-code", "--tags", "origin", `refs/tags/${tag}`], {
    allowFailure: true,
    capture: true,
  });

  if (remoteTag.status === 0) {
    fail(`Remote tag already exists on origin: ${tag}`);
  }

  if (remoteTag.status !== 2) {
    fail(`Unable to check remote tag ${tag} on origin.\n${remoteTag.stderr.trim()}`);
  }
}

function updateVersions(version) {
  updateJsonVersionFile("package.json", version);
  updateJsonVersionFile("src-tauri/tauri.conf.json", version);
  updateCargoTomlVersion("src-tauri/Cargo.toml", version);
  updateCargoLockPackageVersion("src-tauri/Cargo.lock", "codexl", version);
}

function updateJsonVersionFile(path, version) {
  const fullPath = resolve(repoRoot, path);
  const raw = readFileSync(fullPath, "utf8");
  JSON.parse(raw);

  const versionPattern = /^(\s*"version"\s*:\s*")[^"]+(")(\s*,?)/m;
  const next = raw.replace(versionPattern, `$1${version}$2$3`);

  if (next === raw && !versionPattern.test(raw)) {
    fail(`${path} must contain a version field.`);
  }

  JSON.parse(next);
  writeIfChanged(fullPath, raw, next);
}

function updateCargoTomlVersion(path, version) {
  const fullPath = resolve(repoRoot, path);
  const raw = readFileSync(fullPath, "utf8");
  const lines = raw.split("\n");
  let inPackage = false;
  let updated = false;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];

    if (/^\s*\[package\]\s*$/.test(line)) {
      inPackage = true;
      continue;
    }

    if (inPackage && /^\s*\[/.test(line)) {
      break;
    }

    if (inPackage && /^\s*version\s*=/.test(line)) {
      lines[index] = line.replace(/^(\s*version\s*=\s*)"[^"]+"(.*)$/, `$1"${version}"$2`);
      updated = true;
      break;
    }
  }

  if (!updated) {
    fail(`${path} must contain a package version field.`);
  }

  writeIfChanged(fullPath, raw, lines.join("\n"));
}

function updateCargoLockPackageVersion(path, packageName, version) {
  const fullPath = resolve(repoRoot, path);
  const raw = readFileSync(fullPath, "utf8");
  const packagePattern = new RegExp(
    `(\\[\\[package\\]\\]\\nname = "${escapeRegExp(packageName)}"\\nversion = ")[^"]+(")`,
  );
  const next = raw.replace(packagePattern, `$1${version}$2`);

  if (next === raw && !packagePattern.test(raw)) {
    fail(`${path} must contain package ${packageName}.`);
  }

  writeIfChanged(fullPath, raw, next);
}

function writeIfChanged(path, raw, next) {
  if (raw !== next) {
    if (!originalFileContents.has(path)) {
      originalFileContents.set(path, raw);
    }
    writeFileSync(path, next);
  }
}

function revertVersionChanges() {
  for (const [path, content] of originalFileContents) {
    writeFileSync(path, content);
  }
}

function run(commandArgs) {
  const executable = commandArgs[0] === "node" ? process.execPath : commandArgs[0];
  const result = spawnSync(executable, commandArgs.slice(1), {
    cwd: repoRoot,
    stdio: "inherit",
  });

  if (result.status !== 0) {
    fail(`Command failed: ${commandArgs.join(" ")}`);
  }
}

function git(args, options = {}) {
  const result = spawnSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: options.capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });

  if (!options.allowFailure && result.status !== 0) {
    fail(`Git command failed: git ${args.join(" ")}`);
  }

  return result;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
