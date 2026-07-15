use super::*;

#[test]
fn state_evidence_deserializes_partial_payload_with_mapped_confidence() {
    let evidence: StateEvidence = serde_json::from_str(r#"{"cause":"osc133_prompt"}"#).unwrap();

    assert_eq!(evidence.cause, "osc133_prompt");
    assert_eq!(evidence.confidence, StateConfidence::High);
    assert!(evidence.observed_at.is_none());
}

#[test]
fn unobserved_state_evidence_omits_freshness() {
    let evidence = StateEvidence::unobserved(SUMMARY_CAUSE_PERSISTENCE_STALE);

    assert_eq!(evidence.cause, SUMMARY_CAUSE_PERSISTENCE_STALE);
    assert_eq!(evidence.confidence, StateConfidence::Low);
    assert!(evidence.observed_at.is_none());
}

#[test]
fn session_summary_placeholder_uses_shared_lifecycle_contract() {
    let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let summary = SessionSummary::placeholder("sess_1", "work", at);

    assert_eq!(summary.session_id, "sess_1");
    assert_eq!(summary.tmux_name, "work");
    assert_eq!(summary.state, SessionState::Idle);
    assert_eq!(
        summary.state_evidence.cause,
        SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER
    );
    assert!(summary.state_evidence.observed_at.is_none());
    assert_eq!(summary.transport_health, TransportHealth::Healthy);
    assert!(!summary.is_stale);
    assert_eq!(summary.rest_state, RestState::Drowsy);
    assert_eq!(summary.context_limit, 128_000);
}

#[test]
fn session_summary_fallback_reason_parity_is_shared() {
    let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let summary = SessionSummary::live(
        "sess_1",
        "work",
        SessionState::Busy,
        Some("cargo test".to_string()),
        StateEvidence::new("osc133_command"),
        "/tmp/swimmers",
        Some("Codex".to_string()),
        1,
        2,
        at,
    );

    let dropped = summary
        .clone()
        .into_cached_collection_fallback(SummaryFallbackReason::Dropped);
    assert_eq!(SummaryFallbackReason::Dropped.metric_label(), "dropped");
    assert_eq!(dropped.state_evidence.cause, SUMMARY_CAUSE_CACHE_DEGRADED);
    assert_eq!(dropped.transport_health, TransportHealth::Degraded);
    assert!(!dropped.is_stale);

    let timeout = summary.into_cached_collection_fallback(SummaryFallbackReason::Timeout);
    assert_eq!(SummaryFallbackReason::Timeout.metric_label(), "timeout");
    assert_eq!(timeout.state_evidence.cause, SUMMARY_CAUSE_CACHE_OVERLOADED);
    assert_eq!(timeout.transport_health, TransportHealth::Overloaded);
}

#[test]
fn session_summary_stale_and_remote_helpers_preserve_wire_shape() {
    let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let live = SessionSummary::placeholder("sess_1", "work", at);

    let missing = live
        .clone()
        .into_missing_tmux_stale(SUMMARY_CAUSE_TMUX_RECONCILE_MISSING);
    assert_eq!(missing.state, SessionState::Exited);
    assert_eq!(
        missing.state_evidence.cause,
        SUMMARY_CAUSE_TMUX_RECONCILE_MISSING
    );
    assert!(missing.is_stale);
    assert_eq!(missing.transport_health, TransportHealth::Disconnected);
    assert_eq!(missing.rest_state, RestState::DeepSleep);

    let remote = live.into_remote_poll_degraded(Some(at));
    assert_eq!(
        remote.state_evidence.cause,
        SUMMARY_CAUSE_REMOTE_POLL_DEGRADED
    );
    assert_eq!(remote.state, SessionState::Idle);
    assert!(remote.is_stale);
    assert_eq!(remote.transport_health, TransportHealth::Degraded);

    let json = serde_json::to_value(&remote).expect("serialize session summary");
    assert_eq!(json["session_id"], "sess_1");
    assert_eq!(json["tmux_name"], "work");
    assert_eq!(json["is_stale"], true);
    assert_eq!(json["transport_health"], "degraded");
    assert!(
        json.get("fallback_reason").is_none(),
        "helper must not introduce a new wire field"
    );
}

#[test]
fn detect_tool_name_normalizes_aliases_and_paths() {
    assert_eq!(detect_tool_name("claude"), Some("Claude Code"));
    assert_eq!(detect_tool_name("CLAUDE"), Some("Claude Code"));
    assert_eq!(
        detect_tool_name("/usr/local/bin/claude-code"),
        Some("Claude Code")
    );
    assert_eq!(detect_tool_name("codex-cli"), Some("Codex"));
    assert_eq!(detect_tool_name("'codex'"), Some("Codex"));
    assert_eq!(detect_tool_name("grok-cli"), Some("Grok"));
}

#[test]
fn detect_tool_name_preserves_known_aliases() {
    for (alias, name) in TOOL_NAME_ALIASES {
        assert_eq!(detect_tool_name(alias), Some(*name), "alias {alias}");
    }
}

#[test]
fn detect_tool_name_preserves_token_normalization_order() {
    assert_eq!(detect_tool_name("'codex' --model gpt-5"), Some("Codex"));
    assert_eq!(
        detect_tool_name("/usr/local/bin/-claude"),
        Some("Claude Code")
    );
    assert_eq!(detect_tool_name("(/opt/bin/grok-cli)"), Some("Grok"));
}

#[test]
fn detect_tool_name_ignores_unknown_tokens() {
    assert_eq!(detect_tool_name("zsh"), None);
    assert_eq!(detect_tool_name("node"), None);
    assert_eq!(detect_tool_name(""), None);
}

#[test]
fn dependency_health_snapshot_serializes_stable_shape() {
    let checked_at = DateTime::parse_from_rfc3339("2026-05-30T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let seen_at = DateTime::parse_from_rfc3339("2026-05-29T23:59:59Z")
        .unwrap()
        .with_timezone(&Utc);
    let health = DependencyHealthSnapshot::healthy(checked_at)
        .with_last_seen(seen_at)
        .with_detail("dependency", "tmux_discovery");

    let json = serde_json::to_value(&health).expect("serialize health");

    assert_eq!(json["status"], "healthy");
    assert_eq!(json["last_checked_at"], "2026-05-30T00:00:00Z");
    assert_eq!(json["last_seen_at"], "2026-05-29T23:59:59Z");
    assert_eq!(json["last_error_at"], serde_json::Value::Null);
    assert_eq!(json["last_error"], serde_json::Value::Null);
    assert_eq!(json["freshness_ms"], 1000);
    assert_eq!(json["details"]["dependency"], "tmux_discovery");
}

#[test]
fn context_limit_falls_back_for_unknown_tool() {
    assert_eq!(context_limit_for_tool(None), 128_000);
    assert_eq!(context_limit_for_tool(Some("UnknownTool")), 128_000);
}

#[test]
fn spawn_tool_commands_match_cli_entrypoints() {
    assert_eq!(SpawnTool::Claude.command(), "claude");
    assert_eq!(SpawnTool::Codex.command(), "codex");
    assert_eq!(SpawnTool::Grok.command(), "grok");
}

#[test]
fn attention_group_layout_accepts_tmux_names_and_operator_aliases() {
    assert_eq!(
        AttentionGroupLayout::from_env_value("even-horizontal"),
        AttentionGroupLayout::EvenHorizontal
    );
    assert_eq!(
        AttentionGroupLayout::from_env_value("columns"),
        AttentionGroupLayout::EvenHorizontal
    );
    assert_eq!(
        AttentionGroupLayout::from_env_value("stacked"),
        AttentionGroupLayout::EvenVertical
    );
    assert_eq!(
        AttentionGroupLayout::from_env_value("main_left"),
        AttentionGroupLayout::MainVertical
    );
    assert_eq!(
        AttentionGroupLayout::from_env_value("unknown"),
        AttentionGroupLayout::Tiled
    );
}

#[test]
fn create_session_response_serializes_repo_theme() {
    let theme = RepoTheme {
        body: "#B89875".into(),
        outline: "#3D2F24".into(),
        accent: "#1D1914".into(),
        shirt: "#AA9370".into(),
        sprite: Some("jelly".into()),
    };
    let resp = CreateSessionResponse {
        session: Some(SessionSummary {
            session_id: "s1".into(),
            tmux_name: "1".into(),
            tmux_target: crate::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            current_command: None,
            state_evidence: Default::default(),
            cwd: "/tmp/proj".into(),
            tool: None,
            token_count: 0,
            context_limit: 200_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: chrono::Utc::now(),
            repo_theme_id: Some("/tmp/proj".into()),
            batch: None,
            environment: Default::default(),
        }),
        repo_theme: Some(theme),
        launch_receipt: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(
        json.contains("\"repo_theme\""),
        "repo_theme must be present in JSON"
    );
    assert!(
        json.contains("\"sprite\":\"jelly\""),
        "repo_theme sprite must roundtrip when present"
    );

    // When repo_theme is None, the field should be omitted entirely.
    let resp_none = CreateSessionResponse {
        session: resp.session,
        repo_theme: None,
        launch_receipt: None,
    };
    let json_none = serde_json::to_string(&resp_none).unwrap();
    assert!(
        !json_none.contains("\"repo_theme\""),
        "null repo_theme must be omitted"
    );
}

#[test]
fn session_created_payload_serializes_repo_theme() {
    let theme = RepoTheme {
        body: "#B89875".into(),
        outline: "#3D2F24".into(),
        accent: "#1D1914".into(),
        shirt: "#AA9370".into(),
        sprite: Some("balls".into()),
    };
    let payload = SessionCreatedPayload {
        reason: "api_create".into(),
        session: SessionSummary {
            session_id: "s1".into(),
            tmux_name: "1".into(),
            tmux_target: crate::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            current_command: None,
            state_evidence: Default::default(),
            cwd: "/tmp".into(),
            tool: None,
            token_count: 0,
            context_limit: 200_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: chrono::Utc::now(),
            repo_theme_id: Some("/tmp".into()),
            batch: None,
            environment: Default::default(),
        },
        repo_theme: Some(theme),
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("\"repo_theme\""));

    // Deserialize roundtrip
    let parsed: SessionCreatedPayload = serde_json::from_str(&json).unwrap();
    let parsed_theme = parsed.repo_theme.expect("repo theme");
    assert_eq!(parsed_theme.body, "#B89875");
    assert_eq!(parsed_theme.sprite.as_deref(), Some("balls"));
}
