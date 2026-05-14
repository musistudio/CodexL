use super::*;

#[test]
fn maps_file_resources_relative_to_main_document() {
    let candidates = web_path_candidates(
        "file:///Applications/Codex.app/Contents/Resources/app/dist/assets/index.js",
        Some("file:///Applications/Codex.app/Contents/Resources/app/dist/index.html"),
    );

    assert!(candidates.iter().any(|path| path == "assets/index.js"));
}

#[test]
fn matches_bundled_asset_by_path_suffix() {
    let lookup = WebResourceLookup::from_request("/web/assets/index-DXqaRZzf.js", None).unwrap();

    assert!(resource_path_matches_lookup(
        "file:///Applications/Codex.app/Contents/Resources/app/dist/assets/index-DXqaRZzf.js",
        &lookup,
    ));
}

#[test]
fn infers_bundled_asset_url_from_main_document() {
    let lookup = WebResourceLookup::from_request("/web/assets/index-DXqaRZzf.js", None).unwrap();

    assert_eq!(
        infer_resource_url(
            "file:///Applications/Codex.app/Contents/Resources/app/dist/index.html",
            &lookup,
        ),
        Some(
            "file:///Applications/Codex.app/Contents/Resources/app/dist/assets/index-DXqaRZzf.js"
                .to_string()
        )
    );
}

#[test]
fn adds_main_query_variant_for_app_scheme_assets() {
    assert_eq!(
        resource_url_with_main_query(
            "app://-/assets/index-DXqaRZzf.js",
            Some("app://-/index.html?hostId=local"),
        ),
        Some("app://-/assets/index-DXqaRZzf.js?hostId=local".to_string())
    );
}

#[test]
fn runtime_fetch_variants_include_relative_and_absolute_paths() {
    let lookup = WebResourceLookup::from_request("/web/assets/index-DXqaRZzf.js", None).unwrap();
    let variants = runtime_fetch_url_variants(
        "app://-/assets/index-DXqaRZzf.js",
        Some("app://-/index.html?hostId=local"),
        &lookup,
    );

    assert!(variants.contains(&"app://-/assets/index-DXqaRZzf.js".to_string()));
    assert!(variants.contains(&"app://-/assets/index-DXqaRZzf.js?hostId=local".to_string()));
    assert!(variants.contains(&"assets/index-DXqaRZzf.js".to_string()));
    assert!(variants.contains(&"/assets/index-DXqaRZzf.js".to_string()));
}

#[test]
fn preserves_resource_queries_but_ignores_remote_tokens() {
    let lookup = WebResourceLookup::from_request(
        "/web/assets/index.js",
        Some("v=123&token=secret&codexBridgeUrl=ws%3A%2F%2Frelay%2Fweb%2F_bridge"),
    )
    .unwrap();

    assert_eq!(lookup.display_path(), "assets/index.js?v=123");
}

#[test]
fn extension_detection_requires_a_file_extension() {
    assert_eq!(
        extension_from_url("https://example.test/assets/app.js"),
        Some("js".to_string())
    );
    assert_eq!(extension_from_url("https://example.test/assets/app"), None);
}

#[test]
fn rewrites_html_absolute_paths_without_touching_protocol_relative_urls() {
    let html = r#"<script src="/assets/app.js"></script><img src="//cdn.example/a.png"><link href='/style.css'>"#;
    let rewritten = rewrite_html_resource_links(html, "/web");

    assert!(rewritten.contains("src=\"/web/assets/app.js\""));
    assert!(rewritten.contains("src=\"//cdn.example/a.png\""));
    assert!(rewritten.contains("href='/web/style.css'"));
}

#[test]
fn strips_html_content_security_policy_meta_tags() {
    let html = r#"<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="connect-src 'self'">
<META content='default-src none' HTTP-EQUIV='content-security-policy' data-note="x > y">
<meta name="viewport" content="width=device-width">
</head>"#;
    let stripped = strip_html_content_security_policy(html);

    assert!(stripped.contains(r#"<meta charset="utf-8">"#));
    assert!(stripped.contains(r#"<meta name="viewport""#));
    assert!(!stripped
        .to_ascii_lowercase()
        .contains("content-security-policy"));
    assert!(!stripped.contains("connect-src"));
    assert!(!stripped.contains("default-src none"));
}

#[test]
fn injects_web_bridge_before_module_script() {
    let html = r#"<head><script type="module" src="./assets/index.js"></script></head>"#;
    let injected = inject_web_bridge_script(html, None);

    let bridge_index = injected.find(WEB_BRIDGE_SCRIPT_PATH).unwrap();
    let module_index = injected.find("type=\"module\"").unwrap();
    assert!(bridge_index < module_index);
}

#[test]
fn injects_web_bridge_only_once() {
    let html = r#"<head><script src="/web/_bridge.js"></script></head>"#;
    let injected = inject_web_bridge_script(html, None);

    assert_eq!(injected.matches(WEB_BRIDGE_SCRIPT_PATH).count(), 1);
}

#[test]
fn injects_web_bridge_with_auth_query() {
    let html = r#"<head><script type="module" src="./assets/index.js"></script></head>"#;
    let injected = inject_web_bridge_script(
        html,
        Some(
            "v=123&hostId=local&token=secret&codexBridgeUrl=ws%3A%2F%2Frelay%2Fweb%2F_bridge&transport=auto&cloudUser=user-1&jwt=jwt-1&requirePassword=1&e2ee=v1",
        ),
    );

    assert!(injected.contains(
        r#"<script src="/web/_bridge.js?hostId=local&token=secret&cloudUser=user-1&jwt=jwt-1"></script>"#
    ));
    assert!(!injected.contains("codexBridgeUrl"));
    assert!(!injected.contains("transport=auto"));
}

#[test]
fn web_bridge_script_prefers_webtransport_with_websocket_fallback() {
    assert!(WEB_BRIDGE_SCRIPT.contains("new WebTransport"));
    assert!(WEB_BRIDGE_SCRIPT.contains("new WebSocket"));
    assert!(WEB_BRIDGE_SCRIPT.contains("openBridgeConnection"));
    assert!(WEB_BRIDGE_SCRIPT.contains("falling back to WebSocket"));
    assert!(WEB_BRIDGE_SCRIPT.contains("codexBridgeUrl"));
    assert!(WEB_BRIDGE_SCRIPT.contains("codexBridgeTransportUrl"));
    assert!(WEB_BRIDGE_SCRIPT.contains(r#"pageParams.get("token")"#));
    assert!(WEB_BRIDGE_SCRIPT.contains("scheduleBridgeReconnect"));
    assert!(WEB_BRIDGE_SCRIPT.contains("bridgeConnectionStarted"));
    assert!(WEB_BRIDGE_SCRIPT.contains("bridge-heartbeat"));
    assert!(WEB_BRIDGE_SCRIPT.contains("codex-web-bridge-status"));
    assert!(WEB_BRIDGE_SCRIPT.contains("notifyBridgeStatus"));
    assert!(WEB_BRIDGE_SCRIPT.contains("void warmBridgeConnection()"));
    assert!(WEB_BRIDGE_SCRIPT.contains("clearBridgeHeartbeatTimeout"));
    assert!(WEB_BRIDGE_SCRIPT.contains("isCurrentSocket"));
    assert!(WEB_BRIDGE_SCRIPT.contains("isCurrentClient"));
    assert!(!WEB_BRIDGE_SCRIPT.contains(r#"pageParams.get("e2ee")"#));
    assert!(!WEB_BRIDGE_SCRIPT.contains(r#"pageParams.get("requirePassword")"#));
    assert!(!WEB_BRIDGE_SCRIPT.contains("fetch(bridgeUrl"));
}

#[test]
fn web_bridge_script_intercepts_workspace_picker() {
    assert!(WEB_BRIDGE_SCRIPT.contains("electron-pick-workspace-root-option"));
    assert!(WEB_BRIDGE_SCRIPT.contains("electron-add-new-workspace-root-option"));
    assert!(WEB_BRIDGE_SCRIPT.contains("workspace-root-option-picked"));
    assert!(WEB_BRIDGE_SCRIPT.contains(WEB_FILE_PICKER_LIST_MESSAGE));
}

#[test]
fn web_bridge_picker_uses_lucide_and_shadcn_slots() {
    assert!(WEB_BRIDGE_SCRIPT.contains("data-lucide"));
    assert!(WEB_BRIDGE_SCRIPT.contains("dialog-content"));
    assert!(WEB_BRIDGE_SCRIPT.contains("dialog-footer"));
    assert!(WEB_BRIDGE_SCRIPT.contains("codex-web-folder-picker-button-default"));
    assert!(WEB_BRIDGE_SCRIPT.contains("breadcrumb-item"));
}

#[test]
fn web_bridge_dispatch_captures_workspace_navigation_messages() {
    let expression = web_bridge_dispatch_expression(&json!({
        "type": "electron-add-new-workspace-root-option",
        "root": "/tmp/example",
    }));

    assert!(expression.contains("workspace-root-option-added"));
    assert!(expression.contains("active-workspace-roots-updated"));
    assert!(expression.contains("navigate-to-route"));
}

#[test]
fn web_bridge_stream_expression_uses_incremental_buffer() {
    let start = web_bridge_stream_start_expression(&json!({
        "type": "fetch-stream",
        "requestId": "stream-1",
    }));
    let poll = web_bridge_stream_poll_expression("codex-web-test", 64);

    assert!(start.contains("__codexWebBridgeStreams"));
    assert!(start.contains("fetch-stream-event"));
    assert!(start.contains("startIdleTimer"));
    assert!(start.contains("streamKey"));
    assert!(poll.contains("splice(0, limit)"));
    assert!(poll.contains("delete streams[streamKey]"));
}

#[test]
fn web_bridge_notification_expression_forwards_mcp_events() {
    let install = web_bridge_notification_install_expression();
    let poll = web_bridge_notification_poll_expression(128);

    assert!(install.contains("__codexWebBridgeNotifications"));
    assert!(install.contains("data.type === \"mcp-notification\""));
    assert!(install.contains("data.type === \"mcp-request\""));
    assert!(install.contains("data.type === \"mcp-response\""));
    assert!(install.contains("typeof data.message.method === \"string\""));
    assert!(install.contains("data.type === \"terminal-attached\""));
    assert!(install.contains("data.type === \"terminal-data\""));
    assert!(install.contains("data.type === \"terminal-error\""));
    assert!(install.contains("data.type === \"terminal-exit\""));
    assert!(install.contains("data.type === \"terminal-init-log\""));
    assert!(poll.contains("state.messages.splice(0, limit)"));
}

#[test]
fn web_bridge_script_uses_long_timeout_for_stream_and_mcp_requests() {
    assert!(WEB_BRIDGE_SCRIPT.contains("BRIDGE_STREAM_REQUEST_TIMEOUT_MS"));
    assert!(WEB_BRIDGE_SCRIPT.contains("message?.type === \"fetch-stream\""));
    assert!(WEB_BRIDGE_SCRIPT.contains("message?.type === \"mcp-request\""));
}

#[test]
fn parses_websocket_bridge_envelope() {
    let (id, message) = parse_web_bridge_socket_message(
        r#"{"id":"42","message":{"type":"mcp-request","request":{"id":"abc"}}}"#,
    );

    assert_eq!(id.as_deref(), Some("42"));
    assert_eq!(
        message.unwrap().get("type").and_then(Value::as_str),
        Some("mcp-request")
    );
}

#[test]
fn websocket_bridge_response_preserves_messages_and_id() {
    let response = web_bridge_socket_response(
        Some("42".to_string()),
        Ok(json!({ "messages": [{ "type": "mcp-response" }] })),
    );

    assert_eq!(response.get("id").and_then(Value::as_str), Some("42"));
    assert_eq!(
        response
            .get("messages")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn recognizes_websocket_bridge_heartbeat() {
    assert!(is_web_bridge_socket_heartbeat(&json!({
        "type": "bridge-heartbeat"
    })));
    assert!(!is_web_bridge_socket_heartbeat(&json!({
        "type": "mcp-request"
    })));
}

#[test]
fn web_file_picker_payload_lists_directories_only() {
    let root = std::env::temp_dir().join(format!("codex-web-picker-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("zeta")).unwrap();
    fs::create_dir_all(root.join("Alpha")).unwrap();
    fs::write(root.join("note.txt"), b"ignored").unwrap();

    let payload = web_file_picker_directory_payload(Some(root.to_str().unwrap())).unwrap();
    let names = payload
        .get("entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let expected_path = fs::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    assert_eq!(names, vec!["Alpha", "zeta"]);
    assert_eq!(
        payload.get("path").and_then(Value::as_str),
        Some(expected_path.as_str())
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn parses_websocket_resource_request_url() {
    let (id, request) = parse_web_resource_socket_message(
        r#"{"id":7,"type":"resource","url":"http://127.0.0.1:14588/web/assets/app.js?hostId=local&token=secret"}"#,
    );
    let request = request.expect("resource request");

    assert_eq!(id.as_deref(), Some("7"));
    assert_eq!(request.path, "/web/assets/app.js");
    assert_eq!(request.query.as_deref(), Some("hostId=local&token=secret"));
}

#[test]
fn websocket_resource_response_encodes_body() {
    let response = web_resource_socket_response(
        Some("r1".to_string()),
        Ok(WebResourceResponse {
            status: StatusCode::OK,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: Bytes::from_static(b"hello"),
        }),
    );

    assert_eq!(response.get("id").and_then(Value::as_str), Some("r1"));
    assert_eq!(response.get("status").and_then(Value::as_u64), Some(200));
    assert_eq!(
        response.get("bodyBase64").and_then(Value::as_str),
        Some("aGVsbG8=")
    );
}

#[test]
fn web_version_changes_with_main_content() {
    let target = CdpTarget {
        id: "target".to_string(),
        title: "Codex".to_string(),
        target_type: "page".to_string(),
        url: "app://-/index.html?hostId=local".to_string(),
        web_socket_debugger_url: "ws://127.0.0.1/devtools/page/target".to_string(),
    };
    let resources = vec![PageResource {
        frame_id: "target".to_string(),
        is_frame: true,
        is_main_frame: true,
        mime_type: "text/html".to_string(),
        resource_type: "Document".to_string(),
        url: "app://-/index.html?hostId=local".to_string(),
    }];

    assert_ne!(
        web_resource_version(&target, &resources, Some(b"<script src=\"a.js\"></script>")),
        web_resource_version(&target, &resources, Some(b"<script src=\"b.js\"></script>"))
    );
}

#[test]
fn web_cache_manifest_extracts_html_assets() {
    let lookup =
        WebResourceLookup::from_request("/web/_version", Some("hostId=local")).expect("lookup");
    let paths = web_cache_resource_paths(
            &[],
            Some("app://-/index.html?hostId=local"),
            Some(br#"<script type="module" src="./assets/index.js"></script><link href="/assets/app.css">"#),
            &lookup,
        );

    assert!(paths.contains(&"/web/index.html?hostId=local".to_string()));
    assert!(paths.contains(&"/web/_bridge.js".to_string()));
    assert!(paths.contains(&"/web/assets/index.js".to_string()));
    assert!(paths.contains(&"/web/assets/app.css".to_string()));
}

#[test]
fn rewrites_css_absolute_urls_without_touching_protocol_relative_urls() {
    let css = r#"a{background:url(/assets/a.png)}b{background:url("//cdn/a.png")}c{background:url('/fonts/a.woff2')}"#;
    let rewritten = rewrite_css_resource_links(css, "/web");

    assert!(rewritten.contains("url(/web/assets/a.png)"));
    assert!(rewritten.contains("url(\"//cdn/a.png\")"));
    assert!(rewritten.contains("url('/web/fonts/a.woff2')"));
}
