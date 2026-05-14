const CACHE_NAME = "codexl-remote-v40-local-bridge-plain-v2";
const WEB_CACHE_NAME = "codexl-remote-web-v11-local-bridge-plain";
const WEB_CACHE_CONFIG_KEY = new URL("__codex-web-cache-config.json", self.registration.scope).toString();
const WEB_VERSION_CACHE_KEY = new URL("__codex-web-version.json", self.registration.scope).toString();
const WEB_PATH_PREFIX = new URL("web/", self.registration.scope).pathname;
const WEB_SCOPED_PATH_PREFIX = WEB_PATH_PREFIX.endsWith("/") ? WEB_PATH_PREFIX.slice(0, -1) : WEB_PATH_PREFIX;
const APP_SHELL = new URL("index.html", self.registration.scope).toString();
const WEB_TRANSPORT_CONNECT_TIMEOUT_MS = 2500;
const WEB_RESOURCE_SOCKET_CONNECT_TIMEOUT_MS = 10000;
const WEB_RESOURCE_REQUEST_TIMEOUT_MS = 60000;
const WEB_RESOURCE_SHARED_TRANSPORT_IDLE_MS = 60000;
const E2EE_AAD = new TextEncoder().encode("codexl-remote-e2ee-v1");
const WEB_CACHE_IGNORED_PARAMS = [
  "token",
  "hostId",
  "transport",
  "codexBridgeUrl",
  "codexBridgeTransportUrl",
  "cloudUser",
  "jwt",
  "requirePassword",
  "e2ee",
];
let latestWebResourceTransportConfig = null;
let sharedWebResourceTransport = null;
let sharedWebResourceTransportIdleTimer = null;
let sharedWebResourceTransportKey = "";
let sharedWebResourceTransportPromise = null;
let sharedWebResourceTransportUseCount = 0;
let latestWebCacheMessage = null;
let webCachePrepareChain = Promise.resolve();
let webResourceFetchInflight = new Map();
const ASSETS = [
  "./",
  "index.html",
  "control.html",
  "app.js?v=20260513-local-bridge-plain-v2",
  "qrDecoder.js?v=20260513-local-bridge-plain-v2",
  "realtimeTransport.js?v=20260513-local-bridge-plain-v2",
  "react-app.css?v=20260513-local-bridge-plain-v2",
  "react-app.js?v=20260513-local-bridge-plain-v2",
  "vendor/jsQR.js?v=20260513-local-bridge-plain-v2",
  "styles.css?v=20260513-local-bridge-plain-v2",
  "manifest.webmanifest",
  "icon.png",
].map((asset) => new URL(asset, self.registration.scope).toString());

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE_NAME).then(cacheShellAssets));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((key) => key !== CACHE_NAME && key !== WEB_CACHE_NAME).map((key) => caches.delete(key))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("message", (event) => {
  const message = event.data || {};
  if (message.type === "update-web-resource-transport") {
    rememberWebResourceTransport(message);
    event.ports?.[0]?.postMessage({
      requestId: message.requestId,
      type: "web-resource-transport-updated",
    });
    return;
  }
  if (message.type !== "prepare-web-cache") {
    return;
  }
  const replyTarget = event.ports?.[0] || event.source;
  const reply = (payload) => {
    replyTarget?.postMessage(payload);
  };

  const task = prepareWebCache(message)
    .then((result) => {
      reply({
        ...result,
        requestId: message.requestId,
        type: "web-cache-ready",
      });
    })
    .catch((error) => {
      reply({
        error: error?.message || String(error),
        requestId: message.requestId,
        type: "web-cache-ready",
      });
    });
  event.waitUntil?.(task);
});

self.addEventListener("fetch", (event) => {
  if (event.request.method !== "GET") {
    return;
  }

  const request = event.request;
  const url = new URL(request.url);
  if (!isHttpUrl(url)) {
    return;
  }
  if (isWebResourceUrl(url)) {
    if (isWebVersionUrl(url) || isWebResourceSocketUrl(url)) {
      event.respondWith(
        new Response("Codex web control resource is only available through websocket preparation", {
          headers: { "content-type": "text/plain; charset=utf-8" },
          status: 426,
        }),
      );
      return;
    }

    event.respondWith(webResourceResponse(request));
    return;
  }

  const forceFresh =
    request.mode === "navigate" ||
    url.pathname.endsWith(".html") ||
    url.pathname.endsWith(".js") ||
    url.pathname.endsWith(".css") ||
    url.pathname.endsWith(".webmanifest");

  event.respondWith(networkFirst(request, { forceFresh }));
});

async function cacheShellAssets(cache) {
  await Promise.all(
    ASSETS.map(async (asset) => {
      try {
        const response = await fetch(asset, { cache: "reload" });
        if (response.ok) {
          await cache.put(asset, response);
          return;
        }
        console.warn("[remote-sw] shell asset skipped", asset, response.status);
      } catch (error) {
        console.warn("[remote-sw] shell asset skipped", asset, error);
      }
    }),
  );
}

async function prepareWebCache(message) {
  const task = webCachePrepareChain
    .catch(() => {})
    .then(() => prepareWebCacheUnlocked(message));
  webCachePrepareChain = task.catch(() => {});
  return task;
}

async function prepareWebCacheUnlocked(message) {
  const versionUrl = String(message.versionUrl || "");
  const iframeUrl = String(message.iframeUrl || "");
  const cacheIframeUrl = String(message.cacheIframeUrl || iframeUrl);
  const resourceWebSocketUrl = String(message.resourceWebSocketUrl || "");
  if (!versionUrl || !iframeUrl) {
    throw new Error("Missing web cache version URL");
  }
  if (!resourceWebSocketUrl) {
    throw new Error("Missing web resource websocket URL");
  }
  await rememberWebCacheMessage(message);
  rememberWebResourceTransport(message);

  return withSharedWebResourceTransport(message, async (transport) => {
    const versionPayload = await transport.request({ type: "version", url: versionUrl });
    const manifest = JSON.parse(decodeWebResourceText(versionPayload));
    const version = String(manifest.version || "");
    if (!version) {
      throw new Error("Web resource version is missing");
    }

    const previous = await readWebCacheVersion();
    const versionChanged = previous?.version !== version;

    if (versionChanged) {
      await caches.delete(WEB_CACHE_NAME);
    }
    await writeWebCacheVersion({ cached: 0, ts: Date.now(), version });

    return {
      cached: 0,
      total: Array.isArray(manifest.resources) ? manifest.resources.length : 0,
      updated: versionChanged,
      version,
    };
  });
}

async function webResourceResponse(request) {
  return webCacheFirst(request);
}

async function webCacheFirst(request) {
  const cached = await cachedWebResource(request);
  if (cached) {
    return cached;
  }

  if (await prepareWebCacheFromRequestUrl(request)) {
    const prepared = await cachedWebResource(request);
    if (prepared) {
      return prepared;
    }
  }

  const transported = await fetchAndCacheWebResourceThroughTransport(request);
  if (transported) {
    return transported;
  }

  return missingWebResourceResponse();
}

async function prepareWebCacheFromRequestUrl(request) {
  const message = await webCacheMessageFromRequest(request);
  if (!message) {
    return false;
  }

  try {
    await rememberWebCacheMessage(message);
    rememberWebResourceTransport(message);
    return true;
  } catch (error) {
    console.warn("[web-cache] transport configuration failed", request.url, error);
    return false;
  }
}

async function webCacheMessageFromRequest(request) {
  const requestUrl = new URL(request.url);
  const directMessage = webCacheMessageFromRequestUrl(requestUrl, requestUrl);
  if (directMessage) {
    directMessage.requestedResourceUrl = requestUrl.toString();
    return directMessage;
  }

  const referrerUrl = webCacheReferrerUrl(request);
  if (!referrerUrl) {
    return webCacheMessageFromStoredConfig(requestUrl);
  }
  const referrerMessage = webCacheMessageFromRequestUrl(referrerUrl, referrerUrl);
  if (referrerMessage) {
    referrerMessage.requestedResourceUrl = requestUrl.toString();
    return referrerMessage;
  }
  return webCacheMessageFromStoredConfig(requestUrl);
}

function webCacheReferrerUrl(request) {
  const value = String(request.referrer || "");
  if (!value || value === "about:client") {
    return null;
  }
  try {
    const url = new URL(value);
    return isWebResourceUrl(url) ? url : null;
  } catch {
    return null;
  }
}

function webCacheMessageFromRequestUrl(sourceUrl, cacheIframeUrl) {
  const bridgeUrl = webBridgeSocketUrlFromRequest(sourceUrl);
  if (!bridgeUrl) {
    return null;
  }

  const versionUrl = webEndpointHttpUrl(bridgeUrl, "_version");
  const iframeUrl = webEndpointHttpUrl(bridgeUrl, "index.html");
  const resourceWebSocketUrl = webEndpointSocketUrl(bridgeUrl, "_resource", sourceUrl);
  const resourceWebTransportUrl = webResourceTransportUrlFromRequest(sourceUrl);
  if (!versionUrl || !iframeUrl || !resourceWebSocketUrl) {
    return null;
  }

  iframeUrl.search = sourceUrl.search;

  return {
    cacheIframeUrl: cacheIframeUrl.toString(),
    e2eeKey: webResourceTransportRequiresCrypto(resourceWebSocketUrl, resourceWebTransportUrl)
      ? currentWebCacheE2eeKey()
      : "",
    iframeUrl: iframeUrl.toString(),
    requestId: `direct-web-cache-${Date.now()}`,
    resourceWebSocketUrl: resourceWebSocketUrl.toString(),
    resourceWebTransportUrl: resourceWebTransportUrl,
    transportPreference: String(sourceUrl.searchParams.get("transport") || "auto").toLowerCase(),
    type: "prepare-web-cache",
    versionUrl: versionUrl.toString(),
  };
}

function currentWebCacheE2eeKey() {
  return String(
    latestWebCacheMessage?.e2eeKey ||
      latestWebResourceTransportConfig?.e2eeKey ||
      "",
  );
}

async function webCacheMessageFromStoredConfig(requestUrl) {
  const config = latestWebCacheMessage || (await readWebCacheMessage());
  if (!config) {
    return null;
  }
  return {
    ...config,
    requestedResourceUrl: requestUrl.toString(),
    requestId: `stored-web-cache-${Date.now()}`,
    type: "prepare-web-cache",
  };
}

async function rememberWebCacheMessage(message) {
  const config = webCacheConfigMessage(message);
  latestWebCacheMessage = config;
  try {
    const persistedConfig = { ...config, e2eeKey: "" };
    const cache = await caches.open(CACHE_NAME);
    await cache.put(
      WEB_CACHE_CONFIG_KEY,
      new Response(JSON.stringify(persistedConfig), {
        headers: { "content-type": "application/json; charset=utf-8" },
      }),
    );
  } catch {
    // An in-memory copy is still useful for this worker lifetime.
  }
}

function webCacheConfigMessage(message) {
  return {
    cacheIframeUrl: String(message.cacheIframeUrl || message.iframeUrl || ""),
    e2eeKey: String(message.e2eeKey || ""),
    iframeUrl: String(message.iframeUrl || ""),
    resourceWebSocketUrl: String(message.resourceWebSocketUrl || ""),
    resourceWebTransportUrl: String(message.resourceWebTransportUrl || ""),
    transportPreference: String(message.transportPreference || "auto").toLowerCase(),
    type: "prepare-web-cache",
    versionUrl: String(message.versionUrl || ""),
  };
}

async function readWebCacheMessage() {
  try {
    const cache = await caches.open(CACHE_NAME);
    const response = await cache.match(WEB_CACHE_CONFIG_KEY);
    if (!response) {
      return null;
    }
    const config = await response.json();
    if (!config?.versionUrl || !config?.iframeUrl || !config?.resourceWebSocketUrl) {
      return null;
    }
    latestWebCacheMessage = webCacheConfigMessage(config);
    return latestWebCacheMessage;
  } catch {
    return null;
  }
}

function webBridgeSocketUrlFromRequest(url) {
  const bridgeValue = url.searchParams.get("codexBridgeUrl");
  if (bridgeValue) {
    try {
      const bridgeUrl = new URL(bridgeValue);
      if (bridgeUrl.protocol === "ws:" || bridgeUrl.protocol === "wss:") {
        return bridgeUrl;
      }
    } catch {
      return null;
    }
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return null;
  }
  if (!hasRelayWebAuthParams(url)) {
    return null;
  }
  const bridgeUrl = new URL(url);
  bridgeUrl.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  bridgeUrl.pathname = webEndpointPathname(url.pathname, "_bridge");
  bridgeUrl.searchParams.set("hostId", url.searchParams.get("hostId") || "local");
  return bridgeUrl;
}

function hasRelayWebAuthParams(url) {
  return url.searchParams.has("token") || url.searchParams.has("cloudUser") || url.searchParams.has("jwt");
}

function webEndpointSocketUrl(bridgeUrl, endpoint, sourceUrl) {
  const endpointUrl = new URL(bridgeUrl);
  endpointUrl.pathname = webEndpointPathname(bridgeUrl.pathname, endpoint);
  for (const key of ["hostId", "token", "cloudUser", "jwt"]) {
    if (!endpointUrl.searchParams.has(key) && sourceUrl.searchParams.has(key)) {
      endpointUrl.searchParams.set(key, sourceUrl.searchParams.get(key));
    }
  }
  return endpointUrl;
}

function webEndpointHttpUrl(bridgeUrl, endpoint) {
  const endpointUrl = new URL(bridgeUrl);
  if (endpointUrl.protocol === "ws:") {
    endpointUrl.protocol = "http:";
  } else if (endpointUrl.protocol === "wss:") {
    endpointUrl.protocol = "https:";
  }
  endpointUrl.pathname = webEndpointPathname(bridgeUrl.pathname, endpoint);
  return endpointUrl;
}

function webEndpointPathname(pathname, endpoint) {
  const tail = webResourcePathTail(pathname);
  if (tail !== null) {
    return pathname.slice(0, pathname.length - tail.length) + endpoint;
  }
  return pathname.replace(/\/[^/]*$/, `/${endpoint}`);
}

function webResourceTransportUrlFromRequest(url) {
  const bridgeTransportUrl = url.searchParams.get("codexBridgeTransportUrl");
  if (!bridgeTransportUrl) {
    return "";
  }
  try {
    const transportUrl = new URL(bridgeTransportUrl);
    transportUrl.pathname = transportUrl.pathname.replace(/\/wt\/web-bridge$/, "/wt/web-resource");
    for (const key of ["hostId", "token", "cloudUser", "jwt"]) {
      if (!transportUrl.searchParams.has(key) && url.searchParams.has(key)) {
        transportUrl.searchParams.set(key, url.searchParams.get(key));
      }
    }
    return transportUrl.toString();
  } catch {
    return "";
  }
}

async function fetchAndCacheWebResourceThroughTransport(request) {
  if (!latestWebResourceTransportConfig) {
    return null;
  }

  try {
    return await withSharedWebResourceTransport(latestWebResourceTransportConfig, async (transport) => {
      const cache = await caches.open(WEB_CACHE_NAME);
      const cached = await requestAndCacheWebResource(cache, transport, webResourceCacheEntry(request.url));
      return new Response(cached.body, {
        headers: { "content-type": cached.contentType },
        status: cached.status,
      });
    });
  } catch (error) {
    console.warn("[web-cache] on-demand resource fetch failed", request.url, error);
    return null;
  }
}

async function cachedWebResource(request) {
  const key = normalizedWebCacheKey(request.url);
  const cache = await caches.open(WEB_CACHE_NAME);
  return cache.match(key);
}

function missingWebResourceResponse() {
  return new Response("Cached Codex web resource is missing; reconnect to refresh the web cache", {
    headers: { "content-type": "text/plain; charset=utf-8" },
    status: 504,
  });
}

function shouldPreferNetworkWebResources() {
  const hostname = self.location.hostname;
  return (
    hostname === "localhost" ||
    hostname === "127.0.0.1" ||
    hostname === "::1" ||
    hostname.startsWith("192.168.") ||
    hostname.startsWith("10.") ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(hostname)
  );
}

function webResourceCacheEntry(requestUrl, cacheUrl = requestUrl) {
  return {
    cacheUrl: webCacheUrlForResource(cacheUrl || requestUrl),
    requestUrl,
  };
}

async function requestAndCacheWebResource(cache, socket, resource) {
  const cached = await cachedWebResourceEntry(cache, resource);
  if (cached) {
    return cached;
  }

  const key = normalizedWebCacheKey(resource.cacheUrl);
  const inflight = webResourceFetchInflight.get(key);
  if (inflight) {
    return inflight;
  }

  const task = requestAndCacheWebResourceUncached(cache, socket, resource).finally(() => {
    if (webResourceFetchInflight.get(key) === task) {
      webResourceFetchInflight.delete(key);
    }
  });
  webResourceFetchInflight.set(key, task);
  return task;
}

async function cachedWebResourceEntry(cache, resource) {
  const response = await cache.match(normalizedWebCacheKey(resource.cacheUrl));
  if (!response) {
    return null;
  }
  const contentType = response.headers.get("content-type") || contentTypeForUrl(resource.cacheUrl);
  const body = new Uint8Array(await response.clone().arrayBuffer());
  return {
    body,
    cacheUrl: resource.cacheUrl,
    contentType,
    fetched: false,
    requestUrl: resource.requestUrl,
    status: response.status || 200,
  };
}

async function requestAndCacheWebResourceUncached(cache, socket, resource) {
  const payload = await socket.request({ type: "resource", url: resource.requestUrl });
  const status = Number(payload.status || 0);
  if (status < 200 || status >= 300) {
    throw new Error(`Web resource fetch failed: HTTP ${status || "unknown"} ${resource.requestUrl}`);
  }
  const headers = new Headers();
  const contentType = payload.contentType || contentTypeForUrl(resource.cacheUrl);
  if (!isExpectedWebResourceContentType(resource.cacheUrl, contentType)) {
    throw new Error(`Web resource content-type mismatch: ${contentType || "unknown"} ${resource.requestUrl}`);
  }
  headers.set("content-type", contentType);
  const body = cachedWebResourceBytes(payload, contentType);
  await cache.put(
    normalizedWebCacheKey(resource.cacheUrl),
    new Response(body, {
      headers,
      status,
    }),
  );
  return {
    body,
    cacheUrl: resource.cacheUrl,
    contentType,
    fetched: true,
    requestUrl: resource.requestUrl,
    status,
  };
}

function cachedWebResourceBytes(payload, contentType) {
  const bytes = decodeWebResourceBytes(payload);
  if (WEB_SCOPED_PATH_PREFIX === "/web") {
    return bytes;
  }
  if (!contentType.startsWith("text/html") && !contentType.startsWith("text/css")) {
    return bytes;
  }
  try {
    return new TextEncoder().encode(rewriteScopedWebPaths(new TextDecoder().decode(bytes)));
  } catch {
    return bytes;
  }
}

function rewriteScopedWebPaths(text) {
  return text.replace(/(["'(=]\s*)\/web\//g, `$1${WEB_SCOPED_PATH_PREFIX}/`);
}

function rememberWebResourceTransport(message) {
  const nextConfig = {
    e2eeKey: String(message.e2eeKey || ""),
    resourceWebSocketUrl: String(message.resourceWebSocketUrl || ""),
    resourceWebTransportUrl: String(message.resourceWebTransportUrl || ""),
    transportPreference: String(message.transportPreference || "auto").toLowerCase(),
  };
  if (latestWebResourceTransportConfig && webResourceTransportKey(latestWebResourceTransportConfig) !== webResourceTransportKey(nextConfig)) {
    closeSharedWebResourceTransport();
  }
  latestWebResourceTransportConfig = nextConfig;
}

async function withSharedWebResourceTransport(message, callback) {
  const client = await sharedWebResourceTransportFor(message);
  sharedWebResourceTransportUseCount += 1;
  clearSharedWebResourceTransportIdleTimer();
  try {
    return await callback(client);
  } finally {
    sharedWebResourceTransportUseCount = Math.max(0, sharedWebResourceTransportUseCount - 1);
    if (sharedWebResourceTransportUseCount === 0) {
      scheduleSharedWebResourceTransportClose();
    }
  }
}

async function sharedWebResourceTransportFor(message) {
  const config = {
    e2eeKey: String(message.e2eeKey || ""),
    resourceWebSocketUrl: String(message.resourceWebSocketUrl || ""),
    resourceWebTransportUrl: String(message.resourceWebTransportUrl || ""),
    transportPreference: String(message.transportPreference || "auto").toLowerCase(),
  };
  const key = webResourceTransportKey(config);

  if (sharedWebResourceTransport && sharedWebResourceTransportKey === key && sharedWebResourceTransport.isOpen?.() !== false) {
    return sharedWebResourceTransport;
  }

  if (sharedWebResourceTransportKey && sharedWebResourceTransportKey !== key) {
    closeSharedWebResourceTransport();
  }

  if (sharedWebResourceTransportPromise && sharedWebResourceTransportKey === key) {
    return sharedWebResourceTransportPromise;
  }

  sharedWebResourceTransportKey = key;
  clearSharedWebResourceTransportIdleTimer();
  sharedWebResourceTransportPromise = openWebResourceTransport({
    e2eeKey: config.e2eeKey,
    transportPreference: config.transportPreference,
    webSocketUrl: config.resourceWebSocketUrl,
    webTransportUrl: config.resourceWebTransportUrl,
  })
    .then((client) => {
      sharedWebResourceTransport = client;
      return client;
    })
    .catch((error) => {
      if (sharedWebResourceTransportKey === key) {
        closeSharedWebResourceTransport();
      }
      throw error;
    })
    .finally(() => {
      if (sharedWebResourceTransportKey === key) {
        sharedWebResourceTransportPromise = null;
      }
    });

  return sharedWebResourceTransportPromise;
}

function webResourceTransportKey(config) {
  return JSON.stringify([
    config.transportPreference || "",
    config.resourceWebSocketUrl || "",
    config.resourceWebTransportUrl || "",
    config.e2eeKey || "",
  ]);
}

function scheduleSharedWebResourceTransportClose() {
  clearSharedWebResourceTransportIdleTimer();
  sharedWebResourceTransportIdleTimer = setTimeout(() => {
    if (sharedWebResourceTransportUseCount === 0) {
      closeSharedWebResourceTransport();
    }
  }, WEB_RESOURCE_SHARED_TRANSPORT_IDLE_MS);
}

function clearSharedWebResourceTransportIdleTimer() {
  if (sharedWebResourceTransportIdleTimer) {
    clearTimeout(sharedWebResourceTransportIdleTimer);
    sharedWebResourceTransportIdleTimer = null;
  }
}

function closeSharedWebResourceTransport() {
  clearSharedWebResourceTransportIdleTimer();
  try {
    sharedWebResourceTransport?.close();
  } catch {}
  sharedWebResourceTransport = null;
  sharedWebResourceTransportKey = "";
  sharedWebResourceTransportPromise = null;
  sharedWebResourceTransportUseCount = 0;
}

async function openWebResourceTransport({ transportPreference, webSocketUrl, webTransportUrl, e2eeKey = "" }) {
  const requireCrypto = webResourceTransportRequiresCrypto(webSocketUrl, webTransportUrl);
  const crypto = requireCrypto ? await importRemoteCrypto(e2eeKey, { required: true }) : null;
  if (shouldTryWebTransport(transportPreference, webTransportUrl)) {
    try {
      return await openWebResourceWebTransport(webTransportUrl, crypto);
    } catch (error) {
      console.info("[web-cache] WebTransport unavailable, falling back to WebSocket", error);
    }
  }

  return openWebResourceSocket(webSocketUrl, crypto);
}

function webResourceTransportRequiresCrypto(webSocketUrl, webTransportUrl) {
  return urlRequiresRemoteCrypto(webSocketUrl) || urlRequiresRemoteCrypto(webTransportUrl);
}

function urlRequiresRemoteCrypto(value) {
  if (!value) {
    return false;
  }
  try {
    const url = new URL(value);
    return url.searchParams.get("e2ee") === "v1" || url.searchParams.get("requirePassword") === "1";
  } catch {
    return false;
  }
}

function shouldTryWebTransport(transportPreference, webTransportUrl) {
  if (!webTransportUrl || transportPreference === "websocket" || transportPreference === "ws") {
    return false;
  }
  if (typeof WebTransport !== "function") {
    return false;
  }
  return transportPreference === "webtransport" || transportPreference === "wt" || webTransportUrl.startsWith("https:");
}

async function openWebResourceWebTransport(url, crypto = null) {
  const transport = new WebTransport(url, {
    congestionControl: "low-latency",
    requireUnreliable: false,
  });
  const pending = new Map();
  let nextId = 1;
  let reader = null;
  let writer = null;
  let closed = false;

  const rejectPending = (error) => {
    for (const [id, entry] of pending) {
      clearTimeout(entry.timer);
      pending.delete(id);
      entry.reject(error);
    }
  };

  const close = () => {
    if (closed) {
      return;
    }
    closed = true;
    try {
      reader?.cancel();
    } catch {}
    try {
      writer?.close();
    } catch {}
    try {
      transport.close();
    } catch {}
    rejectPending(new Error("Web resource WebTransport closed"));
  };

  try {
    await withTimeout(transport.ready, WEB_TRANSPORT_CONNECT_TIMEOUT_MS, "WebTransport connect timed out", close);
    const stream = await withTimeout(
      transport.createBidirectionalStream(),
      WEB_TRANSPORT_CONNECT_TIMEOUT_MS,
      "WebTransport stream timed out",
      close,
    );
    reader = stream.readable.getReader();
    writer = stream.writable.getWriter();
  } catch (error) {
    close();
    throw error;
  }

  readLengthPrefixedWebTransport(reader, async (payload) => {
    let message;
    try {
      payload = crypto ? await crypto.decryptText(payload) : payload;
      message = JSON.parse(payload);
    } catch (error) {
      rejectPending(new Error(`Invalid web resource WebTransport payload: ${error.message || error}`));
      return;
    }
    const id = message?.id == null ? "" : String(message.id);
    const entry = pending.get(id);
    if (!entry) {
      return;
    }
    clearTimeout(entry.timer);
    pending.delete(id);
    if (message.error) {
      entry.reject(new Error(message.error));
    } else {
      entry.resolve(message);
    }
  }).catch(() => close());

  transport.closed.then(close, close);

  return {
    close,
    isOpen() {
      return !closed;
    },
    request(payload) {
      if (closed || !writer) {
        return Promise.reject(new Error("Web resource WebTransport is not open"));
      }
      const id = String(nextId++);
      return new Promise((resolveRequest, rejectRequest) => {
        const timer = setTimeout(() => {
          pending.delete(id);
          rejectRequest(new Error(`Timed out waiting for web resource ${payload.url || id}`));
        }, WEB_RESOURCE_REQUEST_TIMEOUT_MS);
        pending.set(id, { reject: rejectRequest, resolve: resolveRequest, timer });
        encryptResourceText(crypto, JSON.stringify({ ...payload, id }))
          .then((text) => writeLengthPrefixedWebTransport(writer, text))
          .catch((error) => {
          clearTimeout(timer);
          pending.delete(id);
          rejectRequest(error);
          close();
        });
      });
    },
  };
}

function openWebResourceSocket(url, crypto = null) {
  return new Promise((resolve, reject) => {
    let opened = false;
    let settled = false;
    let clientClosing = false;
    let nextId = 1;
    const pending = new Map();
    const ws = new WebSocket(url);
    const closeSocket = () => {
      clientClosing = true;
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        try {
          ws.close();
        } catch {}
      }
    };
    const openTimer = setTimeout(() => {
      if (settled) {
        return;
      }
      settled = true;
      closeSocket();
      reject(new Error("Web resource websocket connect timed out"));
    }, WEB_RESOURCE_SOCKET_CONNECT_TIMEOUT_MS);

    const rejectPending = (error) => {
      for (const [id, entry] of pending) {
        clearTimeout(entry.timer);
        pending.delete(id);
        entry.reject(error);
      }
    };

    const client = {
      close() {
        closeSocket();
      },
      isOpen() {
        return ws.readyState === WebSocket.OPEN;
      },
      request(payload) {
        if (ws.readyState !== WebSocket.OPEN) {
          return Promise.reject(new Error("Web resource websocket is not open"));
        }
        const id = String(nextId++);
        return new Promise((resolveRequest, rejectRequest) => {
          const timer = setTimeout(() => {
            pending.delete(id);
            rejectRequest(new Error(`Timed out waiting for web resource ${payload.url || id}`));
          }, WEB_RESOURCE_REQUEST_TIMEOUT_MS);
          pending.set(id, { reject: rejectRequest, resolve: resolveRequest, timer });
          encryptResourceText(crypto, JSON.stringify({ ...payload, id }))
            .then((text) => ws.send(text))
            .catch((error) => {
              clearTimeout(timer);
              pending.delete(id);
              rejectRequest(error);
              closeSocket();
            });
        });
      },
    };

    ws.addEventListener("open", () => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(openTimer);
      opened = true;
      resolve(client);
    });
    ws.addEventListener("message", (event) => {
      handleSocketMessage(event.data);
    });
    const handleSocketMessage = async (data) => {
      let payload;
      try {
        const text = crypto ? await crypto.decryptText(data) : data;
        payload = JSON.parse(text);
      } catch (error) {
        rejectPending(new Error(`Invalid web resource websocket payload: ${error.message || error}`));
        return;
      }
      const id = payload?.id == null ? "" : String(payload.id);
      const entry = pending.get(id);
      if (!entry) {
        return;
      }
      clearTimeout(entry.timer);
      pending.delete(id);
      if (payload.error) {
        entry.reject(new Error(payload.error));
      } else {
        entry.resolve(payload);
      }
    };
    ws.addEventListener("close", () => {
      clearTimeout(openTimer);
      if (clientClosing && pending.size === 0) {
        return;
      }
      const error = new Error("Web resource websocket closed");
      if (!opened && !settled) {
        settled = true;
        reject(error);
      }
      rejectPending(error);
    });
    ws.addEventListener("error", () => {
      clearTimeout(openTimer);
      if (clientClosing && pending.size === 0) {
        return;
      }
      const error = new Error("Web resource websocket failed");
      if (!opened && !settled) {
        settled = true;
        reject(error);
      }
      rejectPending(error);
    });
  });
}

async function importRemoteCrypto(keyBase64, { required = false } = {}) {
  const rawKey = String(keyBase64 || "");
  if (!rawKey) {
    if (required) {
      throw new Error("Encrypted web resources require the remote password key");
    }
    return null;
  }
  if (!crypto?.subtle) {
    throw new Error("Encrypted web resources require Web Crypto");
  }
  const key = await crypto.subtle.importKey(
    "raw",
    base64UrlDecode(rawKey),
    { name: "AES-GCM" },
    false,
    ["decrypt", "encrypt"],
  );
  return {
    async decryptText(value) {
      const envelope = JSON.parse(String(value || ""));
      if (envelope?.type !== "e2ee" || envelope.version !== 1) {
        throw new Error("Encrypted web resource payload is required");
      }
      const decrypted = await crypto.subtle.decrypt(
        {
          additionalData: E2EE_AAD,
          iv: base64UrlDecode(String(envelope.nonce || "")),
          name: "AES-GCM",
        },
        key,
        base64UrlDecode(String(envelope.payload || "")),
      );
      return new TextDecoder().decode(decrypted);
    },
    async encryptText(value) {
      const nonce = crypto.getRandomValues(new Uint8Array(12));
      const encrypted = new Uint8Array(
        await crypto.subtle.encrypt(
          { additionalData: E2EE_AAD, iv: nonce, name: "AES-GCM" },
          key,
          new TextEncoder().encode(String(value || "")),
        ),
      );
      return JSON.stringify({
        type: "e2ee",
        version: 1,
        nonce: base64UrlEncode(nonce),
        payload: base64UrlEncode(encrypted),
      });
    },
  };
}

function encryptResourceText(crypto, text) {
  return crypto ? crypto.encryptText(text) : Promise.resolve(text);
}

function base64UrlEncode(value) {
  const bytes = bytesFromStreamValue(value);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function base64UrlDecode(value) {
  const normalized = String(value || "").replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function withTimeout(promise, ms, message, onTimeout) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      onTimeout?.();
      reject(new Error(message));
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

async function readLengthPrefixedWebTransport(reader, onPayload) {
  const decoder = new TextDecoder();
  let buffer = new Uint8Array(0);
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      return;
    }
    buffer = concatBytes(buffer, bytesFromStreamValue(value));
    while (buffer.byteLength >= 4) {
      const payloadLength = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength).getUint32(0);
      if (buffer.byteLength < 4 + payloadLength) {
        break;
      }
      const payload = buffer.slice(4, 4 + payloadLength);
      buffer = buffer.slice(4 + payloadLength);
      await onPayload(decoder.decode(payload));
    }
  }
}

function writeLengthPrefixedWebTransport(writer, text) {
  const payload = new TextEncoder().encode(text);
  const packet = new Uint8Array(4 + payload.byteLength);
  new DataView(packet.buffer).setUint32(0, payload.byteLength);
  packet.set(payload, 4);
  return writer.write(packet);
}

function bytesFromStreamValue(value) {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  return new Uint8Array(0);
}

function concatBytes(left, right) {
  if (left.byteLength === 0) {
    return right;
  }
  if (right.byteLength === 0) {
    return left;
  }
  const bytes = new Uint8Array(left.byteLength + right.byteLength);
  bytes.set(left, 0);
  bytes.set(right, left.byteLength);
  return bytes;
}

function decodeWebResourceText(payload) {
  return new TextDecoder().decode(decodeWebResourceBytes(payload));
}

function decodeWebResourceBytes(payload) {
  const encoded = String(payload.bodyBase64 || "");
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function contentTypeForUrl(value) {
  const pathname = new URL(value, self.registration.scope).pathname;
  if (pathname.endsWith(".html")) return "text/html; charset=utf-8";
  if (pathname.endsWith(".js")) return "application/javascript; charset=utf-8";
  if (pathname.endsWith(".css")) return "text/css; charset=utf-8";
  if (pathname.endsWith(".svg")) return "image/svg+xml";
  if (pathname.endsWith(".json") || pathname.endsWith(".webmanifest")) return "application/json; charset=utf-8";
  return "application/octet-stream";
}

function isExpectedWebResourceContentType(value, contentType) {
  const pathname = new URL(value, self.registration.scope).pathname.toLowerCase();
  const mime = String(contentType || "").split(";")[0].trim().toLowerCase();
  if (!mime) {
    return true;
  }
  if (pathname.endsWith(".js") || pathname.endsWith(".mjs")) {
    return mime === "application/javascript" || mime === "text/javascript" || mime === "application/ecmascript" || mime === "text/ecmascript";
  }
  if (pathname.endsWith(".css")) {
    return mime === "text/css";
  }
  if (pathname.endsWith(".html")) {
    return mime === "text/html";
  }
  if (pathname.endsWith(".wasm")) {
    return mime === "application/wasm";
  }
  if (pathname.endsWith(".json")) {
    return mime === "application/json";
  }
  if (pathname.endsWith(".webmanifest")) {
    return mime === "application/manifest+json" || mime === "application/json";
  }
  if (pathname.endsWith(".svg")) {
    return mime === "image/svg+xml";
  }
  return true;
}

function isWebResourceUrl(url) {
  return url.origin === self.location.origin && webResourcePathTail(url.pathname) !== null;
}

function isWebVersionUrl(url) {
  return webResourcePathTail(url.pathname) === "_version";
}

function isWebResourceSocketUrl(url) {
  return webResourcePathTail(url.pathname) === "_resource";
}

function isHttpUrl(url) {
  return url.protocol === "http:" || url.protocol === "https:";
}

function normalizedWebCacheKey(value) {
  const url = new URL(value, self.registration.scope);
  for (const param of WEB_CACHE_IGNORED_PARAMS) {
    url.searchParams.delete(param);
  }
  return url.toString();
}

function webCacheUrlForResource(value) {
  const url = new URL(value, self.registration.scope);
  const tail = webResourcePathTail(url.pathname);
  if (tail === null) {
    return url.toString();
  }
  const cacheUrl = new URL(tail ? `web/${tail}` : "web/", self.registration.scope);
  cacheUrl.search = url.search;
  return cacheUrl.toString();
}

function webResourcePathTail(pathname) {
  const scopedRoot = WEB_PATH_PREFIX.endsWith("/") ? WEB_PATH_PREFIX.slice(0, -1) : WEB_PATH_PREFIX;
  if (pathname === scopedRoot) {
    return "";
  }
  if (pathname.startsWith(WEB_PATH_PREFIX)) {
    return pathname.slice(WEB_PATH_PREFIX.length);
  }
  if (pathname === "/web") {
    return "";
  }
  const marker = "/web/";
  const index = pathname.indexOf(marker);
  if (index >= 0) {
    return pathname.slice(index + marker.length);
  }
  return null;
}

async function readWebCacheVersion() {
  const cache = await caches.open(WEB_CACHE_NAME);
  const response = await cache.match(WEB_VERSION_CACHE_KEY);
  if (!response) {
    return null;
  }
  return response.json().catch(() => null);
}

async function writeWebCacheVersion(value) {
  const cache = await caches.open(WEB_CACHE_NAME);
  await cache.put(
    WEB_VERSION_CACHE_KEY,
    new Response(JSON.stringify(value), {
      headers: { "content-type": "application/json; charset=utf-8" },
    }),
  );
}

async function networkFirst(request, { forceFresh = false } = {}) {
  try {
    const response = await fetch(forceFresh ? new Request(request, { cache: "reload" }) : request);
    if (response.ok && isHttpUrl(new URL(request.url))) {
      const copy = response.clone();
      caches.open(CACHE_NAME).then((cache) => cache.put(request, copy)).catch(() => {});
    }
    return response;
  } catch {
    const cached = await caches.match(request);
    if (cached) {
      return cached;
    }
    if (request.mode === "navigate") {
      const url = new URL(request.url);
      const page = url.pathname.endsWith("/control.html") ? "control.html" : "index.html";
      const pageResponse = await caches.match(new URL(page, self.registration.scope).toString());
      return pageResponse || caches.match(APP_SHELL);
    }
    return caches.match(APP_SHELL);
  }
}
