use super::*;
use crate::extensions::builtins::{NodeRuntime, RuntimeSource};

fn test_bridge_config(state_dir: Option<PathBuf>) -> BotBridgeConfig {
    BotBridgeConfig {
        extension: BuiltinNodeExtension {
            id: "bot-gateway".to_string(),
            name: "Bot".to_string(),
            version: "test".to_string(),
            root_dir: PathBuf::new(),
            entry_path: PathBuf::new(),
            node: NodeRuntime {
                executable: PathBuf::new(),
                source: RuntimeSource::Explicit,
                version: "test".to_string(),
            },
        },
        state_dir,
        platform: config::BOT_PLATFORM_WEIXIN_ILINK.to_string(),
        tenant_id: "tenant".to_string(),
        integration_id: "integration".to_string(),
        poll_interval: Duration::from_millis(1),
        turn_timeout: Duration::from_secs(1),
        forward_all_codex_messages: false,
        handoff: BotHandoffConfig::default(),
        language: AppLanguage::En,
        log_path: PathBuf::new(),
    }
}

fn temp_test_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("codexl-{}-{}-{}", name, std::process::id(), nanos))
}

#[test]
fn sanitize_lock_component_keeps_lock_file_names_safe() {
    assert_eq!(sanitize_lock_component("discord"), "discord");
    assert_eq!(
        sanitize_lock_component("tenant/integration:one"),
        "tenant_integration_one"
    );
    assert_eq!(sanitize_lock_component("///"), "default");
}

#[cfg(unix)]
#[test]
fn bot_bridge_lease_blocks_duplicate_holder() {
    let state_dir = temp_test_dir("bot-bridge-lease");
    let mut config = test_bridge_config(Some(state_dir.clone()));
    config.platform = "discord".to_string();
    config.integration_id = "integration/with:unsafe".to_string();

    let first = acquire_bot_bridge_lease(&config)
        .expect("acquire first lease")
        .expect("state dir lease");
    let second = acquire_bot_bridge_lease(&config);
    assert!(second.is_err());
    assert!(second
        .err()
        .unwrap()
        .contains("another Bot Gateway bridge is already active"));

    drop(first);
    let third = acquire_bot_bridge_lease(&config);
    assert!(third.is_ok());
    drop(third);
    let _ = fs::remove_dir_all(state_dir);
}

#[test]
fn matches_turn_completed_nested_turn_id() {
    let params = json!({
        "threadId": "thread-1",
        "turn": {
            "id": "turn-1",
            "status": "completed"
        }
    });

    assert!(matches_thread_turn(&params, "thread-1", "turn-1"));
    assert!(!matches_thread_turn(&params, "thread-1", "turn-2"));
}

#[test]
fn matches_item_notifications_with_top_level_turn_id() {
    let params = json!({
        "threadId": "thread-1",
        "turnId": "turn-1",
        "item": {
            "type": "agentMessage",
            "text": "done"
        }
    });

    assert!(matches_thread_turn(&params, "thread-1", "turn-1"));
    assert!(!matches_thread_turn(&params, "thread-2", "turn-1"));
}

#[test]
fn completed_agent_message_text_ignores_non_agent_items() {
    assert_eq!(
        completed_agent_message_text(&json!({
            "item": {
                "type": "agentMessage",
                "text": "visible"
            }
        })),
        Some("visible".to_string())
    );
    assert_eq!(
        completed_agent_message_text(&json!({
            "item": {
                "type": "toolCall",
                "text": "hidden"
            }
        })),
        None
    );
}

#[test]
fn handoff_presence_uses_lock_idle_and_phone_signals() {
    let config = BotHandoffConfig {
        enabled: true,
        idle_seconds: 120,
        screen_lock: true,
        user_idle: true,
        phone_wifi_targets: vec!["iphone.local".to_string()],
        phone_bluetooth_targets: vec!["iPhone".to_string()],
    };

    let unlocked_phone_missing = handoff_presence_from_signals(
        &config,
        HandoffSignals {
            screen_locked: Some(false),
            idle_seconds: Some(5),
            phone_wifi_seen: Some(false),
            phone_bluetooth_seen: Some(false),
        },
    );
    assert!(!unlocked_phone_missing.away);
    assert!(unlocked_phone_missing
        .evidence
        .contains(&"screen unlocked".to_string()));

    let locked_target_seen = handoff_presence_from_signals(
        &config,
        HandoffSignals {
            screen_locked: Some(true),
            idle_seconds: Some(5),
            phone_wifi_seen: Some(true),
            phone_bluetooth_seen: Some(false),
        },
    );
    assert!(locked_target_seen.away);
    assert!(locked_target_seen
        .summary_for_language(AppLanguage::En)
        .contains("screen locked"));
    assert!(locked_target_seen
        .evidence
        .contains(&"wifi target seen".to_string()));

    let locked_phone_missing = handoff_presence_from_signals(
        &config,
        HandoffSignals {
            screen_locked: Some(true),
            idle_seconds: Some(5),
            phone_wifi_seen: Some(false),
            phone_bluetooth_seen: Some(false),
        },
    );
    assert!(locked_phone_missing.away);
    assert!(locked_phone_missing
        .summary_for_language(AppLanguage::En)
        .contains("selected signal not detected"));

    let locked_target_unknown = handoff_presence_from_signals(
        &config,
        HandoffSignals {
            screen_locked: Some(true),
            idle_seconds: Some(5),
            phone_wifi_seen: None,
            phone_bluetooth_seen: None,
        },
    );
    assert!(locked_target_unknown.away);
    assert!(locked_target_unknown
        .summary_for_language(AppLanguage::En)
        .contains("screen locked"));

    let locked_without_targets = handoff_presence_from_signals(
        &BotHandoffConfig {
            phone_wifi_targets: Vec::new(),
            phone_bluetooth_targets: Vec::new(),
            ..config
        },
        HandoffSignals {
            screen_locked: Some(true),
            idle_seconds: Some(5),
            phone_wifi_seen: None,
            phone_bluetooth_seen: None,
        },
    );
    assert!(locked_without_targets.away);
    assert!(locked_without_targets
        .summary_for_language(AppLanguage::En)
        .contains("screen locked"));
}

#[test]
fn parses_screen_lock_from_root_ioreg_output() {
    assert_eq!(
        parse_screen_locked_from_ioreg_output(r#""IOConsoleLocked" = Yes"#),
        Some(true)
    );
    assert_eq!(
        parse_screen_locked_from_ioreg_output(r#""IOConsoleLocked" = No"#),
        Some(false)
    );
    assert_eq!(
        parse_screen_locked_from_ioreg_output(r#""CGSSessionScreenIsLocked" = 1"#),
        Some(true)
    );
}

#[test]
fn parses_arp_scan_targets() {
    let targets = parse_arp_scan_targets(
        "iphone.home (192.168.1.23) at aa:bb:cc:dd:ee:ff on en0 ifscope [ethernet]\n\
             printer.home (192.168.1.40) at (incomplete) on en0 ifscope [ethernet]",
    );

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].source, "wifi");
    assert_eq!(targets[0].label, "iphone.home (192.168.1.23)");
    assert_eq!(targets[0].target, "192.168.1.23");
    assert!(targets[0].detail.contains("aa:bb:cc:dd:ee:ff"));
}

#[test]
fn parses_bluetooth_scan_targets_from_json() {
    let targets = parse_bluetooth_scan_targets(
        r#"{
              "SPBluetoothDataType": [
                {
                  "device_name": "Jin iPhone",
                  "device_address": "AA-BB-CC-DD-EE-FF",
                  "device_connected": "Yes",
                  "device_rssi": "-55"
                }
              ]
            }"#,
    );

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].source, "bluetooth");
    assert_eq!(targets[0].label, "Jin iPhone");
    assert_eq!(targets[0].target, "AA-BB-CC-DD-EE-FF");
    assert!(targets[0].detail.contains("connected Yes"));
}

#[test]
fn bluetooth_target_match_accepts_saved_name_and_id() {
    let target = BotHandoffScanTarget {
        id: "bluetooth:AA-BB-CC-DD-EE-FF".to_string(),
        label: "Jin iPhone".to_string(),
        target: "AA-BB-CC-DD-EE-FF".to_string(),
        detail: "RSSI -55".to_string(),
        source: "bluetooth".to_string(),
    };

    assert!(bluetooth_scan_target_matches(
        &target,
        "Jin iPhone(AA-BB-CC-DD-EE-FF)"
    ));
    assert!(bluetooth_scan_target_matches(&target, "Jin iPhone"));
    assert!(bluetooth_scan_target_matches(&target, "AA:BB:CC:DD:EE:FF"));
}

#[test]
fn empty_bluetooth_scan_is_unknown_not_missing() {
    assert_eq!(
        bluetooth_targets_seen_from_scan_targets(&["Jin iPhone"], &[]),
        None
    );
}

#[test]
fn parses_core_bluetooth_scan_targets_without_name() {
    let targets = parse_bluetooth_scan_targets(
        r#"[{"identifier":"12345678-90AB-CDEF-1234-567890ABCDEF","name":"","rssi":-61}]"#,
    );

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].source, "bluetooth");
    assert_eq!(targets[0].target, "12345678-90AB-CDEF-1234-567890ABCDEF");
    assert!(targets[0].label.contains("Bluetooth device"));
    assert!(targets[0].detail.contains("RSSI -61"));
}

#[test]
fn parses_ioreg_bluetooth_scan_targets() {
    let mut targets = Vec::new();
    collect_ioreg_bluetooth_scan_targets(
        r#"+-o IOBluetoothDevice  <class IOBluetoothDevice, id 0x1, registered>
  | {
  |   "DeviceName" = "Jin iPhone"
  |   "BD_ADDR" = <aabbccddeeff>
  |   "DeviceType" = "Phone"
  | }
"#,
        &mut targets,
    );

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].label, "Jin iPhone");
    assert_eq!(targets[0].target, "aa:bb:cc:dd:ee:ff");
}

#[test]
fn conversation_ref_includes_weixin_context_token() {
    let event = json!({
        "conversation": {
            "id": "user-1",
            "type": "dm"
        },
        "message": {
            "threadId": "session-1"
        },
        "raw": {
            "context_token": "token-1"
        }
    });

    assert_eq!(
        conversation_ref(&event),
        json!({
            "platformConversationId": "user-1",
            "type": "dm",
            "threadId": "session-1",
            "contextToken": "token-1"
        })
    );
}

#[test]
fn dingtalk_stream_message_becomes_gateway_event() {
    let mut config = test_bridge_config(None);
    config.platform = config::BOT_PLATFORM_DINGTALK.to_string();
    config.tenant_id = "tenant-1".to_string();
    config.integration_id = "integration-1".to_string();
    let envelope = json!({
        "specVersion": "1.0",
        "type": "CALLBACK",
        "headers": {
            "topic": "/v1.0/im/bot/messages/get",
            "messageId": "stream-message-1",
            "time": "1690362102194"
        },
        "data": "{\"conversationId\":\"cid-1\",\"conversationType\":\"2\",\"conversationTitle\":\"Dev\",\"msgId\":\"msg-1\",\"senderStaffId\":\"user-1\",\"senderNick\":\"Alice\",\"sessionWebhook\":\"https://oapi.dingtalk.com/robot/sendBySession?session=abc\",\"text\":{\"content\":\" hello \"},\"msgtype\":\"text\"}"
    });

    let queued = dingtalk_queued_event_from_stream(&config, &envelope).expect("queued event");
    let event = queued.get("event").expect("event");

    assert_eq!(queued["id"], "tenant-1:integration-1:dingtalk:msg-1");
    assert_eq!(queued["source"], "dingtalk_stream");
    assert_eq!(event["platform"], config::BOT_PLATFORM_DINGTALK);
    assert_eq!(event["conversation"]["id"], "cid-1");
    assert_eq!(event["conversation"]["type"], "group");
    assert_eq!(event["actor"]["id"], "user-1");
    assert_eq!(event["message"]["text"], "hello");
    assert_eq!(
        event["raw"]["context_token"],
        "https://oapi.dingtalk.com/robot/sendBySession?session=abc"
    );
}

#[test]
fn dingtalk_stream_ping_response_echoes_opaque() {
    let envelope = json!({
        "type": "SYSTEM",
        "headers": {
            "topic": "ping",
            "messageId": "ping-1"
        },
        "data": "{\"opaque\":\"opaque-1\"}"
    });

    let response = dingtalk_stream_response(&envelope, true).expect("response");

    assert_eq!(response["code"], 200);
    assert_eq!(response["headers"]["messageId"], "ping-1");
    assert_eq!(response["data"], "{\"opaque\":\"opaque-1\"}");
}

#[test]
fn feishu_gateway_config_uses_websocket_transport() {
    let mut auth_fields = BTreeMap::new();
    auth_fields.insert("appId".to_string(), "cli_app".to_string());
    auth_fields.insert("appSecret".to_string(), "secret".to_string());
    auth_fields.insert("verificationToken".to_string(), "verify".to_string());
    auth_fields.insert("domain".to_string(), "lark".to_string());
    auth_fields.insert("transport".to_string(), "webhook".to_string());

    let bot = BotProfileConfig {
        enabled: true,
        platform: config::BOT_PLATFORM_FEISHU.to_string(),
        auth_type: config::BOT_AUTH_APP_SECRET.to_string(),
        auth_fields,
        forward_all_codex_messages: false,
        handoff: BotHandoffConfig::default(),
        saved_config_id: String::new(),
        tenant_id: "tenant".to_string(),
        integration_id: "integration".to_string(),
        project_dir: String::new(),
        state_dir: String::new(),
        codex_cwd: String::new(),
        status: String::new(),
        last_login_at: String::new(),
    };

    let (credentials, integration_config) = bot_gateway_integration_auth_payload(&bot);

    assert_eq!(integration_config["transport"], json!("websocket"));
    assert_eq!(integration_config["appId"], json!("cli_app"));
    assert_eq!(integration_config["domain"], json!("lark"));
    assert_eq!(credentials["appSecret"], json!("secret"));
    assert_eq!(credentials["verificationToken"], json!("verify"));
    assert!(is_startable_bot_gateway_platform(
        config::BOT_PLATFORM_FEISHU
    ));
    assert!(is_startable_bot_gateway_platform(
        config::BOT_PLATFORM_DISCORD
    ));
    assert!(is_startable_bot_gateway_platform(
        config::BOT_PLATFORM_SLACK
    ));
    assert!(is_startable_bot_gateway_platform(
        config::BOT_PLATFORM_TELEGRAM
    ));
}

#[test]
fn local_bot_gateway_transports_default_to_socket_first_modes() {
    for (platform, expected_transport) in [
        (config::BOT_PLATFORM_SLACK, "socket"),
        (config::BOT_PLATFORM_DISCORD, "websocket"),
        (config::BOT_PLATFORM_TELEGRAM, "websocket"),
        (config::BOT_PLATFORM_FEISHU, "websocket"),
        (config::BOT_PLATFORM_DINGTALK, "websocket"),
        (config::BOT_PLATFORM_LINE, "websocket"),
        (config::BOT_PLATFORM_WECOM, "websocket"),
        (config::BOT_PLATFORM_WEIXIN_ILINK, "long_polling"),
    ] {
        let bot = BotProfileConfig {
            enabled: true,
            platform: platform.to_string(),
            auth_type: config::BOT_AUTH_BOT_TOKEN.to_string(),
            auth_fields: BTreeMap::new(),
            forward_all_codex_messages: false,
            handoff: BotHandoffConfig::default(),
            saved_config_id: String::new(),
            tenant_id: "tenant".to_string(),
            integration_id: "integration".to_string(),
            project_dir: String::new(),
            state_dir: String::new(),
            codex_cwd: String::new(),
            status: String::new(),
            last_login_at: String::new(),
        };

        let (_, integration_config) = bot_gateway_integration_auth_payload(&bot);
        assert_eq!(integration_config["transport"], json!(expected_transport));
    }
}

#[test]
fn socket_first_transports_override_saved_webhook_mode() {
    for (platform, expected_transport) in [
        (config::BOT_PLATFORM_SLACK, "socket"),
        (config::BOT_PLATFORM_DISCORD, "websocket"),
        (config::BOT_PLATFORM_TELEGRAM, "websocket"),
        (config::BOT_PLATFORM_FEISHU, "websocket"),
        (config::BOT_PLATFORM_DINGTALK, "websocket"),
        (config::BOT_PLATFORM_LINE, "websocket"),
        (config::BOT_PLATFORM_WECOM, "websocket"),
        (config::BOT_PLATFORM_WEIXIN_ILINK, "long_polling"),
    ] {
        let mut auth_fields = BTreeMap::new();
        auth_fields.insert("transport".to_string(), "webhook".to_string());
        auth_fields.insert("botToken".to_string(), "token".to_string());

        let bot = BotProfileConfig {
            enabled: true,
            platform: platform.to_string(),
            auth_type: config::BOT_AUTH_BOT_TOKEN.to_string(),
            auth_fields,
            forward_all_codex_messages: false,
            handoff: BotHandoffConfig::default(),
            saved_config_id: String::new(),
            tenant_id: "tenant".to_string(),
            integration_id: "integration".to_string(),
            project_dir: String::new(),
            state_dir: String::new(),
            codex_cwd: String::new(),
            status: String::new(),
            last_login_at: String::new(),
        };

        let (_, integration_config) = bot_gateway_integration_auth_payload(&bot);
        assert_eq!(integration_config["transport"], json!(expected_transport));
    }
}

#[test]
fn outbound_delivery_must_be_sent() {
    assert!(ensure_outbound_sent(&json!({
        "result": {
            "status": "sent"
        }
    }))
    .is_ok());

    let err = ensure_outbound_sent(&json!({
        "result": {
            "status": "failed",
            "errorCode": "99991663",
            "errorMessage": "bot has no permission to send message"
        }
    }))
    .expect_err("failed delivery should be surfaced");
    assert!(err.contains("status=failed"));
    assert!(err.contains("99991663"));
}

#[test]
fn mcp_tool_approval_prompt_uses_codex_actions_and_card_values() {
    let prompt = build_bot_approval_prompt(
        "mcpServer/elicitation/request",
        "9",
        &json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "serverName": "codexl_bot",
            "mode": "form",
            "_meta": {
                "codex_approval_kind": "mcp_tool_call",
                "persist": ["session", "always"],
                "tool_description": "Send an image to the current external bot conversation.",
                "tool_params_display": [{
                    "name": "path",
                    "display_name": "path",
                    "value": "/tmp/image.png"
                }]
            },
            "message": "Allow the codexl_bot MCP server to run tool \"send_image\"?",
            "requestedSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    )
    .expect("approval prompt");

    let labels: Vec<_> = prompt
        .actions
        .iter()
        .map(|action| action.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["Allow", "Allow for this session", "Always allow", "Cancel"]
    );
    assert_eq!(
        prompt.actions[1].result,
        json!({
            "action": "accept",
            "content": null,
            "_meta": { "persist": "session" }
        })
    );

    let card = bot_approval_card(&prompt, None);
    assert_eq!(card["title"], json!("Codex tool approval"));
    assert_eq!(card["fields"][0]["label"], json!("MCP server"));
    assert_eq!(card["fields"][2]["label"], json!("path"));
    assert_eq!(card["actions"][1]["value"]["kind"], json!("codex_approval"));
    assert_eq!(card["actions"][1]["value"]["requestId"], json!("9"));
    assert_eq!(
        card["actions"][1]["value"]["choice"],
        json!("mcp-accept-session")
    );

    let status_card = bot_approval_card(&prompt, Some(&prompt.actions[1]));
    assert_eq!(status_card["actions"].as_array().expect("actions").len(), 1);
    assert_eq!(
        status_card["actions"][0]["label"],
        json!("Allow for this session")
    );
    assert_eq!(
        status_card["actions"][0]["value"]["choice"],
        json!("mcp-accept-session")
    );
    assert_eq!(status_card["actions"][0]["disabled"], json!(true));
    assert!(status_card["body"]
        .as_str()
        .expect("body")
        .contains("Allow for this session"));
}

#[test]
fn slack_discord_approval_cards_use_string_button_callbacks() {
    let prompt = build_bot_approval_prompt(
        "mcpServer/elicitation/request",
        "9",
        &json!({
            "serverName": "codexl_bot",
            "_meta": {
                "codex_approval_kind": "mcp_tool_call",
                "persist": ["session"]
            },
            "message": "Allow this tool?",
            "requestedSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    )
    .expect("approval prompt");

    let card = bot_approval_string_callback_card(&prompt, None);
    let value = card["actions"][1]["value"].as_str().expect("value string");
    assert_eq!(card["actions"][1]["customId"], json!(value));

    let payload = serde_json::from_str::<Value>(value).expect("callback json");
    assert_eq!(payload["kind"], json!("codex_approval"));
    assert_eq!(payload["requestId"], json!("9"));
    assert_eq!(payload["choice"], json!("mcp-accept-session"));
}

#[test]
fn permissions_approval_prompt_uses_codex_labels_and_results() {
    let prompt = build_bot_approval_prompt(
        "item/permissions/requestApproval",
        "61",
        &json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "call-1",
            "cwd": "/tmp/project",
            "reason": "Need to write generated output",
            "permissions": {
                "network": { "enabled": true },
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    )
    .expect("permissions prompt");

    let labels: Vec<_> = prompt
        .actions
        .iter()
        .map(|action| action.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Yes, grant these permissions for this turn",
            "Yes, grant for this turn with strict auto review",
            "Yes, grant these permissions for this session",
            "No, continue without permissions"
        ]
    );
    assert_eq!(prompt.actions[2].result["scope"], json!("session"));
    assert_eq!(
        prompt.actions[2].result["permissions"]["fileSystem"]["write"][0],
        json!("/tmp/project/out")
    );
    assert_eq!(
        prompt.actions[3].result,
        json!({
            "permissions": {},
            "scope": "turn"
        })
    );
}

#[test]
fn dingtalk_permissions_approval_prompt_uses_action_card_message() {
    let prompt = build_bot_approval_prompt(
        "item/permissions/requestApproval",
        "61",
        &json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "call-1",
            "cwd": "/tmp/project",
            "reason": "Need to write generated output",
            "permissions": {
                "network": { "enabled": true },
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    )
    .expect("permissions prompt");

    let message = dingtalk_approval_action_card(&prompt);

    assert_eq!(message["msgtype"], json!("actionCard"));
    assert_eq!(
        message["actionCard"]["title"],
        json!("Codex permission request")
    );
    assert_eq!(message["actionCard"]["btnOrientation"], json!("0"));
    assert_eq!(
        message["actionCard"]["btns"][0]["title"],
        json!("1. Yes, grant these permissions for this turn")
    );
    assert_eq!(
        message["actionCard"]["btns"][0]["actionURL"],
        json!("dtmd://dingtalkclient/sendMessage?content=1")
    );
    assert_eq!(
        message["actionCard"]["btns"][1]["title"],
        json!("2. Yes, grant for this turn with strict auto review")
    );
    assert_eq!(
        message["actionCard"]["btns"][1]["actionURL"],
        json!("dtmd://dingtalkclient/sendMessage?content=2")
    );
    assert_eq!(
        message["actionCard"]["btns"][2]["title"],
        json!("3. Yes, grant these permissions for this session")
    );
    assert_eq!(
        message["actionCard"]["btns"][3]["title"],
        json!("4. No, continue without permissions")
    );
    let text = message["actionCard"]["text"].as_str().expect("card text");
    assert!(text.contains("Requested permissions"));
    assert!(text.contains("/tmp/project/out"));
    assert!(text.contains("Click a button, or reply with an option number or label."));
}

#[test]
fn dingtalk_action_card_encodes_button_message_content() {
    let prompt = BotApprovalPrompt {
        request_key: "approval-1".to_string(),
        title: "Approve".to_string(),
        body: "Choose an option.".to_string(),
        fields: Vec::new(),
        actions: vec![BotApprovalAction {
            key: "custom".to_string(),
            label: "Allow".to_string(),
            result: json!({}),
        }],
    };

    let message = dingtalk_approval_action_card(&prompt);

    assert_eq!(message["actionCard"]["btnOrientation"], json!("1"));
    assert_eq!(message["actionCard"]["btns"][0]["title"], json!("1. Allow"));
    assert_eq!(
        message["actionCard"]["btns"][0]["actionURL"],
        json!("dtmd://dingtalkclient/sendMessage?content=1")
    );
}

#[test]
fn dingtalk_session_webhook_accepts_context_token_alias() {
    let event = json!({
        "raw": {
            "context_token": "https://oapi.dingtalk.com/robot/sendBySession?session=abc"
        }
    });

    assert_eq!(
        dingtalk_event_session_webhook(&event),
        Some("https://oapi.dingtalk.com/robot/sendBySession?session=abc")
    );
}

#[test]
fn dingtalk_image_markdown_message_uses_media_reference() {
    let body =
        dingtalk_image_markdown_message(Some("screenshot"), "@lADPDgQ9qR6-example", "screen.png");

    assert_eq!(body["msgtype"], json!("markdown"));
    assert_eq!(body["markdown"]["title"], json!("screenshot"));
    assert!(body["markdown"]["text"]
        .as_str()
        .expect("markdown text")
        .contains("![screen.png](@lADPDgQ9qR6-example)"));
}

#[test]
fn dingtalk_media_type_detects_images_for_upload() {
    let mut media = Map::new();
    media.insert(
        "filename".to_string(),
        Value::String("screen.png".to_string()),
    );

    assert_eq!(dingtalk_upload_media_type(&media), "image");
}

#[test]
fn approval_choice_extracts_dingtalk_action_card_reply_number() {
    let prompt = build_permissions_approval_prompt(
        "61",
        &json!({
            "permissions": {
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    );
    let event = json!({
        "platform": config::BOT_PLATFORM_DINGTALK,
        "type": "message.created",
        "conversation": {
            "id": "cid-1"
        },
        "message": {
            "id": "msg-1",
            "text": "3"
        }
    });

    assert_eq!(
        bot_approval_choice_from_event(&event, "61", &prompt.actions),
        Some("permissions-session".to_string())
    );
}

#[test]
fn approval_choice_extracts_dingtalk_raw_text_content() {
    let prompt = build_permissions_approval_prompt(
        "61",
        &json!({
            "permissions": {
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    );
    let event = json!({
        "platform": config::BOT_PLATFORM_DINGTALK,
        "type": "message.created",
        "conversation": {
            "id": "unknown"
        },
        "message": {
            "id": "msg-1"
        },
        "raw": {
            "conversationId": "cid-1",
            "text": {
                "content": " 3 "
            }
        }
    });

    assert_eq!(
        bot_approval_choice_from_event(&event, "61", &prompt.actions),
        Some("permissions-session".to_string())
    );
}

#[test]
fn approval_choice_extracts_feishu_button_value() {
    let prompt = build_permissions_approval_prompt(
        "61",
        &json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "call-1",
            "cwd": "/tmp/project",
            "permissions": {
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    );
    let event = json!({
        "type": "interaction.button_clicked",
        "message": {
            "id": "om_1",
            "text": "",
            "richText": {
                "value": {
                    "value": {
                        "kind": "codex_approval",
                        "requestId": "61",
                        "choice": "permissions-session"
                    }
                }
            }
        }
    });

    assert_eq!(
        bot_approval_choice_from_event(&event, "61", &prompt.actions),
        Some("permissions-session".to_string())
    );
    assert_eq!(
        bot_approval_choice_from_event(&event, "62", &prompt.actions),
        None
    );
}

#[test]
fn approval_choice_extracts_slack_button_value() {
    let prompt = build_permissions_approval_prompt(
        "61",
        &json!({
            "permissions": {
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    );
    let callback = bot_approval_callback_payload_string(&prompt, &prompt.actions[2]);
    let event = json!({
        "type": "interaction.button_clicked",
        "conversation": {
            "id": "unknown"
        },
        "raw": {
            "channel": {
                "id": "C123"
            },
            "actions": [{
                "value": callback
            }]
        }
    });

    assert_eq!(
        bot_approval_choice_from_event(&event, "61", &prompt.actions),
        Some("permissions-session".to_string())
    );
}

#[test]
fn approval_choice_extracts_discord_custom_id() {
    let prompt = build_permissions_approval_prompt(
        "61",
        &json!({
            "permissions": {
                "fileSystem": {
                    "write": ["/tmp/project/out"]
                }
            }
        }),
    );
    let callback = bot_approval_callback_payload_string(&prompt, &prompt.actions[0]);
    let event = json!({
        "type": "interaction.button_clicked",
        "message": {
            "id": "message-1",
            "text": callback,
            "richText": {
                "custom_id": callback
            }
        },
        "raw": {
            "id": "interaction-1",
            "token": "interaction-token",
            "channel_id": "channel-1"
        }
    });

    assert_eq!(
        bot_approval_choice_from_event(&event, "61", &prompt.actions),
        Some("permissions-turn".to_string())
    );
}

#[test]
fn approval_conversation_matches_slack_raw_channel_id() {
    let original_event = json!({
        "conversation": {
            "id": "C123"
        }
    });
    let approval_event = json!({
        "conversation": {
            "id": "unknown"
        },
        "raw": {
            "channel": {
                "id": "C123"
            }
        }
    });

    assert!(same_approval_conversation(
        &original_event,
        &approval_event,
        None
    ));
}

#[test]
fn approval_conversation_matches_dingtalk_raw_conversation_id() {
    let original_event = json!({
        "platform": config::BOT_PLATFORM_DINGTALK,
        "conversation": {
            "id": "unknown"
        },
        "raw": {
            "conversationId": "cid-1"
        },
        "actor": {
            "id": "user-1"
        }
    });
    let approval_event = json!({
        "platform": config::BOT_PLATFORM_DINGTALK,
        "conversation": {
            "id": "cid-1"
        },
        "actor": {
            "id": "user-1"
        }
    });

    assert!(same_approval_conversation(
        &original_event,
        &approval_event,
        None
    ));
}

#[test]
fn codex_input_includes_local_image_attachments() {
    let dir = temp_test_dir("bot-local-image");
    fs::create_dir_all(&dir).expect("create temp dir");
    let image_path = dir.join("image.png");
    fs::write(&image_path, b"not-really-a-png").expect("write image");

    let input = codex_input_from_bot_event(
        "please inspect this",
        &json!({
            "message": {
                "attachments": [{
                    "type": "image",
                    "url": image_path.to_string_lossy(),
                    "name": "image.png",
                    "mimeType": "image/png",
                    "sizeBytes": 16
                }]
            }
        }),
        "bot-session-1",
    );

    assert_eq!(input[0]["type"], json!("text"));
    assert!(input[0]["text"]
        .as_str()
        .expect("text")
        .contains("Bot attachments:"));
    assert!(input[0]["text"]
        .as_str()
        .expect("text")
        .contains("botSessionId=bot-session-1"));
    assert_eq!(input[1]["type"], json!("localImage"));
    assert_eq!(
        input[1]["path"].as_str(),
        Some(image_path.to_string_lossy().as_ref())
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn codex_input_accepts_attachment_only_messages() {
    let input = codex_input_from_bot_event(
        "",
        &json!({
            "message": {
                "attachments": [{
                    "type": "file",
                    "url": "/tmp/report.pdf",
                    "name": "report.pdf",
                    "mimeType": "application/pdf"
                }]
            }
        }),
        "bot-session-2",
    );

    let text = input[0]["text"].as_str().expect("text");
    assert!(text.contains("Please review the attached media/file(s)."));
    assert!(text.contains("report.pdf"));
    assert_eq!(input.as_array().expect("array").len(), 1);
}

#[test]
fn bot_media_mcp_lists_direct_media_tools() {
    let tools = bot_media_mcp_tools();
    let names: Vec<_> = tools
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();

    assert!(names.contains(&"send_media"));
    assert!(names.contains(&"send_image"));
    assert!(names.contains(&"send_file"));
    assert!(names.contains(&"send_video"));
    assert!(names.contains(&"send_audio"));
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["botSessionId"]));
}

#[test]
fn bot_media_mcp_replies_with_jsonl_for_jsonl_clients() {
    let input = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";
    let mut output = Vec::new();

    run_bot_media_mcp_stdio_with_io(input.as_bytes(), &mut output).expect("run mcp server");

    let text = String::from_utf8(output).expect("utf8 output");
    let responses: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(responses[1]["id"], json!(2));
    assert!(responses[1]["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .any(|tool| tool["name"] == json!("send_image")));
}

#[test]
fn bot_media_context_is_loaded_by_session_id() {
    let state_dir = temp_test_dir("bot-media-context-store");
    let config = test_bridge_config(Some(state_dir.clone()));
    let session_key = r#"["tenant","integration","weixin-ilink","user-1"]"#;
    let session_id = "123e4567-e89b-42d3-a456-426614174000".to_string();

    persist_bot_media_context(
        &config,
        BotMediaMcpContext {
            session_id: session_id.clone(),
            session_key: session_key.to_string(),
            thread_id: Some("thread-1".to_string()),
            tenant_id: "tenant".to_string(),
            integration_id: "integration".to_string(),
            platform: config::BOT_PLATFORM_WEIXIN_ILINK.to_string(),
            conversation_ref: json!({ "platformConversationId": "user-1", "type": "dm" }),
            event_id: Some("event-1".to_string()),
            cwd: Some("/tmp/project".to_string()),
            updated_at: 1,
        },
    )
    .expect("persist media context");

    let mut args = Map::new();
    args.insert(
        "botSessionId".to_string(),
        Value::String(session_id.clone()),
    );
    let context =
        load_bot_media_context_for_tool(&config, &args).expect("load media context by session");
    assert_eq!(context.session_id, session_id);
    assert_eq!(context.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(
        context.conversation_ref["platformConversationId"],
        json!("user-1")
    );

    let _ = fs::remove_dir_all(state_dir);
}

#[test]
fn bot_media_session_id_resolves_to_uuid() {
    let state_dir = temp_test_dir("bot-media-session-id");
    let config = test_bridge_config(Some(state_dir.clone()));

    let session_id = resolve_bot_media_session_id(&config, "session-key", None);

    assert_eq!(session_id.len(), 36);
    assert!(is_uuid_like(&session_id));
    assert!(!session_id.starts_with("bot-"));

    let _ = fs::remove_dir_all(state_dir);
}

#[test]
fn bot_media_tool_requires_session_id() {
    let state_dir = temp_test_dir("bot-media-context-missing-session");
    let config = test_bridge_config(Some(state_dir.clone()));
    persist_bot_media_context(
        &config,
        BotMediaMcpContext {
            session_id: "bot-session".to_string(),
            session_key: "session-key".to_string(),
            thread_id: Some("thread-1".to_string()),
            tenant_id: "tenant".to_string(),
            integration_id: "integration".to_string(),
            platform: config::BOT_PLATFORM_WEIXIN_ILINK.to_string(),
            conversation_ref: json!({ "platformConversationId": "user-1", "type": "dm" }),
            event_id: Some("event-1".to_string()),
            cwd: None,
            updated_at: 1,
        },
    )
    .expect("persist media context");

    let args = Map::new();
    let err = load_bot_media_context_for_tool(&config, &args).expect_err("session id is required");
    assert!(err.contains("botSessionId"));

    let _ = fs::remove_dir_all(state_dir);
}

#[test]
fn bot_media_intent_infers_latest_media_fields() {
    let dir = temp_test_dir("bot-video-media");
    fs::create_dir_all(&dir).expect("create temp dir");
    let video_path = dir.join("clip.mp4");
    fs::write(&video_path, b"not-really-a-video").expect("write video");

    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(video_path.to_string_lossy().to_string()),
    );
    args.insert(
        "caption".to_string(),
        Value::String("demo clip".to_string()),
    );
    args.insert(
        "durationMs".to_string(),
        Value::Number(serde_json::Number::from(1234_u64)),
    );

    let intent = build_bot_media_intent(
        BotMediaToolKind::Video,
        &args,
        None,
        config::BOT_PLATFORM_WEIXIN_ILINK,
    )
    .expect("media intent");

    assert_eq!(intent["type"], json!("media"));
    assert_eq!(intent["caption"], json!("demo clip"));
    assert_eq!(intent["media"]["filename"], json!("clip.mp4"));
    assert_eq!(intent["media"]["mimeType"], json!("video/mp4"));
    assert_eq!(intent["media"]["durationMs"], json!(1234));
    assert_eq!(intent["media"]["sizeBytes"], json!(18));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bot_media_intent_keeps_instruction_like_caption() {
    let dir = temp_test_dir("bot-image-instruction-caption");
    fs::create_dir_all(&dir).expect("create temp dir");
    let image_path = dir.join("DSCF7383.jpg");
    fs::write(&image_path, b"not-really-an-image").expect("write image");
    let caption = format!("将{}这个图片通过Bot发给我", image_path.to_string_lossy());

    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(image_path.to_string_lossy().to_string()),
    );
    args.insert("caption".to_string(), Value::String(caption.clone()));

    let intent = build_bot_media_intent(
        BotMediaToolKind::Image,
        &args,
        None,
        config::BOT_PLATFORM_FEISHU,
    )
    .expect("media intent");

    assert_eq!(intent["type"], json!("media"));
    assert_eq!(intent["caption"], json!(caption));
    assert_eq!(intent["fallbackText"], intent["caption"]);
    assert_eq!(intent["media"]["filename"], json!("DSCF7383.jpg"));
    assert_eq!(intent["media"]["mimeType"], json!("image/jpeg"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bot_media_intent_keeps_generated_image_label_caption() {
    let dir = temp_test_dir("bot-image-generated-caption");
    fs::create_dir_all(&dir).expect("create temp dir");
    let image_path = dir.join("DSCF7383_compressed.jpg");
    fs::write(&image_path, b"not-really-an-image").expect("write image");
    let caption = "📷 DSCF7383.jpg - 来自桌面 0322 文件夹";

    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(image_path.to_string_lossy().to_string()),
    );
    args.insert("caption".to_string(), Value::String(caption.to_string()));

    let intent = build_bot_media_intent(
        BotMediaToolKind::Image,
        &args,
        None,
        config::BOT_PLATFORM_FEISHU,
    )
    .expect("media intent");

    assert_eq!(intent["type"], json!("media"));
    assert_eq!(intent["caption"], json!(caption));
    assert_eq!(intent["fallbackText"], intent["caption"]);
    assert_eq!(
        intent["media"]["filename"],
        json!("DSCF7383_compressed.jpg")
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn weixin_media_intent_requires_uploadable_url() {
    let mut args = Map::new();
    args.insert("id".to_string(), Value::String("media-key".to_string()));

    let err = build_bot_media_intent(
        BotMediaToolKind::Media,
        &args,
        None,
        config::BOT_PLATFORM_WEIXIN_ILINK,
    )
    .expect_err("weixin id-only media is not uploadable");

    assert!(err.contains("path or url"));
}

#[test]
fn bot_session_key_is_scoped_to_platform_conversation() {
    let config = test_bridge_config(None);
    let first = bot_session_key(
        &config,
        &json!({
            "tenantId": "tenant",
            "platform": "weixin-ilink",
            "conversation": { "id": "user-1" }
        }),
    );
    let second = bot_session_key(
        &config,
        &json!({
            "tenantId": "tenant",
            "platform": "weixin-ilink",
            "conversation": { "id": "user-2" }
        }),
    );

    assert_ne!(first, second);
    assert!(first.contains("integration"));
    assert!(first.contains("user-1"));
}

#[test]
fn legacy_bot_thread_names_include_integration_and_tenant() {
    let mut config = test_bridge_config(None);
    config.integration_id = "integration-id".to_string();
    config.tenant_id = "profile-name".to_string();

    assert_eq!(
        legacy_bot_thread_names(&config),
        vec![
            "Bot: integration-id".to_string(),
            "Bot: profile-name".to_string()
        ]
    );
}

#[test]
fn bot_session_store_persists_and_removes_state() {
    let state_dir = temp_test_dir("bot-session-store");
    let config = test_bridge_config(Some(state_dir.clone()));
    let key = "session-key";

    persist_bot_session_state(
        &config,
        key,
        PersistedBotSessionState {
            thread_id: Some("thread-1".to_string()),
            selected_cwd: Some("/tmp/project".to_string()),
            media_session_id: Some("123e4567-e89b-42d3-a456-426614174000".to_string()),
            updated_at: 123,
        },
    )
    .expect("persist session");

    let store = load_bot_session_store(&bot_session_store_path(&config));
    let session = store.sessions.get(key).expect("session state");
    assert_eq!(session.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(session.selected_cwd.as_deref(), Some("/tmp/project"));
    assert_eq!(
        session.media_session_id.as_deref(),
        Some("123e4567-e89b-42d3-a456-426614174000")
    );

    persist_bot_session_state(&config, key, PersistedBotSessionState::default())
        .expect("remove session");
    let store = load_bot_session_store(&bot_session_store_path(&config));
    assert!(!store.sessions.contains_key(key));

    let _ = fs::remove_dir_all(state_dir);
}

#[test]
fn migrates_matching_integration_from_legacy_state_dir() {
    let legacy_root = temp_test_dir("bot-legacy-state");
    let target_dir = temp_test_dir("bot-target-state");
    let legacy_state_dir = legacy_root.join(".bot-gateway-state");
    fs::create_dir_all(&legacy_state_dir).expect("legacy state dir");
    fs::write(
        legacy_state_dir.join("integrations.json"),
        serde_json::to_string_pretty(&json!({
            "integrations": [
                {
                    "id": "other-integration",
                    "tenantId": "tenant",
                    "platform": "feishu"
                },
                {
                    "id": "integration",
                    "tenantId": "tenant",
                    "platform": "feishu",
                    "authType": "app_secret"
                }
            ]
        }))
        .expect("serialize legacy integrations"),
    )
    .expect("write legacy integrations");

    let mut config = test_bridge_config(Some(target_dir.clone()));
    config.extension.root_dir = legacy_root.clone();
    migrate_legacy_bot_gateway_integration(&config).expect("migrate integration");

    let store = read_bot_gateway_integration_store(&target_dir.join("integrations.json"))
        .expect("target integrations");
    assert!(integration_store_contains(&store, "integration"));
    assert!(!integration_store_contains(&store, "other-integration"));

    let _ = fs::remove_dir_all(legacy_root);
    let _ = fs::remove_dir_all(target_dir);
}

#[test]
fn finds_ascii_and_fullwidth_use_colons() {
    assert_eq!(find_use_project_colon("bot-gateway: hello"), Some((11, 1)));
    assert_eq!(find_use_project_colon("bot-gateway：hello"), Some((11, 3)));
    assert_eq!(find_use_project_colon("bot-gateway"), None);
}

#[test]
fn parses_bracketed_project_and_thread_selectors() {
    assert_eq!(parse_project_index_selector("[1]"), Some(0));
    assert_eq!(parse_project_index_selector(" [12] "), Some(11));
    assert_eq!(parse_project_index_selector("[1.2]"), None);
    assert_eq!(parse_project_index_selector("[0]"), None);

    assert_eq!(parse_thread_index_selector("[1.1]"), Some((0, 0)));
    assert_eq!(parse_thread_index_selector(" [3.12] "), Some((2, 11)));
    assert_eq!(parse_thread_index_selector("[1]"), None);
    assert_eq!(parse_thread_index_selector("[1.0]"), None);
    assert_eq!(parse_thread_index_selector("[1.2.3]"), None);
}

#[test]
fn thread_summary_prefers_name_over_preview() {
    let thread = ThreadSummary::from_value(&json!({
        "id": "019e05be-4c2e-73a2-a132-51ca1618abe9",
        "name": "Pinned thread name",
        "preview": "User question",
        "cwd": "/tmp/project",
        "updatedAt": 123,
        "status": { "type": "notLoaded" }
    }))
    .expect("thread summary");

    assert_eq!(thread.preview, "Pinned thread name");
    assert_eq!(thread.cwd.as_deref(), Some("/tmp/project"));
    assert_eq!(thread.status.as_deref(), Some("notLoaded"));
}

#[test]
fn projectless_slug_is_ascii_and_stable() {
    assert_eq!(sanitize_path_segment("Hello Codex"), "hello-codex");
    assert_eq!(sanitize_path_segment("测试一下"), "");
    assert_eq!(utc_date_from_unix_seconds(0), (1970, 1, 1));
}

#[test]
fn handoff_notices_follow_app_language() {
    let presence = HandoffPresence {
        away: true,
        reasons: vec!["screen locked".to_string(), "idle for 120s".to_string()],
        evidence: Vec::new(),
    };

    let zh_on = handoff_activation_notice_for_context(
        "1234567890",
        "/tmp/project",
        &presence,
        AppLanguage::Zh,
    );
    assert!(zh_on.contains("接力模式已开启"));
    assert!(zh_on.contains("屏幕已锁定，空闲 120 秒"));
    assert!(zh_on.contains("项目：/tmp/project"));

    let en_on = handoff_activation_notice_for_context(
        "1234567890",
        "/tmp/project",
        &presence,
        AppLanguage::En,
    );
    assert!(en_on.contains("Handoff is now on"));
    assert!(en_on.contains("screen locked, idle for 120s"));
    assert!(en_on.contains("Project: /tmp/project"));
}

#[test]
fn handoff_deactivation_notice_uses_inactive_evidence() {
    let presence = HandoffPresence {
        away: false,
        reasons: Vec::new(),
        evidence: vec!["screen unlocked".to_string()],
    };

    let notice = handoff_deactivation_notice_for_context(
        "abcdef123456",
        PROJECTLESS_PROJECT_LABEL,
        Some(&presence),
        AppLanguage::Zh,
    );

    assert!(notice.contains("接力模式已关闭"));
    assert!(notice.contains("屏幕已解锁"));
    assert!(notice.contains("Session：abcdef12"));
}
