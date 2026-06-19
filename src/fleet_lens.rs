use std::collections::BTreeMap;

use crate::session_labels::{repo_label_for_key, session_repo_key};
use crate::types::{
    ActionCueKind, AdvisoryMetadataSummary, DependencyHealthStatus, EnvironmentCapabilitySummary,
    EnvironmentSummary, FleetLensBucket, FleetLensBucketKind, FleetLensPreset,
    FleetLensPresetMatcher, FleetLensSummary, RestState, SessionEnvironmentScope, SessionState,
    SessionSummary, TransportHealth,
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

        for advisory in &session.environment.advisory {
            insert_advisory_bucket(&mut buckets, advisory);
        }

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

pub fn build_fleet_lens_presets(overlay_presets: Vec<FleetLensPreset>) -> Vec<FleetLensPreset> {
    let mut presets = built_in_fleet_lens_presets();
    let mut seen = presets
        .iter()
        .map(|preset| preset.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for preset in overlay_presets
        .into_iter()
        .filter_map(sanitize_overlay_preset)
    {
        if seen.insert(preset.id.clone()) {
            presets.push(preset);
        }
    }
    presets
}

fn built_in_fleet_lens_presets() -> Vec<FleetLensPreset> {
    vec![
        preset(
            "all",
            "All environments",
            "builtin",
            vec![FleetLensPresetMatcher::All],
        ),
        preset(
            "local",
            "Local",
            "builtin",
            vec![FleetLensPresetMatcher::TargetId {
                id: "local".to_string(),
            }],
        ),
        preset(
            "remote-api",
            "Remote API",
            "builtin",
            vec![FleetLensPresetMatcher::TargetKind {
                kind: "swimmers_api".to_string(),
            }],
        ),
        preset(
            "ssh-handoff",
            "SSH handoff",
            "builtin",
            vec![FleetLensPresetMatcher::TargetKind {
                kind: "ssh_only".to_string(),
            }],
        ),
        preset(
            "current-repo",
            "Current repo",
            "builtin",
            vec![FleetLensPresetMatcher::CurrentRepo],
        ),
        preset(
            "needs-attention",
            "Needs attention",
            "builtin",
            vec![FleetLensPresetMatcher::NeedsAttention],
        ),
        preset(
            "degraded",
            "Degraded",
            "builtin",
            vec![FleetLensPresetMatcher::Degraded],
        ),
    ]
}

fn preset(
    id: &str,
    label: &str,
    source: &str,
    matchers: Vec<FleetLensPresetMatcher>,
) -> FleetLensPreset {
    FleetLensPreset {
        id: id.to_string(),
        label: label.to_string(),
        source: source.to_string(),
        matchers,
    }
}

fn sanitize_overlay_preset(mut preset: FleetLensPreset) -> Option<FleetLensPreset> {
    preset.id = normalize_preset_id(&preset.id)?;
    preset.label = sanitize_preset_label(&preset.label)?;
    preset.source = "overlay".to_string();
    preset.matchers = preset
        .matchers
        .into_iter()
        .filter_map(sanitize_preset_matcher)
        .collect();
    (!preset.matchers.is_empty()).then_some(preset)
}

fn normalize_preset_id(raw: &str) -> Option<String> {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    (!normalized.is_empty()).then_some(normalized)
}

fn sanitize_preset_label(raw: &str) -> Option<String> {
    let label = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!label.is_empty()).then(|| label.chars().take(64).collect())
}

fn sanitize_preset_matcher(matcher: FleetLensPresetMatcher) -> Option<FleetLensPresetMatcher> {
    match matcher {
        FleetLensPresetMatcher::All => Some(FleetLensPresetMatcher::All),
        FleetLensPresetMatcher::FleetBucket { kind, key } => {
            nonempty_preset_value(&key).map(|key| FleetLensPresetMatcher::FleetBucket { kind, key })
        }
        FleetLensPresetMatcher::TargetId { id } => {
            nonempty_preset_value(&id).map(|id| FleetLensPresetMatcher::TargetId { id })
        }
        FleetLensPresetMatcher::TargetKind { kind } => {
            nonempty_preset_value(&kind).map(|kind| FleetLensPresetMatcher::TargetKind { kind })
        }
        FleetLensPresetMatcher::Repo { key } => {
            nonempty_preset_value(&key).map(|key| FleetLensPresetMatcher::Repo { key })
        }
        FleetLensPresetMatcher::CurrentRepo => Some(FleetLensPresetMatcher::CurrentRepo),
        FleetLensPresetMatcher::Readiness { key } => {
            nonempty_preset_value(&key).map(|key| FleetLensPresetMatcher::Readiness { key })
        }
        FleetLensPresetMatcher::Transport { key } => {
            nonempty_preset_value(&key).map(|key| FleetLensPresetMatcher::Transport { key })
        }
        FleetLensPresetMatcher::Capability { key } => {
            nonempty_preset_value(&key).map(|key| FleetLensPresetMatcher::Capability { key })
        }
        FleetLensPresetMatcher::Degraded => Some(FleetLensPresetMatcher::Degraded),
        FleetLensPresetMatcher::NeedsAttention => Some(FleetLensPresetMatcher::NeedsAttention),
    }
}

fn nonempty_preset_value(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub fn fleet_preset_matches_session(
    preset: &FleetLensPreset,
    session: &SessionSummary,
    current_repo_key: Option<&str>,
) -> bool {
    preset
        .matchers
        .iter()
        .all(|matcher| preset_matcher_matches_session(matcher, session, current_repo_key))
}

fn preset_matcher_matches_session(
    matcher: &FleetLensPresetMatcher,
    session: &SessionSummary,
    current_repo_key: Option<&str>,
) -> bool {
    match matcher {
        FleetLensPresetMatcher::All => true,
        FleetLensPresetMatcher::FleetBucket { kind, key } => {
            session_matches_bucket(session, *kind, key)
        }
        FleetLensPresetMatcher::TargetId { id } => target_key(session) == *id,
        FleetLensPresetMatcher::TargetKind { kind } => {
            normalized_eq(&session.environment.target_kind, kind)
        }
        FleetLensPresetMatcher::Repo { key } => session_repo_key(session) == *key,
        FleetLensPresetMatcher::CurrentRepo => {
            current_repo_key.is_some_and(|key| session_repo_key(session) == key)
        }
        FleetLensPresetMatcher::Readiness { key } => readiness_key(session) == key,
        FleetLensPresetMatcher::Transport { key } => transport_key(session.transport_health) == key,
        FleetLensPresetMatcher::Capability { key } => session_capability_summary(session)
            .is_some_and(|capabilities| capability_enabled(&capabilities, key)),
        FleetLensPresetMatcher::Degraded => session_is_degraded(session),
        FleetLensPresetMatcher::NeedsAttention => session_needs_attention(session),
    }
}

pub fn fleet_preset_matches_environment(
    preset: &FleetLensPreset,
    environment: &EnvironmentSummary,
    current_repo_key: Option<&str>,
) -> bool {
    preset
        .matchers
        .iter()
        .all(|matcher| preset_matcher_matches_environment(matcher, environment, current_repo_key))
}

fn preset_matcher_matches_environment(
    matcher: &FleetLensPresetMatcher,
    environment: &EnvironmentSummary,
    current_repo_key: Option<&str>,
) -> bool {
    match matcher {
        FleetLensPresetMatcher::All => true,
        FleetLensPresetMatcher::FleetBucket { kind, key } => match kind {
            FleetLensBucketKind::Target => environment_target_key(environment) == *key,
            _ => false,
        },
        FleetLensPresetMatcher::TargetId { id } => environment_target_key(environment) == *id,
        FleetLensPresetMatcher::TargetKind { kind } => normalized_eq(&environment.kind, kind),
        FleetLensPresetMatcher::Repo { .. } | FleetLensPresetMatcher::CurrentRepo => {
            current_repo_key.is_none()
        }
        FleetLensPresetMatcher::Readiness { .. } | FleetLensPresetMatcher::Transport { .. } => {
            false
        }
        FleetLensPresetMatcher::Capability { key } => {
            capability_enabled(&environment.capabilities, key)
        }
        FleetLensPresetMatcher::Degraded => matches!(
            environment.status,
            DependencyHealthStatus::Degraded
                | DependencyHealthStatus::Unavailable
                | DependencyHealthStatus::Unknown
        ),
        FleetLensPresetMatcher::NeedsAttention => false,
    }
}

fn session_matches_bucket(session: &SessionSummary, kind: FleetLensBucketKind, key: &str) -> bool {
    match kind {
        FleetLensBucketKind::Target => target_key(session) == key,
        FleetLensBucketKind::Repo => session_repo_key(session) == key,
        FleetLensBucketKind::Advisory => session
            .environment
            .advisory
            .iter()
            .filter_map(advisory_key)
            .any(|advisory_key| advisory_key == key),
        FleetLensBucketKind::State => session_state_key(session.state) == key,
        FleetLensBucketKind::Readiness => readiness_key(session) == key,
        FleetLensBucketKind::Transport => transport_key(session.transport_health) == key,
    }
}

fn session_capability_summary(session: &SessionSummary) -> Option<EnvironmentCapabilitySummary> {
    match session.environment.scope {
        SessionEnvironmentScope::Local => Some(EnvironmentCapabilitySummary::local()),
        SessionEnvironmentScope::Remote => match session.environment.target_kind.as_str() {
            "swimmers_api" => Some(EnvironmentCapabilitySummary::remote_swimmers_api(
                session.transport_health == TransportHealth::Healthy,
                session.environment.local_cwd.is_some() || session.environment.remote_cwd.is_some(),
                false,
            )),
            "ssh_only" => Some(EnvironmentCapabilitySummary::ssh_handoff(false)),
            _ => None,
        },
    }
}

fn capability_enabled(capabilities: &EnvironmentCapabilitySummary, key: &str) -> bool {
    match key.trim().to_ascii_lowercase().as_str() {
        "observe" | "observe_sessions" => capabilities.observe_sessions,
        "launch" | "launch_session" => capabilities.launch_session,
        "send" | "send_input" => capabilities.send_input,
        "group" | "group_input" => capabilities.group_input,
        "dirs" | "remote_dir_inventory" => capabilities.remote_dir_inventory,
        "native_attach" => capabilities.native_attach,
        "ssh" | "ssh_attach_hint" => capabilities.ssh_attach_hint,
        "bootstrap" | "bootstrap_hint" => capabilities.bootstrap_hint,
        "external" | "advisory" | "advisory_metadata" => capabilities.advisory_metadata,
        "health" | "health_probe" => capabilities.health_probe,
        _ => false,
    }
}

fn environment_target_key(environment: &EnvironmentSummary) -> String {
    first_non_empty(&[
        environment.id.as_str(),
        environment.display_host.as_str(),
        environment.label.as_str(),
    ])
    .unwrap_or_default()
    .to_string()
}

fn normalized_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
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

pub fn advisory_key(advisory: &AdvisoryMetadataSummary) -> Option<String> {
    advisory
        .group_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let source = advisory.source.trim();
            let label = advisory.label.trim();
            let value = advisory.value.trim();
            (!source.is_empty() && !label.is_empty() && !value.is_empty()).then(|| {
                [source, label, value]
                    .into_iter()
                    .map(|part| part.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(":")
            })
        })
}

pub(crate) fn advisory_label(advisory: &AdvisoryMetadataSummary) -> Option<String> {
    let label = advisory.label.trim();
    let value = advisory.value.trim();
    if label.is_empty() || value.is_empty() {
        return None;
    }
    Some(format!("{label}: {value}"))
}

fn insert_advisory_bucket(
    buckets: &mut BTreeMap<(FleetLensBucketKind, String), FleetLensBucket>,
    advisory: &AdvisoryMetadataSummary,
) {
    let Some(key) = advisory_key(advisory) else {
        return;
    };
    let Some(label) = advisory_label(advisory) else {
        return;
    };
    let bucket = buckets
        .entry((FleetLensBucketKind::Advisory, key.clone()))
        .or_insert_with(|| FleetLensBucket {
            kind: FleetLensBucketKind::Advisory,
            key,
            label,
            count: 0,
            degraded_count: 0,
            stale_count: 0,
            attention_count: 0,
            commit_ready_count: 0,
        });
    bucket.count += 1;
    if advisory.stale {
        bucket.stale_count += 1;
        bucket.degraded_count += 1;
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
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "remote",
            remote.cwd.clone(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        remote.transport_health = TransportHealth::Degraded;
        remote.environment.advisory = vec![AdvisoryMetadataSummary {
            source: "c0".to_string(),
            label: "c0 group".to_string(),
            value: "swimmers".to_string(),
            status: "external".to_string(),
            stale: true,
            group_key: Some("c0:swimmers".to_string()),
            observed_at: None,
            freshness_ms: None,
        }];

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

        let advisory = summary
            .buckets
            .iter()
            .find(|bucket| {
                bucket.kind == FleetLensBucketKind::Advisory && bucket.key == "c0:swimmers"
            })
            .expect("advisory bucket");
        assert_eq!(advisory.label, "c0 group: swimmers");
        assert_eq!(advisory.count, 1);
        assert_eq!(advisory.stale_count, 1);
        assert_eq!(advisory.degraded_count, 1);
    }
}
