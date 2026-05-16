# CodexL

CodexL 是一个 Tauri 桌面启动器，用来管理本机 Codex App 的多个工作空间，并为每个工作空间提供 LAN 远控、可选云中继和内置扩展集成。

![https://codexl.io/_next/static/media/ui-preview.0-xm_pna~f-hv.webp](https://codexl.io/_next/static/media/ui-preview.0-xm_pna~f-hv.webp)

English documentation is available in [README.md](README.md).

## 项目概览

CodexL 面向需要同时维护多个 Codex 工作空间的本地开发者。它可以为不同 workspace 准备独立的 Codex home、模型供应商配置和代理设置，并通过桌面端统一启动、停止和管理 Codex App。

主要能力：

- 管理多个 Codex workspace，并为非默认 workspace 生成独立的 Codex home。
- 启动 Codex App 时注入 CDP、CLI middleware、模型供应商、代理和语言等运行参数。
- 为 workspace 启动手机远控服务，支持 LAN QR URL、访问 token、可选密码和云中继端到端加密。
- 内置 Bot Gateway 和 NeXT AI Gateway 扩展，用于连接 IM 平台或将其他协议接口转换给 Codex 使用。
- 提供远控 PWA，可扫码连接，也可直接打开带 token 的远控 URL。
- 支持 Tauri updater，Release 构建产物可通过应用内检查更新安装。

## 目录结构

| 路径 | 用途 |
| --- | --- |
| `src/` | 桌面端 React UI。 |
| `src-tauri/` | Tauri/Rust 后端，负责启动 Codex、配置、HTTP/CDP 代理、远控和内置扩展。 |
| `remote/control-pwa/` | 手机远控 PWA。 |
| `extensions/builtins/` | 内置 Bot Gateway 和 NeXT AI Gateway 扩展夹具。 |
| `src-tauri/builtin-plugin-packages/` | 打进 Tauri 包里的内置扩展 `.tar.gz`。 |
| `scripts/` | 发布、PWA 部署、图标和内置扩展打包脚本。 |

## 环境要求

- Node.js 20+。
- pnpm 9.x，本仓库声明的包管理器为 `pnpm@9.15.1`。
- Rust toolchain。
- Tauri 2 所需的系统依赖。
- 本机已安装 Codex App。macOS 上会尝试自动查找 `Codex.app` 或 `OpenAI Codex.app`。

## 本地开发

安装依赖：

```sh
pnpm install
```

启动桌面端开发环境：

```sh
pnpm tauri dev
```

常用检查：

```sh
pnpm run build
cd src-tauri && cargo check
```

常用打包：

```sh
pnpm tauri build
pnpm run build:pwa
pnpm run package:builtin-plugins
```

## 远控 PWA

根应用会为 workspace 启动远控服务，默认监听：

```text
0.0.0.0:3147
```

手机可以打开 workspace 卡片里的 QR URL，例如：

```text
http://192.168.1.10:3147/?token=...
```

远控 URL 中的 token 应视为敏感信息。远控 PWA 的更多细节，包括扫码、缓存 Web 资源、Cloudflare Pages 发布和 HTTPS 限制，见 `remote/control-pwa/README.md`。

## 内置扩展

仓库内置两个可选扩展：

| 扩展 | 用途 |
| --- | --- |
| Bot Gateway | 将 Codex 连接到 IM 平台，并支持 Bot 登录、消息转发和 handoff 配置。 |
| NeXT AI Gateway | 将其他协议接口转换为 Codex 可使用的 provider。 |

扩展默认关闭，可在应用设置中启用。启用扩展时，运行时需要 Node.js 20+。

## 桌面端发布和自动更新

仓库已配置 `.github/workflows/release.yml`。推送 tag 后，GitHub Actions 会构建 macOS Apple Silicon、macOS Intel 和 Windows x64 安装包，创建 GitHub Release，并上传 Tauri updater 使用的 `latest.json`。

首次发布前需要在 GitHub 仓库的 `Settings -> Secrets and variables -> Actions` 配置：

| Secret | 用途 |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater 私钥，必须和 `src-tauri/tauri.conf.json` 里的 `plugins.updater.pubkey` 匹配。 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码；如果生成私钥时没有密码，可以不设置或留空。 |

如果需要重新生成 updater 签名密钥：

```sh
pnpm tauri signer generate --ci -w .secrets/codexl-updater.key
```

把输出里的 public key 写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，把 `.secrets/codexl-updater.key` 的内容写入 GitHub Secret `TAURI_SIGNING_PRIVATE_KEY`。

发布流程：

```sh
pnpm release v1.0.1
```

`release` 命令会统一更新 app 版本号，创建 release commit，创建 annotated tag，并把当前分支和 tag 推到 `origin`。可以用 `--dry-run` 预览版本改动，或用 `--no-push` 只在本地创建 commit 和 tag。CI 会拒绝版本号不一致的 tag。Release 发布成功后，应用内“检查更新”会读取：

```text
https://github.com/musistudio/codexl/releases/latest/download/latest.json
```

并下载对应平台的签名更新包完成安装和重启。

## 相关运行时路径

```text
~/.codexl/config.json                         # CodexL 主配置
~/.codexl/codex-homes/<workspace-slug>/       # 非默认 workspace 的 Codex home
~/.codexl/bin/codexl-codex-cli-middleware     # Codex CLI middleware
~/.codexl/extensions/<extension>/<version>/   # 内置扩展安装目录
~/.codexl/next-ai-gateway/gateway.config.json # Gateway 默认配置
```

## 常用环境变量

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `CODEXL_CONFIG_PATH` | `~/.codexl/config.json` | 覆盖 CodexL 配置文件路径。 |
| `CODEXL_CODEX_HOME` / `CODEX_HOME` | `~/.codex` | 覆盖默认 Codex home。 |
| `CODEXL_CODEX_PATH` | 空 | 手动指定 Codex App 可执行文件路径。 |
| `CODEXL_CDP_HOST` | `127.0.0.1` | Codex CDP host。 |
| `CODEXL_CDP_PORT` | `9222` | Codex CDP 起始端口。 |
| `CODEXL_HTTP_HOST` | `0.0.0.0` | 本地 HTTP 代理 host。 |
| `CODEXL_HTTP_PORT` | `14588` | 本地 HTTP 代理端口。 |
| `CODEXL_REMOTE_CONTROL_HOST` | `0.0.0.0` | 远控服务 host。 |
| `CODEXL_REMOTE_CONTROL_PORT` | `3147` | 远控服务起始端口。 |
| `CODEXL_LANGUAGE` | `en` | 默认界面语言，可使用 `en` 或 `zh`。 |
| `CODEXL_APPEARANCE` | `system` | 默认外观，可使用 `system`、`light` 或 `dark`。 |
| `CODEXL_EXTENSIONS_ENABLED` | `false` | 是否默认启用扩展总开关。 |

## 安全边界

- QR token 应视为敏感信息，不要截图或分享给不可信对象。
- `/json/*`、`/devtools/*` 和 `/web/_bridge` 都能控制或影响 Codex App，不要暴露到不可信网络。
- LAN 远控默认监听 `0.0.0.0`，只应在可信局域网中使用。
- 启用云中继时，端到端加密需要远控密码；密码不会替代 token 的敏感性。

## 开源协议

Copyright (C) 2026 musistudio.

CodexL 采用 GNU Affero General Public License version 3 only 开源协议。完整协议文本见 [LICENSE](LICENSE)。
