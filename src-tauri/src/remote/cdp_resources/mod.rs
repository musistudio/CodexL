use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::header::{CONTENT_TYPE, LOCATION};
use hyper::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

mod bridge;
mod bridge_script;
mod cdp;
mod file_picker;
mod resource;

#[cfg(test)]
mod tests;

pub use bridge::{
    dispatch_web_bridge_message, dispatch_web_bridge_socket_payload_with_emitter,
    handle_web_bridge_websocket, spawn_web_bridge_notification_pump,
};
pub use resource::{
    dispatch_web_resource_socket_payload, get_web_resource, handle_web_resource_websocket,
};

#[cfg(test)]
use bridge::{
    is_web_bridge_socket_heartbeat, parse_web_bridge_socket_message,
    web_bridge_dispatch_expression, web_bridge_notification_install_expression,
    web_bridge_notification_poll_expression, web_bridge_socket_response,
    web_bridge_stream_poll_expression, web_bridge_stream_start_expression,
};
#[cfg(test)]
use bridge_script::WEB_BRIDGE_SCRIPT;
#[cfg(test)]
use file_picker::web_file_picker_directory_payload;
#[cfg(test)]
use resource::{
    extension_from_url, infer_resource_url, inject_web_bridge_script,
    parse_web_resource_socket_message, resource_path_matches_lookup, resource_url_with_main_query,
    rewrite_css_resource_links, rewrite_html_resource_links, runtime_fetch_url_variants,
    strip_html_content_security_policy, web_cache_resource_paths, web_path_candidates,
    web_resource_socket_response, web_resource_version,
};

const CDP_COMMAND_TIMEOUT_MS: u64 = 15000;
const DEBUG_RESOURCE_SAMPLE_LIMIT: usize = 12;
const WEB_BRIDGE_SCRIPT_PATH: &str = "_bridge.js";
const WEB_FILE_PICKER_LIST_MESSAGE: &str = "web-file-picker-list";
const WEB_FILE_PICKER_ENTRY_LIMIT: usize = 500;
const WEB_RESOURCE_SOCKET_PATH: &str = "_resource";
const WEB_RESOURCE_VERSION_PATH: &str = "_version";
const WEB_PATH_PREFIX: &str = "/web";

struct LoadedResourceContent {
    content_type: Option<String>,
    result: Value,
    source: &'static str,
    url: String,
}

#[derive(Debug, Clone)]
struct WebResourceSocketRequest {
    request_type: String,
    path: String,
    query: Option<String>,
    url: String,
}

#[derive(Debug)]
pub struct WebResourceResponse {
    status: StatusCode,
    content_type: String,
    body: Bytes,
}

impl WebResourceResponse {
    pub fn into_response(self) -> Result<Response<Full<Bytes>>, String> {
        Response::builder()
            .status(self.status)
            .header("Cache-Control", "no-store")
            .header(CONTENT_TYPE, self.content_type)
            .body(Full::new(self.body))
            .map_err(|e| e.to_string())
    }
}

pub fn web_root_redirect(request_query: Option<&str>) -> Result<Response<Full<Bytes>>, String> {
    let location = match request_query {
        Some(query) if !query.is_empty() => format!("{}/?{}", WEB_PATH_PREFIX, query),
        _ => format!("{}/", WEB_PATH_PREFIX),
    };
    Response::builder()
        .status(StatusCode::PERMANENT_REDIRECT)
        .header("Cache-Control", "no-store")
        .header(LOCATION, location)
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CdpTarget {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default, rename = "type")]
    target_type: String,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct PageResource {
    frame_id: String,
    is_frame: bool,
    is_main_frame: bool,
    mime_type: String,
    resource_type: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebResourceLookup {
    is_index: bool,
    is_resource_list: bool,
    is_resource_version: bool,
    path: String,
    query: Option<String>,
}
