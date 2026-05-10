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

CLI 会输出带 `room` 和 `token` 的远端 URL，手机浏览器打开或扫码即可。如果 URL 没有 `room`，两端都会使用 `default`。本机进程会向 `/ws/host` 建立出站 WebSocket；远端浏览器会在浏览器和 HTTPS 源支持时优先尝试 `/wt/session` 的低延迟 WebTransport 单会话，失败后回退到 `/ws/control` 和 `/ws/frame`；每个 `room` 对应一个 Durable Object，用来中继控制 JSON 和二进制画面帧。

### 自托管 WebTransport 中继

Cloudflare Worker remote 模式仍以 WebSocket 为主。要在自己的服务器上启用低延迟 WebTransport 中继，可以部署项目内置的 Deno relay。服务器需要真实 TLS 证书，并且同一个公网端口同时开放 TCP 和 UDP：

```bash
docker build -f selfhost/Dockerfile -t codex-app-remotely-relay .

docker run -d --name codex-app-remotely-relay \
  --restart unless-stopped \
  -p 443:8443/tcp \
  -p 443:8443/udp \
  -v /etc/letsencrypt:/etc/letsencrypt:ro \
  -e TLS_CERT=/etc/letsencrypt/live/remote.example.com/fullchain.pem \
  -e TLS_KEY=/etc/letsencrypt/live/remote.example.com/privkey.pem \
  codex-app-remotely-relay
```

然后把本机 host 指向你的 relay：

```bash
car --mode remote --remote-url "https://remote.example.com"
```

自托管 relay 会负责提供移动端页面、接受本机 host 的 `/ws/host` 连接、提供 `/ws/control` 和 `/ws/frame` 的 WebSocket 回退，并在 `/wt/session` 上接受 WebTransport。WebTransport 需要 HTTPS 和可访问的 UDP 端口；如果 UDP/HTTP3 被网络拦截，浏览器会自动回退到 WebSocket。

#### 搭配 Nginx 使用

WebTransport 不能按普通 WebSocket 反代来处理。建议用 Nginx 的 HTTP 反代处理页面和 WebSocket 回退，同时用顶层 `stream {}` 把 UDP/443 透传给 relay。除非由 Nginx 自己终止并服务 WebTransport，否则不要给这个域名配置 `listen 443 quic`，因为内置 relay 需要直接收到 HTTP/3/QUIC 流量。

先让 relay 监听本机端口，并挂载同一份 TLS 证书：

```bash
docker run -d --name codex-app-remotely-relay \
  --restart unless-stopped \
  -p 127.0.0.1:8443:8443/tcp \
  -p 127.0.0.1:8443:8443/udp \
  -v /etc/letsencrypt:/etc/letsencrypt:ro \
  -e TLS_CERT=/etc/letsencrypt/live/remote.example.com/fullchain.pem \
  -e TLS_KEY=/etc/letsencrypt/live/remote.example.com/privkey.pem \
  codex-app-remotely-relay
```

然后添加类似下面的 Nginx 配置。`http {}` 负责 HTTPS 和 WebSocket upgrade 头；顶层 `stream {}` 负责把 QUIC/WebTransport 的 UDP 包转发到 relay：

```nginx
# /etc/nginx/nginx.conf
stream {
    server {
        listen 443 udp reuseport;
        proxy_pass 127.0.0.1:8443;
        proxy_timeout 10m;
    }
}

http {
    map $http_upgrade $connection_upgrade {
        default upgrade;
        ''      close;
    }

    server {
        listen 443 ssl;
        server_name remote.example.com;

        ssl_certificate     /etc/letsencrypt/live/remote.example.com/fullchain.pem;
        ssl_certificate_key /etc/letsencrypt/live/remote.example.com/privkey.pem;

        location / {
            proxy_pass https://127.0.0.1:8443;
            proxy_http_version 1.1;
            proxy_set_header Host $host;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto https;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection $connection_upgrade;
            proxy_read_timeout 1h;

            # relay 本身也会在本机终止 TLS，用于 UDP 上的 WebTransport。
            proxy_ssl_server_name on;
            proxy_ssl_name remote.example.com;
        }
    }
}
```

重载 Nginx 后，确认防火墙同时放行 `443/tcp` 和 `443/udp`。本机 host 仍然使用普通 HTTPS URL：

```bash
car --mode remote --remote-url "https://remote.example.com"
```

如果不使用 Docker，也可以安装 Deno 2.2+ 后直接启动：

```bash
TLS_CERT=/etc/letsencrypt/live/remote.example.com/fullchain.pem \
TLS_KEY=/etc/letsencrypt/live/remote.example.com/privkey.pem \
PORT=443 \
npm run relay:selfhost
```

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
- `selfhost/relay.js`：基于 Deno 的自托管中继，提供 HTTPS、WebSocket 回退和 HTTP/3 WebTransport。
- `public/`：移动端控制页面，内置 WebTransport 优先的传输适配器和 WebSocket 回退。WebTransport 协议用可靠双向流传控制 JSON，用服务端单向流或分片 datagram 独立传画面帧，过期帧可直接丢弃，不阻塞输入。

## 安全说明

CDP 拥有很高权限，本项目不会直接暴露 CDP 端口；移动端只连接本地 WebSocket 服务或 Cloudflare 中继。移动端连接需要启动时生成的一次性 `token`。remote 模式下，拿到完整远端 URL 的人都可以在本机 host 运行期间控制应用，请把该 URL 当作密钥处理。由本服务自动启动的 Codex app 会在服务关闭时一同退出；使用 `--no-launch` 连接手动启动的 Codex 时，本服务不会关闭该外部进程。
