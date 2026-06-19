use super::*;

#[test]
fn api_error_banner_shows_after_three_failures_and_clears_on_success() {
    let mut health = ApiRefreshHealth::default();
    assert!(health.banner_text().is_none());

    health.record_failure();
    assert!(health.banner_text().is_none());
    health.record_failure();
    assert!(health.banner_text().is_none());
    health.record_failure();
    assert_eq!(health.banner_text(), Some(API_STALE_BANNER_TEXT));

    health.record_success();
    assert!(health.banner_text().is_none());
}

#[test]
fn finish_voice_recording_plan_defers_when_interaction_is_pending() {
    assert_eq!(
        plan_finish_voice_recording(true, 42),
        FinishVoiceRecordingPlan::WaitForPendingInteraction
    );
}

#[test]
fn finish_voice_recording_plan_captures_generation_for_transcription() {
    assert_eq!(
        plan_finish_voice_recording(false, 42),
        FinishVoiceRecordingPlan::StartTranscription { generation: 42 }
    );
}

#[test]
fn toggle_voice_recording_plan_requires_open_composer_first() {
    assert_eq!(
        plan_toggle_voice_recording(false, true, true),
        ToggleVoiceRecordingPlan::ShowMessage("open an initial request first")
    );
}

#[test]
fn toggle_voice_recording_plan_waits_for_active_transcription() {
    assert_eq!(
        plan_toggle_voice_recording(true, true, true),
        ToggleVoiceRecordingPlan::ShowMessage("wait for voice transcription to finish")
    );
}

#[test]
fn toggle_voice_recording_plan_finishes_existing_recording() {
    assert_eq!(
        plan_toggle_voice_recording(true, false, true),
        ToggleVoiceRecordingPlan::FinishRecording
    );
}

#[test]
fn toggle_voice_recording_plan_starts_when_ready_and_not_recording() {
    assert_eq!(
        plan_toggle_voice_recording(true, false, false),
        ToggleVoiceRecordingPlan::StartRecording
    );
}

#[test]
fn remote_inventory_target_omits_case_insensitive_local_target() {
    assert_eq!(remote_inventory_target(None), None);
    assert_eq!(remote_inventory_target(Some("  ")), None);
    assert_eq!(remote_inventory_target(Some("local")), None);
    assert_eq!(remote_inventory_target(Some(" LOCAL ")), None);
    assert_eq!(
        remote_inventory_target(Some("skillbox")),
        Some("skillbox".to_string())
    );
}

fn healthy_backend_health() -> BackendHealthResponse {
    BackendHealthResponse {
        status: "healthy".to_string(),
        thought_bridge: BackendThoughtBridgeHealth {
            status: "healthy".to_string(),
            ..BackendThoughtBridgeHealth::default()
        },
        persistence: BackendPersistenceHealth {
            available: true,
            ok: true,
            last_successful_operation: Some("save_sessions".to_string()),
            ..BackendPersistenceHealth::default()
        },
        dependencies: Some(healthy_dependency_ledger()),
    }
}

fn healthy_dependency_ledger() -> BackendDependencyLedger {
    BackendDependencyLedger {
        tmux_discovery: BackendDependencySnapshot {
            status: "healthy".to_string(),
            last_error: None,
        },
        tmux_capture: BackendDependencySnapshot {
            status: "healthy".to_string(),
            last_error: None,
        },
        native_scripts: BackendDependencySnapshot {
            status: "healthy".to_string(),
            last_error: None,
        },
        remote_targets: BackendDependencySnapshot {
            status: "not_configured".to_string(),
            last_error: None,
        },
    }
}

#[test]
fn backend_health_banner_covers_persistence_degraded_and_recovered() {
    let mut health = healthy_backend_health();
    assert!(backend_health_warning_text(&health).is_none());

    health.persistence.ok = false;
    health.persistence.consecutive_failures = 2;
    health.persistence.last_failed_operation = Some("save_thought".to_string());
    health.persistence.last_error = Some("permission denied".to_string());
    assert_eq!(
        backend_health_warning_text(&health),
        Some("persistence degraded: save_thought: permission denied".to_string())
    );

    health.persistence.ok = true;
    health.persistence.consecutive_failures = 0;
    health.persistence.last_error = None;
    assert!(backend_health_warning_text(&health).is_none());
}

#[test]
fn backend_health_banner_covers_thought_degraded_and_unhealthy() {
    let mut health = healthy_backend_health();
    health.thought_bridge.status = "degraded".to_string();
    health.thought_bridge.last_backend_error = Some("model timeout".to_string());
    assert_eq!(
        backend_health_warning_text(&health),
        Some("thought bridge degraded: model timeout".to_string())
    );

    health.thought_bridge.status = "unhealthy".to_string();
    health.thought_bridge.last_backend_error = None;
    health.thought_bridge.shutdown_reason = Some("self fence tripped".to_string());
    assert_eq!(
        backend_health_warning_text(&health),
        Some("thought bridge unhealthy: self fence tripped".to_string())
    );

    health.thought_bridge.status = "healthy".to_string();
    health.thought_bridge.shutdown_reason = None;
    assert!(backend_health_warning_text(&health).is_none());
}

#[test]
fn dependency_degradation_line_reports_degraded_and_unavailable() {
    let mut ledger = healthy_dependency_ledger();
    assert!(dependency_degradation_line(&ledger).is_none());

    ledger.native_scripts.status = "unavailable".to_string();
    ledger.native_scripts.last_error = Some("script not found".to_string());
    assert_eq!(
        dependency_degradation_line(&ledger),
        Some("native scripts unavailable: script not found".to_string())
    );

    ledger.remote_targets.status = "degraded".to_string();
    ledger.remote_targets.last_error = Some("timeout".to_string());
    let line = dependency_degradation_line(&ledger).unwrap();
    assert!(line.contains("native scripts unavailable"));
    assert!(line.contains("remote targets degraded: timeout"));
}

#[test]
fn dependency_degradation_line_ignores_healthy_and_not_configured() {
    let mut ledger = healthy_dependency_ledger();
    ledger.remote_targets.status = "not_configured".to_string();
    assert!(dependency_degradation_line(&ledger).is_none());
}

#[test]
fn tmux_unavailable_detected_from_dependency_ledger() {
    let mut health = healthy_backend_health();
    assert!(!tmux_unavailable_from_health(health.dependencies.as_ref()));

    health.dependencies.as_mut().unwrap().tmux_discovery.status = "unavailable".to_string();
    assert!(tmux_unavailable_from_health(health.dependencies.as_ref()));
}

fn tmux_unavailable_from_health(deps: Option<&BackendDependencyLedger>) -> bool {
    deps.map(|d| d.tmux_discovery.status == "unavailable")
        .unwrap_or(false)
}

fn app_test_session(session_id: &str, tmux_name: &str) -> SessionSummary {
    SessionSummary::placeholder(session_id, tmux_name, Utc::now())
}

#[test]
fn merge_session_entities_reuses_existing_entity_state_and_updates_summary() {
    let field = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let mut existing = SessionEntity::new(app_test_session("s1", "swimmers-1"), field);
    existing.x = 12.0;
    existing.y = 5.0;
    existing.vx = -0.25;
    existing.vy = 0.75;
    existing.swim_anchor_x = 13.0;
    existing.swim_anchor_y = 6.0;
    existing.swim_center_y = 7.0;
    existing.bob_phase = 1.25;

    let merged = merge_session_entities(
        vec![existing],
        vec![app_test_session("s1", "swimmers-01-renamed")],
        field,
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].session.tmux_name, "swimmers-01-renamed");
    assert_eq!(merged[0].x, 12.0);
    assert_eq!(merged[0].y, 5.0);
    assert_eq!(merged[0].vx, -0.25);
    assert_eq!(merged[0].vy, 0.75);
    assert_eq!(merged[0].swim_anchor_x, 13.0);
    assert_eq!(merged[0].swim_anchor_y, 6.0);
    assert_eq!(merged[0].swim_center_y, 7.0);
    assert_eq!(merged[0].bob_phase, 1.25);
}

#[test]
fn merge_session_entities_drops_absent_sessions_and_sorts_refresh_result() {
    let field = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let removed = SessionEntity::new(app_test_session("old", "swimmers-1"), field);

    let merged = merge_session_entities(
        vec![removed],
        vec![
            app_test_session("s10", "swimmers-10"),
            app_test_session("s2", "swimmers-2"),
        ],
        field,
    );

    let ids = merged
        .iter()
        .map(|entity| entity.session.session_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["s2", "s10"]);
}

#[test]
fn empty_aquarium_message_preserves_priority_text_and_color() {
    assert_eq!(
        empty_aquarium_message(true, true, true),
        ("no swimmers match filters", Color::DarkGrey)
    );
    assert_eq!(
        empty_aquarium_message(false, false, true),
        (
            "tmux unavailable - run swimmers config doctor",
            Color::Yellow,
        )
    );
    assert_eq!(
        empty_aquarium_message(false, true, false),
        (
            "no tmux sessions found - press r after starting one",
            Color::DarkGrey,
        )
    );
}

#[test]
fn empty_aquarium_position_preserves_centering_math() {
    let field = Rect {
        x: 3,
        y: 4,
        width: 20,
        height: 6,
    };
    assert_eq!(
        centered_empty_aquarium_position(field, "1234567890"),
        (8, 7)
    );
    assert_eq!(
        centered_empty_aquarium_position(Rect { width: 4, ..field }, "1234567890",),
        (3, 7)
    );
}
