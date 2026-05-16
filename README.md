# CodexL

CodexL is a Tauri desktop launcher for managing multiple local Codex App workspaces, with LAN remote control, optional cloud relay, and built-in extension integrations.

![https://codexl.io/_next/static/media/ui-preview.0-xm_pna~f-hv.webp](https://codexl.io/_next/static/media/ui-preview.0-xm_pna~f-hv.webp)

中文文档见 [README_zh.md](README_zh.md).

## Overview

CodexL is built for local developers who need to keep several Codex workspaces ready at the same time. It can prepare separate Codex homes, model-provider settings, and proxy settings for each workspace, then launch, stop, and manage Codex App from one desktop UI.

Highlights:

- Manage multiple Codex workspaces, with generated Codex homes for non-default workspaces.
- Launch Codex App with CDP, CLI middleware, model-provider, proxy, and language settings.
- Start mobile remote control per workspace with LAN QR URLs, access tokens, optional passwords, and cloud-relay end-to-end encryption.
- Ship built-in Bot Gateway and NeXT AI Gateway extensions for IM-platform integrations and provider protocol conversion.
- Include a mobile remote-control PWA that can scan a QR code or open a tokenized URL directly.
- Support Tauri updater artifacts for in-app update checks.

## Repository Layout

| Path | Purpose |
| --- | --- |
| `src/` | Desktop React UI. |
| `src-tauri/` | Tauri/Rust backend for launching Codex, configuration, HTTP/CDP proxying, remote control, and built-in extensions. |
| `remote/control-pwa/` | Mobile remote-control PWA. |
| `extensions/builtins/` | Built-in Bot Gateway and NeXT AI Gateway extension fixtures. |
| `src-tauri/builtin-plugin-packages/` | Built-in extension `.tar.gz` packages bundled into the Tauri app. |
| `scripts/` | Scripts for release checks, PWA publishing, icon generation, and built-in extension packaging. |

## Requirements

- Node.js 20+.
- pnpm 9.x. This repository declares `pnpm@9.15.1`.
- Rust toolchain.
- System dependencies required by Tauri 2.
- Codex App installed locally. On macOS, CodexL tries to discover `Codex.app` or `OpenAI Codex.app` automatically.

## Local Development

Install dependencies:

```sh
pnpm install
```

Start the desktop development app:

```sh
pnpm tauri dev
```

Common checks:

```sh
pnpm run build
cd src-tauri && cargo check
```

Common packaging commands:

```sh
pnpm tauri build
pnpm run build:pwa
pnpm run package:builtin-plugins
```

## Remote-Control PWA

The desktop app starts a remote-control service for a workspace. By default, it listens on:

```text
0.0.0.0:3147
```

Open the QR URL from a workspace card on your phone, for example:

```text
http://192.168.1.10:3147/?token=...
```

The token in the remote URL is sensitive. For more details about scanning, cached web resources, Cloudflare Pages publishing, and HTTPS constraints, see `remote/control-pwa/README.md`.

## Built-In Extensions

This repository includes two optional extensions:

| Extension | Purpose |
| --- | --- |
| Bot Gateway | Connect Codex to IM platforms, with Bot login, message forwarding, and handoff settings. |
| NeXT AI Gateway | Convert other protocol interfaces into providers that Codex can use. |

Extensions are disabled by default and can be enabled in app settings. When extensions are enabled, the runtime requires Node.js 20+.

## Desktop Release and Auto Update

The repository includes `.github/workflows/release.yml`. After a tag is pushed, GitHub Actions builds macOS Apple Silicon, macOS Intel, and Windows x64 installers, creates a GitHub Release, and uploads the `latest.json` file used by the Tauri updater.

Before the first release, configure these secrets in `Settings -> Secrets and variables -> Actions` for the GitHub repository:

| Secret | Purpose |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key. It must match `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Private-key password. If the key was generated without a password, this can be omitted or left empty. |

To regenerate the updater signing key:

```sh
pnpm tauri signer generate --ci -w .secrets/codexl-updater.key
```

Write the generated public key to `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`, and write the contents of `.secrets/codexl-updater.key` to the GitHub Secret `TAURI_SIGNING_PRIVATE_KEY`.

Release flow:

```sh
pnpm release v1.0.1
```

The release command updates the app versions, creates a release commit, creates an annotated tag, and pushes both the branch and tag to `origin`. Use `--dry-run` to preview the version changes, or `--no-push` to leave the commit and tag local. CI rejects tags where the versions do not match. After a release is published, the in-app update checker reads:

```text
https://github.com/musistudio/codexl/releases/latest/download/latest.json
```

It then downloads the signed update package for the current platform, installs it, and restarts the app.

## Runtime Paths

```text
~/.codexl/config.json                         # Main CodexL config
~/.codexl/codex-homes/<workspace-slug>/       # Codex home for non-default workspaces
~/.codexl/bin/codexl-codex-cli-middleware     # Codex CLI middleware
~/.codexl/extensions/<extension>/<version>/   # Built-in extension install directory
~/.codexl/next-ai-gateway/gateway.config.json # Default Gateway config
```

## Common Environment Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `CODEXL_CONFIG_PATH` | `~/.codexl/config.json` | Override the CodexL config file path. |
| `CODEXL_CODEX_HOME` / `CODEX_HOME` | `~/.codex` | Override the default Codex home. |
| `CODEXL_CODEX_PATH` | Empty | Manually set the Codex App executable path. |
| `CODEXL_CDP_HOST` | `127.0.0.1` | Codex CDP host. |
| `CODEXL_CDP_PORT` | `9222` | Starting Codex CDP port. |
| `CODEXL_HTTP_HOST` | `0.0.0.0` | Local HTTP proxy host. |
| `CODEXL_HTTP_PORT` | `14588` | Local HTTP proxy port. |
| `CODEXL_REMOTE_CONTROL_HOST` | `0.0.0.0` | Remote-control service host. |
| `CODEXL_REMOTE_CONTROL_PORT` | `3147` | Starting remote-control service port. |
| `CODEXL_LANGUAGE` | `en` | Default UI language, either `en` or `zh`. |
| `CODEXL_APPEARANCE` | `system` | Default appearance: `system`, `light`, or `dark`. |
| `CODEXL_EXTENSIONS_ENABLED` | `false` | Whether the extension master switch is enabled by default. |

## Security Boundaries

- Treat QR tokens as sensitive. Do not screenshot or share them with untrusted parties.
- `/json/*`, `/devtools/*`, and `/web/_bridge` can control or affect Codex App. Do not expose them to untrusted networks.
- LAN remote control listens on `0.0.0.0` by default and should only be used on trusted local networks.
- When cloud relay is enabled, end-to-end encryption requires a remote password; the password does not make the token non-sensitive.

## License

Copyright (C) 2026 musistudio.

CodexL is licensed under the GNU Affero General Public License version 3 only. See [LICENSE](LICENSE) for the full text.
