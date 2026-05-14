# CodexL Remote PWA

Standalone mobile controller for CodexL remote instances.

Open the PWA without a token to scan a CodexL connection QR code. The QR can
also be pasted manually as a URL such as:

```text
http://192.168.1.10:3147/?token=...
```

Opening the same URL directly skips the scanner and starts the controller.

## Remote authentication

Each LAN remote-control server start creates a fresh 256-bit random access
token. Clients can present it as the `token` query parameter, a `Bearer` token
for HTTP API requests, or the short-lived HttpOnly `codexl_remote_token` cookie
set after an explicitly authenticated `/web` request.

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
