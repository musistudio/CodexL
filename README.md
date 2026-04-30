# codex-app-remotely

English | [中文](./README.zh-CN.md)

Remotely control Codex.app on your computer from a mobile browser. This tool starts the local Codex Electron app with Chrome DevTools Protocol enabled, streams the app screen, and forwards touch, keyboard, and scroll input from the mobile web controller.

## Screenshot

![codex-app-remotely screenshot](./imgs/screenshot.png)

## Requirements

- Node.js 22+
- Codex Electron app on macOS
- Phone and computer on the same local network

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

After startup, the server prints a LAN URL with a `token` and renders a QR code in the terminal. Scan it with your phone or open the URL in a mobile browser.

By default, the launcher starts looking for a free CDP port from `9222`. If `9222` is already used by Chrome or another process, it automatically switches to the next available port.

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
  --cdp-port 9333 \
  --screencast-every-nth-frame 1 \
  --screencast-max-fps 12 \
  --screenshot-max-width 1920 \
  --screenshot-max-height 1350 \
  --screenshot-quality 68
```

You can also configure the server with environment variables:

- `CODEX_APP_PATH`
- `CODEX_EXECUTABLE`
- `CDP_HOST`
- `CDP_PORT`
- `HOST`
- `PORT`
- `REMOTE_TOKEN`
- `SCREENCAST_EVERY_NTH_FRAME`
- `SCREENCAST_MAX_FPS`
- `SCREENSHOT_MAX_WIDTH`
- `SCREENSHOT_MAX_HEIGHT`
- `SCREENSHOT_QUALITY`
- `NO_LAUNCH=1`

## Architecture

- `src/server.js`: entry point, orchestrates the launcher, CDP bridge, and mobile WebSocket server.
- `src/codexLauncher.js`: locates and starts the Codex Electron app with `--remote-debugging-port`.
- `src/cdpClient.js`: connects to the CDP target, receives frames with `Page.startScreencast`, and maps click, scroll, text, and key events to CDP `Input` commands.
- `src/mobileWsServer.js`: dependency-light WebSocket server for mobile message transport.
- `src/staticServer.js`: static file server plus small status APIs.
- `public/`: mobile control page.

## Security Notes

CDP has high privileges. This project does not expose the CDP port directly to the mobile device; the phone connects only to this server's WebSocket endpoint. Mobile access requires the one-time `token` generated at startup. Use it only on trusted local networks and stop the server when finished. If this tool launches the Codex app, it also quits that app during shutdown. When using `--no-launch` to connect to an external Codex process, this tool does not stop that process.
