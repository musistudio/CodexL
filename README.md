# codex-app-remotely

English | [中文](./README.zh-CN.md)

Remotely control Codex.app on your computer from a mobile browser. This tool starts the local Codex Electron app with Chrome DevTools Protocol enabled, streams the app screen, and forwards touch, keyboard, and scroll input from the mobile web controller.

## Screenshot

![codex-app-remotely screenshot](./imgs/screenshot.png)

## Requirements

- Node.js 22+
- Codex Electron app on macOS
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

The CLI prints a remote URL containing `room` and `token`. Open or scan that URL from the remote browser. If `room` is omitted, both sides use `default`. The local process opens an outbound WebSocket to `/ws/host`; remote browsers use `/ws/control` and `/ws/frame`; one Durable Object instance per `room` relays control JSON and binary frames.

### Specify the Codex App Path

If the Codex app is not in the default location, pass the app path explicitly:

```bash
npx codex-app-remotely --app "/Applications/Codex.app"
```

Equivalent command after global installation:

```bash
car --app "/Applications/Codex.app"
```

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
"/Applications/Codex.app/Contents/MacOS/Codex" --remote-debugging-port=9222 --remote-allow-origins=*
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
- `public/`: mobile control page.

## Security Notes

CDP has high privileges. This project does not expose the CDP port directly to the mobile device; the phone connects only to this server's WebSocket endpoint or to the Cloudflare relay. Mobile access requires the one-time `token` generated at startup. In remote mode, anyone with the full remote URL can control the app until the local host stops, so treat the URL as a secret. If this tool launches the Codex app, it also quits that app during shutdown. When using `--no-launch` to connect to an external Codex process, this tool does not stop that process.
