# codex-app-remotely

English | [中文](./README.zh-CN.md)

Remotely control Codex.app on your computer from a mobile browser. This tool starts the local Codex Electron app with Chrome DevTools Protocol enabled, streams the app screen, and forwards touch, keyboard, and scroll input from the mobile web controller.

## Screenshot

![codex-app-remotely screenshot](./imgs/screenshot.png)

## Requirements

- Node.js 22+
- Codex Electron app on macOS or Windows
- Cloudflare Workers + Durable Objects when using remote mode

## Installation and Usage

Run without installing:

```bash
npx codex-app-remotely
```

Or install the CLI globally:

```bash
npm install -g codex-app-remotely
```

Then run:

```bash
car
```

After startup, the server prints an access URL with a `token` and renders a QR code in the terminal. Scan it with your phone or open the URL in a mobile browser.

By default, the launcher starts looking for a free CDP port from `9222`. If `9222` is already used by Chrome or another process, it automatically switches to the next available port.

### Remote Mode Through Cloudflare

Remote mode lets the phone connect through a Cloudflare Worker relay. Deploy the included Worker first:

```bash
npx codex-app-remotely deploy
```

This command runs Wrangler against the bundled `wrangler.toml`, `worker/`, and `public/` files. You can pass Wrangler deploy options after `deploy`, for example `npx codex-app-remotely deploy --env production`.

Then start the local host in remote mode:

```bash
car --mode remote --remote-url "https://codex-app-remotely-remote.<account>.workers.dev"
```

When running through npm scripts, use npm's `--` separator, or use the supported positional shorthand:

```bash
npm run start -- --mode remote --remote-url "https://codex-app-remotely-remote.<account>.workers.dev"
npm run start remote "https://codex-app-remotely-remote.<account>.workers.dev"
```

The CLI prints a remote URL containing `room` and `token`. Open or scan that URL from the remote browser. If `room` is omitted, both sides use `default`. The local process opens an outbound WebSocket to `/ws/host`; remote browsers first try one low-latency WebTransport session on `/wt/session` when supported by the browser and HTTPS origin, then fall back to `/ws/control` and `/ws/frame`; one Durable Object instance per `room` relays control JSON and binary frames.

### Self-Hosted WebTransport Relay

Cloudflare Worker remote mode remains WebSocket-based. To run the low-latency WebTransport relay on your own server, deploy the bundled Deno relay with a real TLS certificate and open both TCP and UDP on the same public port:

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

Then point the local host at your relay:

```bash
car --mode remote --remote-url "https://remote.example.com"
```

The self-hosted relay serves the mobile page, accepts the local host on `/ws/host`, accepts WebSocket fallback clients on `/ws/control` and `/ws/frame`, and accepts WebTransport clients on `/wt/session`. WebTransport requires HTTPS and a reachable UDP port; if UDP/HTTP3 is blocked, browsers automatically fall back to WebSocket.

#### Using Nginx In Front

WebTransport is not the same as WebSocket proxying. Use normal Nginx HTTP reverse proxying for the page and WebSocket fallback, and pass UDP/443 through to the relay for WebTransport. Do not configure Nginx `listen 443 quic` for this hostname unless Nginx itself is terminating and serving WebTransport; the bundled relay needs to receive the HTTP/3/QUIC traffic.

Run the relay on localhost with its TLS certificate:

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

Then add an Nginx config like this. The `http {}` block handles HTTPS and WebSocket upgrade headers; the top-level `stream {}` block passes QUIC/WebTransport UDP packets to the relay:

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

            # The relay also terminates TLS locally so it can serve WebTransport on UDP.
            proxy_ssl_server_name on;
            proxy_ssl_name remote.example.com;
        }
    }
}
```

After reloading Nginx, make sure the firewall allows both `443/tcp` and `443/udp`. You can still point the local host at the normal HTTPS URL:

```bash
car --mode remote --remote-url "https://remote.example.com"
```

If you run without Docker, install Deno 2.2+ and start the relay directly:

```bash
TLS_CERT=/etc/letsencrypt/live/remote.example.com/fullchain.pem \
TLS_KEY=/etc/letsencrypt/live/remote.example.com/privkey.pem \
PORT=443 \
npm run relay:selfhost
```

### Debugger Mode

Debugger mode starts or connects to Codex with CDP enabled and opens DevTools for the selected Codex target:

```bash
car debugger
```

Equivalent forms:

```bash
car --mode debugger
car --open-devtools
```

When launching Codex itself, this mode also passes `--auto-open-devtools-for-tabs`. When connecting to an already running Codex process, it opens the CDP DevTools frontend in your default browser:

```bash
car debugger --no-launch --cdp-port 9222
```

### Specify the Codex App Path

If the Codex app is not in the default location, pass the app path explicitly. On macOS this can be the `.app` bundle:

```bash
npx codex-app-remotely --app "/Applications/Codex.app"
```

Equivalent command after global installation:

```bash
car --app "/Applications/Codex.app"
```

On Windows this can be the install directory or the executable:

```powershell
npx codex-app-remotely --app "$env:LOCALAPPDATA\Programs\Codex"
npx codex-app-remotely --executable "$env:LOCALAPPDATA\Programs\Codex\Codex.exe"
```

By default, Windows auto-launch checks common Electron install locations, including `%LOCALAPPDATA%\Programs\Codex\Codex.exe` and `%LOCALAPPDATA%\Programs\OpenAI Codex\OpenAI Codex.exe`.

### Connect to an Already Running Codex App

If you already started Codex manually with CDP enabled:

```bash
npx codex-app-remotely --no-launch --cdp-port 9222
```

Equivalent command after global installation:

```bash
car --no-launch --cdp-port 9222
```

The manual Electron CDP launch command usually looks like this:

```bash
"/Applications/Codex.app/Contents/MacOS/Codex" \
  --remote-debugging-port=9222 \
  --remote-allow-origins=* \
  --disable-renderer-backgrounding \
  --disable-background-timer-throttling \
  --disable-backgrounding-occluded-windows
```

On Windows:

```powershell
& "$env:LOCALAPPDATA\Programs\Codex\Codex.exe" `
  --remote-debugging-port=9222 `
  --remote-allow-origins=* `
  --disable-renderer-backgrounding `
  --disable-background-timer-throttling `
  --disable-backgrounding-occluded-windows
```

## Common Options

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

You can also configure the server with environment variables:

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

## Architecture

- `src/server.js`: entry point, orchestrates the launcher, CDP bridge, and mobile WebSocket server.
- `src/codexLauncher.js`: locates and starts the Codex Electron app with `--remote-debugging-port`.
- `src/cdpClient.js`: connects to the CDP target, immediately ACKs `Page.screencastFrame`, adapts JPEG screencast profiles, and maps click, pointer, scroll, text, and key events to CDP `Input` commands.
- `src/mobileWsServer.js`: dependency-light WebSocket server with separate control/frame channels and latest-frame-only binary delivery.
- `src/remoteRelayClient.js`: outbound host WebSocket client for Cloudflare remote mode.
- `src/staticServer.js`: static file server plus small status APIs.
- `src/workerDeploy.js`: deploy command wrapper for the bundled Cloudflare Worker relay.
- `worker/index.js`: Cloudflare Worker + Durable Object relay for remote rooms.
- `selfhost/relay.js`: Deno-based self-hosted relay with HTTPS, WebSocket fallback, and WebTransport over HTTP/3.
- `public/`: mobile control page with a WebTransport-first transport adapter and WebSocket fallback. The WebTransport protocol uses a reliable bidirectional stream for control JSON and independent frame delivery via server unidirectional streams or chunked datagrams so stale frames can be dropped without blocking input.

## Security Notes

CDP has high privileges. This project does not expose the CDP port directly to the mobile device; the phone connects only to this server's WebSocket endpoint or to the Cloudflare relay. Mobile access requires the one-time `token` generated at startup. In remote mode, anyone with the full remote URL can control the app until the local host stops, so treat the URL as a secret. If this tool launches the Codex app, it also quits that app during shutdown. When using `--no-launch` to connect to an external Codex process, this tool does not stop that process.
