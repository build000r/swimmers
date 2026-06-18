use std::collections::BTreeMap;

use crate::session_labels::{repo_label_for_key, session_repo_key};
use crate::types::{
    ActionCueKind, FleetLensBucket, FleetLensBucketKind, FleetLensSummary, RestState,
    SessionEnvironmentScope, SessionState, SessionSummary, TransportHealth,
};

pub fn build_fleet_lens_summary(sessions: &[SessionSummary]) -> FleetLensSummary {
    let mut buckets = BTreeMap::<(FleetLensBucketKind, String), FleetLensBucket>::new();
    for session in sessions {
        insert_bucket(
            &mut buckets,
            FleetLensBucketKind::Target,
            target_key(session),
            target_label(session),
            session,
        );

        let repo_key = session_repo_key(session);
        let repo_label = repo_label_for_key(&repo_key);
        insert_bucket(
            &mut buckets,
            FleetLensBucketKind::Repo,
            repo_key,
            repo_label,
            session,
        );

        insert_bucket(
            &mut buckets,
            FleetLensBucketKind::State,
            session_state_key(session.state).to_string(),
            session_state_label(session.state).to_string(),
            session,
        );

        insert_bucket(
            &mut buckets,
            FleetLensBucketKind::Readiness,
            readiness_key(session).to_string(),
            readiness_label(session).to_string(),
            session,
        );

        insert_bucket(
            &mut buckets,
            FleetLensBucketKind::Transport,
            transport_key(session.transport_health).to_string(),
            transport_label(session.transport_health).to_string(),
            session,
        );
    }

    let mut buckets = buckets.into_values().collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| right.count.cmp(&left.count))
            .then_with(|| right.degraded_count.cmp(&left.degraded_count))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.key.cmp(&right.key))
    });

    FleetLensSummary {
        total_sessions: sessions.len(),
        buckets,
    }
}

fn insert_bucket(
    buckets: &mut BTreeMap<(FleetLensBucketKind, String), FleetLensBucket>,
    kind: FleetLensBucketKind,
    key: String,
    label: String,
    session: &SessionSummary,
) {
    let bucket = buckets
        .entry((kind, key.clone()))
        .or_insert_with(|| FleetLensBucket {
            kind,
            key,
            label,
            count: 0,
            degraded_count: 0,
            stale_count: 0,
            attention_count: 0,
            commit_ready_count: 0,
        });
    bucket.count += 1;
    if session_is_degraded(session) {
        bucket.degraded_count += 1;
    }
    if session.is_stale {
        bucket.stale_count += 1;
    }
    if session_needs_attention(session) {
        bucket.attention_count += 1;
    }
    if session.commit_candidate {
        bucket.commit_ready_count += 1;
    }
}

pub(crate) fn target_key(session: &SessionSummary) -> String {
    let environment = &session.environment;
    if environment.scope == SessionEnvironmentScope::Remote {
        return first_non_empty(&[
            environment.target_id.as_str(),
            environment.display_host.as_str(),
            environment.target_label.as_str(),
        ])
        .unwrap_or("remote")
        .to_string();
    }
    "local".to_string()
}

pub(crate) fn target_label(session: &SessionSummary) -> String {
    let environment = &session.environment;
    if environment.scope == SessionEnvironmentScope::Remote {
        return first_non_empty(&[
            environment.display_host.as_str(),
            environment.target_label.as_str(),
            environment.target_id.as_str(),
        ])
        .unwrap_or("remote")
        .to_string();
    }
    "local".to_string()
}

fn first_non_empty<'a>(values: &[&'a str]) -> Option<&'a str> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

fn session_state_key(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Busy => "busy",
        SessionState::Error => "error",
        SessionState::Attention => "attention",
        SessionState::Exited => "exited",
    }
}

fn session_state_label(state: SessionState) -> &'static str {
    session_state_key(state)
}

fn readiness_key(session: &SessionSummary) -> &'static str {
    if session_needs_attention(session) {
        "needs_attention"
    } else if session.state == SessionState::Busy {
        "working"
    } else if matches!(
        session.rest_state,
        RestState::Sleeping | RestState::DeepSleep
    ) {
        "sleeping"
    } else {
        "quiet"
    }
}

fn readiness_label(session: &SessionSummary) -> &'static str {
    match readiness_key(session) {
        "needs_attention" => "needs attention",
        "working" => "working",
        "sleeping" => "sleeping",
        _ => "quiet",
    }
}

fn transport_key(health: TransportHealth) -> &'static str {
    match health {
        TransportHealth::Healthy => "healthy",
        TransportHealth::Degraded => "degraded",
        TransportHealth::Overloaded => "overloaded",
        TransportHealth::Disconnected => "disconnected",
    }
}

fn transport_label(health: TransportHealth) -> &'static str {
    transport_key(health)
}

fn session_is_degraded(session: &SessionSummary) -> bool {
    session.is_stale || session.transport_health != TransportHealth::Healthy
}

pub(crate) fn session_needs_attention(session: &SessionSummary) -> bool {
    session.state == SessionState::Attention
        || session.commit_candidate
        || session.action_cues.iter().any(|cue| {
            matches!(
                cue.kind,
                ActionCueKind::AwaitingUser
                    | ActionCueKind::CommitReady
                    | ActionCueKind::ValidationMissingAfterEdit
                    | ActionCueKind::DirtyCheckMissing
            )
        })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::types::{
        ActionCue, ActionCueConfidence, ActionCueSource, ActionCueStatus, LaunchTargetSummary,
        SessionEnvironmentSummary, StateEvidence,
    };

    fn session(id: &str, cwd: &str, state: SessionState) -> SessionSummary {
        SessionSummary::live(
            id,
            id,
            state,
            None,
            StateEvidence::new("test"),
            cwd,
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        )
    }

    fn cue(kind: ActionCueKind) -> ActionCue {
        ActionCue {
            kind,
            status: ActionCueStatus::Active,
            source: ActionCueSource::Transcript,
            confidence: ActionCueConfidence::Deterministic,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn fleet_lens_groups_mapped_remote_and_local_repo_without_hiding_degradation() {
        let mut local = session(
            "local",
            "/Users/b/repos/opensource/swimmers",
            SessionState::Idle,
        );
        local.environment = SessionEnvironmentSummary::local(local.cwd.clone());
        local.action_cues.push(cue(ActionCueKind::AwaitingUser));

        let mut remote = session("remote", "/srv/skillbox/repos/swimmers", SessionState::Busy);
        remote.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                path_mappings: Vec::new(),
            },
            "remote",
            remote.cwd.clone(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        remote.transport_health = TransportHealth::Degraded;

        let summary = build_fleet_lens_summary(&[local, remote]);

        assert_eq!(summary.total_sessions, 2);
        let repo = summary
            .buckets
            .iter()
            .find(|bucket| {
                bucket.kind == FleetLensBucketKind::Repo
                    && bucket.key == "/Users/b/repos/opensource/swimmers"
            })
            .expect("canonical repo bucket");
        assert_eq!(repo.label, "opensource/swimmers");
        assert_eq!(repo.count, 2);
        assert_eq!(repo.degraded_count, 1);
        assert_eq!(repo.attention_count, 1);

        let target_labels = summary
            .buckets
            .iter()
            .filter(|bucket| bucket.kind == FleetLensBucketKind::Target)
            .map(|bucket| bucket.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(target_labels, vec!["Skillbox devbox", "local"]);
    }
}
