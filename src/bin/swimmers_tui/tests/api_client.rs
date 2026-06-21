use super::*;

const SECRET_BACKEND_USER: &str = "leaky-user";
const SECRET_BACKEND_PASSWORD: &str = "leaky-password";
const SECRET_BACKEND_TOKEN: &str = "leaky-token";
const SECRET_BACKEND_FRAGMENT: &str = "frag-secret";

fn secret_bearing_backend_url(addr: impl std::fmt::Display) -> String {
    format!(
        "http://{SECRET_BACKEND_USER}:{SECRET_BACKEND_PASSWORD}@{addr}/?token={SECRET_BACKEND_TOKEN}#{SECRET_BACKEND_FRAGMENT}"
    )
}

fn assert_backend_url_secrets_redacted(message: &str) {
    for forbidden in [
        SECRET_BACKEND_USER,
        SECRET_BACKEND_PASSWORD,
        SECRET_BACKEND_TOKEN,
        SECRET_BACKEND_FRAGMENT,
        "token=",
    ] {
        assert!(
            !message.contains(forbidden),
            "message leaked {forbidden}: {message}"
        );
    }
}

#[test]
fn api_client_targets_local_backend_for_loopback_hosts() {
    let client = test_api_client("http://127.0.0.1:3210".to_string(), None);
    assert!(client.targets_local_backend());

    let localhost = test_api_client("http://localhost:3210".to_string(), None);
    assert!(localhost.targets_local_backend());

    let remote = test_api_client("http://100.101.123.63:3210".to_string(), None);
    assert!(!remote.targets_local_backend());
}

#[test]
fn api_client_startup_errors_redact_secret_bearing_backend_urls() {
    let client = test_api_client(
        secret_bearing_backend_url("100.64.1.2:3210"),
        Some("auth-token"),
    );

    let error = client.startup_access_error("/v1/sessions", reqwest::StatusCode::UNAUTHORIZED);

    assert!(error.contains("backend at http://100.64.1.2:3210 requires valid auth"));
    assert_backend_url_secrets_redacted(&error);
}

#[test]
fn thought_config_response_deserializes_flattened_api_shape() {
    let value = serde_json::json!({
        "enabled": true,
        "model": "haiku",
        "backend": "claude",
        "cadence_hot_ms": 15000,
        "cadence_warm_ms": 45000,
        "cadence_cold_ms": 120000,
        "daemon_defaults": {
            "model": "haiku",
            "backend": "claude",
            "agent_prompt": "agent",
            "terminal_prompt": "terminal"
        }
    });

    let response: ThoughtConfigResponse =
        serde_json::from_value(value).expect("flattened thought config response");

    assert_eq!(response.config.backend, "claude");
    assert_eq!(response.config.model, "haiku");
    assert_eq!(response.version, 0);
    assert_eq!(
        response
            .daemon_defaults
            .as_ref()
            .map(|defaults| defaults.backend.as_str()),
        Some("claude")
    );
}

#[tokio::test]
async fn update_thought_config_sends_if_match_and_parses_version() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::put;
    use axum::{Json, Router};

    let observed_if_match = Arc::new(Mutex::new(None));
    let route_if_match = Arc::clone(&observed_if_match);
    let app = Router::new().route(
        "/v1/thought-config",
        put(move |headers: HeaderMap, Json(body): Json<ThoughtConfig>| {
            let route_if_match = Arc::clone(&route_if_match);
            async move {
                *route_if_match.lock().unwrap() = headers
                    .get("if-match")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                assert_eq!(body.backend, "claude");
                assert_eq!(body.model, "haiku");
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "enabled": body.enabled,
                        "model": body.model,
                        "backend": body.backend,
                        "cadence_hot_ms": body.cadence_hot_ms,
                        "cadence_warm_ms": body.cadence_warm_ms,
                        "cadence_cold_ms": body.cadence_cold_ms,
                        "version": 8,
                    })),
                )
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    let client = test_api_client(format!("http://{addr}"), None);
    let response = client
        .update_thought_config(
            ThoughtConfig {
                backend: "claude".to_string(),
                model: "haiku".to_string(),
                ..ThoughtConfig::default()
            },
            Some(7),
        )
        .await
        .expect("update thought config");

    assert_eq!(observed_if_match.lock().unwrap().as_deref(), Some("7"));
    assert_eq!(response.version, 8);
    assert_eq!(response.config.backend, "claude");

    handle.abort();
}

async fn spawn_guarded_startup_server(
    expected_token: &str,
    selection_status: axum::http::StatusCode,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, put};
    use axum::Router;

    let expected_sessions_auth = format!("Bearer {expected_token}");
    let expected_selection_auth = expected_sessions_auth.clone();

    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move |headers: HeaderMap| {
                let expected_auth = expected_sessions_auth.clone();
                async move {
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        == Some(expected_auth.as_str())
                    {
                        StatusCode::OK
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }
            }),
        )
        .route(
            "/v1/selection",
            put(move |headers: HeaderMap| {
                let expected_auth = expected_selection_auth.clone();
                async move {
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        == Some(expected_auth.as_str())
                    {
                        selection_status
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn api_client_transport_errors_are_actionable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = test_api_client(
        secret_bearing_backend_url(format!("127.0.0.1:{port}")),
        None,
    );

    let error = client
        .fetch_sessions()
        .await
        .expect_err("closed localhost port should fail");
    assert!(error.contains(&format!(
        "swimmers API unavailable at http://127.0.0.1:{port}"
    )));
    assert!(error.contains("Start `swimmers` or set SWIMMERS_TUI_URL."));
    assert!(!error.contains("error sending request for url"));
    assert_backend_url_secrets_redacted(&error);
}

#[tokio::test]
async fn api_client_preserves_api_error_codes_in_messages() {
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};

    async fn failing_sessions() -> (StatusCode, Json<swimmers::types::ErrorResponse>) {
        (
            StatusCode::BAD_REQUEST,
            Json(swimmers::types::ErrorResponse::with_message(
                "VALIDATION_FAILED",
                "bad sessions request",
            )),
        )
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/sessions", get(failing_sessions)),
        )
        .await
        .expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let error = client
        .fetch_sessions()
        .await
        .expect_err("server error should preserve response code");

    handle.abort();
    assert_eq!(error, "VALIDATION_FAILED: bad sessions request");
}

#[tokio::test]
async fn api_client_test_thought_config_falls_back_when_local_backend_is_unreachable() {
    let _lock = lock_test_env();
    let original = env::var("CLAWGS_BIN").ok();
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    env::set_var("CLAWGS_BIN", fake_bin.as_os_str());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = test_api_client(format!("http://127.0.0.1:{port}"), None);
    let response = client
        .test_thought_config(ThoughtConfig::default())
        .await
        .expect("local transport error should fall back to local probe");

    restore_env_var("CLAWGS_BIN", original);

    assert!(response.ok);
    assert_eq!(response.message, "probe succeeded");
    assert_eq!(response.llm_calls, 1);
}

#[tokio::test]
async fn api_client_test_thought_config_falls_back_when_backend_route_is_missing() {
    use axum::Router;

    let _lock = lock_test_env();
    let original = env::var("CLAWGS_BIN").ok();
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_bin = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    env::set_var("CLAWGS_BIN", fake_bin.as_os_str());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve empty test api");
    });

    let client = test_api_client(format!("http://{addr}"), None);
    let response = client
        .test_thought_config(ThoughtConfig::default())
        .await
        .expect("404 fallback should return local probe result");

    handle.abort();
    restore_env_var("CLAWGS_BIN", original);

    assert!(response.ok);
    assert_eq!(response.message, "probe succeeded");
    assert_eq!(response.llm_calls, 1);
}

async fn spawn_delayed_api_server(
    sessions_delay: Option<Duration>,
    native_open_delay: Option<Duration>,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::http::StatusCode;
    use axum::routing::{get, post, put};
    use axum::{Json, Router};

    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move || async move {
                if let Some(delay) = sessions_delay {
                    tokio::time::sleep(delay).await;
                }
                Json(SessionListResponse {
                    sessions: vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
                    version: 1,
                    repo_themes: HashMap::new(),
                    environments: Vec::new(),
                    fleet_lens: Default::default(),
                    fleet_presets: Vec::new(),
                })
            }),
        )
        .route("/v1/selection", put(|| async { StatusCode::OK }))
        .route(
            "/v1/native/open",
            post(move || async move {
                if let Some(delay) = native_open_delay {
                    tokio::time::sleep(delay).await;
                }
                Json(NativeDesktopOpenResponse {
                    session_id: "sess-1".to_string(),
                    status: "focused".to_string(),
                    pane_id: Some("pane-1".to_string()),
                })
            }),
        )
        .route(
            "/v1/native/app",
            put(|Json(body): Json<NativeDesktopConfigRequest>| async move {
                Json(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("macos".to_string()),
                    app_id: Some(body.app),
                    ghostty_mode: (body.app == NativeDesktopApp::Ghostty)
                        .then_some(GhosttyOpenMode::Swap),
                    app: Some(body.app.display_name().to_string()),
                    reason: None,
                })
            }),
        )
        .route(
            "/v1/native/mode",
            put(|Json(body): Json<NativeDesktopModeRequest>| async move {
                Json(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("macos".to_string()),
                    app_id: Some(NativeDesktopApp::Ghostty),
                    ghostty_mode: Some(body.mode),
                    app: Some(NativeDesktopApp::Ghostty.display_name().to_string()),
                    reason: None,
                })
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_delayed_create_api_server(
    create_delay: Duration,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::post;
    use axum::{Json, Router};

    let app = Router::new().route(
        "/v1/sessions",
        post(move || async move {
            tokio::time::sleep(create_delay).await;
            Json(create_response("sess-1", "7", TEST_REPO_SWIMMERS))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_capturing_create_api_server() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<Mutex<Option<CreateSessionRequest>>>,
) {
    use axum::routing::post;
    use axum::{Json, Router};

    let captured_request = Arc::new(Mutex::new(None));
    let route_captured_request = Arc::clone(&captured_request);
    let app = Router::new().route(
        "/v1/sessions",
        post(move |Json(body): Json<CreateSessionRequest>| {
            let captured_request = Arc::clone(&route_captured_request);
            async move {
                *captured_request.lock().expect("captured request lock") = Some(body);
                Json(create_response("sess-1", "7", TEST_REPO_SWIMMERS))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle, captured_request)
}

async fn spawn_capturing_batch_create_api_server() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<Mutex<Option<CreateSessionsBatchRequest>>>,
) {
    use axum::routing::post;
    use axum::{Json, Router};

    let captured_request = Arc::new(Mutex::new(None));
    let route_captured_request = Arc::clone(&captured_request);
    let app = Router::new().route(
        "/v1/sessions/batch",
        post(move |Json(body): Json<CreateSessionsBatchRequest>| {
            let captured_request = Arc::clone(&route_captured_request);
            async move {
                *captured_request
                    .lock()
                    .expect("captured batch request lock") = Some(body);
                Json(create_batch_response(&[
                    ("sess-alpha", "alpha", TEST_REPO_ALPHA),
                    ("sess-beta", "beta", TEST_REPO_BETA),
                ]))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle, captured_request)
}

async fn spawn_delayed_dirs_api_server(
    list_delay: Duration,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::get;
    use axum::{Json, Router};

    let app = Router::new().route(
        "/v1/dirs",
        get(move || async move {
            tokio::time::sleep(list_delay).await;
            Json(dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });

    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn api_client_open_session_allows_slower_native_open_responses() {
    let (base_url, handle) = spawn_delayed_api_server(None, Some(Duration::from_millis(150))).await;
    let client = test_api_client(base_url, None);

    let response = client
        .open_session("sess-1")
        .await
        .expect("terminal handoff should outlive the default polling timeout");

    handle.abort();
    assert_eq!(response.session_id, "sess-1");
    assert_eq!(response.status, "focused");
    assert_eq!(response.pane_id.as_deref(), Some("pane-1"));
}

#[tokio::test]
async fn api_client_create_session_allows_slower_session_creation_responses() {
    let (base_url, handle) = spawn_delayed_create_api_server(Duration::from_millis(150)).await;
    let client = test_api_client(base_url, None);

    let response = client
        .create_session(TEST_REPO_SWIMMERS, SpawnTool::Codex, None, None)
        .await
        .expect("create session should outlive the default polling timeout");

    handle.abort();
    let session = response.session.as_ref().expect("created session");
    assert_eq!(session.session_id, "sess-1");
    assert_eq!(session.tmux_name, "7");
}

#[tokio::test]
async fn api_client_create_session_sends_launch_target_and_initial_request() {
    let (base_url, handle, captured_request) = spawn_capturing_create_api_server().await;
    let client = test_api_client(base_url, None);

    client
        .create_session(
            TEST_REPO_SWIMMERS,
            SpawnTool::Grok,
            Some("jeremy-skillbox".to_string()),
            Some("move this off laptop".to_string()),
        )
        .await
        .expect("create session should preserve remote launch metadata");

    handle.abort();
    let request = captured_request
        .lock()
        .expect("captured request lock")
        .take()
        .expect("captured create-session request");
    assert_eq!(request.cwd.as_deref(), Some(TEST_REPO_SWIMMERS));
    assert_eq!(request.spawn_tool, Some(SpawnTool::Grok));
    assert_eq!(request.launch_target.as_deref(), Some("jeremy-skillbox"));
    assert_eq!(
        request.initial_request.as_deref(),
        Some("move this off laptop")
    );
}

#[tokio::test]
async fn api_client_create_sessions_batch_sends_launch_target_and_initial_request() {
    let (base_url, handle, captured_request) = spawn_capturing_batch_create_api_server().await;
    let client = test_api_client(base_url, None);

    client
        .create_sessions_batch(
            vec![TEST_REPO_ALPHA.to_string(), TEST_REPO_BETA.to_string()],
            SpawnTool::Codex,
            Some("jeremy-skillbox".to_string()),
            Some("fan out remotely".to_string()),
        )
        .await
        .expect("batch create should preserve remote launch metadata");

    handle.abort();
    let request = captured_request
        .lock()
        .expect("captured batch request lock")
        .take()
        .expect("captured batch create request");
    assert_eq!(
        request.dirs,
        vec![TEST_REPO_ALPHA.to_string(), TEST_REPO_BETA.to_string()]
    );
    assert_eq!(request.spawn_tool, Some(SpawnTool::Codex));
    assert_eq!(request.launch_target.as_deref(), Some("jeremy-skillbox"));
    assert_eq!(request.initial_request.as_deref(), Some("fan out remotely"));
}

#[tokio::test]
async fn api_client_list_dirs_allows_slower_directory_listing_responses() {
    let (base_url, handle) = spawn_delayed_dirs_api_server(Duration::from_millis(150)).await;
    let client = test_api_client(base_url, None);

    let response = client
        .list_dirs(None, true, None, None)
        .await
        .expect("list dirs should outlive the default polling timeout");

    handle.abort();
    assert_eq!(response.path, TEST_REPOS_ROOT);
    assert_eq!(
        response
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["swimmers"]
    );
}

#[tokio::test]
async fn api_client_can_switch_native_app_without_restart() {
    let (base_url, handle) = spawn_delayed_api_server(None, None).await;
    let client = test_api_client(base_url, None);

    let response = client
        .set_native_app(NativeDesktopApp::Ghostty)
        .await
        .expect("terminal handoff target switch should succeed");

    handle.abort();
    assert_eq!(response.app_id, Some(NativeDesktopApp::Ghostty));
    assert_eq!(response.ghostty_mode, Some(GhosttyOpenMode::Swap));
    assert_eq!(response.app.as_deref(), Some("Ghostty"));
}

#[tokio::test]
async fn api_client_can_switch_ghostty_mode_without_restart() {
    let (base_url, handle) = spawn_delayed_api_server(None, None).await;
    let client = test_api_client(base_url, None);

    let response = client
        .set_native_mode(GhosttyOpenMode::Add)
        .await
        .expect("native mode switch should succeed");

    handle.abort();
    assert_eq!(response.app_id, Some(NativeDesktopApp::Ghostty));
    assert_eq!(response.ghostty_mode, Some(GhosttyOpenMode::Add));
}

#[tokio::test]
async fn api_client_set_native_app_reports_restart_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(secret_bearing_backend_url(addr), None);

    let error = client
        .set_native_app(NativeDesktopApp::Ghostty)
        .await
        .expect_err("missing route should surface restart hint");

    handle.abort();
    assert!(error.contains(&format!("backend at http://{addr}")));
    assert!(error.contains("does not support runtime terminal handoff target switching yet"));
    assert!(error.contains("restart `swimmers`"));
    assert_backend_url_secrets_redacted(&error);
}

#[tokio::test]
async fn api_client_set_native_mode_reports_restart_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(secret_bearing_backend_url(addr), None);

    let error = client
        .set_native_mode(GhosttyOpenMode::Add)
        .await
        .expect_err("missing route should surface restart hint");

    handle.abort();
    assert!(error.contains(&format!("backend at http://{addr}")));
    assert!(error.contains("does not support runtime Ghostty handoff placement switching yet"));
    assert!(error.contains("restart `swimmers`"));
    assert_backend_url_secrets_redacted(&error);
}

#[tokio::test]
async fn api_client_list_dirs_reports_feature_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(secret_bearing_backend_url(addr), None);

    let error = client
        .list_dirs(None, true, None, None)
        .await
        .expect_err("missing route should explain the required runtime switch");

    handle.abort();
    assert!(error.contains(&format!("backend at http://{addr}")));
    assert!(error.contains("does not expose /v1/dirs"));
    assert!(error.contains("SWIMMERS_PERSONAL_WORKFLOWS=1"));
    assert!(error.contains("make tui"));
    assert_backend_url_secrets_redacted(&error);
}

#[tokio::test]
async fn api_client_fetch_session_skills_reports_feature_hint_on_404() {
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, Router::new())
            .await
            .expect("serve test api");
    });
    let client = test_api_client(secret_bearing_backend_url(addr), None);

    let error = client
        .fetch_session_skills("sess-1")
        .await
        .expect_err("missing route should explain the required runtime switch");

    handle.abort();
    assert!(error.contains(&format!("backend at http://{addr}")));
    assert!(error.contains("does not expose session skills"));
    assert!(error.contains("SWIMMERS_PERSONAL_WORKFLOWS=1"));
    assert!(error.contains("make tui"));
    assert_backend_url_secrets_redacted(&error);
}

#[tokio::test]
async fn api_client_fetch_sessions_overrides_default_client_timeout() {
    let (base_url, handle) = spawn_delayed_api_server(Some(Duration::from_millis(150)), None).await;
    let client = test_api_client(base_url.clone(), None);

    let sessions = client
        .fetch_sessions()
        .await
        .expect("session refresh should allow slow-but-healthy local responses");

    handle.abort();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-1");
}

#[tokio::test]
async fn api_client_fetch_session_snapshot_uses_single_sessions_envelope() {
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let session_hits = Arc::new(AtomicUsize::new(0));
    let environment_hits = Arc::new(AtomicUsize::new(0));
    let session_hits_route = Arc::clone(&session_hits);
    let environment_hits_route = Arc::clone(&environment_hits);
    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move || {
                let session_hits = Arc::clone(&session_hits_route);
                async move {
                    session_hits.fetch_add(1, Ordering::SeqCst);
                    let mut environment = EnvironmentSummary::local();
                    environment.id = "remote-devbox".to_string();
                    environment.kind = "ssh".to_string();
                    Json(SessionListResponse {
                        sessions: vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
                        version: 1,
                        repo_themes: HashMap::new(),
                        environments: vec![environment],
                        fleet_lens: Default::default(),
                        fleet_presets: vec![FleetLensPreset {
                            id: "remote-devbox".to_string(),
                            label: "Remote devbox".to_string(),
                            source: "test".to_string(),
                            matchers: vec![FleetLensPresetMatcher::TargetId {
                                id: "remote-devbox".to_string(),
                            }],
                        }],
                    })
                }
            }),
        )
        .route(
            "/v1/environments",
            get(move || {
                let environment_hits = Arc::clone(&environment_hits_route);
                async move {
                    environment_hits.fetch_add(1, Ordering::SeqCst);
                    Json(EnvironmentListResponse {
                        environments: Vec::new(),
                        fleet_presets: Vec::new(),
                    })
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let snapshot = client.fetch_session_snapshot().await.expect("snapshot");

    handle.abort();
    assert_eq!(session_hits.load(Ordering::SeqCst), 1);
    assert_eq!(environment_hits.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot.sessions.len(), 1);
    assert_eq!(snapshot.environments[0].id, "remote-devbox");
    assert_eq!(snapshot.fleet_presets[0].id, "remote-devbox");
}

#[tokio::test]
async fn api_client_fetch_environment_metadata_uses_environment_endpoint() {
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let session_hits = Arc::new(AtomicUsize::new(0));
    let environment_hits = Arc::new(AtomicUsize::new(0));
    let session_hits_route = Arc::clone(&session_hits);
    let environment_hits_route = Arc::clone(&environment_hits);
    let app = Router::new()
        .route(
            "/v1/sessions",
            get(move || {
                let session_hits = Arc::clone(&session_hits_route);
                async move {
                    session_hits.fetch_add(1, Ordering::SeqCst);
                    Json(SessionListResponse {
                        sessions: Vec::new(),
                        version: 1,
                        repo_themes: HashMap::new(),
                        environments: Vec::new(),
                        fleet_lens: Default::default(),
                        fleet_presets: Vec::new(),
                    })
                }
            }),
        )
        .route(
            "/v1/environments",
            get(move || {
                let environment_hits = Arc::clone(&environment_hits_route);
                async move {
                    environment_hits.fetch_add(1, Ordering::SeqCst);
                    let mut environment = EnvironmentSummary::local();
                    environment.id = "remote-devbox".to_string();
                    environment.kind = "ssh".to_string();
                    Json(EnvironmentListResponse {
                        environments: vec![environment],
                        fleet_presets: vec![FleetLensPreset {
                            id: "remote-devbox".to_string(),
                            label: "Remote devbox".to_string(),
                            source: "test".to_string(),
                            matchers: vec![FleetLensPresetMatcher::TargetId {
                                id: "remote-devbox".to_string(),
                            }],
                        }],
                    })
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let metadata = client
        .fetch_environment_metadata()
        .await
        .expect("environment metadata");

    handle.abort();
    assert_eq!(environment_hits.load(Ordering::SeqCst), 1);
    assert_eq!(session_hits.load(Ordering::SeqCst), 0);
    assert_eq!(metadata.environments[0].id, "remote-devbox");
    assert_eq!(metadata.fleet_presets[0].id, "remote-devbox");
}

#[tokio::test]
async fn api_client_fetch_environment_metadata_falls_back_to_sessions_envelope_on_404() {
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let session_hits = Arc::new(AtomicUsize::new(0));
    let session_hits_route = Arc::clone(&session_hits);
    let app = Router::new().route(
        "/v1/sessions",
        get(move || {
            let session_hits = Arc::clone(&session_hits_route);
            async move {
                session_hits.fetch_add(1, Ordering::SeqCst);
                let mut environment = EnvironmentSummary::local();
                environment.id = "legacy-devbox".to_string();
                environment.kind = "ssh".to_string();
                Json(SessionListResponse {
                    sessions: vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
                    version: 1,
                    repo_themes: HashMap::new(),
                    environments: vec![environment],
                    fleet_lens: Default::default(),
                    fleet_presets: vec![FleetLensPreset {
                        id: "legacy-devbox".to_string(),
                        label: "Legacy devbox".to_string(),
                        source: "legacy".to_string(),
                        matchers: vec![FleetLensPresetMatcher::TargetId {
                            id: "legacy-devbox".to_string(),
                        }],
                    }],
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), None);

    let metadata = client
        .fetch_environment_metadata()
        .await
        .expect("environment metadata fallback");

    handle.abort();
    assert_eq!(session_hits.load(Ordering::SeqCst), 1);
    assert_eq!(metadata.environments[0].id, "legacy-devbox");
    assert_eq!(metadata.fleet_presets[0].id, "legacy-devbox");
}

#[tokio::test]
async fn api_client_fetch_environment_metadata_fallback_preserves_bearer_auth() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let session_hits = Arc::new(AtomicUsize::new(0));
    let session_hits_route = Arc::clone(&session_hits);
    let app = Router::new().route(
        "/v1/sessions",
        get(move |headers: HeaderMap| {
            let session_hits = Arc::clone(&session_hits_route);
            async move {
                let authorized = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer testtoken");
                if !authorized {
                    return StatusCode::UNAUTHORIZED.into_response();
                }

                session_hits.fetch_add(1, Ordering::SeqCst);
                let mut environment = EnvironmentSummary::local();
                environment.id = "legacy-token-devbox".to_string();
                Json(SessionListResponse {
                    sessions: Vec::new(),
                    version: 1,
                    repo_themes: HashMap::new(),
                    environments: vec![environment],
                    fleet_lens: Default::default(),
                    fleet_presets: Vec::new(),
                })
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test api");
    });
    let client = test_api_client(format!("http://{addr}"), Some("testtoken"));

    let metadata = client
        .fetch_environment_metadata()
        .await
        .expect("authenticated environment metadata fallback");

    handle.abort();
    assert_eq!(session_hits.load(Ordering::SeqCst), 1);
    assert_eq!(metadata.environments[0].id, "legacy-token-devbox");
}

#[tokio::test]
async fn startup_preflight_waits_for_slow_local_sessions() {
    let (base_url, handle) = spawn_delayed_api_server(Some(Duration::from_millis(150)), None).await;
    let client = test_api_client(base_url, None);

    let result = client.preflight_startup_access().await;

    handle.abort();
    assert!(
        result.is_ok(),
        "local startup preflight should allow cold responses"
    );
}

#[tokio::test]
async fn api_client_normalizes_trailing_slash_base_url_for_external_calls() {
    let (base_url, handle) = spawn_delayed_api_server(None, None).await;
    let client = test_api_client(format!("{base_url}/"), None);

    let sessions = client
        .fetch_sessions()
        .await
        .expect("trailing slash base URL should still fetch sessions");
    let preflight = client.preflight_startup_access().await;

    handle.abort();
    assert_eq!(client.base_url, base_url);
    assert_eq!(sessions.len(), 1);
    assert!(
        preflight.is_ok(),
        "trailing slash base URL should still preflight startup routes"
    );
}

#[tokio::test]
async fn startup_preflight_retries_until_local_listener_is_ready() {
    use axum::http::StatusCode;
    use axum::routing::{get, put};
    use axum::{Json, Router};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let app = Router::new()
            .route(
                "/v1/sessions",
                get(|| async {
                    Json(SessionListResponse {
                        sessions: vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
                        version: 1,
                        repo_themes: HashMap::new(),
                        environments: Vec::new(),
                        fleet_lens: Default::default(),
                        fleet_presets: Vec::new(),
                    })
                }),
            )
            .route("/v1/selection", put(|| async { StatusCode::OK }));
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .expect("bind delayed startup server");
        axum::serve(listener, app)
            .await
            .expect("serve delayed startup api");
    });

    let client = test_api_client(format!("http://127.0.0.1:{port}"), None);
    let result = client.preflight_startup_access().await;

    handle.abort();
    assert!(
        result.is_ok(),
        "startup preflight should retry local transport errors"
    );
}

#[tokio::test]
async fn startup_preflight_times_out_after_local_warmup_budget() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = ApiClient {
        http: test_http_client(Duration::from_millis(100)),
        startup_http: test_http_client(Duration::from_millis(100)),
        base_url: format!("http://127.0.0.1:{port}"),
        auth_token: None,
        startup_wait_timeout: Duration::from_millis(40),
        startup_retry_interval: Duration::from_millis(10),
    };

    let error = client
        .preflight_startup_access()
        .await
        .expect_err("missing local backend should fail after startup budget");

    assert!(error.contains("swimmers API unavailable at"));
    assert!(error.contains("Start `swimmers` or set SWIMMERS_TUI_URL."));
}

#[tokio::test]
async fn startup_preflight_accepts_matching_bearer_token() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
    let client = test_api_client(base_url, Some("testtoken"));

    let result = client.preflight_startup_access().await;

    handle.abort();
    assert!(
        result.is_ok(),
        "matching token should pass startup preflight"
    );
}

#[tokio::test]
async fn startup_preflight_requires_matching_auth_for_sessions() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
    let client = test_api_client(base_url.clone(), None);

    let error = client
        .preflight_startup_access()
        .await
        .expect_err("missing auth should fail startup preflight");

    handle.abort();
    assert!(error.contains(&base_url));
    assert!(error.contains("/v1/sessions"));
    assert!(error.contains("AUTH_MODE=token"));
    assert!(error.contains("AUTH_TOKEN"));
}

#[tokio::test]
async fn startup_preflight_requires_selection_scope() {
    let (base_url, handle) =
        spawn_guarded_startup_server("testtoken", axum::http::StatusCode::FORBIDDEN).await;
    let client = test_api_client(base_url.clone(), Some("testtoken"));

    let error = client
        .preflight_startup_access()
        .await
        .expect_err("selection auth failure should fail startup preflight");

    handle.abort();
    assert!(error.contains(&base_url));
    assert!(error.contains("/v1/selection"));
    assert!(error.contains("required session scope"));
}
