use super::assets::*;
use super::ws_events::{
    build_ready_payload, control_event_delivery_payload, control_event_ws_payload,
    encode_terminal_output_frame, event_stream_lagged_payload, lifecycle_event_delivery_payload,
    lifecycle_event_session_id, lifecycle_event_ws_payload, subscribe_outcome_notice,
};
use super::ws_messages::{
    decode_binary_input, decode_input_text, decode_submit_line, decode_text_client_message,
    MAX_WS_INPUT_BYTES, MAX_WS_TEXT_FRAME_BYTES,
};
use super::*;
use crate::session::actor::{OutputFrame, ReplayCursor, SubscribeOutcome};
use crate::session::supervisor::LifecycleEvent;
use crate::types::{opcodes, ControlEvent, SessionSummary};
use crate::types::{KnownControlEventPayload, SessionTitlePayload};
use axum::body::to_bytes;
use axum::response::IntoResponse;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::sync::{broadcast, mpsc};

async fn html_string(response: impl IntoResponse) -> String {
    let response = response.into_response();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("html body");
    String::from_utf8(body.to_vec()).expect("utf8 html")
}

async fn response_json(response: Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("json body");
    serde_json::from_slice(&body).expect("json response")
}

async fn response_text(response: Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("text body");
    String::from_utf8(body.to_vec()).expect("utf8 text")
}

fn boot_payload_from_html(html: &str) -> serde_json::Value {
    let marker = "window.__SWIMMERS_BOOT__ = ";
    let start = html.find(marker).expect("boot assignment") + marker.len();
    let rest = &html[start..];
    let end = rest.find(";</script>").expect("boot script close");
    serde_json::from_str(&rest[..end]).expect("boot json")
}

#[test]
fn valid_pkg_dir_requires_js_and_wasm() {
    let dir = tempdir().expect("tempdir");
    assert!(!valid_frankentui_pkg_dir(dir.path()));

    std::fs::write(
        dir.path().join("FrankenTerm.js"),
        "export default async () => {}",
    )
    .expect("write js");
    assert!(!valid_frankentui_pkg_dir(dir.path()));

    std::fs::write(dir.path().join("FrankenTerm_bg.wasm"), b"wasm").expect("write wasm");
    assert!(valid_frankentui_pkg_dir(dir.path()));
}

fn authenticated_scopes_for_token(config: &Config, token: &str) -> AuthInfo {
    let message = Message::Text(format!(r#"{{"type":"auth","token":"{token}"}}"#).into());
    match decode_ws_auth_message(config, &message) {
        WsAuthDecision::Authenticated(auth) => auth,
        other => panic!("unexpected {token} auth decision: {other:?}"),
    }
}

#[test]
fn websocket_first_message_auth_accepts_observer_and_operator_tokens() {
    let config = Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("operator".into()),
        observer_token: Some("observer".into()),
        ..Config::default()
    };

    let operator = authenticated_scopes_for_token(&config, "operator");
    assert!(operator.has_scope(AuthScope::StreamWrite));

    let observer = authenticated_scopes_for_token(&config, "observer");
    assert!(observer.has_scope(AuthScope::SessionsRead));
    assert!(!observer.has_scope(AuthScope::StreamWrite));
}

#[test]
fn websocket_first_message_auth_rejects_invalid_and_non_auth_messages() {
    let config = Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("operator".into()),
        observer_token: Some("observer".into()),
        ..Config::default()
    };

    for msg in [
        Message::Text(r#"{"type":"ping"}"#.into()),
        Message::Text(r#"{"type":"auth","token":""}"#.into()),
        Message::Binary(b"operator".to_vec().into()),
    ] {
        assert!(
            matches!(
                decode_ws_auth_message(&config, &msg),
                WsAuthDecision::Reject { .. }
            ),
            "{msg:?}"
        );
    }
}

#[test]
fn websocket_first_message_classifier_handles_close_timeout_and_auth_message() {
    let config = Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("operator".into()),
        observer_token: Some("observer".into()),
        ..Config::default()
    };

    assert!(matches!(
        decode_ws_auth_first_message(&config, WsAuthFirstMessage::Closed),
        WsAuthDecision::Close
    ));

    match decode_ws_auth_first_message(&config, WsAuthFirstMessage::Timeout) {
        WsAuthDecision::Reject { code, message } => {
            assert_eq!(code, "WS_AUTH_TIMEOUT");
            assert!(message.contains("must authenticate"));
        }
        other => panic!("unexpected timeout auth decision: {other:?}"),
    }

    let auth_msg = Message::Text(r#"{"type":"auth","token":"operator"}"#.into());
    match decode_ws_auth_first_message(&config, WsAuthFirstMessage::Message(auth_msg)) {
        WsAuthDecision::Authenticated(auth) => {
            assert!(auth.has_scope(AuthScope::StreamWrite));
        }
        other => panic!("unexpected auth decision: {other:?}"),
    }
}

#[test]
fn query_flag_enabled_accepts_framed_values_only() {
    assert!(["1", "true", "yes", "on", "framed", "framed_v1"]
        .into_iter()
        .all(|value| query_flag_enabled(Some(value))));
    assert!([None, Some(""), Some("0"), Some("false"), Some("raw")]
        .into_iter()
        .all(|value| !query_flag_enabled(value)));
}

#[test]
fn framed_terminal_output_prefixes_opcode_and_sequence() {
    let frame = OutputFrame {
        seq: 0x0102_0304_0506_0708,
        data: b"hello".to_vec(),
    };

    let encoded = encode_terminal_output_frame(frame);

    assert_eq!(encoded[0], opcodes::TERMINAL_OUTPUT);
    let seq = u64::from_be_bytes(encoded[1..9].try_into().expect("seq bytes"));
    assert_eq!(seq, 0x0102_0304_0506_0708);
    assert_eq!(&encoded[9..], b"hello");
}

#[test]
fn control_event_ws_payload_preserves_session_event_and_payload() {
    let event = ControlEvent {
        event: "session_state".to_string(),
        session_id: "sess_0".to_string(),
        payload: serde_json::json!({
            "state": "attention",
            "previous_state": "idle",
            "current_command": "cargo test",
            "transport_health": "healthy",
            "at": "2026-05-15T10:00:00Z",
        }),
    };

    let payload = control_event_ws_payload(&event);

    assert_eq!(payload["type"], "control_event");
    assert_eq!(payload["event"], "session_state");
    assert_eq!(payload["sessionId"], "sess_0");
    assert_eq!(payload["payload"]["state"], "attention");
    assert_eq!(payload["payload"]["current_command"], "cargo test");
}

#[test]
fn control_event_ws_payload_serializes_title_and_thought_update_contracts() {
    let title = ControlEvent {
        event: "session_title".to_string(),
        session_id: "sess_0".to_string(),
        payload: serde_json::json!({
            "title": "/tmp/swimmers",
            "at": "2026-05-15T10:00:02Z",
        }),
    };
    let title_payload = control_event_ws_payload(&title);
    assert_eq!(title_payload["event"], "session_title");
    assert_eq!(title_payload["payload"]["title"], "/tmp/swimmers");

    let thought = ControlEvent {
        event: "thought_update".to_string(),
        session_id: "sess_0".to_string(),
        payload: serde_json::json!({
            "thought": "operator response needed",
            "token_count": 64000,
            "context_limit": 128000,
            "thought_state": "holding",
            "thought_source": "llm",
            "rest_state": "active",
            "commit_candidate": true,
            "objective_changed": true,
            "at": "2026-05-15T10:00:04Z",
        }),
    };
    let thought_payload = control_event_ws_payload(&thought);
    assert_eq!(thought_payload["event"], "thought_update");
    assert_eq!(thought_payload["payload"]["token_count"], 64000);
    assert_eq!(thought_payload["payload"]["commit_candidate"], true);
}

#[test]
fn control_event_payload_contract_preserves_unknown_events() {
    let event = ControlEvent {
        event: "future_event".to_string(),
        session_id: "sess_future".to_string(),
        payload: serde_json::json!({ "newField": true }),
    };

    let payload = control_event_ws_payload(&event);

    assert_eq!(payload["type"], "control_event");
    assert_eq!(payload["event"], "future_event");
    assert_eq!(payload["sessionId"], "sess_future");
    assert_eq!(payload["payload"]["newField"], true);
}

#[test]
fn known_control_event_payload_uses_tagged_serde_shape() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-05-15T10:00:02Z")
        .expect("timestamp")
        .with_timezone(&chrono::Utc);
    let tagged = serde_json::to_value(KnownControlEventPayload::SessionTitle(
        SessionTitlePayload {
            title: "/tmp/swimmers".to_string(),
            at,
        },
    ))
    .expect("tagged control event");

    assert_eq!(tagged["event"], "session_title");
    assert_eq!(tagged["payload"]["title"], "/tmp/swimmers");
}

#[test]
fn lifecycle_event_ws_payload_reports_deleted_session() {
    let event = LifecycleEvent::Deleted {
        session_id: "sess_0".to_string(),
        reason: "tmux_reconcile_missing".to_string(),
        delete_mode: crate::config::SessionDeleteMode::DetachBridge,
        tmux_session_alive: false,
    };

    let payload = lifecycle_event_ws_payload(&event);

    assert_eq!(lifecycle_event_session_id(&event), "sess_0");
    assert_eq!(payload["type"], "lifecycle_event");
    assert_eq!(payload["event"], "session_deleted");
    assert_eq!(payload["sessionId"], "sess_0");
    assert_eq!(payload["reason"], "tmux_reconcile_missing");
    assert_eq!(payload["deleteMode"], "detach_bridge");
    assert_eq!(payload["tmuxSessionAlive"], false);
}

#[test]
fn event_stream_lagged_payload_keeps_replay_quality_visible() {
    let payload = event_stream_lagged_payload("thought_events", 7);

    assert_eq!(payload["type"], "event_stream_lagged");
    assert_eq!(payload["stream"], "thought_events");
    assert_eq!(payload["skipped"], 7);
}

#[test]
fn control_event_delivery_payload_matches_session_lagged_and_closed_cases() {
    let event = ControlEvent {
        event: "session_title".to_string(),
        session_id: "sess_0".to_string(),
        payload: serde_json::json!({
            "title": "swimmers",
            "at": "2026-05-15T10:00:02Z",
        }),
    };
    let matching: Result<ControlEvent, broadcast::error::RecvError> = Ok(event);
    let payload = control_event_delivery_payload("sess_0", "session_events", &matching)
        .expect("matching event payload");
    assert_eq!(payload["type"], "control_event");
    assert_eq!(payload["sessionId"], "sess_0");
    assert!(control_event_delivery_payload("other", "session_events", &matching).is_none());

    let lagged: Result<ControlEvent, broadcast::error::RecvError> =
        Err(broadcast::error::RecvError::Lagged(4));
    let payload = control_event_delivery_payload("sess_0", "thought_events", &lagged)
        .expect("lagged payload");
    assert_eq!(payload["type"], "event_stream_lagged");
    assert_eq!(payload["stream"], "thought_events");
    assert_eq!(payload["skipped"], 4);

    let closed: Result<ControlEvent, broadcast::error::RecvError> =
        Err(broadcast::error::RecvError::Closed);
    assert!(control_event_delivery_payload("sess_0", "session_events", &closed).is_none());
}

#[test]
fn lifecycle_event_delivery_payload_matches_session_lagged_and_closed_cases() {
    let event = LifecycleEvent::Deleted {
        session_id: "sess_0".to_string(),
        reason: "tmux_reconcile_missing".to_string(),
        delete_mode: crate::config::SessionDeleteMode::DetachBridge,
        tmux_session_alive: false,
    };
    let matching: Result<LifecycleEvent, broadcast::error::RecvError> = Ok(event);
    let payload =
        lifecycle_event_delivery_payload("sess_0", &matching).expect("matching event payload");
    assert_eq!(payload["type"], "lifecycle_event");
    assert_eq!(payload["sessionId"], "sess_0");
    assert!(lifecycle_event_delivery_payload("other", &matching).is_none());

    let lagged: Result<LifecycleEvent, broadcast::error::RecvError> =
        Err(broadcast::error::RecvError::Lagged(9));
    let payload = lifecycle_event_delivery_payload("sess_0", &lagged).expect("lagged payload");
    assert_eq!(payload["type"], "event_stream_lagged");
    assert_eq!(payload["stream"], "lifecycle_events");
    assert_eq!(payload["skipped"], 9);

    let closed: Result<LifecycleEvent, broadcast::error::RecvError> =
        Err(broadcast::error::RecvError::Closed);
    assert!(lifecycle_event_delivery_payload("sess_0", &closed).is_none());
}

#[tokio::test]
async fn index_shell_includes_new_web_parity_sheets() {
    let html = html_string(render_index(false).await).await;
    assert!(html.contains("swimmers-react-root"));
    assert!(html.find("swimmers-react-root").unwrap() < html.find("terminal-stage").unwrap());
    assert!(html.contains("thought-config-sheet"));
    assert!(html.contains("native-sheet"));
    assert!(html.contains("mermaid-sheet"));
    assert!(html.contains("mermaid-plan-tabs"));
    assert!(html.contains("dirs-list"));
    assert!(html.contains("dirs-search"));
    assert!(html.contains("create-batch-visible"));
    assert!(html.contains("dirs-spawn-here"));
    assert!(html.contains("create-launch-target"));
    assert!(html.contains("mobile-kb-proxy"));
    assert!(html.contains("terminal-control-strip"));
    assert!(html.contains("terminal-workbench"));
    assert!(html.contains("terminal-workbench-toggle"));
    assert!(html.contains("terminal-trogdor-back"));
    assert!(html.contains("terminal-workbench-pressure"));
    assert!(html.contains("terminal-workbench-actions"));
    assert!(html.contains("terminal-workbench-widgets"));
    assert!(html.contains("terminal-input-dock"));
    assert!(html.contains("terminal-inline-input"));
    assert!(html.contains("terminal-key-strip"));
    assert!(html.contains("data-terminal-key=\"ctrl-c\""));
    assert!(html.contains("data-terminal-key=\"arrow-up\""));
    assert!(html.contains("palette-sheet"));
    assert!(html.contains("terminal-a11y-mirror"));
    assert!(html.contains("terminal-status-strip"));
    assert!(html.contains("terminal-link-tools"));
    assert!(html.contains("send-mode"));
    assert!(html.contains("send-history"));
    assert!(html.contains(FRANKENTERM_FONT_ROUTE));
    assert!(html.contains("window.__SWIMMERS_BOOT__"));
}

#[tokio::test]
async fn index_boot_payload_includes_frankenterm_asset_manifest_fields() {
    let html = html_string(render_index(false).await).await;
    assert!(html.contains("\"franken_term_font_url\""));
    assert!(html.contains("\"franken_term_asset_info\""));
    assert!(html.contains(FRANKENTERM_FONT_ROUTE));
}

#[tokio::test]
async fn document_routes_preserve_boot_payload_no_store_and_script_order() {
    let index_response = index().await.into_response();
    assert_eq!(index_response.status(), StatusCode::OK);
    assert_eq!(
        index_response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let index_html = response_text(index_response).await;
    let index_boot = boot_payload_from_html(&index_html);
    assert_eq!(index_boot["follow_published_selection"], false);
    assert_eq!(index_boot["focus_layout"], false);
    assert_eq!(
        index_boot["franken_term_js_url"],
        serde_json::Value::String(FRANKENTERM_JS_ROUTE.to_string())
    );
    assert_eq!(
        index_boot["franken_term_wasm_url"],
        serde_json::Value::String(FRANKENTERM_WASM_ROUTE.to_string())
    );
    assert_eq!(
        index_boot["franken_term_font_url"],
        serde_json::Value::String(FRANKENTERM_FONT_ROUTE.to_string())
    );
    assert!(
        index_html.find("window.__SWIMMERS_BOOT__").unwrap()
            < index_html.find("<script type=\"module\"").unwrap()
    );

    let selected_response = selected_index().await.into_response();
    assert_eq!(selected_response.status(), StatusCode::OK);
    assert_eq!(
        selected_response
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap(),
        "no-store"
    );
    let selected_html = response_text(selected_response).await;
    let selected_boot = boot_payload_from_html(&selected_html);
    assert_eq!(selected_boot["follow_published_selection"], true);
    assert_eq!(selected_boot["focus_layout"], true);
    assert!(selected_html.contains("app-body published-focus"));
}

#[tokio::test]
async fn vite_manifest_tags_select_built_entry_css_and_compatibility_css_boundary() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".vite")).expect("manifest dir");
    std::fs::write(
        dir.path().join(".vite/manifest.json"),
        r#"{
  "src/web/app.js": {
    "file": "assets/app-12345678.js",
    "src": "src/web/app.js",
    "isEntry": true,
    "css": ["assets/app-87654321.css"],
    "imports": ["_surface-aaaaaaaa.js", "_trogdor-bbbbbbbb.js"]
  },
  "_surface-aaaaaaaa.js": {
    "file": "assets/surface-aaaaaaaa.js"
  },
  "_trogdor-bbbbbbbb.js": {
    "file": "assets/trogdor-bbbbbbbb.js",
    "imports": ["_surface-aaaaaaaa.js"]
  },
  "virtual:swimmers-app-css.css": {
    "file": "assets/appCss-abcdef12.css",
    "src": "virtual:swimmers-app-css.css",
    "isEntry": true
  }
}"#,
    )
    .expect("write manifest");

    let tags = frontend_asset_tags_from_dist_dir(dir.path())
        .await
        .expect("vite tags");
    assert_eq!(
        tags.stylesheets,
        [
            "/assets/vite/assets/app-87654321.css",
            "/assets/vite/assets/appCss-abcdef12.css"
        ]
    );
    assert_eq!(
        tags.module_preloads,
        [
            "/assets/vite/assets/surface-aaaaaaaa.js",
            "/assets/vite/assets/trogdor-bbbbbbbb.js"
        ]
    );
    assert_eq!(tags.module_scripts, ["/assets/vite/assets/app-12345678.js"]);

    std::fs::write(
        dir.path().join(".vite/manifest.json"),
        r#"{
  "src/web/app.js": {
    "file": "assets/app-12345678.js",
    "src": "src/web/app.js",
    "isEntry": true
  }
}"#,
    )
    .expect("write manifest without css");
    let tags = frontend_asset_tags_from_dist_dir(dir.path())
        .await
        .expect("vite tags without css");
    assert_eq!(tags.stylesheets, [APP_CSS_ROUTE]);
    assert!(tags.module_preloads.is_empty());
    assert_eq!(tags.module_scripts, ["/assets/vite/assets/app-12345678.js"]);
}

#[tokio::test]
async fn vite_dist_asset_route_serves_built_js_css_and_chunks_with_cache_policy() {
    let dir = tempdir().expect("tempdir");
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).expect("assets dir");
    std::fs::write(
        assets_dir.join("app-12345678.js"),
        "import './chunk-abcdef12.js';",
    )
    .expect("write app js");
    std::fs::write(assets_dir.join("appCss-12345678.css"), ".terminal-stage{}").expect("write css");
    std::fs::write(
        assets_dir.join("chunk-abcdef12.js"),
        "export const chunk = true;",
    )
    .expect("write chunk");
    std::fs::write(assets_dir.join("app.js"), "console.log('alias');").expect("write alias");
    std::fs::write(assets_dir.join("app-12345678.js.map"), "{}").expect("write map");

    let cases = [
        (
            "assets/app-12345678.js",
            "application/javascript; charset=utf-8",
            "public, max-age=31536000, immutable",
            "chunk-abcdef12",
        ),
        (
            "assets/appCss-12345678.css",
            "text/css; charset=utf-8",
            "public, max-age=31536000, immutable",
            "terminal-stage",
        ),
        (
            "assets/chunk-abcdef12.js",
            "application/javascript; charset=utf-8",
            "public, max-age=31536000, immutable",
            "export const chunk",
        ),
        (
            "assets/app.js",
            "application/javascript; charset=utf-8",
            "no-store",
            "alias",
        ),
    ];
    for (route, content_type, cache_control, needle) in cases {
        let response = serve_vite_dist_asset(dir.path(), route).await;
        assert_eq!(response.status(), StatusCode::OK, "{route}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            content_type,
            "{route}"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            cache_control,
            "{route}"
        );
        let body = response_text(response).await;
        assert!(body.contains(needle), "{route}");
    }

    for route in [
        "../secrets.js",
        "assets/../secrets.js",
        r"assets\app-12345678.js",
        "assets/app-12345678.js.map",
        ".vite/manifest.json",
    ] {
        let response = serve_vite_dist_asset(dir.path(), route).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{route}");
        let body = response_json(response).await;
        assert_eq!(body["code"], "VITE_ASSET_NOT_FOUND", "{route}");
    }

    let response = serve_vite_dist_asset(dir.path(), "assets/missing-12345678.js").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "VITE_ASSET_NOT_FOUND");
    assert!(!body["message"]
        .as_str()
        .expect("message")
        .contains(dir.path().to_str().expect("temp path")));
}

#[test]
fn vite_manifest_tags_reject_backslash_asset_paths() {
    let manifest = serde_json::from_str::<std::collections::BTreeMap<String, ViteManifestEntry>>(
        r#"{
  "src/web/app.js": {
    "file": "assets/nested\\app-12345678.js",
    "src": "src/web/app.js",
    "isEntry": true
  }
}"#,
    )
    .expect("manifest");

    let err = frontend_asset_tags_from_manifest(&manifest).expect_err("backslash paths fail");
    assert!(
        err.contains("unsupported file path"),
        "unexpected manifest error: {err}"
    );
}

#[test]
fn vite_dev_origin_tags_use_vite_modules_without_replacing_backend_css() {
    assert_eq!(
        normalize_vite_dev_origin(" http://127.0.0.1:5173/ "),
        Some("http://127.0.0.1:5173".to_string())
    );
    assert!(normalize_vite_dev_origin("ftp://127.0.0.1:5173").is_none());
    assert!(normalize_vite_dev_origin("http://127.0.0.1:5173\"").is_none());
    assert!(normalize_vite_dev_origin("http://127.0.0.1:5173/selected").is_none());
    assert!(normalize_vite_dev_origin("http://127.0.0.1:5173?x=1").is_none());
    assert!(normalize_vite_dev_origin("http://127.0.0.1:5173#hash").is_none());
    assert!(normalize_vite_dev_origin("http://user@127.0.0.1:5173").is_none());

    let tags = frontend_asset_tags_for_vite_dev_origin("http://127.0.0.1:5173/");
    assert_eq!(tags.stylesheets, [APP_CSS_ROUTE]);
    assert!(tags.module_preloads.is_empty());
    assert_eq!(
        tags.module_scripts,
        [
            "http://127.0.0.1:5173/@vite/client",
            "http://127.0.0.1:5173/src/web/app.js"
        ]
    );
    assert!(tags
        .module_scripts
        .iter()
        .chain(tags.module_preloads.iter())
        .chain(tags.stylesheets.iter())
        .all(|route| !route.contains("/assets/frankenterm/")));
}

#[test]
fn frontend_module_preload_tags_render_initial_vite_import_hints() {
    let tags = assets::FrontendAssetTags {
        stylesheets: vec![],
        module_preloads: vec![
            "/assets/vite/assets/surface-aaaaaaaa.js".to_string(),
            "/assets/vite/assets/trogdor-bbbbbbbb.js".to_string(),
        ],
        module_scripts: vec![],
    };
    assert_eq!(
        frontend_module_preload_tags(&tags),
        r#"<link rel="modulepreload" crossorigin href="/assets/vite/assets/surface-aaaaaaaa.js" />
    <link rel="modulepreload" crossorigin href="/assets/vite/assets/trogdor-bbbbbbbb.js" />"#
    );
}

#[tokio::test]
async fn browser_js_asset_handlers_cover_app_module_graph() {
    let assets = [
        (APP_JS_ROUTE, app_js().await, "from \"./api_client.js\""),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./session_persistence.js\"",
        ),
        (
            API_CLIENT_JS_ROUTE,
            api_client_js().await,
            "export function createApiClient",
        ),
        (
            SESSION_PERSISTENCE_JS_ROUTE,
            session_persistence_js().await,
            "export function createSessionPersistenceController",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./app_event_handlers.js\"",
        ),
        (
            APP_EVENT_HANDLERS_JS_ROUTE,
            app_event_handlers_js().await,
            "from \"./app_event_bindings.js\"",
        ),
        (
            APP_EVENT_BINDINGS_JS_ROUTE,
            app_event_bindings_js().await,
            "export function bindAppEvents",
        ),
        (
            APP_EVENT_HANDLERS_JS_ROUTE,
            app_event_handlers_js().await,
            "export function createAppEventHandlers",
        ),
        (APP_JS_ROUTE, app_js().await, "from \"./trogdor_island.js\""),
        (
            TROGDOR_ISLAND_JS_ROUTE,
            trogdor_island_js().await,
            "export function createTrogdorAtlasIsland",
        ),
        (
            TROGDOR_ISLAND_JS_ROUTE,
            trogdor_island_js().await,
            "from \"./trogdor_surface_controller.js\"",
        ),
        (
            TROGDOR_ISLAND_JS_ROUTE,
            trogdor_island_js().await,
            "from \"./trogdor_event_bindings.js\"",
        ),
        (
            TROGDOR_SURFACE_CONTROLLER_JS_ROUTE,
            trogdor_surface_controller_js().await,
            "export function createTrogdorSurfaceController",
        ),
        (
            TROGDOR_EVENT_BINDINGS_JS_ROUTE,
            trogdor_event_bindings_js().await,
            "export function bindTrogdorEvents",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./send_controller.js\"",
        ),
        (
            SEND_CONTROLLER_JS_ROUTE,
            send_controller_js().await,
            "from \"./send_sheet.js\"",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./thought_config_sheet.js\"",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./native_desktop_sheet.js\"",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./mermaid_artifact_controller.js\"",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./command_palette_controller.js\"",
        ),
        (
            RENDERED_SURFACE_JS_ROUTE,
            rendered_surface_js().await,
            "export function buildSurfaceFrame",
        ),
        (
            RENDERED_SURFACE_DRAW_JS_ROUTE,
            rendered_surface_draw_js().await,
            "export function computeSurfaceLayout",
        ),
        (
            INPUT_SUPPORT_JS_ROUTE,
            input_support_js().await,
            "export function eventCell",
        ),
        (
            SURFACE_ACTION_PLANS_JS_ROUTE,
            surface_action_plans_js().await,
            "export function surfaceActionDispatchPlan",
        ),
        (
            SEND_SHEET_JS_ROUTE,
            send_sheet_js().await,
            "export function sendSheetSubmitPlan",
        ),
        (
            THOUGHT_CONFIG_SHEET_JS_ROUTE,
            thought_config_sheet_js().await,
            "export function createThoughtConfigSheetController",
        ),
        (
            NATIVE_DESKTOP_SHEET_JS_ROUTE,
            native_desktop_sheet_js().await,
            "export function createNativeDesktopSheetController",
        ),
        (APP_JS_ROUTE, app_js().await, "from \"./terminal_focus.js\""),
        (
            TERMINAL_FOCUS_JS_ROUTE,
            terminal_focus_js().await,
            "export function createTerminalFocusController",
        ),
        (
            TERMINAL_SURFACE_SETUP_JS_ROUTE,
            terminal_surface_setup_js().await,
            "export async function initializeTerminalSurface",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./terminal_island.js\"",
        ),
        (
            TERMINAL_ISLAND_JS_ROUTE,
            terminal_island_js().await,
            "export function createTerminalSurfaceIsland",
        ),
        (
            TERMINAL_ISLAND_JS_ROUTE,
            terminal_island_js().await,
            "from \"./terminal_surface_controller.js\"",
        ),
        (
            TERMINAL_SURFACE_CONTROLLER_JS_ROUTE,
            terminal_surface_controller_js().await,
            "export function createFrankenTermRuntimeAdapter",
        ),
        (
            TERMINAL_SURFACE_CONTROLLER_JS_ROUTE,
            terminal_surface_controller_js().await,
            "from \"./terminal_surface_setup.js\"",
        ),
        (
            TERMINAL_RESIZE_JS_ROUTE,
            terminal_resize_js().await,
            "export function runTerminalSurfaceResize",
        ),
        (
            GLOBAL_SHORTCUT_DISPATCH_JS_ROUTE,
            global_shortcut_dispatch_js().await,
            "export function runGlobalShortcutAction",
        ),
        (
            SESSION_REFRESH_JS_ROUTE,
            session_refresh_js().await,
            "export async function runSessionRefresh",
        ),
        (
            AGENT_CONTEXT_REFRESH_JS_ROUTE,
            agent_context_refresh_js().await,
            "export async function runAgentContextRefresh",
        ),
        (
            MERMAID_ARTIFACT_JS_ROUTE,
            mermaid_artifact_js().await,
            "export function boundedArtifactText",
        ),
        (
            MERMAID_ARTIFACT_CONTROLLER_JS_ROUTE,
            mermaid_artifact_controller_js().await,
            "export function createMermaidArtifactController",
        ),
        (
            TERMINAL_SAFETY_JS_ROUTE,
            terminal_safety_js().await,
            "export function safeAnchorHref",
        ),
        (
            TERMINAL_SEARCH_LINKS_JS_ROUTE,
            terminal_search_links_js().await,
            "export function createTerminalSearchLinksController",
        ),
        (
            APP_JS_ROUTE,
            app_js().await,
            "from \"./terminal_status.js\"",
        ),
        (APP_JS_ROUTE, app_js().await, "from \"./terminal_input.js\""),
        (
            TERMINAL_STATUS_JS_ROUTE,
            terminal_status_js().await,
            "export function createTerminalStatusController",
        ),
        (
            TERMINAL_PROTOCOL_JS_ROUTE,
            terminal_protocol_js().await,
            "export function buildSessionSocketUrl",
        ),
        (
            TERMINAL_INPUT_JS_ROUTE,
            terminal_input_js().await,
            "export function createTerminalInputController",
        ),
        (
            SESSION_SOCKET_CONTROLLER_JS_ROUTE,
            session_socket_controller_js().await,
            "export function createSessionSocketController",
        ),
        (
            DIR_BROWSER_JS_ROUTE,
            dir_browser_js().await,
            "export function renderDirEntries",
        ),
        (
            DIR_BROWSER_CONTROLLER_JS_ROUTE,
            dir_browser_controller_js().await,
            "from \"./dir_browser.js\"",
        ),
        (
            COMMAND_PALETTE_JS_ROUTE,
            command_palette_js().await,
            "export function commandPaletteExecutionPlan",
        ),
        (
            COMMAND_PALETTE_CONTROLLER_JS_ROUTE,
            command_palette_controller_js().await,
            "from \"./command_palette.js\"",
        ),
        (
            TROGDOR_LOGIC_JS_ROUTE,
            trogdor_logic_js().await,
            "from \"./trogdor_dom_logic.js\"",
        ),
        (
            TROGDOR_DOM_LOGIC_JS_ROUTE,
            trogdor_dom_logic_js().await,
            "export function trogdorDragonPose",
        ),
        (
            TROGDOR_RENDER_JS_ROUTE,
            trogdor_render_js().await,
            "export function renderTrogdorSurfaceFrame",
        ),
        (
            WORKBENCH_DOM_JS_ROUTE,
            workbench_dom_js().await,
            "export function writeWorkbenchWidgetsHtmlToDom",
        ),
        (
            WORKBENCH_RENDER_JS_ROUTE,
            workbench_render_js().await,
            "export function buildWorkbenchWidgetsHtml",
        ),
        (
            WORKBENCH_LOG_LENS_JS_ROUTE,
            workbench_log_lens_js().await,
            "export function renderWorkbenchLogLens",
        ),
        (
            WORKBENCH_REFRESH_JS_ROUTE,
            workbench_refresh_js().await,
            "export async function runWorkbenchWidgetRefresh",
        ),
        (
            WORKBENCH_RECORDS_JS_ROUTE,
            workbench_records_js().await,
            "export function transcriptRecordDisplay",
        ),
        (
            TERMINAL_WORKBENCH_CONTROLLER_JS_ROUTE,
            terminal_workbench_controller_js().await,
            "export function createTerminalWorkbenchController",
        ),
    ];

    for (route, response, needle) in assets {
        assert_eq!(response.status(), StatusCode::OK, "{route}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8",
            "{route}"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store",
            "{route}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("js body");
        let text = String::from_utf8(body.to_vec()).expect("utf8 js asset");
        assert!(text.contains(needle), "{route} did not contain {needle}");
    }
}

#[tokio::test]
async fn direct_app_js_route_defers_react_imports_to_vite_shell_path() {
    let app = response_text(app_js().await).await;
    assert!(!app.contains("from \"react\""));
    assert!(!app.contains("from \"react-dom/client\""));
    assert!(app.contains("import(\"./react_shell.js\")"));

    let react_shell = include_str!("react_shell.js");
    assert!(react_shell.contains("from \"react\""));
    assert!(react_shell.contains("from \"react-dom/client\""));
}

#[tokio::test]
async fn app_css_serves_concatenated_partials_with_existing_headers() {
    let response = app_css().await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/css; charset=utf-8"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("css body");
    let css = String::from_utf8(body.to_vec()).expect("utf8 css asset");
    let order_needles = [
        ".loading-overlay.visible",
        ".trogdor-surface",
        ".loading-label",
        ".surface-sheet",
        "#create-sheet.create-console",
        ".sheet-result",
        ".hidden",
        "@media (max-width: 700px)",
        ".trogdor-frame",
        "@media (prefers-reduced-motion: reduce)",
        "/* claude:scrollbar-style */",
    ];
    let mut last = 0;
    for needle in order_needles {
        let offset = css[last..]
            .find(needle)
            .unwrap_or_else(|| panic!("missing css marker {needle}"));
        last += offset + needle.len();
    }
}

#[tokio::test]
async fn franken_term_asset_file_info_reports_size_and_crc() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("FrankenTerm.js");
    let contents = b"export default async () => {}";
    std::fs::write(&path, contents).expect("write asset");

    let info = franken_term_asset_file_info(&path, FRANKENTERM_JS_ROUTE)
        .await
        .expect("asset info");

    assert_eq!(info.route, FRANKENTERM_JS_ROUTE);
    assert_eq!(info.size_bytes, contents.len() as u64);
    assert_eq!(
        info.checksum,
        format!("crc32:{:08x}", crc32fast::hash(contents))
    );
}

#[test]
fn franken_term_font_path_resolves_availability_states() {
    assert!(matches!(
        franken_term_font_path(None),
        FrankenTermFontPath::AssetsUnavailable
    ));
    assert!(matches!(
        franken_term_font_path(Some(PathBuf::from("/"))),
        FrankenTermFontPath::RootUnavailable
    ));

    let dir = tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("pkg");
    let expected = dir.path().join("fonts").join("pragmasevka-nf-subset.woff2");
    match franken_term_font_path(Some(pkg_dir)) {
        FrankenTermFontPath::Available(path) => assert_eq!(path, expected),
        other => panic!("unexpected font path state: {other:?}"),
    }
}

#[tokio::test]
async fn serve_franken_term_font_reports_unavailable_path_states() {
    let response = serve_franken_term_font(FrankenTermFontPath::AssetsUnavailable)
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_ASSET_UNAVAILABLE");

    let response = serve_franken_term_font(FrankenTermFontPath::RootUnavailable)
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_FONT_UNAVAILABLE");
}

#[tokio::test]
async fn serve_franken_term_font_serves_font_bytes_and_error_payloads() {
    let dir = tempdir().expect("tempdir");
    let font_path = dir.path().join("pragmasevka-nf-subset.woff2");
    std::fs::write(&font_path, b"font-bytes").expect("write font");

    let response = serve_franken_term_font(FrankenTermFontPath::Available(font_path.clone()))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "font/woff2"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("font body");
    assert_eq!(&body[..], b"font-bytes");

    let response = serve_franken_term_font(FrankenTermFontPath::Available(
        dir.path().join("missing.woff2"),
    ))
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_FONT_UNAVAILABLE");
    assert!(!body["message"]
        .as_str()
        .expect("message")
        .contains(dir.path().to_str().expect("temp path")));

    let response = serve_franken_term_font(FrankenTermFontPath::Available(dir.path().into()))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_FONT_READ_FAILED");
    assert!(!body["message"]
        .as_str()
        .expect("message")
        .contains(dir.path().to_str().expect("temp path")));
}

#[tokio::test]
async fn frankentui_asset_response_helpers_preserve_headers_and_hide_host_paths() {
    let response = frankentui_asset_response("application/javascript", b"asset".to_vec());
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/javascript"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("asset body");
    assert_eq!(&body[..], b"asset");

    let response = frankentui_asset_unavailable_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_ASSET_UNAVAILABLE");

    let dir = tempdir().expect("tempdir");
    let response = frankentui_asset_read_error_response(
        "FrankenTerm.js",
        dir.path(),
        std::io::Error::from(std::io::ErrorKind::NotFound),
    );
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_ASSET_MISSING");
    assert!(body["message"]
        .as_str()
        .expect("message")
        .contains("FrankenTerm.js"));
    assert!(!body["message"]
        .as_str()
        .expect("message")
        .contains(dir.path().to_str().expect("temp path")));

    let response = frankentui_asset_read_error_response(
        "FrankenTerm_bg.wasm",
        dir.path(),
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
    );
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(body["code"], "FRANKENTERM_ASSET_READ_FAILED");
    assert!(body["message"]
        .as_str()
        .expect("message")
        .contains("FrankenTerm_bg.wasm"));
    assert!(!body["message"]
        .as_str()
        .expect("message")
        .contains(dir.path().to_str().expect("temp path")));
}

#[tokio::test]
async fn frankenterm_backend_asset_readers_serve_js_and_wasm_without_vite_routes() {
    let dir = tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("FrankenTerm.js"),
        "export default async function init() {}",
    )
    .expect("write js");
    std::fs::write(dir.path().join("FrankenTerm_bg.wasm"), b"\0asm").expect("write wasm");

    let js_response = read_frankentui_asset_response(
        "FrankenTerm.js",
        "application/javascript; charset=utf-8",
        dir.path(),
    )
    .await;
    assert_eq!(js_response.status(), StatusCode::OK);
    assert_eq!(
        js_response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(
        js_response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let js = response_text(js_response).await;
    assert!(js.contains("export default"));

    let wasm_response =
        read_frankentui_asset_response("FrankenTerm_bg.wasm", "application/wasm", dir.path()).await;
    assert_eq!(wasm_response.status(), StatusCode::OK);
    assert_eq!(
        wasm_response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/wasm"
    );
    assert_eq!(
        wasm_response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    let body = to_bytes(wasm_response.into_body(), usize::MAX)
        .await
        .expect("wasm body");
    assert_eq!(&body[..], b"\0asm");
}

#[tokio::test]
async fn trogdor_dragon_asset_route_serves_embedded_png() {
    // Spot-check three combinations spanning the expanded 8 poses × 8 frames
    // grid: the legacy mouth-closed/right pair, the newly-served mouth-open
    // pose, and one of the previously-unreachable 3/4-view body frames.
    let cases = [
        ("mouth-closed", "right.png"),
        ("mouth-open", "back-left.png"),
        ("fire-right-mid", "3q-left.png"),
    ];
    for (pose, frame) in cases {
        let response = trogdor_dragon_asset(AxumPath((pose.to_string(), frame.to_string())))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK, "{pose}/{frame}");
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png",
            "{pose}/{frame}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("dragon sprite body");
        assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n", "{pose}/{frame}");
    }

    // Unknown frames still 404.
    let response = trogdor_dragon_asset(AxumPath((
        "mouth-closed".to_string(),
        "diagonal.png".to_string(),
    )))
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn app_js_and_css_wire_trogdor_sprite_burn_loop() {
    let js = include_str!("app.js");
    let trogdor_logic = include_str!("trogdor_logic.js");
    let trogdor_dom_logic = include_str!("trogdor_dom_logic.js");
    let trogdor_render = include_str!("trogdor_render.js");
    let trogdor_surface_controller = include_str!("trogdor_surface_controller.js");
    let trogdor_island = include_str!("trogdor_island.js");
    assert!(trogdor_render.contains("TROGDOR_DRAGON_ASSET_BASE = \"/assets/dragon\""));
    assert!(trogdor_logic.contains("from \"./trogdor_dom_logic.js\""));
    assert!(trogdor_dom_logic.contains("export function trogdorDragonPose(groups, summary"));
    assert!(trogdor_render.contains("trogdor-dragon-sprite"));
    assert!(trogdor_render.contains("agent-burn-flame"));
    assert!(trogdor_render.contains("const dragonTarget = dragonPose || TROGDOR_DRAGON_TARGET"));
    assert!(js.contains("createTrogdorAtlasIsland"));
    assert!(trogdor_surface_controller.contains("renderTrogdorSurfaceFrame"));
    assert!(trogdor_island.contains("createTrogdorSurfaceController"));
    assert!(trogdor_island.contains("createTrogdorEventBindings"));

    let css = app_css_body();
    assert!(css.contains("@keyframes dragon-walk-around"));
    assert!(css.contains("@keyframes dragon-sprite-fire"));
    assert!(css.contains(".agent-burn-flame"));
    assert!(css.contains("@media (prefers-reduced-motion: reduce)"));
}

#[test]
fn directory_browser_modules_include_overlay_filter_chip_logic() {
    let js = include_str!("app.js");
    let dir_browser = include_str!("dir_browser.js");
    let dir_browser_controller = include_str!("dir_browser_controller.js");
    assert!(dir_browser.contains("response?.overlay_label"));
    assert!(dir_browser.contains("managedButton.dataset.filter = \"managed\""));
    assert!(dir_browser.contains("allButton.textContent = \"all folders\""));
    assert!(js.contains("createDirBrowserController"));
    assert!(dir_browser_controller.contains("dirGroupChipClickPlan"));
    assert!(dir_browser_controller.contains("managedOnlyStorageKey"));
}

#[test]
fn app_js_retries_saved_out_of_base_dir_from_default_base() {
    let js = include_str!("app.js");
    let dir_browser_controller = include_str!("dir_browser_controller.js");
    let input_support = include_str!("input_support.js");
    assert!(js.contains("createDirBrowserController"));
    assert!(dir_browser_controller.contains("shouldRetryDirListingFromBase"));
    assert!(dir_browser_controller.contains("storage.removeItem(pathStorageKey)"));
    assert!(dir_browser_controller.contains("retriedFromBase: true"));
    assert!(dir_browser_controller.contains("preferredLaunchTarget: options.preferredLaunchTarget"));
    assert!(dir_browser_controller.contains("outside the allowed base directory"));
    assert!(input_support.contains("rawStoredDirPath.trim() === \"/\" ? \"\" : rawStoredDirPath"));
}

#[test]
fn app_js_exposes_terminal_viewer_ergonomics() {
    let js = include_str!("app.js");
    let terminal_search_links = include_str!("terminal_search_links.js");
    let terminal_surface_setup = include_str!("terminal_surface_setup.js");
    let terminal_surface_controller = include_str!("terminal_surface_controller.js");
    let terminal_zoom_input = include_str!("terminal_zoom_input.js");
    let agent_context_refresh = include_str!("agent_context_refresh.js");
    let workbench_render = include_str!("workbench_render.js");
    let mermaid_artifact_controller = include_str!("mermaid_artifact_controller.js");
    assert!(js.contains("TERMINAL_ZOOM_STORAGE_KEY"));
    assert!(js.contains("createTerminalZoomInputController"));
    assert!(terminal_zoom_input.contains("setZoom"));
    assert!(terminal_zoom_input.contains("terminalZoomPersistencePlan"));
    assert!(js.contains("focusMobileKeyboard"));
    assert!(js.contains("mobileKeyboardProxy"));
    assert!(js.contains("function openCommandPalette()"));
    let terminal_island = include_str!("terminal_island.js");
    assert!(js.contains("createTerminalSurfaceIsland"));
    assert!(terminal_island.contains("createFrankenTermRuntimeAdapter"));
    assert!(terminal_island.contains("createTerminalSurfaceIslandElements"));
    assert!(terminal_surface_controller.contains("createTerminalSurfaceRuntimeHelpers"));
    assert!(js.contains("syncTerminalAccessibilityMirror,"));
    assert!(terminal_surface_setup.contains("function syncTerminalAccessibilityMirror"));
    assert!(js.contains("createTerminalSearchLinksController"));
    assert!(terminal_search_links.contains("function drainTerminalLinkClicks()"));
    let terminal_status = include_str!("terminal_status.js");
    assert!(js.contains("createSendController"));
    assert!(js.contains("rememberSendHistory,"));
    assert!(terminal_zoom_input.contains("await sendLineToSession(state.selectedSessionId, text)"));
    assert!(terminal_zoom_input.contains("rememberSendHistory(text);"));
    assert!(js.contains("createTerminalStatusController"));
    assert!(terminal_status.contains("function syncTerminalStatusStrip()"));
    assert!(js.contains("function refreshAgentContextForSelectedSession"));
    assert!(js.contains("function refreshWorkbenchWidgetsForSelectedSession"));
    assert!(workbench_render.contains("export function operatorPressureSummary"));
    assert!(workbench_render.contains("Tool calls"));
    assert!(agent_context_refresh.contains("/agent-context"));
    assert!(workbench_render.contains("/pane-tail"));
    assert!(workbench_render.contains("/transcript"));
    assert!(workbench_render.contains("function renderTurnsPanel"));
    assert!(terminal_surface_setup.contains("function flushPendingTerminalBytes"));
    assert!(workbench_render.contains("Post-turn JSONL"));
    assert!(mermaid_artifact_controller.contains("/mermaid-artifact"));
    assert!(workbench_render.contains("/git-diff"));
    assert!(workbench_render.contains("function renderDiffHtml"));
    assert!(js.contains("function syncTerminalWorkbench()"));
}

#[test]
fn app_js_dedupes_surface_actions_and_stable_resizes() {
    let js = include_str!("app.js");
    let app_event_bindings = include_str!("app_event_bindings.js");
    let resize = include_str!("terminal_resize.js");
    let terminal_surface_controller = include_str!("terminal_surface_controller.js");
    let terminal_stage_controller = include_str!("terminal_stage_controller.js");
    assert!(terminal_stage_controller.contains("function stopSurfaceEvent(event)"));
    assert!(terminal_stage_controller.contains("event.stopImmediatePropagation"));
    assert!(terminal_surface_controller.contains(
        "runtime.runTerminalSurfaceResize({ pushResize, force }, runtime.terminalResizeRuntime)"
    ));
    assert!(resize.contains("terminalResizeGeometryPlan({"));
    assert!(resize.contains("if (!resizePlan.shouldResize)"));
    assert!(js.contains("queueMeasureAndResizeSurface,"));
    assert!(app_event_bindings.contains("queueMeasureAndResizeSurface(true, false)"));
}

#[test]
fn app_js_hides_hud_when_live_terminal_is_focused() {
    let js = include_str!("app.js");
    let terminal_surface_controller = include_str!("terminal_surface_controller.js");
    assert!(js.contains("syncTerminalPresentation,"));
    assert!(terminal_surface_controller.contains("function syncTerminalPresentation()"));
    assert!(terminal_surface_controller.contains("terminal-focus-mode"));
    assert!(terminal_surface_controller.contains(
        "runtime.terminalPresentationPlan({ hasCurrentSession: Boolean(runtime.currentSession())"
    ));
    assert!(terminal_surface_controller
        .contains("el.hudCanvas.classList.toggle(\"hidden\", plan.hudHidden)"));
    assert!(
            terminal_surface_controller.contains("[el.hudCanvas.style.display, el.hudCanvas.style.visibility] = [plan.hudDisplay, plan.hudVisibility]")
        );
    assert!(terminal_surface_controller
        .contains("el.terminalCanvas.classList.toggle(\"hidden\", plan.terminalCanvasHidden)"));
}

#[test]
fn app_js_falls_back_when_live_terminal_canvas_does_not_paint() {
    let js = include_str!("app.js");
    let terminal_input = include_str!("terminal_input.js");
    let terminal_surface_setup = include_str!("terminal_surface_setup.js");
    let terminal_surface_controller = include_str!("terminal_surface_controller.js");
    let terminal_stage_controller = include_str!("terminal_stage_controller.js");
    assert!(terminal_surface_controller.contains("function feedTerminalBytes(bytes)"));
    assert!(terminal_surface_controller.contains("runtime.flushEncodedInputBytes();"));
    assert!(terminal_surface_controller.contains("function terminalCanvasHasVisiblePixels()"));
    assert!(terminal_surface_controller.contains("function verifyTerminalPaintOrFallback()"));
    assert!(
        terminal_surface_setup.contains("activateTerminalSurfaceFallback(rendererPlan, runtime)")
    );
    assert!(js.contains("setTerminalTextFallbackActive,"));
    assert!(terminal_surface_setup
        .contains("runtime.setTerminalTextFallbackActive(true, { clearText: plan.clearText })"));
    assert!(terminal_input.contains("function sendFallbackTerminalEvent(event)"));
    assert!(terminal_surface_setup.contains("function updateTerminalFallbackText(text)"));
    assert!(terminal_stage_controller.contains("function terminalFallbackOwnsPointer(event)"));
    let css = app_css_body();
    assert!(css.contains("white-space: pre-wrap"));
    assert!(css.contains("overflow-wrap: anywhere"));
    assert!(css.contains("pointer-events: auto"));
}

#[test]
fn app_js_trogdor_agent_click_opens_terminal() {
    let js = include_str!("app.js");
    let terminal_input = include_str!("terminal_input.js");
    let trogdor_logic = include_str!("trogdor_logic.js");
    let trogdor_surface_controller = include_str!("trogdor_surface_controller.js");
    let trogdor_island = include_str!("trogdor_island.js");
    assert!(js.contains("function closeTrogdorAtlasForTerminal()"));
    assert!(js.contains("function openTrogdorAtlas()"));
    assert!(js.contains("terminalTrogdorBack"));
    assert!(terminal_input.contains("function sendTerminalControlKey(actionId)"));
    assert!(terminal_input.contains("terminalKeyActionForDomEvent(event)"));
    assert!(js.contains("async function openTrogdorAgentTerminal(sessionId)"));
    assert!(js.contains("trogdorAtlasTransitionState,"));
    assert!(js.contains("Object.assign(state, trogdorAtlasTransitionState(\"close_terminal\"))"));
    assert!(js.contains("Object.assign(state, trogdorAtlasTransitionState(\"open\"))"));
    assert!(trogdor_logic.contains("case \"close_terminal\":"));
    assert!(trogdor_logic.contains("...trogdorHoverReaderResetState(),"));
    assert!(js.contains("function applyTrogdorAtlasVisibility()"));
    assert!(js.contains("bindTrogdorEvents: () => trogdorAtlasIsland.bindTrogdorEvents()"));
    assert!(trogdor_island.contains("bindTrogdorEvents"));
    assert!(trogdor_island.contains("handleSurfaceAction: runtime.handleSurfaceAction"));
    assert!(trogdor_surface_controller
        .contains("el.trogdorSurface.style.display = visible ? \"\" : \"none\""));
    assert!(trogdor_surface_controller
        .contains("documentRef.body.classList.toggle(\"trogdor-mode\", visible)"));
    assert!(js.contains("closeTrogdorAtlasForTerminal();"));
    assert!(js.contains("closeTrogdorAtlasForTerminal()"));
    assert!(js.contains("await selectSession(normalized)"));
    assert!(js.contains("focusTerminalInputSurface({ preventScroll: true })"));
    assert!(js.contains("refreshAgentContextForSelectedSession({ force: true })"));
    assert!(js.contains("openTrogdorAgentTerminal(plan.sessionId)"));
}

#[tokio::test]
async fn published_route_shell_sets_follow_focus_mode() {
    let html = html_string(render_index(true).await).await;
    assert!(html.contains("published-focus"));
    assert!(html.contains("follow_published_selection"));
}

// --- decode_client_message unit tests ---

fn operator_auth() -> AuthInfo {
    AuthInfo::new(OPERATOR_SCOPES.to_vec())
}

fn observer_auth() -> AuthInfo {
    AuthInfo::new(OBSERVER_SCOPES.to_vec())
}

// --- build_ready_payload / subscribe_outcome_notice (handshake branches) ---

#[test]
fn build_ready_payload_observer_is_read_only_with_framed_protocol() {
    let cursor = ReplayCursor {
        latest_seq: 42,
        replay_window_start_seq: 10,
    };
    let payload = build_ready_payload("sess-1", false, cursor, 7, WsOutputMode::Framed, &None);
    assert_eq!(payload["type"], "ready");
    assert_eq!(payload["sessionId"], "sess-1");
    assert_eq!(payload["readOnly"], true);
    assert_eq!(payload["replay"]["latestSeq"], 42);
    assert_eq!(payload["replay"]["windowStartSeq"], 10);
    assert_eq!(payload["replay"]["resumeFromSeq"], 7);
    assert_eq!(payload["protocol"]["output"], "framed_v1");
    assert_eq!(payload["summary"], serde_json::Value::Null);
}

#[test]
fn build_ready_payload_writer_is_not_read_only_with_raw_protocol() {
    let cursor = ReplayCursor {
        latest_seq: 1,
        replay_window_start_seq: 0,
    };
    let payload = build_ready_payload("s", true, cursor, 0, WsOutputMode::Raw, &None);
    assert_eq!(payload["readOnly"], false);
    assert_eq!(payload["protocol"]["output"], "raw");
}

#[test]
fn subscribe_outcome_notice_ok_sends_nothing() {
    assert!(subscribe_outcome_notice(&SubscribeOutcome::Ok).is_none());
}

#[test]
fn subscribe_outcome_notice_rejected_overloads_and_closes() {
    let (notice, should_close) = subscribe_outcome_notice(&SubscribeOutcome::Rejected {
        reason: "session is busy".to_string(),
    })
    .expect("rejected outcome should produce a notice");
    assert!(
        should_close,
        "a rejected subscription must close the socket"
    );
    assert_eq!(notice["type"], "overloaded");
    assert_eq!(notice["code"], "SESSION_OVERLOADED");
    assert_eq!(notice["message"], "session is busy");
    assert_eq!(notice["retryAfterMs"], 4000);
}

#[test]
fn subscribe_outcome_notice_replay_truncated_notifies_and_continues() {
    let (notice, should_close) = subscribe_outcome_notice(&SubscribeOutcome::ReplayTruncated {
        requested_resume_from_seq: 5,
        replay_window_start_seq: 9,
        latest_seq: 20,
    })
    .expect("replay-truncated outcome should produce a notice");
    assert!(
        !should_close,
        "replay-truncated must keep streaming, not close"
    );
    assert_eq!(notice["type"], "replay_truncated");
    assert_eq!(notice["requestedResumeFromSeq"], 5);
    assert_eq!(notice["windowStartSeq"], 9);
    assert_eq!(notice["latestSeq"], 20);
}

// --- session_ws_authenticated_inner live-socket integration ---

fn test_state() -> Arc<AppState> {
    test_state_with_config(Config::default())
}

fn test_state_with_config(config: Config) -> Arc<AppState> {
    use tokio::sync::RwLock;
    let config = Arc::new(config);
    let supervisor = crate::session::supervisor::SessionSupervisor::new(config.clone());
    Arc::new(AppState {
        supervisor,
        config,
        thought_config: Arc::new(RwLock::new(
            crate::thought::runtime_config::ThoughtConfig::default(),
        )),
        native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
        ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
        sync_request_sequence: Arc::new(crate::thought::protocol::SyncRequestSequence::new()),
        daemon_defaults: crate::api::once_lock_with(None),
        file_store: crate::api::once_lock_with(None),
        bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
            std::time::Duration::from_secs(15),
        )),
        published_selection: Arc::new(RwLock::new(crate::api::PublishedSelectionState::default())),
        repo_actions: crate::host_actions::RepoActionTracker::default(),
    })
}

async fn spawn_session_ws_test_server(
    state: Arc<AppState>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = super::routes().with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ws test server");
    let addr = listener.local_addr().expect("server addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, server)
}

fn assert_session_ws_http_error(
    err: tokio_tungstenite::tungstenite::Error,
    status: StatusCode,
) -> serde_json::Value {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status().as_u16(), status.as_u16());
            let body = response.body().as_deref().expect("http error body");
            serde_json::from_slice(body).expect("json error body")
        }
        other => panic!("expected websocket HTTP {status} error, got {other:?}"),
    }
}

#[test]
fn session_ws_output_mode_parses_query_flags() {
    let raw = WsQuery {
        token: None,
        resume_from_seq: None,
        framed: None,
    };
    assert_eq!(raw.output_mode(), WsOutputMode::Raw);

    let framed = WsQuery {
        token: None,
        resume_from_seq: None,
        framed: Some("ON".to_string()),
    };
    assert_eq!(framed.output_mode(), WsOutputMode::Framed);

    let explicit_raw = WsQuery {
        token: None,
        resume_from_seq: None,
        framed: Some("false".to_string()),
    };
    assert_eq!(explicit_raw.output_mode(), WsOutputMode::Raw);
}

#[tokio::test]
async fn session_ws_token_mode_rejects_query_token_before_upgrade() {
    let state = test_state_with_config(Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("operator".to_string()),
        observer_token: Some("observer".to_string()),
        ..Config::default()
    });
    let (addr, server) = spawn_session_ws_test_server(state).await;

    let url = format!("ws://{addr}/ws/sessions/missing?token=operator");
    let err = match tokio_tungstenite::connect_async(url).await {
        Ok(_) => panic!("query-token websocket should fail before upgrade"),
        Err(err) => err,
    };

    let body = assert_session_ws_http_error(err, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "WS_QUERY_TOKEN_UNSUPPORTED");
    server.abort();
}

#[tokio::test]
async fn session_ws_local_trust_missing_session_returns_http_not_found() {
    let state = test_state();
    let (addr, server) = spawn_session_ws_test_server(state).await;

    let url = format!("ws://{addr}/ws/sessions/missing");
    let err = match tokio_tungstenite::connect_async(url).await {
        Ok(_) => panic!("missing-session websocket should fail before upgrade"),
        Err(err) => err,
    };

    let body = assert_session_ws_http_error(err, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "SESSION_NOT_FOUND");
    server.abort();
}

#[tokio::test]
async fn session_ws_token_mode_missing_session_sends_ws_error_after_auth() {
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    let state = test_state_with_config(Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("operator".to_string()),
        observer_token: Some("observer".to_string()),
        ..Config::default()
    });
    let (addr, server) = spawn_session_ws_test_server(state).await;

    let url = format!("ws://{addr}/ws/sessions/missing");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect before token auth");
    ws.send(ClientMessage::Text(
        r#"{"type":"auth","token":"operator"}"#.into(),
    ))
    .await
    .expect("send websocket auth message");

    let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("session-not-found payload within timeout")
        .expect("ws stream item")
        .expect("ws message");
    let text = match first {
        ClientMessage::Text(text) => text,
        other => panic!("expected text error frame, got {other:?}"),
    };
    let payload: serde_json::Value =
        serde_json::from_str(&text).expect("session-not-found payload is json");
    assert_eq!(payload["type"], "error");
    assert_eq!(payload["code"], "SESSION_NOT_FOUND");

    let _ = ws.close(None).await;
    server.abort();
}

// Drives the real authenticated WS loop end-to-end over a live socket. Under
// LocalTrust there is no first-frame handshake, so connecting to a live
// session exercises session_ws_authenticated_inner through the replay-cursor
// request, output subscribe (Ok), summary fetch, ready payload, and teardown.
// A minimal fake actor answers the three handshake commands the path needs.
#[tokio::test]
async fn session_ws_authenticated_inner_streams_ready_payload_over_live_socket() {
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(16);
    let (subscribe_resume_tx, subscribe_resume_rx) = oneshot::channel();
    tokio::spawn(async move {
        let mut subscribe_resume_tx = Some(subscribe_resume_tx);
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetReplayCursor(reply) => {
                    let _ = reply.send(ReplayCursor {
                        latest_seq: 5,
                        replay_window_start_seq: 1,
                    });
                }
                SessionCommand::Subscribe {
                    ack,
                    resume_from_seq,
                    ..
                } => {
                    if let Some(tx) = subscribe_resume_tx.take() {
                        let _ = tx.send(resume_from_seq);
                    }
                    let _ = ack.send(SubscribeOutcome::Ok);
                }
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(SessionSummary::placeholder(
                        "ws-sess",
                        "ws-sess",
                        chrono::Utc::now(),
                    ));
                }
                _ => {}
            }
        }
    });

    let state = test_state();
    let handle = ActorHandle::test_handle("ws-sess", "ws-sess", cmd_tx);
    state.supervisor.insert_test_handle(handle).await;

    let (addr, server) = spawn_session_ws_test_server(state.clone()).await;

    let url = format!("ws://{addr}/ws/sessions/ws-sess?framed=framed_v1&resume_from_seq=3");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");

    let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("ready payload within timeout")
        .expect("ws stream item")
        .expect("ws message");
    let text = match first {
        ClientMessage::Text(text) => text,
        other => panic!("expected text ready frame, got {other:?}"),
    };
    let payload: serde_json::Value = serde_json::from_str(&text).expect("ready payload is json");
    assert_eq!(payload["type"], "ready");
    assert_eq!(payload["sessionId"], "ws-sess");
    // LocalTrust grants operator scopes, so the stream is writable.
    assert_eq!(payload["readOnly"], false);
    assert_eq!(payload["replay"]["latestSeq"], 5);
    assert_eq!(payload["replay"]["windowStartSeq"], 1);
    assert_eq!(payload["replay"]["resumeFromSeq"], 3);
    assert_eq!(payload["protocol"]["output"], "framed_v1");
    assert_eq!(
        subscribe_resume_rx.await.expect("subscribe resume seq"),
        Some(3)
    );

    let _ = ws.close(None).await;
    server.abort();
}

#[test]
fn decode_client_message_close_returns_close() {
    let msg = Message::Close(None);
    assert!(matches!(
        decode_client_message(&operator_auth(), &msg),
        WsClientDecision::Close
    ));
}

#[test]
fn decode_client_message_pong_is_ignored() {
    let msg = Message::Pong(b"ping".to_vec().into());
    assert!(matches!(
        decode_client_message(&operator_auth(), &msg),
        WsClientDecision::Ignore
    ));
}

#[test]
fn decode_client_message_ping_frame_sends_pong() {
    let msg = Message::Ping(b"abc".to_vec().into());
    match decode_client_message(&operator_auth(), &msg) {
        WsClientDecision::SendPong(bytes) => assert_eq!(bytes, b"abc"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn rejected_input_delivery_marks_rejected_with_message_clone() {
    let delivery = rejected_input_delivery("not allowed");
    assert!(!delivery.delivered);
    assert_eq!(delivery.method, "rejected");
    assert_eq!(delivery.message.as_deref(), Some("not allowed"));
}

#[test]
fn decode_client_message_binary_without_write_scope_is_read_only_error() {
    let msg = Message::Binary(b"hello".to_vec().into());
    match decode_client_message(&observer_auth(), &msg) {
        WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_client_message_binary_with_write_scope_forwards_write_input() {
    let msg = Message::Binary(b"hello".to_vec().into());
    match decode_client_message(&operator_auth(), &msg) {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInput(data),
            ..
        } => {
            assert_eq!(data, b"hello")
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_client_message_empty_binary_is_ignored() {
    let msg = Message::Binary(b"".to_vec().into());
    assert!(matches!(
        decode_client_message(&operator_auth(), &msg),
        WsClientDecision::Ignore
    ));
}

#[test]
fn decode_binary_input_covers_empty_oversized_and_forwarded_frames() {
    assert!(matches!(decode_binary_input(&[]), WsClientDecision::Ignore));

    let oversized = vec![b'x'; MAX_WS_INPUT_BYTES + 1];
    match decode_binary_input(&oversized) {
        WsClientDecision::SendError {
            code,
            client_message_id,
            ..
        } => {
            assert_eq!(code, "INPUT_TOO_LARGE");
            assert_eq!(client_message_id, None);
        }
        other => panic!("unexpected oversized decision: {other:?}"),
    }

    match decode_binary_input(b"\xff\x00input") {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInput(data),
            client_message_id,
        } => {
            assert_eq!(data, b"\xff\x00input");
            assert_eq!(client_message_id, None);
        }
        other => panic!("unexpected forwarded decision: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_invalid_json_sends_error() {
    match decode_text_client_message(&operator_auth(), "not-json") {
        WsClientDecision::SendError { code, .. } => assert_eq!(code, "INVALID_MESSAGE"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_ping_replies_pong() {
    assert!(matches!(
        decode_text_client_message(&operator_auth(), r#"{"type":"ping"}"#),
        WsClientDecision::ReplyPong
    ));
}

#[test]
fn decode_text_client_message_auth_after_ready_is_ignored() {
    assert!(matches!(
        decode_text_client_message(&operator_auth(), r#"{"type":"auth","token":"secret"}"#),
        WsClientDecision::Ignore
    ));
}

#[test]
fn decode_text_client_message_input_text_without_scope_is_read_only() {
    let json = r#"{"type":"input_text","data":"hello","clientMessageId":"ro-1"}"#;
    match decode_text_client_message(&observer_auth(), json) {
        WsClientDecision::SendError {
            code,
            client_message_id,
            ..
        } => {
            assert_eq!(code, "READ_ONLY");
            assert_eq!(client_message_id.as_deref(), Some("ro-1"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_input_text_forwards_write_input() {
    let json = r#"{"type":"input_text","data":"hello","client_message_id":"ack-1"}"#;
    match decode_text_client_message(&operator_auth(), json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInputAck { data, .. },
            client_message_id,
        } => {
            assert_eq!(data, b"hello");
            assert_eq!(client_message_id.as_deref(), Some("ack-1"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_submit_line_without_scope_is_read_only() {
    let json = r#"{"type":"submit_line","data":"hello"}"#;
    match decode_text_client_message(&observer_auth(), json) {
        WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_submit_line_forwards_submit_line() {
    let json = r#"{"type":"submit_line","data":"hello","clientMessageId":"line-1"}"#;
    match decode_text_client_message(&operator_auth(), json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::SubmitLineAck { text, .. },
            client_message_id,
        } => {
            assert_eq!(text, "hello");
            assert_eq!(client_message_id.as_deref(), Some("line-1"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_blank_submit_line_is_ignored() {
    let json = r#"{"type":"submit_line","data":"   "}"#;
    assert!(matches!(
        decode_text_client_message(&operator_auth(), json),
        WsClientDecision::Ignore
    ));
}

#[test]
fn decode_text_client_message_oversized_input_text_is_rejected() {
    let big = "x".repeat(MAX_WS_INPUT_BYTES + 1);
    let json = format!(r#"{{"type":"input_text","data":"{big}"}}"#);
    match decode_text_client_message(&operator_auth(), &json) {
        WsClientDecision::SendError { code, .. } => assert_eq!(code, "INPUT_TOO_LARGE"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_oversized_control_frame_is_rejected_before_parse() {
    let frame = "{".repeat(MAX_WS_TEXT_FRAME_BYTES + 1);

    match decode_text_client_message(&operator_auth(), &frame) {
        WsClientDecision::SendError {
            code,
            message,
            client_message_id,
        } => {
            assert_eq!(code, "INPUT_TOO_LARGE");
            assert_eq!(
                message,
                format!("terminal control frame exceeds {MAX_WS_TEXT_FRAME_BYTES} byte limit")
            );
            assert_eq!(client_message_id, None);
        }
        other => panic!("unexpected oversized frame decision: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_accepts_near_limit_payload_with_json_overhead() {
    let payload = "x".repeat(MAX_WS_INPUT_BYTES);
    let json = format!(r#"{{"type":"input_text","data":"{payload}"}}"#);
    assert!(
        json.len() <= MAX_WS_TEXT_FRAME_BYTES,
        "control-frame slack should allow a max-sized data field"
    );

    match decode_text_client_message(&operator_auth(), &json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInputAck { data, .. },
            ..
        } => assert_eq!(data.len(), MAX_WS_INPUT_BYTES),
        other => panic!("unexpected near-limit frame decision: {other:?}"),
    }
}

#[test]
fn decode_input_text_covers_empty_oversized_and_forwarded_text() {
    assert!(matches!(
        decode_input_text(String::new(), Some("empty-1".to_string())),
        WsClientDecision::Ignore
    ));

    let oversized = "x".repeat(MAX_WS_INPUT_BYTES + 1);
    match decode_input_text(oversized, Some("big-1".to_string())) {
        WsClientDecision::SendError {
            code,
            message,
            client_message_id,
        } => {
            assert_eq!(code, "INPUT_TOO_LARGE");
            assert_eq!(
                message,
                format!("terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit")
            );
            assert_eq!(client_message_id.as_deref(), Some("big-1"));
        }
        other => panic!("unexpected oversized decision: {other:?}"),
    }

    match decode_input_text("λ\n".to_string(), Some("ack-1".to_string())) {
        WsClientDecision::Forward {
            cmd: SessionCommand::WriteInputAck { data, .. },
            client_message_id,
        } => {
            assert_eq!(data, "λ\n".as_bytes());
            assert_eq!(client_message_id.as_deref(), Some("ack-1"));
        }
        other => panic!("unexpected forwarded decision: {other:?}"),
    }
}

#[test]
fn decode_submit_line_covers_blank_oversized_and_forwarded_text() {
    assert!(matches!(
        decode_submit_line(" \t\n".to_string(), Some("blank-1".to_string())),
        WsClientDecision::Ignore
    ));

    let oversized = "x".repeat(MAX_WS_INPUT_BYTES + 1);
    match decode_submit_line(oversized, Some("big-line-1".to_string())) {
        WsClientDecision::SendError {
            code,
            message,
            client_message_id,
        } => {
            assert_eq!(code, "INPUT_TOO_LARGE");
            assert_eq!(
                message,
                format!("terminal input frame exceeds {MAX_WS_INPUT_BYTES} byte limit")
            );
            assert_eq!(client_message_id.as_deref(), Some("big-line-1"));
        }
        other => panic!("unexpected oversized decision: {other:?}"),
    }

    match decode_submit_line("cargo test".to_string(), Some("line-ack-1".to_string())) {
        WsClientDecision::Forward {
            cmd: SessionCommand::SubmitLineAck { text, .. },
            client_message_id,
        } => {
            assert_eq!(text, "cargo test");
            assert_eq!(client_message_id.as_deref(), Some("line-ack-1"));
        }
        other => panic!("unexpected forwarded decision: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_oversized_submit_line_is_rejected() {
    let big = "x".repeat(MAX_WS_INPUT_BYTES + 1);
    let json = format!(r#"{{"type":"submit_line","data":"{big}","client_message_id":"big-1"}}"#);
    match decode_text_client_message(&operator_auth(), &json) {
        WsClientDecision::SendError {
            code,
            client_message_id,
            ..
        } => {
            assert_eq!(code, "INPUT_TOO_LARGE");
            assert_eq!(client_message_id.as_deref(), Some("big-1"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn resolve_ws_auth_rejects_empty_token_in_token_mode() {
    let config = Config {
        auth_mode: AuthMode::Token,
        auth_token: Some("secret".to_string()),
        ..Config::default()
    };
    // An explicit empty auth-frame token must never authenticate, mirroring
    // the HTTP bearer-token empty guard.
    assert!(resolve_ws_auth(&config, Some("")).is_err());
    assert!(resolve_ws_auth(&config, None).is_err());
    assert!(resolve_ws_auth(&config, Some("secret")).is_ok());
}

#[test]
fn decode_text_client_message_resize_without_scope_is_read_only() {
    let json = r#"{"type":"resize","cols":80,"rows":24}"#;
    match decode_text_client_message(&observer_auth(), json) {
        WsClientDecision::SendError { code, .. } => assert_eq!(code, "READ_ONLY"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_resize_forwards_resize_command() {
    let json = r#"{"type":"resize","cols":80,"rows":24}"#;
    match decode_text_client_message(&operator_auth(), json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::Resize { cols, rows },
            ..
        } => {
            assert_eq!(cols, 80);
            assert_eq!(rows, 24);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_resize_clamps_tiny_payloads() {
    let json = r#"{"type":"resize","cols":0,"rows":1}"#;
    match decode_text_client_message(&operator_auth(), json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::Resize { cols, rows },
            ..
        } => {
            assert_eq!(cols, crate::types::TERMINAL_RESIZE_MIN_COLS);
            assert_eq!(rows, crate::types::TERMINAL_RESIZE_MIN_ROWS);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn decode_text_client_message_resize_clamps_huge_payloads() {
    let json = r#"{"type":"resize","cols":65535,"rows":65535}"#;
    match decode_text_client_message(&operator_auth(), json) {
        WsClientDecision::Forward {
            cmd: SessionCommand::Resize { cols, rows },
            ..
        } => {
            assert_eq!(cols, crate::types::TERMINAL_RESIZE_MAX_COLS);
            assert_eq!(rows, crate::types::TERMINAL_RESIZE_MAX_ROWS);
        }
        other => panic!("unexpected: {other:?}"),
    }
}
