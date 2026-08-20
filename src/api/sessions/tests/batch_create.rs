use super::*;

#[tokio::test]
async fn create_sessions_batch_requires_write_scope() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: vec!["/tmp/project".to_string()],
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_sessions_batch_rejects_empty_dirs() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_remote_sessions_batch_response_maps_validation_errors() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: Some("remote-target".to_string()),
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_sessions_batch_rejects_blank_dirs() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: vec!["/tmp/project".to_string(), " \t\n".to_string()],
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not include blank entries");
}

#[tokio::test]
async fn create_sessions_batch_rejects_oversized_batches() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: (0..=BATCH_CREATE_MAX_DIRS)
                .map(|idx| format!("/tmp/project-{idx}"))
                .collect(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(
        json["message"],
        format!("dirs must include at most {BATCH_CREATE_MAX_DIRS} entries")
    );
}

#[tokio::test]
async fn create_sessions_batch_assigns_shared_batch_metadata() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);

    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(CreateSessionsBatchRequest {
            dirs,
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: Some("wire jwt refresh + tests".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CREATED);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");
    let first_batch = &results[0]["session"]["batch"];
    let second_batch = &results[1]["session"]["batch"];

    assert!(first_batch["id"]
        .as_str()
        .expect("batch id")
        .starts_with("batch-"));
    assert_eq!(second_batch["id"], first_batch["id"]);
    assert_eq!(first_batch["label"], "wire jwt refresh + tests");
    assert_eq!(first_batch["prompt_excerpt"], "wire jwt refresh + tests");
    assert_eq!(first_batch["index"], 0);
    assert_eq!(second_batch["index"], 1);
    assert_eq!(first_batch["total"], 2);
    assert_eq!(second_batch["total"], 2);
    assert!(first_batch["created_at"].is_string());

    cleanup_created_sessions(&state, &json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_permutation_preserves_cwd_result_classes() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");

    for (case_index, names) in generated_dir_name_sets().into_iter().enumerate() {
        let dirs = create_case_dirs(root.path(), case_index, &names);
        let reversed_dirs = dirs.iter().rev().cloned().collect::<Vec<_>>();

        let response = create_batch(state.clone(), dirs.clone()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let forward_json = response_json(response).await;

        let response = create_batch(state.clone(), reversed_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let reversed_json = response_json(response).await;

        assert_eq!(
            cwd_result_classes(&forward_json),
            cwd_result_classes(&reversed_json)
        );

        cleanup_created_sessions(&state, &forward_json).await;
        cleanup_created_sessions(&state, &reversed_json).await;
    }
}

#[tokio::test]
async fn create_sessions_batch_mr_additive_valid_dir_increases_success_count() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let base_dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
    let mut extended_dirs = base_dirs.clone();
    extended_dirs.extend(create_case_dirs(root.path(), 1, &["docs".to_string()]));

    let response = create_batch(state.clone(), base_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let base_json = response_json(response).await;

    let response = create_batch(state.clone(), extended_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let extended_json = response_json(response).await;

    assert_eq!(success_count(&extended_json), success_count(&base_json) + 1);
    assert_eq!(
        extended_json["results"].as_array().expect("results").len(),
        base_json["results"].as_array().expect("results").len() + 1
    );

    cleanup_created_sessions(&state, &base_json).await;
    cleanup_created_sessions(&state, &extended_json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_invalid_dir_injection_is_exclusive() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let valid_dirs = create_case_dirs(
        root.path(),
        0,
        &["frontend".to_string(), "backend".to_string()],
    );
    let missing_dir = root.path().join("missing").to_string_lossy().into_owned();
    let dirs = vec![
        valid_dirs[0].clone(),
        missing_dir.clone(),
        valid_dirs[1].clone(),
    ];

    let response = create_batch(state.clone(), dirs).await;
    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");

    assert_eq!(results.len(), 3);
    assert_eq!(success_count(&json), 2);
    assert_eq!(results[1]["index"], 1);
    assert_eq!(results[1]["cwd"], missing_dir);
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["error"]["code"], "VALIDATION_FAILED");
    assert!(results[0]["session"]["session_id"].is_string());
    assert!(results[2]["session"]["session_id"].is_string());

    cleanup_created_sessions(&state, &json).await;
}

fn cass_fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("read {name}: {error}"))
}

fn cass_intent() -> crate::types::CassAdmissionIntent {
    crate::types::CassAdmissionIntent::from_json_str(&cass_fixture("cass_admission_intent_v1.json"))
        .expect("intent fixture")
}

struct CassEnv {
    _dir: tempfile::TempDir,
    log: std::path::PathBuf,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl CassEnv {
    fn requests(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("Cass command request JSON"))
            .collect()
    }
}

impl Drop for CassEnv {
    fn drop(&mut self) {
        for (key, previous) in self.previous.drain(..) {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        std::env::remove_var("SWIMMERS_CASS_TEST_PROVIDER_UUID");
        std::env::remove_var("SWIMMERS_CASS_TEST_CONSUME_UNBOUND");
        std::env::remove_var("SWIMMERS_CASS_TEST_CONSUME_DELAY_MS");
        std::env::remove_var("SWIMMERS_CASS_TEST_ERROR_MESSAGE");
        std::env::remove_var("SWIMMERS_CASS_TEST_RESERVE_UNBOUND");
        std::env::remove_var("SWIMMERS_CASS_TEST_RESERVE_DELAY_AFTER_FIRST");
        std::env::remove_var("SWIMMERS_CASS_TEST_RELEASE_FAIL_FIRST");
    }
}

fn install_cass_admission_command(fail_reserve: bool) -> CassEnv {
    let dir = tempdir().expect("cass cmd dir");
    let script = dir.path().join("cass-admit");
    let book = dir.path().join("book.json");
    let log = dir.path().join("ops.log");
    let refined = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cass_provider_identity_v1_refined.json");
    let contents = format!(
        r#"#!/usr/bin/env python3
import fcntl, json, os, sys, hashlib, time
from pathlib import Path
book_path = Path({book:?})
log_path = Path({log:?})
lock_file = book_path.with_suffix(".lock").open("a+")
fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
req = json.load(sys.stdin)
prior = log_path.read_text() if log_path.exists() else ""
log_path.write_text(prior + json.dumps(req) + "\n")
book = json.loads(book_path.read_text()) if book_path.exists() else {{}}
op = req.get("op")
if {fail} and op == "reserve":
    json.dump({{"schema_version":"cass_admission_command/v1","ok":False,"error":{{"code":"reservation_failed","message":os.environ.get("SWIMMERS_CASS_TEST_ERROR_MESSAGE", "forced reserve failure")}}}}, sys.stdout)
    sys.exit(2)
if op == "reserve":
    reserve_count = sum(1 for line in log_path.read_text().splitlines() if json.loads(line).get("op") == "reserve")
    if os.environ.get("SWIMMERS_CASS_TEST_RESERVE_DELAY_AFTER_FIRST") and reserve_count > 1:
        time.sleep(5)
    intent = req["intent"]
    rid = "rsv_" + hashlib.sha256(json.dumps(intent, sort_keys=True).encode()).hexdigest()
    book[rid] = {{"state":"reserved","intent":intent,"consumed":False}}
    book_path.write_text(json.dumps(book))
    batch_id = "018f0e11-7c3a-7000-8000-ffffffffffff" if os.environ.get("SWIMMERS_CASS_TEST_RESERVE_UNBOUND") else intent["batch_id"]
    json.dump({{"schema_version":"cass_admission_command/v1","ok":True,"reservation_id":rid,"state":"reserved","batch_id":batch_id,"batch_index":intent["batch_index"]}}, sys.stdout)
elif op == "consume":
    consume_delay_ms = int(os.environ.get("SWIMMERS_CASS_TEST_CONSUME_DELAY_MS", "0"))
    if consume_delay_ms > 0:
        time.sleep(consume_delay_ms / 1000)
    rid = req.get("reservation_id")
    rec = book.get(rid)
    if not rec or rec.get("consumed") or rec.get("state") != "reserved":
        json.dump({{"schema_version":"cass_admission_command/v1","ok":False,"error":{{"code":"reservation_mismatch","message":"wrong/replayed/foreign reservation"}}}}, sys.stdout)
        sys.exit(2)
    if rec["intent"]["batch_id"] != req.get("batch_id") or rec["intent"]["batch_index"] != req.get("batch_index"):
        json.dump({{"schema_version":"cass_admission_command/v1","ok":False,"error":{{"code":"reservation_mismatch","message":"correlation mismatch"}}}}, sys.stdout)
        sys.exit(2)
    rec["consumed"] = True
    book[rid] = rec
    book_path.write_text(json.dumps(book))
    batch_id = "018f0e11-7c3a-7000-8000-ffffffffffff" if os.environ.get("SWIMMERS_CASS_TEST_CONSUME_UNBOUND") else req.get("batch_id")
    json.dump({{"schema_version":"cass_admission_command/v1","ok":True,"reservation_id":rid,"state":"consumed","batch_id":batch_id,"batch_index":req.get("batch_index")}}, sys.stdout)
elif op == "refine":
    identity = req.get("identity") or {{}}
    subject = json.loads(Path({refined:?}).read_text())
    subject["producer_session_id"] = identity.get("provider_session_id")
    rec = book.get(req.get("reservation_id"))
    if rec is not None:
        rec["state"] = "refined"
        book[req["reservation_id"]] = rec
        book_path.write_text(json.dumps(book))
    json.dump({{"schema_version":"cass_admission_command/v1","ok":True,"reservation_id":req.get("reservation_id"),"state":"refined","subject":subject}}, sys.stdout)
elif op == "release":
    release_failure = book_path.with_suffix(".release-failed")
    if os.environ.get("SWIMMERS_CASS_TEST_RELEASE_FAIL_FIRST") and not release_failure.exists():
        release_failure.write_text("failed")
        json.dump({{"schema_version":"cass_admission_command/v1","ok":False,"error":{{"code":"reservation_failed","message":"forced first release failure"}}}}, sys.stdout)
        sys.exit(2)
    rec = book.get(req.get("reservation_id"))
    if rec is not None:
        rec["state"] = "released"
        rec["reason"] = "pre_provider_failure"
        book[req["reservation_id"]] = rec
        book_path.write_text(json.dumps(book))
    json.dump({{"schema_version":"cass_admission_command/v1","ok":True,"reservation_id":req.get("reservation_id"),"state":"released"}}, sys.stdout)
elif op == "reconcile":
    rec = book.get(req.get("reservation_id"))
    if rec is not None:
        rec["state"] = "unresolved"
        rec["reason"] = req.get("cause")
        book[req["reservation_id"]] = rec
        book_path.write_text(json.dumps(book))
    json.dump({{"schema_version":"cass_admission_command/v1","ok":True,"reservation_id":req.get("reservation_id"),"state":"unresolved"}}, sys.stdout)
else:
    json.dump({{"schema_version":"cass_admission_command/v1","ok":False,"error":{{"code":"document_invalid","message":"unknown op"}}}}, sys.stdout)
    sys.exit(2)
"#,
        book = book,
        log = log,
        fail = if fail_reserve { "True" } else { "False" },
        refined = refined,
    );
    write_executable(&script, &contents);
    let cmd = serde_json::json!([script.to_string_lossy()]).to_string();
    let pairs = [
        ("SKILLBOX_CASS_ADMISSION_CMD_JSON", cmd),
        ("SKILLBOX_CASS_ADMISSION_TIMEOUT_MS", "2000".to_string()),
    ];
    let previous = pairs
        .iter()
        .map(|(key, value)| {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            (*key, previous)
        })
        .collect();
    CassEnv {
        _dir: dir,
        log,
        previous,
    }
}

fn reset_cass_side_effect_counters() {
    crate::session::supervisor::reset_cass_admission_test_hooks();
    crate::session::supervisor::set_cass_post_refinement_failure(None);
    let _ = crate::session::supervisor::take_tmux_spawn_calls();
    let _ = crate::api::remote_sessions::take_remote_batch_posts();
}

#[test]
fn cass_admission_intent_rejects_provider_session_uuid() {
    let err = crate::types::CassAdmissionIntent::from_json_str(&cass_fixture(
        "cass_admission_intent_v1_reject_session.json",
    ))
    .expect_err("intent must refuse a provider session UUID");
    assert_eq!(err.code, "document_invalid");
    assert!(err
        .message
        .contains("must not carry a provider session UUID"));

    let mut extra: serde_json::Value =
        serde_json::from_str(&cass_fixture("cass_admission_intent_v1.json")).expect("intent JSON");
    extra["unexpected_claim"] = serde_json::json!("not allowed");
    assert_eq!(
        crate::types::CassAdmissionIntent::from_value(&extra)
            .expect_err("extra intent claim")
            .code,
        "document_invalid"
    );

    let mut ambiguous: serde_json::Value =
        serde_json::from_str(&cass_fixture("cass_admission_intent_v1.json")).expect("intent JSON");
    ambiguous["publisher_locality"] = serde_json::json!("remote");
    assert_eq!(
        crate::types::CassAdmissionIntent::from_value(&ambiguous)
            .expect_err("ambiguous locality")
            .code,
        "document_invalid"
    );
}

#[test]
fn cass_provider_identity_fixtures_are_strict() {
    let local = crate::types::CassProviderIdentity::from_json_str(&cass_fixture(
        "cass_provider_identity_v1_local.json",
    ))
    .expect("local identity");
    assert_eq!(local.origin, crate::types::CassOrigin::Local);
    let remote = crate::types::CassProviderIdentity::from_json_str(&cass_fixture(
        "cass_provider_identity_v1_remote.json",
    ))
    .expect("remote identity");
    assert_eq!(remote.origin, crate::types::CassOrigin::Remote);
    assert_eq!(
        crate::types::CassProviderIdentity::from_json_str(&cass_fixture(
            "cass_provider_identity_v1_reject_secret.json",
        ))
        .expect_err("secret")
        .code,
        "secret_shaped_key"
    );
    assert_eq!(
        crate::types::CassProviderIdentity::from_json_str(&cass_fixture(
            "cass_provider_identity_v1_reject_extra.json",
        ))
        .expect_err("extra")
        .code,
        "document_invalid"
    );
    let reservation = crate::types::CassAdmissionReservationEnvelope::from_value(
        &serde_json::from_str(&cass_fixture("cass_admission_intent_v1_reservation.json")).unwrap(),
    )
    .expect("reservation envelope");
    assert!(reservation.reservation_id.starts_with("rsv_"));
    let subject = crate::types::CassAdmissionSubject::from_value(
        &serde_json::from_str(&cass_fixture("cass_provider_identity_v1_refined.json")).unwrap(),
    )
    .expect("refined subject");
    assert_eq!(
        subject.schema_version,
        crate::types::CASS_ADMISSION_SUBJECT_SCHEMA
    );
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_admission_preflight_failed_reserve_has_zero_provider_tmux_remote_side_effects() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(true);
    std::env::set_var(
        "SWIMMERS_CASS_TEST_ERROR_MESSAGE",
        "provider-private-message-must-not-reach-http",
    );
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);

    let response = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![cass_intent()],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = response_json(response).await;
    assert_eq!(response["code"], "CASS_RESERVATION_FAILED");
    assert_eq!(
        response["message"],
        "Cass reservation was rejected; run admission preflight again"
    );
    assert!(!response
        .to_string()
        .contains("provider-private-message-must-not-reach-http"));
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    assert_eq!(crate::api::remote_sessions::take_remote_batch_posts(), 0);

    let batch = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: None,
            cass_reservations: Vec::new(),
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    assert_eq!(batch.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    assert_eq!(crate::api::remote_sessions::take_remote_batch_posts(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_unbound_reserve_result_is_reconciled_before_preflight_returns() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cass = install_cass_admission_command(false);
    std::env::set_var("SWIMMERS_CASS_TEST_RESERVE_UNBOUND", "1");
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);

    let response = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![cass_intent()],
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = response_json(response).await;
    assert_eq!(response["code"], "CASS_ADMISSION_UNRESOLVED");
    assert_eq!(
        cass.requests()
            .iter()
            .map(|request| request["op"].as_str().expect("op"))
            .collect::<Vec<_>>(),
        vec!["reserve", "reconcile"]
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_canceled_preflight_reconciles_reservations_already_created() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cass = install_cass_admission_command(false);
    std::env::set_var("SWIMMERS_CASS_TEST_RESERVE_DELAY_AFTER_FIRST", "1");
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
    let first_intent = cass_intent();
    let mut second_intent = first_intent.clone();
    second_intent.batch_index = 1;

    let task = tokio::spawn(super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![first_intent, second_intent],
        }),
    ));

    for _ in 0..100 {
        if cass
            .requests()
            .iter()
            .filter(|request| request["op"] == "reserve")
            .count()
            >= 2
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        cass.requests()
            .iter()
            .filter(|request| request["op"] == "reserve")
            .count(),
        2,
        "second reserve must be in flight before cancellation"
    );
    task.abort();
    let _ = task.await;

    for _ in 0..100 {
        if cass
            .requests()
            .iter()
            .any(|request| request["op"] == "reconcile")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(cass
        .requests()
        .iter()
        .any(|request| request["op"] == "reconcile"));
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_non_enforce_release_attempts_every_attached_reservation_after_failure() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cass = install_cass_admission_command(false);
    std::env::set_var("SWIMMERS_CASS_TEST_RELEASE_FAIL_FIRST", "1");
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
    let batch_id = "018f0e11-7c3a-7000-8000-0000000000aa";
    let reservations = (0..2)
        .map(|index| crate::types::CassAdmissionReservationRef {
            reservation_id: format!("rsv_release_all_{index}"),
            batch_id: batch_id.to_string(),
            batch_index: index,
            index: Some(index),
            target_id: Some("local".to_string()),
        })
        .collect();

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(batch_id.to_string()),
            cass_reservations: reservations,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Shadow),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        cass.requests()
            .iter()
            .filter(|request| request["op"] == "release")
            .count(),
        2
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass mode reads.
#[tokio::test]
async fn cass_enforce_requires_preflight_target_binding_before_any_side_effect() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let batch_id = "018f0e11-7c3a-7000-8000-0000000000aa";

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(batch_id.to_string()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: "rsv_missing_target_binding".to_string(),
                batch_id: batch_id.to_string(),
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::api::remote_sessions::take_remote_batch_posts(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_disabled_and_shadow_preflight_never_create_live_reservations() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);

    for mode in [
        crate::types::CassAdmissionMode::Disabled,
        crate::types::CassAdmissionMode::Shadow,
    ] {
        reset_cass_side_effect_counters();
        let mut intent = cass_intent();
        intent.batch_id = uuid::Uuid::new_v4().to_string();
        let response = super::super::core_routes::admit_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(crate::types::CassAdmissionPreflightRequest {
                dirs: dirs.clone(),
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                launch_target: None,
                cass_admission_mode: Some(mode),
                intents: vec![intent],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response_json(response).await["reservations"]
            .as_array()
            .expect("reservations")
            .is_empty());
        assert!(crate::session::supervisor::take_cass_command_ops().is_empty());
    }
}

#[test]
fn cass_request_cannot_downgrade_target_enforce_mode() {
    assert_eq!(
        crate::types::strongest_cass_admission_mode(
            crate::types::CassAdmissionMode::Enforce,
            Some(crate::types::CassAdmissionMode::Disabled),
        ),
        crate::types::CassAdmissionMode::Enforce
    );
}

#[tokio::test]
async fn cass_enforce_mode_blocks_single_session_route_without_preflight() {
    reset_cass_side_effect_counters();
    let response = super::super::core_routes::create_session_with_cass_mode(
        &AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        &test_state(),
        crate::types::CreateSessionRequest {
            name: None,
            cwd: None,
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        },
        crate::types::CassAdmissionMode::Enforce,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "CASS_RESERVATION_FAILED");
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    assert!(crate::session::supervisor::take_cass_command_ops().is_empty());
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_non_enforce_batch_releases_attached_reservation_before_legacy_spawn() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: None,
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    reset_cass_side_effect_counters();

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state.clone(),
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Disabled),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let json = response_json(response).await;
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(
        crate::session::supervisor::take_cass_command_ops(),
        vec!["release"]
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 1);
    cleanup_created_sessions(&state, &json).await;
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_provider_unavailable_releases_and_never_spawns_unadmitted_session() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Claude),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    reset_cass_side_effect_counters();

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Claude),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let json = response_json(response).await;
    assert_eq!(json["results"][0]["ok"], false);
    assert_eq!(
        json["results"][0]["error"]["message"],
        "Cass admission is unavailable on the selected target"
    );
    assert_eq!(
        crate::session::supervisor::take_cass_command_ops(),
        vec!["consume", "release"]
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_provider_without_preallocated_uuid_removes_private_prompt_before_returning() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let mut intent = cass_intent();
    intent.batch_id = uuid::Uuid::new_v4().to_string();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Grok),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    reset_cass_side_effect_counters();
    let prompt_marker = format!("cass-private-prompt-{}", uuid::Uuid::new_v4());

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Grok),
            tmux_target: None,
            launch_target: None,
            initial_request: Some(prompt_marker.clone()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;

    let json = response_json(response).await;
    assert_eq!(json["results"][0]["ok"], false);
    assert_eq!(
        crate::session::supervisor::take_cass_command_ops(),
        vec!["consume", "release"]
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    let prompt_dir = std::env::temp_dir().join("provider_launch_prompts");
    let marker_survived = std::fs::read_dir(prompt_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .any(|contents| contents.contains(&prompt_marker));
    assert!(!marker_survived, "rejected provider prompt must be removed");
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_http_cancellation_does_not_cancel_inflight_settlement() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let mut intent = cass_intent();
    intent.batch_id = uuid::Uuid::new_v4().to_string();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Claude),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    std::env::set_var("SWIMMERS_CASS_TEST_CONSUME_DELAY_MS", "200");
    reset_cass_side_effect_counters();

    let task = tokio::spawn(super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Claude),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    ));

    for _ in 0..100 {
        if cass
            .requests()
            .iter()
            .any(|request| request["op"] == "consume")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(cass
        .requests()
        .iter()
        .any(|request| request["op"] == "consume"));
    task.abort();
    let _ = task.await;

    for _ in 0..100 {
        if cass
            .requests()
            .iter()
            .any(|request| request["op"] == "release")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let requests = cass.requests();
    assert!(requests.iter().any(|request| request["op"] == "release"));
    assert!(!requests.iter().any(|request| request["op"] == "reconcile"));
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_unbound_consume_result_reconciles_before_any_provider_or_tmux_side_effect() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    reset_cass_side_effect_counters();
    std::env::set_var("SWIMMERS_CASS_TEST_CONSUME_UNBOUND", "1");
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let json = response_json(response).await;
    assert_eq!(json["results"][0]["ok"], false);
    assert_eq!(
        crate::session::supervisor::take_cass_command_ops(),
        vec!["consume", "reconcile"]
    );
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_pre_provider_validation_failure_releases_before_returning() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let missing = root.path().join("does-not-exist").display().to_string();
    let intent = cass_intent();
    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: vec![missing.clone()],
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    let preflight = response_json(preflight).await;
    reset_cass_side_effect_counters();
    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs: vec![missing],
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id: preflight["reservations"][0]["reservation_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                batch_id: intent.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let json = response_json(response).await;
    assert_eq!(json["results"][0]["ok"], false);
    assert_eq!(
        crate::session::supervisor::take_cass_command_ops(),
        vec!["release"]
    );
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_admission_wrong_replayed_foreign_reservation_fails_without_side_effects() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();

    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    let preflight_json = response_json(preflight).await;
    let reservation_id = preflight_json["reservations"][0]["reservation_id"]
        .as_str()
        .expect("reservation_id")
        .to_string();

    let foreign = crate::types::CassAdmissionReservationRef {
        reservation_id: "rsv_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_string(),
        batch_id: intent.batch_id.clone(),
        batch_index: 0,
        index: Some(0),
        target_id: Some("local".to_string()),
    };
    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state.clone(),
        CreateSessionsBatchRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![foreign],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    assert_ne!(response.status(), StatusCode::CREATED);
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);

    let good = crate::types::CassAdmissionReservationRef {
        reservation_id: reservation_id.clone(),
        batch_id: intent.batch_id.clone(),
        batch_index: 0,
        index: Some(0),
        target_id: Some("local".to_string()),
    };
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );
    let first = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state.clone(),
        CreateSessionsBatchRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![good.clone()],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let first_json = response_json(first).await;
    cleanup_created_sessions(&state, &first_json).await;

    reset_cass_side_effect_counters();
    let replay = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(intent.batch_id.clone()),
            cass_reservations: vec![good],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    assert_ne!(replay.status(), StatusCode::CREATED);
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_admission_refine_after_uuid_before_session_success() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );

    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    let preflight_json = response_json(preflight).await;
    let batch_id = intent.batch_id.clone();
    let reservation = crate::types::CassAdmissionReservationRef {
        reservation_id: preflight_json["reservations"][0]["reservation_id"]
            .as_str()
            .unwrap()
            .to_string(),
        batch_id: intent.batch_id,
        batch_index: 0,
        index: Some(0),
        target_id: Some("local".to_string()),
    };

    let response = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state.clone(),
        CreateSessionsBatchRequest {
            dirs,
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(batch_id),
            cass_reservations: vec![reservation],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    let json = response_json(response).await;
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        1
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 1);
    let identity = crate::session::supervisor::take_last_cass_provider_identity()
        .expect("identity emitted after UUID");
    assert_eq!(
        identity.provider_session_id,
        "018f0e11-7c3a-7000-8000-000000000001"
    );
    assert_eq!(
        identity.swimmers_session_id,
        json["results"][0]["session"]["session_id"]
            .as_str()
            .expect("created Swimmers session id")
    );
    let ops = crate::session::supervisor::take_cass_command_ops();
    let refine_at = ops.iter().position(|op| op == "refine").expect("refine");
    assert!(
        ops[..refine_at].iter().any(|op| op == "consume"),
        "consume must precede refine: {ops:?}"
    );
    assert!(
        json["results"][0]["ok"].as_bool() == Some(true)
            || json["results"][0]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("exact cleanup")),
        "refine must complete after UUID and before session success: {json}"
    );
    cleanup_created_sessions(&state, &json).await;
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_admission_mixed_and_rebound_batch_ids_fail_before_side_effects() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "web".to_string()]);
    let first = cass_intent();
    let mut second = cass_intent();
    second.batch_id = uuid::Uuid::new_v4().to_string();
    second.batch_index = 1;

    let mixed_preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![first.clone(), second.clone()],
        }),
    )
    .await;
    assert_eq!(mixed_preflight.status(), StatusCode::BAD_REQUEST);
    assert!(crate::session::supervisor::take_cass_command_ops().is_empty());

    let reservations = vec![
        crate::types::CassAdmissionReservationRef {
            reservation_id: "rsv_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            batch_id: first.batch_id.clone(),
            batch_index: 0,
            index: Some(0),
            target_id: Some("local".to_string()),
        },
        crate::types::CassAdmissionReservationRef {
            reservation_id: "rsv_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            batch_id: second.batch_id,
            batch_index: 1,
            index: Some(1),
            target_id: Some("local".to_string()),
        },
    ];
    let mixed_create = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state.clone(),
        CreateSessionsBatchRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(first.batch_id.clone()),
            cass_reservations: reservations,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    assert_eq!(mixed_create.status(), StatusCode::BAD_REQUEST);

    let rebound_batch_id = uuid::Uuid::new_v4().to_string();
    let rebound_create = super::super::core_routes::create_sessions_batch_with_cass(
        AuthInfo::new(OPERATOR_SCOPES.to_vec()),
        state,
        CreateSessionsBatchRequest {
            dirs: vec![dirs[0].clone()],
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            tmux_target: None,
            launch_target: None,
            initial_request: Some("continue".to_string()),
        },
        crate::types::CassBatchAdmissionAttachment {
            cass_batch_id: Some(rebound_batch_id),
            cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                reservation_id:
                    "rsv_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                batch_id: first.batch_id,
                batch_index: 0,
                index: Some(0),
                target_id: Some("local".to_string()),
            }],
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            cass_preflight_target_id: Some("local".to_string()),
        },
    )
    .await;
    assert_eq!(rebound_create.status(), StatusCode::BAD_REQUEST);
    assert!(crate::session::supervisor::take_cass_command_ops().is_empty());
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
    assert_eq!(crate::api::remote_sessions::take_remote_batch_posts(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_admission_interleaved_batches_keep_canonical_identity_isolated() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["alpha".to_string(), "beta".to_string()]);
    let intent_a = cass_intent();
    let mut intent_b = cass_intent();
    intent_b.batch_id = uuid::Uuid::new_v4().to_string();
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );

    let admit = |dir: String, intent: crate::types::CassAdmissionIntent| {
        super::super::core_routes::admit_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(crate::types::CassAdmissionPreflightRequest {
                dirs: vec![dir],
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                launch_target: None,
                cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
                intents: vec![intent],
            }),
        )
    };
    let preflight_a = response_json(admit(dirs[0].clone(), intent_a.clone()).await).await;
    let preflight_b = response_json(admit(dirs[1].clone(), intent_b.clone()).await).await;
    assert_eq!(preflight_a["batch_id"], intent_a.batch_id);
    assert_eq!(preflight_b["batch_id"], intent_b.batch_id);

    let attachment = |preflight: &serde_json::Value| crate::types::CassBatchAdmissionAttachment {
        cass_batch_id: Some(preflight["batch_id"].as_str().unwrap().to_string()),
        cass_reservations: vec![crate::types::CassAdmissionReservationRef {
            reservation_id: preflight["reservations"][0]["reservation_id"]
                .as_str()
                .unwrap()
                .to_string(),
            batch_id: preflight["batch_id"].as_str().unwrap().to_string(),
            batch_index: 0,
            index: Some(0),
            target_id: Some("local".to_string()),
        }],
        cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
        cass_preflight_target_id: Some("local".to_string()),
    };
    let create = |dir: String, cass| {
        super::super::core_routes::create_sessions_batch_with_cass(
            AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            state.clone(),
            CreateSessionsBatchRequest {
                dirs: vec![dir],
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                tmux_target: None,
                launch_target: None,
                initial_request: Some("continue".to_string()),
            },
            cass,
        )
    };
    reset_cass_side_effect_counters();
    let (response_a, response_b) = tokio::join!(
        create(dirs[0].clone(), attachment(&preflight_a)),
        create(dirs[1].clone(), attachment(&preflight_b)),
    );
    let json_a = response_json(response_a).await;
    let json_b = response_json(response_b).await;
    assert_eq!(
        json_a["results"][0]["session"]["batch"]["id"],
        intent_a.batch_id
    );
    assert_eq!(
        json_b["results"][0]["session"]["batch"]["id"],
        intent_b.batch_id
    );

    let refine_batch_ids = cass
        .requests()
        .into_iter()
        .filter(|request| request["op"] == "refine")
        .map(|request| {
            request["identity"]["batch_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        refine_batch_ids,
        [intent_a.batch_id.clone(), intent_b.batch_id.clone()]
            .into_iter()
            .collect()
    );
    let ops = crate::session::supervisor::take_cass_command_ops();
    assert_eq!(ops.iter().filter(|op| op.as_str() == "consume").count(), 2);
    assert_eq!(ops.iter().filter(|op| op.as_str() == "refine").count(), 2);
    assert!(!ops.iter().any(|op| op == "release" || op == "reconcile"));
    cleanup_created_sessions(&state, &json_a).await;
    cleanup_created_sessions(&state, &json_b).await;
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_post_refinement_faults_reconcile_exactly_once() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );

    for point in [
        "tmux_spawn_failure",
        "cleanup_authority_failure",
        "provider_start_failure",
        "provider_receipt_failure",
        "provider_receipt_persistence_failure",
    ] {
        reset_cass_side_effect_counters();
        crate::session::supervisor::set_cass_post_refinement_failure(Some(point));
        let dirs = create_case_dirs(root.path(), 0, &[point.to_string()]);
        let mut intent = cass_intent();
        intent.batch_id = uuid::Uuid::new_v4().to_string();
        let preflight = super::super::core_routes::admit_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(crate::types::CassAdmissionPreflightRequest {
                dirs: dirs.clone(),
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                launch_target: None,
                cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
                intents: vec![intent.clone()],
            }),
        )
        .await;
        let preflight = response_json(preflight).await;
        let response = super::super::core_routes::create_sessions_batch_with_cass(
            AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            state.clone(),
            CreateSessionsBatchRequest {
                dirs,
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                tmux_target: None,
                launch_target: None,
                initial_request: Some("continue".to_string()),
            },
            crate::types::CassBatchAdmissionAttachment {
                cass_batch_id: Some(intent.batch_id.clone()),
                cass_reservations: vec![crate::types::CassAdmissionReservationRef {
                    reservation_id: preflight["reservations"][0]["reservation_id"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                    batch_id: intent.batch_id,
                    batch_index: 0,
                    index: Some(0),
                    target_id: Some("local".to_string()),
                }],
                cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
                cass_preflight_target_id: Some("local".to_string()),
            },
        )
        .await;
        let json = response_json(response).await;
        assert_eq!(json["results"][0]["ok"], false, "fault point {point}");
        let ops = crate::session::supervisor::take_cass_command_ops();
        assert_eq!(
            ops.iter().filter(|op| op.as_str() == "consume").count(),
            1,
            "{point}: {ops:?}"
        );
        assert_eq!(
            ops.iter().filter(|op| op.as_str() == "refine").count(),
            1,
            "{point}: {ops:?}"
        );
        assert_eq!(
            ops.iter().filter(|op| op.as_str() == "reconcile").count(),
            1,
            "{point}: {ops:?}"
        );
        assert!(!ops.iter().any(|op| op == "release"), "{point}: {ops:?}");
    }
    crate::session::supervisor::set_cass_post_refinement_failure(None);
}

#[tokio::test]
async fn cass_admission_observer_cannot_preflight() {
    let response = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: vec!["/tmp/project".to_string()],
            spawn_tool: None,
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![cass_intent()],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_http_enforce_env_dirs_only_refuses_missing_reservation_ids() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();

    let response = super::super::core_routes::create_sessions_batch_http(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(crate::types::CassAwareCreateSessionsBatchRequest {
            request: CreateSessionsBatchRequest {
                dirs: vec!["/tmp/project".to_string()],
                spawn_tool: None,
                tmux_target: None,
                launch_target: None,
                initial_request: None,
            },
            cass: crate::types::CassBatchAdmissionAttachment {
                cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
                ..Default::default()
            },
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "CASS_RESERVATION_FAILED");
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        0
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 0);
}

#[allow(clippy::await_holding_lock)] // Serializes process-wide Cass command fixture env.
#[tokio::test]
async fn cass_http_enforce_consumes_reservation_body() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _cass = install_cass_admission_command(false);
    reset_cass_side_effect_counters();
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string()]);
    let intent = cass_intent();
    std::env::set_var(
        "SWIMMERS_CASS_TEST_PROVIDER_UUID",
        "018f0e11-7c3a-7000-8000-000000000001",
    );

    let preflight = super::super::core_routes::admit_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAdmissionPreflightRequest {
            dirs: dirs.clone(),
            spawn_tool: Some(crate::types::SpawnTool::Codex),
            launch_target: None,
            cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
            intents: vec![intent.clone()],
        }),
    )
    .await;
    assert_eq!(preflight.status(), StatusCode::OK);
    let preflight_json = response_json(preflight).await;
    let batch_id = intent.batch_id.clone();
    let reservation = crate::types::CassAdmissionReservationRef {
        reservation_id: preflight_json["reservations"][0]["reservation_id"]
            .as_str()
            .unwrap()
            .to_string(),
        batch_id: intent.batch_id,
        batch_index: 0,
        index: Some(0),
        target_id: Some("local".to_string()),
    };

    let response = super::super::core_routes::create_sessions_batch_http(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(crate::types::CassAwareCreateSessionsBatchRequest {
            request: CreateSessionsBatchRequest {
                dirs,
                spawn_tool: Some(crate::types::SpawnTool::Codex),
                tmux_target: None,
                launch_target: None,
                initial_request: Some("continue".to_string()),
            },
            cass: crate::types::CassBatchAdmissionAttachment {
                cass_batch_id: Some(batch_id),
                cass_reservations: vec![reservation],
                cass_admission_mode: Some(crate::types::CassAdmissionMode::Enforce),
                cass_preflight_target_id: Some("local".to_string()),
            },
        }),
    )
    .await;
    let json = response_json(response).await;
    assert_ne!(json["code"], "CASS_RESERVATION_FAILED");
    assert_eq!(
        crate::session::supervisor::take_codex_app_server_launch_calls(),
        1
    );
    assert_eq!(crate::session::supervisor::take_tmux_spawn_calls(), 1);
    cleanup_created_sessions(&state, &json).await;
}
