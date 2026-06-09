use super::bridge::web_bridge_id_to_string;
use super::bridge_script::WEB_BRIDGE_SCRIPT;
use super::plugin_runtime::{web_plugin_runtime_script, web_plugin_runtime_version};
use super::*;

const WEB_RESOURCE_REWRITE_VERSION: &str =
    "bridge-script-plugin-entry-v13-mobile-model-submenu-overlay";

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
    hash = fnv1a64_with_seed(web_plugin_runtime_script().as_bytes(), hash);
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
        &web_path_with_query(WEB_BRIDGE_SCRIPT_PATH, None),
    );
    push_web_cache_path(&mut paths, &web_plugin_runtime_script_src());

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
    (id, Ok(WebResourceSocketRequest { path, query }))
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
pub(super) fn infer_resource_url(main_url: &str, lookup: &WebResourceLookup) -> Option<String> {
    let base = reqwest::Url::parse(main_url).ok()?;
    let relative = lookup.display_path();
    base.join(&relative).ok().map(|url| url.to_string())
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

pub(super) fn append_html_asset_auth_query(
    input: &str,
    prefix: &str,
    request_query: Option<&str>,
) -> String {
    let Some(auth_query) = web_bridge_script_auth_query(request_query) else {
        return input.to_string();
    };
    let mut output = input.to_string();
    for marker in ["src=\"", "src='", "href=\"", "href='"] {
        output = append_auth_query_after_html_marker(&output, marker, prefix, &auth_query);
    }
    output
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
    let tags = [(!input.contains(WEB_BRIDGE_SCRIPT_PATH)).then(|| {
        format!(
            r#"<script src="{}"></script>"#,
            web_bridge_script_src(request_query)
        )
    })]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if tags.is_empty() {
        return input.to_string();
    }
    let tag = tags.join("\n");
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

fn web_plugin_runtime_script_src() -> String {
    format!(
        "{}/{}?v={}",
        WEB_PATH_PREFIX,
        WEB_PLUGIN_RUNTIME_SCRIPT_PATH,
        web_plugin_runtime_version()
    )
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

fn append_auth_query_after_html_marker(
    input: &str,
    marker: &str,
    prefix: &str,
    auth_query: &str,
) -> String {
    let quote = marker.as_bytes().last().copied().unwrap_or(b'"') as char;
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(relative_pos) = input[index..].find(marker) {
        let marker_start = index + relative_pos;
        let value_start = marker_start + marker.len();
        let Some(value_end_offset) = input[value_start..].find(quote) else {
            break;
        };
        let value_end = value_start + value_end_offset;
        let value = &input[value_start..value_end];
        output.push_str(&input[index..value_start]);
        if html_value_is_local_asset(value, prefix) && !html_value_has_auth_query(value) {
            output.push_str(&append_query_to_html_value(value, auth_query));
        } else {
            output.push_str(value);
        }
        index = value_end;
    }
    output.push_str(&input[index..]);
    output
}

fn html_value_is_local_asset(value: &str, prefix: &str) -> bool {
    if value.is_empty()
        || value.starts_with('#')
        || value.starts_with("data:")
        || value.starts_with("http:")
        || value.starts_with("https:")
        || value.starts_with("//")
    {
        return false;
    }
    let path = value
        .split('#')
        .next()
        .unwrap_or(value)
        .split('?')
        .next()
        .unwrap_or(value)
        .trim_start_matches("./");
    path.starts_with("assets/")
        || path.starts_with(&format!("{}/assets/", prefix.trim_start_matches('/')))
        || path.starts_with("/assets/")
        || path.starts_with(&format!("{}/assets/", prefix))
}

fn html_value_has_auth_query(value: &str) -> bool {
    let Some(query) = value
        .split('#')
        .next()
        .unwrap_or(value)
        .split_once('?')
        .map(|(_, query)| query)
    else {
        return false;
    };
    html_query_has_key(query, "token")
        || html_query_has_key(query, "auth")
        || html_query_has_key(query, "jwt")
}

fn html_query_has_key(query: &str, key: &str) -> bool {
    query
        .split('&')
        .any(|part| part.split_once('=').map(|(name, _)| name).unwrap_or(part) == key)
}

fn append_query_to_html_value(value: &str, query: &str) -> String {
    let (without_hash, hash) = match value.split_once('#') {
        Some((value, hash)) => (value, Some(hash)),
        None => (value, None),
    };
    let separator = if without_hash.contains('?') { '&' } else { '?' };
    let mut output = format!("{}{}{}", without_hash, separator, query);
    if let Some(hash) = hash {
        output.push('#');
        output.push_str(hash);
    }
    output
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
