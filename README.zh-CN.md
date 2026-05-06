# codex-app-remotely

远程操作你电脑上的 Codex.app。这个项目会以受控方式启动本机 Codex Electron App，开启 Electron 的 Chrome DevTools Protocol，然后把画面和输入事件转发到移动端 Web 页面。

## 截图

![codex-app-remotely 截图](./imgs/screenshot.png)

## 要求

- Node.js 22+
- macOS 或 Windows 上的 Codex Electron App
- 使用 remote 模式时需要 Cloudflare Workers + Durable Objects

## 安装和使用

无需安装即可运行：

```bash
npx codex-app-remotely
```

也可以全局安装 CLI：

```bash
npm install -g codex-app-remotely
```

安装后使用：

```bash
car
```

服务启动后会输出一个带 `token` 的访问 URL，并在终端显示该链接的二维码，手机扫码或在浏览器打开链接即可。

默认从 `9222` 开始自动选择一个空闲 CDP 端口。如果 `9222` 已经被 Chrome 或其他程序占用，服务会自动切到下一个可用端口。

### 通过 Cloudflare 的 remote 模式

remote 模式通过 Cloudflare Worker 中继访问。先部署项目内置 Worker：

```bash
npx codex-app-remotely deploy
```

这个命令会使用包内置的 `wrangler.toml`、`worker/` 和 `public/` 执行 Wrangler 部署。需要传 Wrangler deploy 参数时，直接跟在 `deploy` 后面，例如 `npx codex-app-remotely deploy --env production`。

然后以 remote 模式启动本机 host：

```bash
car --mode remote --remote-url "https://codex-app-remotely-remote.<account>.workers.dev"
```

如果通过 npm scripts 启动，需要使用 npm 的 `--` 分隔符；也可以使用已支持的位置参数简写：

```bash
npm run start -- --mode remote --remote-url "https://codex-app-remotely-remote.<account>.workers.dev"
npm run start remote "https://codex-app-remotely-remote.<account>.workers.dev"
```

CLI 会输出带 `room` 和 `token` 的远端 URL，手机浏览器打开或扫码即可。如果 URL 没有 `room`，两端都会使用 `default`。本机进程会向 `/ws/host` 建立出站 WebSocket；远端浏览器使用 `/ws/control` 和 `/ws/frame`；每个 `room` 对应一个 Durable Object，用来中继控制 JSON 和二进制画面帧。

### debugger 模式

debugger 模式会启动或连接 Codex，并为选中的 Codex target 打开 DevTools：

```bash
car debugger
```

等价写法：

```bash
car --mode debugger
car --open-devtools
```

如果由本工具启动 Codex，这个模式还会追加 `--auto-open-devtools-for-tabs`。如果连接的是已经启动的 Codex 进程，则会在默认浏览器里打开 CDP DevTools frontend：

```bash
car debugger --no-launch --cdp-port 9222
```

### 指定 Codex App 路径

如果 Codex app 不在默认位置，可以显式指定路径。macOS 上可以传 `.app` bundle：

```bash
npx codex-app-remotely --app "/Applications/Codex.app"
```

全局安装后等价写法：

```bash
car --app "/Applications/Codex.app"
```

Windows 上可以传安装目录或可执行文件路径：

```powershell
npx codex-app-remotely --app "$env:LOCALAPPDATA\Programs\Codex"
npx codex-app-remotely --executable "$env:LOCALAPPDATA\Programs\Codex\Codex.exe"
```

默认情况下，Windows 自动启动会检查常见 Electron 安装路径，包括 `%LOCALAPPDATA%\Programs\Codex\Codex.exe` 和 `%LOCALAPPDATA%\Programs\OpenAI Codex\OpenAI Codex.exe`。

### 连接已启动的 Codex

如果你已经手动启动了 Codex 并开启 CDP：

```bash
npx codex-app-remotely --no-launch --cdp-port 9222
```

全局安装后等价写法：

```bash
car --no-launch --cdp-port 9222
```

手动启动 Electron CDP 的参数形式通常是：

```bash
"/Applications/Codex.app/Contents/MacOS/Codex" \
  --remote-debugging-port=9222 \
  --remote-allow-origins=* \
  --disable-renderer-backgrounding \
  --disable-background-timer-throttling \
  --disable-backgrounding-occluded-windows
```

Windows 上：

```powershell
& "$env:LOCALAPPDATA\Programs\Codex\Codex.exe" `
  --remote-debugging-port=9222 `
  --remote-allow-origins=* `
  --disable-renderer-backgrounding `
  --disable-background-timer-throttling `
  --disable-backgrounding-occluded-windows
```

## 常用配置

```bash
car \
  --host 0.0.0.0 \
  --port 3147 \
  --mode remote \
  --remote-url https://codex-app-remotely-remote.<account>.workers.dev \
  --cdp-port 9333 \
  --screencast-every-nth-frame 1 \
  --screenshot-max-width 1440 \
  --screenshot-max-height 900 \
  --screenshot-quality 74
```

也可以使用环境变量：

- `CODEX_APP_PATH`
- `CODEX_EXECUTABLE`
- `CDP_HOST`
- `CDP_PORT`
- `HOST`
- `PORT`
- `REMOTE_TOKEN`
- `REMOTE_MODE`
- `REMOTE_WORKER_URL`
- `REMOTE_ROOM`
- `SCREENCAST_EVERY_NTH_FRAME`
- `SCREENSHOT_MAX_WIDTH`
- `SCREENSHOT_MAX_HEIGHT`
- `SCREENSHOT_QUALITY`
- `OPEN_DEVTOOLS=1`
- `CODEX_DEBUGGER=1`
- `NO_LAUNCH=1`

## 架构

- `src/server.js`：入口，编排启动器、CDP 桥和移动端 WebSocket。
- `src/codexLauncher.js`：定位并启动 Codex Electron App，注入 `--remote-debugging-port`。
- `src/cdpClient.js`：连接 CDP target，收到 `Page.screencastFrame` 后立即 ACK，动态切换 JPEG screencast 档位，并把点击、指针、滚动、文字、按键转换为 CDP `Input` 命令。
- `src/mobileWsServer.js`：零依赖 WebSocket 服务端，使用独立控制/画面通道，并按“只保留最新帧”的方式推送二进制画面。
- `src/remoteRelayClient.js`：remote 模式下连接 Cloudflare 的本机出站 WebSocket host。
- `src/staticServer.js`：静态页面和少量状态 API。
- `src/workerDeploy.js`：部署内置 Cloudflare Worker 中继的命令封装。
- `worker/index.js`：Cloudflare Worker + Durable Object 远端 room 中继。
- `public/`：移动端控制页面。

## 安全说明

CDP 拥有很高权限，本项目不会直接暴露 CDP 端口；移动端只连接本地 WebSocket 服务或 Cloudflare 中继。移动端连接需要启动时生成的一次性 `token`。remote 模式下，拿到完整远端 URL 的人都可以在本机 host 运行期间控制应用，请把该 URL 当作密钥处理。由本服务自动启动的 Codex app 会在服务关闭时一同退出；使用 `--no-launch` 连接手动启动的 Codex 时，本服务不会关闭该外部进程。
