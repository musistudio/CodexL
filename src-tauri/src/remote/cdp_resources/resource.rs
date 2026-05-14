use super::bridge::web_bridge_id_to_string;
use super::bridge_script::{web_bridge_script_response, WEB_BRIDGE_SCRIPT};
use super::cdp::{cdp_send, connect_target, list_targets, select_target};
use super::*;
use crate::remote::crypto::RemoteCrypto;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Instant;
use tokio::sync::Mutex as AsyncMutex;

const WEB_RESOURCE_TREE_CACHE_TTL_MS: u64 = 30_000;
const WEB_RESOURCE_REWRITE_VERSION: &str = "bridge-script-auth-query-v1";

static WEB_RESOURCE_TREE_CACHE: OnceLock<StdMutex<Option<CachedWebResourceTree>>> = OnceLock::new();
static WEB_RESOURCE_TREE_LOAD_LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

#[derive(Clone)]
struct WebResourceTreeSnapshot {
    main_url: Option<String>,
    resources: Vec<PageResource>,
    target: CdpTarget,
}

struct CachedWebResourceTree {
    cdp_host: String,
    cdp_port: u16,
    expires_at: Instant,
    snapshot: WebResourceTreeSnapshot,
}

pub async fn get_web_resource(
    cdp_host: &str,
    cdp_port: u16,
    request_path: &str,
    request_query: Option<&str>,
) -> Result<WebResourceResponse, String> {
    let lookup = WebResourceLookup::from_request(request_path, request_query)?;
    log_web_resource_start(cdp_host, cdp_port, request_path, &lookup);

    if lookup.path == WEB_BRIDGE_SCRIPT_PATH {
        return Ok(web_bridge_script_response());
    }

    let (snapshot, mut socket, mut next_id) = prepare_web_resource_cdp(cdp_host, cdp_port).await?;
    let target = snapshot.target;
    let resources = snapshot.resources;
    let main_url_owned = snapshot.main_url;
    let main_url = main_url_owned.as_deref();
    let main = main_document(&resources);
    log_web_resource_tree(&resources, main_url, &lookup);

    if lookup.is_resource_list {
        return Ok(WebResourceResponse {
            status: StatusCode::OK,
            content_type: "application/json; charset=utf-8".to_string(),
            body: Bytes::from(
                serde_json::to_vec(&json!({ "target": target, "resources": resources }))
                    .unwrap_or_else(|_| b"{}".to_vec()),
            ),
        });
    }

    if lookup.is_resource_version {
        let main_content = match main {
            Some(main) => load_resource_content(
                &mut socket,
                &mut next_id,
                main,
                main_url,
                &lookup,
                "version-main-document",
            )
            .await
            .ok()
            .and_then(|loaded| resource_content_bytes(&loaded.result).ok()),
            None => None,
        };
        return Ok(web_resource_version_response(
            &target,
            &resources,
            main_url,
            main_content.as_deref(),
            &lookup,
        ));
    }

    let matched = find_resource(&resources, main_url, &lookup).cloned();
    let (resource, match_source) = match matched {
        Some(resource) => (resource, "resource-tree"),
        None => match inferred_resource_from_main_document(main, &lookup) {
            Some(resource) => {
                eprintln!(
                    "[codex-web] inferred resource URL from main document: lookup={} url={} frameId={}",
                    lookup.display_path(),
                    resource.url,
                    resource.frame_id
                );
                (resource, "inferred-main-document")
            }
            None => {
                log_web_resource_not_found(&resources, main_url, &lookup);
                return Err(if lookup.is_index {
                    "Codex document resource was not found".to_string()
                } else {
                    format!("Codex resource was not found: {}", lookup.display_path())
                });
            }
        },
    };

    eprintln!(
        "[codex-web] matched resource: source={} lookup={} type={} mime={} frameId={} url={}",
        match_source,
        lookup.display_path(),
        resource.resource_type,
        empty_label(&resource.mime_type),
        resource.frame_id,
        resource.url
    );

    let loaded = match load_resource_content(
        &mut socket,
        &mut next_id,
        &resource,
        main_url,
        &lookup,
        match_source,
    )
    .await
    {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!(
                "[codex-web] resource content load failed: source={} lookup={} frameId={} url={} error={}",
                match_source,
                lookup.display_path(),
                resource.frame_id,
                resource.url,
                err
            );
            return Err(err);
        }
    };
    let result = loaded.result;
    let mut bytes = resource_content_bytes(&result)?;
    let mut loaded_resource = resource.clone();
    loaded_resource.url = loaded.url;
    if let Some(content_type) = loaded.content_type.as_deref() {
        loaded_resource.mime_type = content_type_without_params(content_type).to_string();
    }
    let content_type = loaded
        .content_type
        .as_deref()
        .map(with_charset)
        .unwrap_or_else(|| content_type_for(&loaded_resource));
    eprintln!(
        "[codex-web] resource content loaded: lookup={} contentSource={} contentType={} bytes={} base64Encoded={}",
        lookup.display_path(),
        loaded.source,
        content_type,
        bytes.len(),
        result
            .get("base64Encoded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    );

    if content_type.starts_with("text/html") {
        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
            let sanitized = strip_html_content_security_policy(&text);
            let rewritten = rewrite_html_resource_links(&sanitized, WEB_PATH_PREFIX);
            bytes = Bytes::from(inject_web_bridge_script(&rewritten, request_query));
        }
    } else if content_type.starts_with("text/css") {
        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
            bytes = Bytes::from(rewrite_css_resource_links(&text, WEB_PATH_PREFIX));
        }
    }

    Ok(WebResourceResponse {
        status: StatusCode::OK,
        content_type,
        body: bytes,
    })
}

async fn prepare_web_resource_cdp(
    cdp_host: &str,
    cdp_port: u16,
) -> Result<
    (
        WebResourceTreeSnapshot,
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        u64,
    ),
    String,
> {
    if let Some(snapshot) = cached_web_resource_tree(cdp_host, cdp_port) {
        match connect_initialized_target(&snapshot.target).await {
            Ok((socket, next_id)) => {
                eprintln!(
                    "[codex-web] resource tree cache hit: resources={} target={}",
                    snapshot.resources.len(),
                    target_debug_label(&snapshot.target)
                );
                return Ok((snapshot, socket, next_id));
            }
            Err(err) => {
                clear_cached_web_resource_tree(cdp_host, cdp_port);
                eprintln!("[codex-web] resource tree cache target failed: {}", err);
            }
        }
    }

    let load_lock = WEB_RESOURCE_TREE_LOAD_LOCK.get_or_init(|| AsyncMutex::new(()));
    let _load_guard = load_lock.lock().await;
    if let Some(snapshot) = cached_web_resource_tree(cdp_host, cdp_port) {
        match connect_initialized_target(&snapshot.target).await {
            Ok((socket, next_id)) => {
                eprintln!(
                    "[codex-web] resource tree cache hit after wait: resources={} target={}",
                    snapshot.resources.len(),
                    target_debug_label(&snapshot.target)
                );
                return Ok((snapshot, socket, next_id));
            }
            Err(err) => {
                clear_cached_web_resource_tree(cdp_host, cdp_port);
                eprintln!("[codex-web] resource tree cache target failed: {}", err);
            }
        }
    }

    let targets = list_targets(cdp_host, cdp_port).await?;
    let target = select_target(&targets)
        .ok_or_else(|| "no page target with webSocketDebuggerUrl".to_string())?;
    log_web_resource_targets(&targets, &target);

    let (mut socket, mut next_id) = connect_initialized_target(&target).await?;
    let tree = match cdp_send(&mut socket, &mut next_id, "Page.getResourceTree", json!({})).await {
        Ok(tree) => tree,
        Err(err) => {
            eprintln!("[codex-web] Page.getResourceTree failed: {}", err);
            return Err(err);
        }
    };
    let resources = resources_from_tree(&tree);
    let main_url = main_document(&resources).map(|resource| resource.url.clone());
    let snapshot = WebResourceTreeSnapshot {
        main_url,
        resources,
        target,
    };
    store_web_resource_tree(cdp_host, cdp_port, snapshot.clone());
    Ok((snapshot, socket, next_id))
}

async fn connect_initialized_target(
    target: &CdpTarget,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, u64), String> {
    let mut socket = connect_target(target).await?;
    let mut next_id = 1;

    if let Err(err) = cdp_send(&mut socket, &mut next_id, "Page.enable", json!({})).await {
        eprintln!("[codex-web] Page.enable failed: {}", err);
        return Err(err);
    }
    if let Err(err) = cdp_send(&mut socket, &mut next_id, "Runtime.enable", json!({})).await {
        eprintln!("[codex-web] Runtime.enable failed: {}", err);
        return Err(err);
    }
    Ok((socket, next_id))
}

fn cached_web_resource_tree(cdp_host: &str, cdp_port: u16) -> Option<WebResourceTreeSnapshot> {
    let cache = WEB_RESOURCE_TREE_CACHE.get_or_init(|| StdMutex::new(None));
    let mut guard = cache.lock().ok()?;
    let Some(cached) = guard.as_ref() else {
        return None;
    };
    if cached.cdp_host == cdp_host && cached.cdp_port == cdp_port {
        if cached.expires_at > Instant::now() {
            return Some(cached.snapshot.clone());
        }
        *guard = None;
    }
    None
}

fn store_web_resource_tree(cdp_host: &str, cdp_port: u16, snapshot: WebResourceTreeSnapshot) {
    let cache = WEB_RESOURCE_TREE_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedWebResourceTree {
            cdp_host: cdp_host.to_string(),
            cdp_port,
            expires_at: Instant::now() + Duration::from_millis(WEB_RESOURCE_TREE_CACHE_TTL_MS),
            snapshot,
        });
    }
}

fn clear_cached_web_resource_tree(cdp_host: &str, cdp_port: u16) {
    let cache = WEB_RESOURCE_TREE_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        if guard
            .as_ref()
            .map(|cached| cached.cdp_host == cdp_host && cached.cdp_port == cdp_port)
            .unwrap_or(false)
        {
            *guard = None;
        }
    }
}

fn web_resource_version_response(
    target: &CdpTarget,
    resources: &[PageResource],
    main_url: Option<&str>,
    main_content: Option<&[u8]>,
    lookup: &WebResourceLookup,
) -> WebResourceResponse {
    let resource_paths = web_cache_resource_paths(resources, main_url, main_content, lookup);
    let version = web_resource_version(target, resources, main_content);
    eprintln!(
        "[codex-web] resource version: version={} resources={} main={} target={}",
        version,
        resource_paths.len(),
        empty_label(main_url.unwrap_or("")),
        target_debug_label(target)
    );
    WebResourceResponse {
        status: StatusCode::OK,
        content_type: "application/json; charset=utf-8".to_string(),
        body: Bytes::from(
            serde_json::to_vec(&json!({
                "version": version,
                "mainUrl": main_url.unwrap_or(""),
                "resources": resource_paths,
                "target": target,
            }))
            .unwrap_or_else(|_| b"{}".to_vec()),
        ),
    }
}

pub(super) fn web_resource_version(
    target: &CdpTarget,
    resources: &[PageResource],
    main_content: Option<&[u8]>,
) -> String {
    let mut parts = Vec::new();
    parts.push(target.id.as_str());
    parts.push(target.title.as_str());
    parts.push(target.url.as_str());
    for resource in resources {
        parts.push(resource.url.as_str());
        parts.push(resource.mime_type.as_str());
        parts.push(resource.resource_type.as_str());
    }
    let mut hash = fnv1a64(parts.join("\n").as_bytes());
    if let Some(content) = main_content {
        hash = fnv1a64_with_seed(content, hash);
    }
    hash = fnv1a64_with_seed(WEB_BRIDGE_SCRIPT.as_bytes(), hash);
    hash = fnv1a64_with_seed(WEB_RESOURCE_REWRITE_VERSION.as_bytes(), hash);
    format!("{:016x}", hash)
}

pub(super) fn web_cache_resource_paths(
    resources: &[PageResource],
    main_url: Option<&str>,
    main_content: Option<&[u8]>,
    lookup: &WebResourceLookup,
) -> Vec<String> {
    let mut paths = Vec::new();
    push_web_cache_path(
        &mut paths,
        &web_path_with_query("index.html", lookup.query.as_deref()),
    );
    push_web_cache_path(
        &mut paths,
        &format!("{}/{}", WEB_PATH_PREFIX, WEB_BRIDGE_SCRIPT_PATH),
    );

    for resource in resources {
        if resource.url.starts_with("data:") || resource.is_main_frame {
            continue;
        }
        if let Some(path) = best_web_cache_path(&resource.url, main_url) {
            push_web_cache_path(&mut paths, &path);
        }
    }

    if let Some(content) = main_content.and_then(|content| std::str::from_utf8(content).ok()) {
        for path in extract_html_resource_paths(content) {
            push_web_cache_path(&mut paths, &path);
        }
    }

    paths
}

fn web_path_with_query(path: &str, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{}/{}?{}", WEB_PATH_PREFIX, path, query),
        _ => format!("{}/{}", WEB_PATH_PREFIX, path),
    }
}

fn best_web_cache_path(resource_url: &str, main_url: Option<&str>) -> Option<String> {
    let mut candidates = web_path_candidates(resource_url, main_url)
        .into_iter()
        .filter(|candidate| {
            !candidate.is_empty()
                && !candidate.starts_with('?')
                && !candidate.starts_with("data:")
                && !candidate.contains("://")
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| candidate.len());
    candidates
        .into_iter()
        .next()
        .map(|candidate| format!("{}/{}", WEB_PATH_PREFIX, candidate.trim_start_matches('/')))
}

fn extract_html_resource_paths(input: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for marker in [
        "src=\"",
        "src='",
        "href=\"",
        "href='",
        "data-src=\"",
        "data-src='",
    ] {
        collect_html_resource_paths_after_marker(input, marker, &mut paths);
    }
    paths
}

fn collect_html_resource_paths_after_marker(input: &str, marker: &str, paths: &mut Vec<String>) {
    let quote = marker.as_bytes().last().copied().unwrap_or(b'"') as char;
    let mut index = 0;
    while let Some(relative_pos) = input[index..].find(marker) {
        let value_start = index + relative_pos + marker.len();
        let Some(value_end) = input[value_start..].find(quote) else {
            break;
        };
        let value = &input[value_start..value_start + value_end];
        if let Some(path) = html_resource_value_to_web_path(value) {
            push_web_cache_path(paths, &path);
        }
        index = value_start + value_end + 1;
    }
}

fn html_resource_value_to_web_path(value: &str) -> Option<String> {
    if value.is_empty()
        || value.starts_with('#')
        || value.starts_with("data:")
        || value.starts_with("http:")
        || value.starts_with("https:")
        || value.starts_with("//")
    {
        return None;
    }
    if value.starts_with(WEB_PATH_PREFIX) {
        return Some(value.to_string());
    }
    let trimmed = value.trim_start_matches("./").trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("{}/{}", WEB_PATH_PREFIX, trimmed))
}

fn push_web_cache_path(paths: &mut Vec<String>, path: &str) {
    if !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_string());
    }
}

fn fnv1a64(input: &[u8]) -> u64 {
    fnv1a64_with_seed(input, 0xcbf29ce484222325)
}

fn fnv1a64_with_seed(input: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for byte in input {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub async fn handle_web_resource_websocket<S>(
    websocket: WebSocketStream<S>,
    cdp_host: String,
    cdp_port: u16,
    crypto: Option<Arc<RemoteCrypto>>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    eprintln!(
        "[codex-web] resource websocket opened: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    let (mut write, mut read) = websocket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    let writer = async {
        while let Some(message) = rx.recv().await {
            write.send(message).await.map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };

    let reader = async {
        while let Some(message) = read.next().await {
            match message.map_err(|e| e.to_string())? {
                Message::Text(raw) => {
                    handle_web_resource_socket_text(
                        &tx,
                        cdp_host.clone(),
                        cdp_port,
                        raw,
                        crypto.clone(),
                    );
                }
                Message::Binary(bytes) => match String::from_utf8(bytes) {
                    Ok(raw) => {
                        handle_web_resource_socket_text(
                            &tx,
                            cdp_host.clone(),
                            cdp_port,
                            raw,
                            crypto.clone(),
                        );
                    }
                    Err(err) => {
                        let response = web_resource_socket_response(None, Err(err.to_string()));
                        if let Some(text) =
                            encrypt_resource_socket_text(crypto.as_deref(), response.to_string())
                        {
                            let _ = tx.send(Message::Text(text));
                        }
                    }
                },
                Message::Ping(payload) => {
                    let _ = tx.send(Message::Pong(payload));
                }
                Message::Close(frame) => {
                    let _ = tx.send(Message::Close(frame));
                    break;
                }
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    let result = tokio::select! {
        result = writer => result,
        result = reader => result,
    };
    eprintln!(
        "[codex-web] resource websocket closed: cdp=http://{}:{}",
        cdp_host, cdp_port
    );
    result
}

pub async fn dispatch_web_resource_socket_payload(
    cdp_host: &str,
    cdp_port: u16,
    raw: &str,
) -> Value {
    let (id, request) = parse_web_resource_socket_message(raw);
    eprintln!(
        "[codex-web] resource socket message: id={} parseOk={}",
        id.as_deref().unwrap_or("<none>"),
        request.is_ok()
    );
    let result = match request {
        Ok(request) => {
            eprintln!(
                "[codex-web] resource socket request: id={} type={} path={} url={}",
                id.as_deref().unwrap_or("<none>"),
                request.request_type,
                request.path,
                request.url
            );
            get_web_resource(cdp_host, cdp_port, &request.path, request.query.as_deref()).await
        }
        Err(err) => Err(err),
    };
    web_resource_socket_response(id, result)
}

fn handle_web_resource_socket_text(
    tx: &mpsc::UnboundedSender<Message>,
    cdp_host: String,
    cdp_port: u16,
    raw: String,
    crypto: Option<Arc<RemoteCrypto>>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let raw = match decrypt_resource_socket_text(crypto.as_deref(), &raw) {
            Ok(raw) => raw,
            Err(err) => {
                let response = web_resource_socket_response(None, Err(err));
                if let Some(text) =
                    encrypt_resource_socket_text(crypto.as_deref(), response.to_string())
                {
                    let _ = tx.send(Message::Text(text));
                }
                return;
            }
        };
        let response = dispatch_web_resource_socket_payload(&cdp_host, cdp_port, &raw).await;
        if let Some(text) = encrypt_resource_socket_text(crypto.as_deref(), response.to_string()) {
            let _ = tx.send(Message::Text(text));
        }
    });
}

fn encrypt_resource_socket_text(crypto: Option<&RemoteCrypto>, raw: String) -> Option<String> {
    match crypto {
        Some(crypto) => match crypto.encrypt_text(&raw) {
            Ok(encrypted) => Some(encrypted),
            Err(err) => {
                eprintln!("[codex-web] resource payload encryption failed: {}", err);
                None
            }
        },
        None => Some(raw),
    }
}

fn decrypt_resource_socket_text(
    crypto: Option<&RemoteCrypto>,
    raw: &str,
) -> Result<String, String> {
    match crypto {
        Some(crypto) => crypto.decrypt_text(raw),
        None => Ok(raw.to_string()),
    }
}

pub(super) fn parse_web_resource_socket_message(
    raw: &str,
) -> (Option<String>, Result<WebResourceSocketRequest, String>) {
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(err) => return (None, Err(err.to_string())),
    };
    let id = value.get("id").and_then(web_bridge_id_to_string);
    let request_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("resource")
        .to_string();
    if request_type != "resource" && request_type != "version" {
        return (
            id,
            Err(format!(
                "unsupported resource request type: {}",
                request_type
            )),
        );
    }
    let Some(url) = value
        .get("url")
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
    else {
        return (id, Err("missing resource url".to_string()));
    };
    let (path, query) = match web_resource_socket_path_query(url) {
        Ok(value) => value,
        Err(err) => return (id, Err(err)),
    };
    let tail = path
        .strip_prefix(WEB_PATH_PREFIX)
        .unwrap_or("")
        .trim_start_matches('/');
    if request_type == "version" && tail != WEB_RESOURCE_VERSION_PATH {
        return (
            id,
            Err(format!(
                "version request must target {}/{}",
                WEB_PATH_PREFIX, WEB_RESOURCE_VERSION_PATH
            )),
        );
    }
    if tail == WEB_RESOURCE_SOCKET_PATH {
        return (
            id,
            Err("resource websocket cannot fetch itself".to_string()),
        );
    }
    (
        id,
        Ok(WebResourceSocketRequest {
            request_type,
            path,
            query,
            url: url.to_string(),
        }),
    )
}

fn web_resource_socket_path_query(value: &str) -> Result<(String, Option<String>), String> {
    let (path, query) = match reqwest::Url::parse(value) {
        Ok(url) => (url.path().to_string(), url.query().map(ToString::to_string)),
        Err(_) => match value.split_once('?') {
            Some((path, query)) => (path.to_string(), Some(query.to_string())),
            None => (value.to_string(), None),
        },
    };
    let path = web_resource_path_from_any_prefix(&path)
        .ok_or_else(|| format!("resource path must include {}", WEB_PATH_PREFIX))?;
    Ok((path, query))
}

fn web_resource_path_from_any_prefix(path: &str) -> Option<String> {
    if path == WEB_PATH_PREFIX || path.starts_with(&format!("{}/", WEB_PATH_PREFIX)) {
        return Some(path.to_string());
    }
    if let Some(index) = path.find(&format!("{}/", WEB_PATH_PREFIX)) {
        return Some(path[index..].to_string());
    }
    None
}

pub(super) fn web_resource_socket_response(
    id: Option<String>,
    result: Result<WebResourceResponse, String>,
) -> Value {
    let mut response = match result {
        Ok(response) => json!({
            "bodyBase64": encode_base64(response.body.as_ref()),
            "contentType": response.content_type,
            "status": response.status.as_u16(),
        }),
        Err(error) => json!({
            "error": error,
            "status": StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        }),
    };
    if let Value::Object(map) = &mut response {
        if let Some(id) = id {
            map.insert("id".to_string(), Value::String(id));
        }
    }
    response
}

impl WebResourceLookup {
    pub(super) fn from_request(path: &str, query: Option<&str>) -> Result<Self, String> {
        if path != WEB_PATH_PREFIX && !path.starts_with(&format!("{}/", WEB_PATH_PREFIX)) {
            return Err(format!("path must start with {}", WEB_PATH_PREFIX));
        }

        let tail = path
            .strip_prefix(WEB_PATH_PREFIX)
            .unwrap_or("")
            .trim_start_matches('/');
        let query = resource_query(query);
        let is_index = tail.is_empty() || tail == "index.html";
        Ok(Self {
            is_index,
            is_resource_list: tail == "_resources",
            is_resource_version: tail == WEB_RESOURCE_VERSION_PATH,
            path: tail.to_string(),
            query,
        })
    }

    pub(super) fn display_path(&self) -> String {
        match self.query.as_deref() {
            Some(query) if !query.is_empty() => format!("{}?{}", self.path, query),
            _ => self.path.clone(),
        }
    }
}

fn log_web_resource_start(
    cdp_host: &str,
    cdp_port: u16,
    request_path: &str,
    lookup: &WebResourceLookup,
) {
    eprintln!(
        "[codex-web] request: cdp=http://{}:{} path={} lookup={} index={} resourceList={}",
        cdp_host,
        cdp_port,
        request_path,
        lookup.display_path(),
        lookup.is_index,
        lookup.is_resource_list
    );
}

pub(super) fn log_web_resource_targets(targets: &[CdpTarget], selected: &CdpTarget) {
    eprintln!(
        "[codex-web] targets: count={} selected={}",
        targets.len(),
        target_debug_label(selected)
    );
    for target in targets.iter().take(DEBUG_RESOURCE_SAMPLE_LIMIT) {
        eprintln!("[codex-web] target sample: {}", target_debug_label(target));
    }
    if targets.len() > DEBUG_RESOURCE_SAMPLE_LIMIT {
        eprintln!(
            "[codex-web] target sample: ... {} more",
            targets.len() - DEBUG_RESOURCE_SAMPLE_LIMIT
        );
    }
}

fn log_web_resource_tree(
    resources: &[PageResource],
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) {
    eprintln!(
        "[codex-web] resource tree: total={} main={} typeCounts={}",
        resources.len(),
        main_url.unwrap_or("<none>"),
        resource_type_counts(resources)
    );

    let samples = resource_debug_samples(resources, main_url, lookup);
    if samples.is_empty() {
        eprintln!("[codex-web] resource samples: <empty>");
        return;
    }
    for sample in samples {
        eprintln!("[codex-web] resource sample: {}", sample);
    }
}

fn log_web_resource_not_found(
    resources: &[PageResource],
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) {
    eprintln!(
        "[codex-web] resource not found before inference: lookup={} totalResources={} main={}",
        lookup.display_path(),
        resources.len(),
        main_url.unwrap_or("<none>")
    );
    let samples = resource_debug_samples(resources, main_url, lookup);
    for sample in samples {
        eprintln!("[codex-web] not-found sample: {}", sample);
    }
}

fn target_debug_label(target: &CdpTarget) -> String {
    format!(
        "id={} type={} title={} url={}",
        empty_label(&target.id),
        empty_label(&target.target_type),
        empty_label(&target.title),
        empty_label(&target.url)
    )
}

fn resource_type_counts(resources: &[PageResource]) -> String {
    let mut counts = BTreeMap::<&str, usize>::new();
    for resource in resources {
        *counts
            .entry(if resource.resource_type.is_empty() {
                "<empty>"
            } else {
                resource.resource_type.as_str()
            })
            .or_default() += 1;
    }
    if counts.is_empty() {
        return "<empty>".to_string();
    }
    counts
        .into_iter()
        .map(|(kind, count)| format!("{}:{}", kind, count))
        .collect::<Vec<_>>()
        .join(",")
}

fn resource_debug_samples(
    resources: &[PageResource],
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) -> Vec<String> {
    let relevant = resources
        .iter()
        .filter(|resource| resource_looks_relevant(resource, main_url, lookup))
        .take(DEBUG_RESOURCE_SAMPLE_LIMIT)
        .map(|resource| resource_debug_label(resource, main_url))
        .collect::<Vec<_>>();
    if !relevant.is_empty() {
        return relevant;
    }

    resources
        .iter()
        .take(DEBUG_RESOURCE_SAMPLE_LIMIT)
        .map(|resource| resource_debug_label(resource, main_url))
        .collect()
}

fn resource_looks_relevant(
    resource: &PageResource,
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) -> bool {
    let lookup_path = lookup.path.as_str();
    let lookup_file_name = lookup_path.rsplit('/').next().unwrap_or(lookup_path);
    if !lookup_path.is_empty() && resource.url.contains(lookup_path) {
        return true;
    }
    if !lookup_file_name.is_empty() && resource.url.contains(lookup_file_name) {
        return true;
    }
    if resource_path_matches_lookup(&resource.url, lookup) {
        return true;
    }

    let display_path = lookup.display_path();
    web_path_candidates(&resource.url, main_url)
        .iter()
        .any(|candidate| {
            candidate == &display_path
                || candidate == lookup_path
                || (!lookup_path.is_empty() && candidate.ends_with(&format!("/{}", lookup_path)))
        })
}

fn resource_debug_label(resource: &PageResource, main_url: Option<&str>) -> String {
    let candidates = web_path_candidates(&resource.url, main_url);
    let candidates = if candidates.is_empty() {
        "<empty>".to_string()
    } else {
        candidates.into_iter().take(4).collect::<Vec<_>>().join("|")
    };
    format!(
        "type={} mime={} main={} frame={} url={} candidates={}",
        empty_label(&resource.resource_type),
        empty_label(&resource.mime_type),
        resource.is_main_frame,
        empty_label(&resource.frame_id),
        empty_label(&resource.url),
        candidates
    )
}

fn empty_label(value: &str) -> &str {
    if value.is_empty() {
        "<empty>"
    } else {
        value
    }
}
async fn load_resource_content(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: &mut u64,
    resource: &PageResource,
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
    match_source: &str,
) -> Result<LoadedResourceContent, String> {
    let mut errors = Vec::new();
    for url in page_content_url_variants(&resource.url, main_url) {
        match cdp_send(
            socket,
            next_id,
            "Page.getResourceContent",
            json!({
                "frameId": resource.frame_id,
                "url": url,
            }),
        )
        .await
        {
            Ok(result) => {
                eprintln!(
                    "[codex-web] Page.getResourceContent succeeded: source={} lookup={} url={}",
                    match_source,
                    lookup.display_path(),
                    url
                );
                return Ok(LoadedResourceContent {
                    content_type: None,
                    result,
                    source: "page-resource-content",
                    url,
                });
            }
            Err(err) => {
                eprintln!(
                    "[codex-web] Page.getResourceContent failed: source={} lookup={} frameId={} url={} error={}",
                    match_source,
                    lookup.display_path(),
                    resource.frame_id,
                    url,
                    err
                );
                errors.push(format!("Page.getResourceContent {}: {}", url, err));
            }
        }
    }

    for url in runtime_fetch_url_variants(&resource.url, main_url, lookup) {
        match runtime_fetch_resource(socket, next_id, &url).await {
            Ok(loaded) => {
                eprintln!(
                    "[codex-web] Runtime.fetch fallback succeeded: source={} lookup={} requestedUrl={} responseUrl={} status={} contentType={}",
                    match_source,
                    lookup.display_path(),
                    url,
                    loaded
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or("<empty>"),
                    loaded
                        .get("status")
                        .and_then(Value::as_u64)
                        .map(|status| status.to_string())
                        .unwrap_or_else(|| "<none>".to_string()),
                    loaded
                        .get("contentType")
                        .and_then(Value::as_str)
                        .unwrap_or("<empty>")
                );
                return Ok(LoadedResourceContent {
                    content_type: loaded
                        .get("contentType")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(ToString::to_string),
                    result: json!({
                        "base64Encoded": loaded
                            .get("base64Encoded")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        "content": loaded
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    }),
                    source: "runtime-fetch",
                    url: loaded
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or(&url)
                        .to_string(),
                });
            }
            Err(err) => {
                eprintln!(
                    "[codex-web] Runtime.fetch fallback failed: source={} lookup={} url={} error={}",
                    match_source,
                    lookup.display_path(),
                    url,
                    err
                );
                errors.push(format!("Runtime.fetch {}: {}", url, err));
            }
        }
    }

    Err(errors.join("; "))
}

async fn runtime_fetch_resource(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: &mut u64,
    url: &str,
) -> Result<Value, String> {
    let expression = runtime_fetch_expression(url);
    let result = cdp_send(
        socket,
        next_id,
        "Runtime.evaluate",
        json!({
            "awaitPromise": true,
            "expression": expression,
            "returnByValue": true,
        }),
    )
    .await?;
    let value = result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| "Runtime.evaluate returned no value".to_string())?;
    let has_content = value
        .get("content")
        .and_then(Value::as_str)
        .map(|content| !content.is_empty())
        .unwrap_or(false);
    let byte_length = value.get("byteLength").and_then(Value::as_u64).unwrap_or(0);
    if !value.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && !has_content
        && byte_length == 0
    {
        return Err(value
            .get("error")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| value.to_string()));
    }
    Ok(value)
}

fn runtime_fetch_expression(url: &str) -> String {
    let url = serde_json::to_string(url).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(async () => {{
            const url = {url};
            try {{
              const response = await fetch(url);
              const contentType = response.headers.get("content-type") || "";
              const bytes = new Uint8Array(await response.arrayBuffer());
              let binary = "";
              const chunkSize = 32768;
              for (let i = 0; i < bytes.length; i += chunkSize) {{
                binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
              }}
              return {{
                base64Encoded: true,
                byteLength: bytes.length,
                content: btoa(binary),
                contentType,
                ok: response.ok,
                status: response.status,
                statusText: response.statusText,
                url: response.url || url
              }};
            }} catch (error) {{
              return {{
                error: error && error.message ? error.message : String(error),
                ok: false,
                url
              }};
            }}
          }})()"#
    )
}
fn page_content_url_variants(resource_url: &str, main_url: Option<&str>) -> Vec<String> {
    let mut variants = Vec::new();
    push_unique_string(&mut variants, resource_url.to_string());
    if let Some(with_query) = resource_url_with_main_query(resource_url, main_url) {
        push_unique_string(&mut variants, with_query);
    }
    variants
}

pub(super) fn runtime_fetch_url_variants(
    resource_url: &str,
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) -> Vec<String> {
    let mut variants = page_content_url_variants(resource_url, main_url);
    let display_path = lookup.display_path();
    push_unique_string(&mut variants, display_path.clone());
    if !display_path.starts_with('/') {
        push_unique_string(&mut variants, format!("/{}", display_path));
    }
    variants
}

pub(super) fn resource_url_with_main_query(
    resource_url: &str,
    main_url: Option<&str>,
) -> Option<String> {
    let main = reqwest::Url::parse(main_url?).ok()?;
    let query = main.query()?;
    let mut resource = reqwest::Url::parse(resource_url).ok()?;
    if resource.query().is_some() {
        return None;
    }
    if url_origin_key(&resource) != url_origin_key(&main) {
        return None;
    }
    resource.set_query(Some(query));
    Some(resource.to_string())
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}
fn resources_from_tree(tree: &Value) -> Vec<PageResource> {
    let mut resources = Vec::new();
    if let Some(frame_tree) = tree.get("frameTree") {
        collect_frame_resources(frame_tree, true, &mut resources);
    }
    resources
}

fn collect_frame_resources(
    frame_tree: &Value,
    is_main_frame: bool,
    resources: &mut Vec<PageResource>,
) {
    let Some(frame) = frame_tree.get("frame") else {
        return;
    };
    let frame_id = frame.get("id").and_then(Value::as_str).unwrap_or("");
    if frame_id.is_empty() {
        return;
    }

    let frame_url = frame.get("url").and_then(Value::as_str).unwrap_or("");
    if !frame_url.is_empty() {
        push_unique_resource(
            resources,
            PageResource {
                frame_id: frame_id.to_string(),
                is_frame: true,
                is_main_frame,
                mime_type: frame
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("text/html")
                    .to_string(),
                resource_type: "Document".to_string(),
                url: frame_url.to_string(),
            },
        );
    }

    if let Some(items) = frame_tree.get("resources").and_then(Value::as_array) {
        for item in items {
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            if url.is_empty() {
                continue;
            }
            push_unique_resource(
                resources,
                PageResource {
                    frame_id: frame_id.to_string(),
                    is_frame: false,
                    is_main_frame: false,
                    mime_type: item
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    resource_type: item
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("Other")
                        .to_string(),
                    url: url.to_string(),
                },
            );
        }
    }

    if let Some(children) = frame_tree.get("childFrames").and_then(Value::as_array) {
        for child in children {
            collect_frame_resources(child, false, resources);
        }
    }
}

fn push_unique_resource(resources: &mut Vec<PageResource>, resource: PageResource) {
    if resources
        .iter()
        .any(|existing| existing.frame_id == resource.frame_id && existing.url == resource.url)
    {
        return;
    }
    resources.push(resource);
}

fn main_document(resources: &[PageResource]) -> Option<&PageResource> {
    resources
        .iter()
        .find(|resource| resource.is_main_frame)
        .or_else(|| {
            resources
                .iter()
                .find(|resource| resource.resource_type == "Document")
        })
}

fn find_resource<'a>(
    resources: &'a [PageResource],
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) -> Option<&'a PageResource> {
    if lookup.is_index {
        return main_document(resources);
    }
    resources
        .iter()
        .find(|resource| resource_matches_lookup(resource, main_url, lookup))
}

fn inferred_resource_from_main_document(
    main: Option<&PageResource>,
    lookup: &WebResourceLookup,
) -> Option<PageResource> {
    let main = main?;
    if lookup.is_index || lookup.is_resource_list {
        return None;
    }
    let url = infer_resource_url(&main.url, lookup)?;
    Some(PageResource {
        frame_id: main.frame_id.clone(),
        is_frame: false,
        is_main_frame: false,
        mime_type: String::new(),
        resource_type: "Other".to_string(),
        url,
    })
}

pub(super) fn infer_resource_url(main_url: &str, lookup: &WebResourceLookup) -> Option<String> {
    let base = reqwest::Url::parse(main_url).ok()?;
    let relative = lookup.display_path();
    base.join(&relative).ok().map(|url| url.to_string())
}

fn resource_matches_lookup(
    resource: &PageResource,
    main_url: Option<&str>,
    lookup: &WebResourceLookup,
) -> bool {
    if resource_path_matches_lookup(&resource.url, lookup) {
        return true;
    }

    let candidates = web_path_candidates(&resource.url, main_url);
    let display_path = lookup.display_path();
    if candidates
        .iter()
        .any(|candidate| candidate == &display_path)
    {
        return true;
    }

    if lookup.query.is_none() {
        return candidates
            .iter()
            .filter_map(|candidate| candidate.split_once('?').map(|(path, _)| path))
            .any(|candidate_path| candidate_path == lookup.path);
    }
    false
}

pub(super) fn resource_path_matches_lookup(resource_url: &str, lookup: &WebResourceLookup) -> bool {
    let parsed = reqwest::Url::parse(resource_url).ok();
    let resource_path = parsed
        .as_ref()
        .map(|url| url.path())
        .unwrap_or(resource_url)
        .trim_start_matches('/');
    let path_matches =
        resource_path == lookup.path || resource_path.ends_with(&format!("/{}", lookup.path));
    if !path_matches {
        return false;
    }

    match lookup.query.as_deref() {
        Some(query) => parsed
            .as_ref()
            .and_then(|url| url.query())
            .map(|resource_query| resource_query == query)
            .unwrap_or(false),
        None => true,
    }
}

pub(super) fn web_path_candidates(resource_url: &str, main_url: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    let parsed = reqwest::Url::parse(resource_url).ok();
    let main = main_url.and_then(|url| reqwest::Url::parse(url).ok());

    if let Some(resource) = parsed.as_ref() {
        push_path_candidate(&mut candidates, resource.path(), resource.query());

        if let Some(main) = main.as_ref() {
            let resource_origin = url_origin_key(resource);
            let main_origin = url_origin_key(main);
            if resource_origin == main_origin {
                let base_dir = url_directory(main.path());
                if let Some(relative) = resource.path().strip_prefix(&base_dir) {
                    push_path_candidate(&mut candidates, relative, resource.query());
                }
                if resource.path() == main.path() {
                    push_candidate(&mut candidates, "", resource.query());
                    push_candidate(&mut candidates, "index.html", resource.query());
                }
            }
        }
    } else {
        push_candidate(&mut candidates, resource_url.trim_start_matches('/'), None);
    }

    candidates
}

fn push_path_candidate(candidates: &mut Vec<String>, path: &str, query: Option<&str>) {
    push_candidate(candidates, path.trim_start_matches('/'), query);
}

fn push_candidate(candidates: &mut Vec<String>, path: &str, query: Option<&str>) {
    let candidate = match query {
        Some(query) if !query.is_empty() => format!("{}?{}", path, query),
        _ => path.to_string(),
    };
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn url_directory(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..=index].to_string(),
        None => String::new(),
    }
}

fn url_origin_key(url: &reqwest::Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or(""),
        url.port_or_known_default().unwrap_or(0)
    )
}

fn resource_content_bytes(result: &Value) -> Result<Bytes, String> {
    let content = result
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing resource content".to_string())?;
    if result
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return decode_base64(content)
            .map(Bytes::from)
            .ok_or_else(|| "failed to decode resource content".to_string());
    }
    Ok(Bytes::from(content.to_string()))
}

fn content_type_for(resource: &PageResource) -> String {
    let mime = resource.mime_type.trim();
    if !mime.is_empty() {
        return with_charset(mime);
    }
    with_charset(match extension_from_url(&resource.url).as_deref() {
        Some("css") => "text/css",
        Some("gif") => "image/gif",
        Some("htm") | Some("html") => "text/html",
        Some("ico") => "image/x-icon",
        Some("jpeg") | Some("jpg") => "image/jpeg",
        Some("js") | Some("mjs") => "application/javascript",
        Some("json") | Some("map") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    })
}

fn with_charset(mime: &str) -> String {
    let mime = mime.trim();
    if mime.contains("charset=") {
        return mime.to_string();
    }
    if mime.starts_with("text/")
        || mime == "application/javascript"
        || mime == "application/json"
        || mime == "image/svg+xml"
    {
        format!("{}; charset=utf-8", mime)
    } else {
        mime.to_string()
    }
}

fn content_type_without_params(content_type: &str) -> &str {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
}

pub(super) fn extension_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok();
    let path = parsed.as_ref().map(|url| url.path()).unwrap_or(url);
    let file_name = path.rsplit('/').next()?;
    file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
}

fn resource_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    let filtered = query
        .split('&')
        .filter(|part| {
            let key = part.split_once('=').map(|(key, _)| key).unwrap_or(*part);
            key != "token" && key != "codexBridgeUrl"
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("&");
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

pub(super) fn rewrite_html_resource_links(input: &str, prefix: &str) -> String {
    let mut output = input.to_string();
    for marker in [
        "src=\"",
        "src='",
        "href=\"",
        "href='",
        "action=\"",
        "action='",
        "poster=\"",
        "poster='",
        "data-src=\"",
        "data-src='",
        "content=\"",
        "content='",
    ] {
        output = rewrite_absolute_paths_after_marker(&output, marker, prefix);
    }
    rewrite_css_resource_links(&output, prefix)
}

pub(super) fn strip_html_content_security_policy(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative_pos) = input[index..].find('<') {
        let tag_start = index + relative_pos;
        output.push_str(&input[index..tag_start]);
        let Some(tag_end) = html_tag_end(input, tag_start) else {
            output.push_str(&input[tag_start..]);
            return output;
        };
        let tag = &input[tag_start..tag_end];
        if !is_html_csp_meta_tag(tag) {
            output.push_str(tag);
        }
        index = tag_end;
    }
    output.push_str(&input[index..]);
    output
}

fn html_tag_end(input: &str, tag_start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (offset, byte) in input.as_bytes()[tag_start..].iter().copied().enumerate() {
        match (quote, byte) {
            (Some(active), value) if value == active => quote = None,
            (None, b'"') | (None, b'\'') => quote = Some(byte),
            (None, b'>') => return Some(tag_start + offset + 1),
            _ => {}
        }
    }
    None
}

fn is_html_csp_meta_tag(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    lower.starts_with("<meta")
        && lower
            .as_bytes()
            .get(5)
            .map(|byte| byte.is_ascii_whitespace() || *byte == b'/' || *byte == b'>')
            .unwrap_or(false)
        && lower.contains("http-equiv")
        && lower.contains("content-security-policy")
}

pub(super) fn inject_web_bridge_script(input: &str, request_query: Option<&str>) -> String {
    if input.contains(WEB_BRIDGE_SCRIPT_PATH) {
        return input.to_string();
    }
    let tag = format!(
        r#"<script src="{}"></script>"#,
        web_bridge_script_src(request_query)
    );
    for marker in [
        "<script type=\"module\"",
        "<script type='module'",
        "</head>",
    ] {
        if let Some(index) = input.find(marker) {
            let mut output = String::with_capacity(input.len() + tag.len() + 1);
            output.push_str(&input[..index]);
            output.push_str(&tag);
            output.push('\n');
            output.push_str(&input[index..]);
            return output;
        }
    }
    format!("{}\n{}", tag, input)
}

fn web_bridge_script_src(request_query: Option<&str>) -> String {
    let base = format!("{}/{}", WEB_PATH_PREFIX, WEB_BRIDGE_SCRIPT_PATH);
    match web_bridge_script_auth_query(request_query) {
        Some(query) => format!("{}?{}", base, query),
        None => base,
    }
}

fn web_bridge_script_auth_query(request_query: Option<&str>) -> Option<String> {
    let query = request_query?;
    let filtered = query
        .split('&')
        .filter(|part| {
            let key = part.split_once('=').map(|(key, _)| key).unwrap_or(*part);
            matches!(key, "auth" | "cloudUser" | "hostId" | "jwt" | "token")
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("&");
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

fn rewrite_absolute_paths_after_marker(input: &str, marker: &str, prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative_pos) = input[index..].find(marker) {
        let marker_start = index + relative_pos;
        let value_start = marker_start + marker.len();
        output.push_str(&input[index..value_start]);
        let value = &input[value_start..];
        if value.starts_with('/') && !value.starts_with("//") {
            output.push_str(prefix);
        }
        index = value_start;
    }
    output.push_str(&input[index..]);
    output
}

pub(super) fn rewrite_css_resource_links(input: &str, prefix: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative_pos) = input[index..].find("url(") {
        let marker_start = index + relative_pos;
        let value_start = marker_start + "url(".len();
        output.push_str(&input[index..value_start]);
        let value = &input[value_start..];
        let trimmed = value.trim_start();
        let whitespace_len = value.len() - trimmed.len();
        output.push_str(&value[..whitespace_len]);
        let path_start = trimmed
            .strip_prefix('"')
            .or_else(|| trimmed.strip_prefix('\''))
            .unwrap_or(trimmed);
        let quote = if path_start.len() != trimmed.len() {
            &trimmed[..1]
        } else {
            ""
        };
        output.push_str(quote);
        if path_start.starts_with('/') && !path_start.starts_with("//") {
            output.push_str(prefix);
        }
        index = value_start + whitespace_len + quote.len();
    }
    output.push_str(&input[index..]);
    output
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(third & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' | b'\t' | b' ' => continue,
            _ => return None,
        } as u32;

        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    Some(output)
}
