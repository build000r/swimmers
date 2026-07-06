use super::*;

#[test]
fn thought_snapshot_for_summary_matches_active_tmux_pane() {
    let summary = SessionSummary {
        session_id: "sess_1".to_string(),
        tmux_name: "work".to_string(),
        tmux_target: crate::tmux_target::TmuxTarget::Default,
        state: SessionState::Idle,
        current_command: None,
        state_evidence: Default::default(),
        cwd: "/tmp".to_string(),
        tool: None,
        token_count: 0,
        context_limit: 0,
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
        transport_health: crate::types::TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
        batch: None,
        environment: Default::default(),
    };

    let older = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let newer = DateTime::parse_from_rfc3339("2026-03-08T14:00:06Z")
        .expect("timestamp")
        .with_timezone(&Utc);

    let snapshots = HashMap::from([
        (
            "tmux:work:1.0:%1".to_string(),
            ThoughtSnapshot {
                thought: Some("pane one".to_string()),
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: older,
                delivery: ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 1,
                },
            },
        ),
        (
            "tmux:work:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: newer,
                delivery: ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 2,
                },
            },
        ),
    ]);

    let matched = thought_snapshot_for_summary(&summary, Some("tmux:work:1.1:%2"), &snapshots)
        .expect("tmux pane snapshot");
    assert_eq!(matched.thought.as_deref(), Some("pane two"));
    assert_eq!(matched.delivery.emission_seq, 2);
}

#[test]
fn thought_snapshot_for_summary_does_not_fall_back_to_latest_tmux_pane_without_active_binding() {
    let summary = SessionSummary {
        session_id: "sess_1".to_string(),
        tmux_name: "work".to_string(),
        tmux_target: crate::tmux_target::TmuxTarget::Default,
        state: SessionState::Idle,
        current_command: None,
        state_evidence: Default::default(),
        cwd: "/tmp".to_string(),
        tool: None,
        token_count: 0,
        context_limit: 0,
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
        transport_health: crate::types::TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
        batch: None,
        environment: Default::default(),
    };

    let snapshots = HashMap::from([
        (
            "tmux:work:1.0:%1".to_string(),
            ThoughtSnapshot {
                thought: Some("pane one".to_string()),
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        ),
        (
            "tmux:work:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        ),
    ]);

    assert!(thought_snapshot_for_summary(&summary, None, &snapshots).is_none());
}

#[test]
fn thought_snapshot_for_summary_prefers_direct_snapshot_over_active_tmux_pane() {
    let summary = test_summary("sess_1", SessionState::Idle);
    let snapshots = HashMap::from([
        (
            "sess_1".to_string(),
            test_thought_snapshot("direct session", ThoughtState::Holding),
        ),
        (
            "tmux:tmux-sess_1:1.1:%2".to_string(),
            test_thought_snapshot("active pane", ThoughtState::Active),
        ),
    ]);

    let matched =
        thought_snapshot_for_summary(&summary, Some("tmux:tmux-sess_1:1.1:%2"), &snapshots)
            .expect("direct session snapshot");

    assert_eq!(matched.thought.as_deref(), Some("direct session"));
    assert_eq!(matched.thought_state, ThoughtState::Holding);
}

#[test]
fn active_pane_lookup_not_required_without_thought_snapshots() {
    let summaries = [
        test_summary("sess-live", SessionState::Idle),
        test_summary("sess-busy", SessionState::Busy),
    ];
    let snapshots = HashMap::new();

    let tmux_names = tmux_names_requiring_active_pane_lookup(summaries.iter(), &snapshots);

    assert!(tmux_names.is_empty());
}
