# CodexLR PWA

Standalone mobile controller for CodexL remote instances.

Open the PWA without a token to scan a CodexL connection QR code. The QR can
also be pasted manually as a URL such as:

```text
http://192.168.1.10:3147/?token=...
```

Opening the same URL directly skips the scanner and starts the controller.
When installed to the iOS Home Screen from an authorized LAN URL, the local
server gives iOS a list-page start URL with the current connection embedded.
The list opens first and automatically adds that LAN instance without starting
the controller.

## Remote authentication

Each provider profile keeps a persistent 256-bit random LAN remote-control
access token. The token is created the first time that profile starts remote
control and is reused across later remote-control restarts. Clients can present
it as the `token` query parameter, a `Bearer` token for HTTP API requests, or
the short-lived HttpOnly `codexl_remote_token` cookie set after an explicitly
authenticated `/web` request.

The control page defaults to `Web` mode and hides the old Screen selector. Web
mode embeds the mirrored Codex frontend from `/web/` and forwards host messages
over the authenticated web bridge. The bridge URL carries the `token` parameter
when needed.

Before Web mode loads the iframe, the PWA asks its service worker to open the
authenticated resource transport with the same `token`. The worker checks
`/web/_version`, streams every listed `/web/` resource into same-origin Cache
Storage when the version changed or the cache is incomplete, and then lets the
iframe load the cached `/web/index.html` from the PWA's own service worker
scope. The cached iframe receives the authenticated web bridge URL in its query
string.

## Codex App web asset registry

For environments that can run Codex CLI but do not have Codex App installed,
CodexL can load the Codex App frontend from a hosted static registry instead of
mirroring `/web/` resources from the local app.

Extract the webview bundle from an installed Codex App package:

```sh
pnpm run extract:codex-web -- --app /Applications/Codex.app --clean
```

The script writes a versioned static registry to `dist/codex-app-web`:

```text
dist/codex-app-web/
  versions.json
  latest.json
  latest/index.html
  26.513.31313/index.html
  26.513.31313/manifest.json
  26.513.31313/assets/...
```

`index.html` is prepared for static hosting: the CodexL web bridge is injected
before the Codex module bundle, CSP placeholders are removed, and root asset
URLs are made relative to the version directory. The bridge then loads the
CodexL plugin runtime from the same stable runtime base. `latest/index.html`
redirects to the newest extracted version while preserving the remote bridge
query string.

The injected CodexL runtime is intentionally outside the versioned Codex App
bundle. By default, versioned pages load `../codexl-runtime/_codexl_bridge.js`,
which in turn loads `../codexl-runtime/_codexl_plugin.js`, so bridge or plugin
fixes can be published by replacing only the small `codexl-runtime/` files. To
host the runtime from a separate project or domain, pass `--runtime-base-url`
during extraction or publishing:

```sh
pnpm run publish:codex-web -- --runtime-base-url https://codexl-runtime.example.com
```

Publish the registry to Cloudflare Pages:

```sh
pnpm run publish:codex-web -- --project-name codexl-codex-app-web
```

Then configure the desktop app or its launch environment with the hosted base
URL. The version defaults to `latest`:

```sh
CODEXL_REMOTE_WEB_ASSET_REGISTRY_URL=https://web.codexl.io
CODEXL_REMOTE_WEB_ASSET_VERSION=latest
```

Remote connection URLs include `webAssetBaseUrl` and `webAssetVersion`. QR
codes keep only the short token URL; after connecting, the control page reads
the registry metadata from `/api/remote-info`, loads that hosted bundle by
default, resolves the selected bundle manifest to pass the current CodexL
runtime URL into the iframe, and shows a bundle selector when the registry
exposes `versions.json`.

Camera scanning requires a browser secure context, such as HTTPS or localhost.
The PWA uses the browser's native QR detector when available and falls back to
the bundled `jsQR` decoder, then the CodexL-specific local decoder, on mobile
browsers that do not expose native QR scanning.

## Cloudflare Pages

Create the Pages project once:

```sh
pnpm dlx wrangler@latest pages project create codexl-remote-pwa --production-branch main
```

Then publish the static PWA directory from the repo root:

```sh
pnpm run publish
```

Use another project name or a preview branch when needed:

```sh
pnpm run publish -- --project-name my-pages-project
pnpm run publish -- --project-name my-pages-project --branch preview
```

For CI, provide `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. A hosted
HTTPS PWA should connect to HTTPS control URLs; direct LAN HTTP control URLs may
be blocked by browsers as mixed content.
