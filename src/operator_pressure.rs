use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::remote_sessions;
use crate::types::{
    ActionCueKind, RestState, SessionBatchMembership, SessionState, SessionSummary,
    StateConfidence, TransportHealth,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorPressureReasonKind {
    AwaitingUser,
    CommitReady,
    ValidationMissingAfterEdit,
    DirtyCheckMissing,
    NeedsInput,
    Error,
    Sleeping,
    UntrustedState,
    Stale,
    Transport,
    Busy,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorPressureTone {
    Quiet,
    Working,
    Warning,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPressure {
    pub score: u8,
    pub reason: String,
    pub reason_kind: OperatorPressureReasonKind,
    pub glyph: String,
    pub tone: OperatorPressureTone,
    pub needs_input: bool,
    pub launch_ready: bool,
    pub commit_ready: bool,
    pub action_cue_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPressureSession {
    pub session_id: String,
    pub repo_key: String,
    pub repo_label: String,
    pub pressure: OperatorPressure,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batch_send_session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPressureRepo {
    pub repo_key: String,
    pub repo_label: String,
    pub score: u8,
    pub reason: String,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPressureSummary {
    pub max_score: u8,
    pub action_cues: usize,
    pub batch_send_groups: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorPressureResponse {
    pub sessions: Vec<OperatorPressureSession>,
    pub repos: Vec<OperatorPressureRepo>,
    pub summary: OperatorPressureSummary,
}

pub fn operator_pressure_for_session(session: &SessionSummary) -> OperatorPressure {
    let primary_cue = primary_action_cue_kind(session);
    let needs_input = has_action_cue(session, ActionCueKind::AwaitingUser)
        || session.rest_state == RestState::Sleeping
        || session.state == SessionState::Attention;
    let commit_ready =
        has_action_cue(session, ActionCueKind::CommitReady) || session.commit_candidate;

    let mut score: u16 = 0;
    if has_action_cue(session, ActionCueKind::AwaitingUser) {
        score += 55;
    }
    if has_action_cue(session, ActionCueKind::CommitReady) {
        score += 45;
    }
    if has_action_cue(session, ActionCueKind::ValidationMissingAfterEdit) {
        score += 40;
    }
    if has_action_cue(session, ActionCueKind::DirtyCheckMissing) {
        score += 35;
    }
    if session.state == SessionState::Attention {
        score += 45;
    }
    if session.state == SessionState::Busy {
        score += 12;
    }
    if session.state == SessionState::Error {
        score += 55;
    }
    if session.rest_state == RestState::Sleeping {
        score += 35;
    }
    if session.rest_state == RestState::DeepSleep {
        score += 20;
    }
    if session.commit_candidate {
        score += 25;
    }
    if state_evidence_is_unverified(session) {
        score += 15;
    }
    if session.is_stale {
        score += 10;
    }
    if session.transport_health != TransportHealth::Healthy {
        score += 20;
    }

    let reason_kind = pressure_reason_kind(session, primary_cue);
    let score = score.clamp(1, 99) as u8;
    OperatorPressure {
        score,
        reason: pressure_reason_label(reason_kind).to_string(),
        reason_kind,
        glyph: pressure_glyph(session, primary_cue).to_string(),
        tone: pressure_tone(session, score, primary_cue),
        needs_input,
        launch_ready: needs_input,
        commit_ready,
        action_cue_count: session.action_cues.len(),
    }
}

pub fn build_operator_pressure_response(sessions: &[SessionSummary]) -> OperatorPressureResponse {
    let batch_send_map = batch_send_session_map(sessions);
    let mut summary = OperatorPressureSummary {
        max_score: 0,
        action_cues: 0,
        batch_send_groups: batch_send_map
            .values()
            .map(|ids| ids.join("\0"))
            .collect::<std::collections::HashSet<_>>()
            .len(),
    };

    let mut repo_map: HashMap<String, OperatorPressureRepo> = HashMap::new();
    let mut pressure_sessions = Vec::with_capacity(sessions.len());
    for session in sessions {
        let pressure = operator_pressure_for_session(session);
        let repo_key = repo_key(session);
        let repo_label = repo_label(&repo_key);
        summary.max_score = summary.max_score.max(pressure.score);
        summary.action_cues += pressure.action_cue_count;

        let repo = repo_map
            .entry(repo_key.clone())
            .or_insert_with(|| OperatorPressureRepo {
                repo_key: repo_key.clone(),
                repo_label: repo_label.clone(),
                score: 0,
                reason: "quiet".to_string(),
                session_ids: Vec::new(),
            });
        repo.session_ids.push(session.session_id.clone());
        if pressure.score > repo.score {
            repo.score = pressure.score;
            repo.reason = pressure.reason.clone();
        }

        pressure_sessions.push(OperatorPressureSession {
            session_id: session.session_id.clone(),
            repo_key,
            repo_label,
            pressure,
            batch_send_session_ids: batch_send_map
                .get(&session.session_id)
                .cloned()
                .unwrap_or_default(),
        });
    }

    let mut repos = repo_map.into_values().collect::<Vec<_>>();
    repos.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.repo_label.cmp(&right.repo_label))
    });
    pressure_sessions.sort_by(|left, right| {
        right
            .pressure
            .score
            .cmp(&left.pressure.score)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });

    OperatorPressureResponse {
        sessions: pressure_sessions,
        repos,
        summary,
    }
}

pub fn session_ready_for_operator_group_input(session: &SessionSummary) -> bool {
    !session.is_stale
        && session.transport_health == TransportHealth::Healthy
        && session.state_evidence.observed_at.is_some()
        && session.state != SessionState::Exited
        && session.rest_state != RestState::DeepSleep
        && (session.rest_state == RestState::Sleeping || session.state == SessionState::Attention)
}

fn primary_action_cue_kind(session: &SessionSummary) -> Option<ActionCueKind> {
    [
        ActionCueKind::AwaitingUser,
        ActionCueKind::CommitReady,
        ActionCueKind::ValidationMissingAfterEdit,
        ActionCueKind::DirtyCheckMissing,
    ]
    .into_iter()
    .find(|kind| has_action_cue(session, *kind))
}

fn has_action_cue(session: &SessionSummary, kind: ActionCueKind) -> bool {
    session.action_cues.iter().any(|cue| cue.kind == kind)
}

fn state_evidence_is_unverified(session: &SessionSummary) -> bool {
    session.state_evidence.observed_at.is_none()
        || session.state_evidence.confidence != StateConfidence::High
}

fn pressure_reason_kind(
    session: &SessionSummary,
    primary_cue: Option<ActionCueKind>,
) -> OperatorPressureReasonKind {
    match primary_cue {
        Some(ActionCueKind::AwaitingUser) => OperatorPressureReasonKind::AwaitingUser,
        Some(ActionCueKind::CommitReady) => OperatorPressureReasonKind::CommitReady,
        Some(ActionCueKind::ValidationMissingAfterEdit) => {
            OperatorPressureReasonKind::ValidationMissingAfterEdit
        }
        Some(ActionCueKind::DirtyCheckMissing) => OperatorPressureReasonKind::DirtyCheckMissing,
        None if session.state == SessionState::Attention => OperatorPressureReasonKind::NeedsInput,
        None if session.state == SessionState::Error => OperatorPressureReasonKind::Error,
        None if session.commit_candidate => OperatorPressureReasonKind::CommitReady,
        None if session.rest_state == RestState::Sleeping => OperatorPressureReasonKind::Sleeping,
        None if state_evidence_is_unverified(session) => OperatorPressureReasonKind::UntrustedState,
        None if session.is_stale => OperatorPressureReasonKind::Stale,
        None if session.transport_health != TransportHealth::Healthy => {
            OperatorPressureReasonKind::Transport
        }
        None if session.state == SessionState::Busy => OperatorPressureReasonKind::Busy,
        None => OperatorPressureReasonKind::Idle,
    }
}

fn pressure_reason_label(kind: OperatorPressureReasonKind) -> &'static str {
    match kind {
        OperatorPressureReasonKind::AwaitingUser => "awaiting user",
        OperatorPressureReasonKind::CommitReady => "commit ready",
        OperatorPressureReasonKind::ValidationMissingAfterEdit => "validate",
        OperatorPressureReasonKind::DirtyCheckMissing => "dirty check",
        OperatorPressureReasonKind::NeedsInput => "needs input",
        OperatorPressureReasonKind::Error => "error",
        OperatorPressureReasonKind::Sleeping => "sleeping",
        OperatorPressureReasonKind::UntrustedState => "untrusted",
        OperatorPressureReasonKind::Stale => "stale",
        OperatorPressureReasonKind::Transport => "transport",
        OperatorPressureReasonKind::Busy => "busy",
        OperatorPressureReasonKind::Idle => "idle",
    }
}

fn pressure_glyph(session: &SessionSummary, primary_cue: Option<ActionCueKind>) -> &'static str {
    match primary_cue {
        Some(ActionCueKind::AwaitingUser) => "!",
        Some(ActionCueKind::CommitReady) => "$",
        Some(ActionCueKind::ValidationMissingAfterEdit) => "v",
        Some(ActionCueKind::DirtyCheckMissing) => "d",
        None if session.state == SessionState::Attention => "!",
        None if session.state == SessionState::Error => "x",
        None if session.commit_candidate => "$",
        None => "a",
    }
}

fn pressure_tone(
    session: &SessionSummary,
    score: u8,
    primary_cue: Option<ActionCueKind>,
) -> OperatorPressureTone {
    if primary_cue.is_some() || session.state == SessionState::Error || score >= 70 {
        OperatorPressureTone::Danger
    } else if score >= 35 || session.commit_candidate || session.rest_state == RestState::Sleeping {
        OperatorPressureTone::Warning
    } else if session.state == SessionState::Busy {
        OperatorPressureTone::Working
    } else {
        OperatorPressureTone::Quiet
    }
}

fn repo_key(session: &SessionSummary) -> String {
    let cwd = session.cwd.trim();
    if cwd.is_empty() {
        session.tmux_name.clone()
    } else {
        cwd.to_string()
    }
}

fn repo_label(repo_key: &str) -> String {
    repo_key
        .split('/')
        .rfind(|part| !part.is_empty())
        .unwrap_or(repo_key)
        .to_string()
}

fn batch_send_session_map(sessions: &[SessionSummary]) -> HashMap<String, Vec<String>> {
    let mut batches: HashMap<String, Vec<(&SessionBatchMembership, &SessionSummary)>> =
        HashMap::new();
    for session in sessions {
        let Some(batch) = session.batch.as_ref() else {
            continue;
        };
        if remote_sessions::split_remote_session_id(&session.session_id).is_some() {
            continue;
        }
        if session_ready_for_operator_group_input(session) {
            batches
                .entry(batch.id.clone())
                .or_default()
                .push((batch, session));
        }
    }

    let mut map = HashMap::new();
    for mut sessions in batches.into_values() {
        sessions.sort_by_key(|(batch, session)| (batch.index, session.session_id.clone()));
        let ids = sessions
            .iter()
            .map(|(_, session)| session.session_id.clone())
            .collect::<Vec<_>>();
        if ids.len() < 2 {
            continue;
        }
        for id in &ids {
            map.insert(id.clone(), ids.clone());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::types::{
        ActionCue, ActionCueConfidence, ActionCueSource, ActionCueStatus, StateEvidence,
        ThoughtSource, ThoughtState,
    };

    fn cue(kind: ActionCueKind) -> ActionCue {
        ActionCue {
            kind,
            status: ActionCueStatus::Active,
            source: ActionCueSource::Transcript,
            confidence: ActionCueConfidence::Deterministic,
            evidence: ActionCue::expected_evidence(kind)
                .iter()
                .map(|value| value.to_string())
                .collect(),
        }
    }

    fn summary(id: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            tmux_name: id.to_string(),
            state,
            current_command: None,
            state_evidence: StateEvidence::new("test"),
            cwd: "/tmp/repos/swimmers".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 200_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Active,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
        }
    }

    #[test]
    fn action_cues_drive_pressure_without_new_backend_names() {
        let mut session = summary("s1", SessionState::Idle);
        session.action_cues = vec![cue(ActionCueKind::CommitReady)];

        let pressure = operator_pressure_for_session(&session);

        assert_eq!(
            pressure.reason_kind,
            OperatorPressureReasonKind::CommitReady
        );
        assert_eq!(pressure.reason, "commit ready");
        assert_eq!(pressure.glyph, "$");
        assert!(pressure.commit_ready);
    }

    #[test]
    fn awaiting_user_outranks_other_action_cues() {
        let mut session = summary("s1", SessionState::Idle);
        session.action_cues = vec![
            cue(ActionCueKind::DirtyCheckMissing),
            cue(ActionCueKind::AwaitingUser),
        ];

        let pressure = operator_pressure_for_session(&session);

        assert_eq!(
            pressure.reason_kind,
            OperatorPressureReasonKind::AwaitingUser
        );
        assert_eq!(pressure.glyph, "!");
        assert!(pressure.needs_input);
    }

    #[test]
    fn batch_send_ids_follow_existing_group_input_readiness() {
        let created_at = Utc::now();
        let mut left = summary("left", SessionState::Attention);
        let mut right = summary("right", SessionState::Idle);
        right.rest_state = RestState::Sleeping;
        let mut busy = summary("busy", SessionState::Busy);
        for (index, session) in [&mut left, &mut right, &mut busy].into_iter().enumerate() {
            session.batch = Some(SessionBatchMembership {
                id: "batch-a".to_string(),
                label: "batch".to_string(),
                index,
                total: 3,
                created_at,
                prompt_excerpt: None,
            });
        }

        let response = build_operator_pressure_response(&[left, right, busy]);
        let ready = response
            .sessions
            .iter()
            .find(|session| session.session_id == "left")
            .expect("left session");
        let busy = response
            .sessions
            .iter()
            .find(|session| session.session_id == "busy")
            .expect("busy session");

        assert_eq!(
            ready.batch_send_session_ids,
            vec!["left".to_string(), "right".to_string()]
        );
        assert!(busy.batch_send_session_ids.is_empty());
        assert_eq!(response.summary.batch_send_groups, 1);
    }

    #[test]
    fn batch_send_ids_exclude_namespaced_remote_sessions() {
        let created_at = Utc::now();
        let mut local = summary("local", SessionState::Attention);
        let mut remote = summary(
            &remote_sessions::namespace_session_id("jeremy-skillbox", "remote"),
            SessionState::Attention,
        );
        for (index, session) in [&mut local, &mut remote].into_iter().enumerate() {
            session.batch = Some(SessionBatchMembership {
                id: "batch-remote".to_string(),
                label: "batch".to_string(),
                index,
                total: 2,
                created_at,
                prompt_excerpt: None,
            });
        }

        let response = build_operator_pressure_response(&[local, remote]);

        assert_eq!(response.summary.batch_send_groups, 0);
        for session in response.sessions {
            assert!(
                session.batch_send_session_ids.is_empty(),
                "remote-backed group input must not be advertised for {}",
                session.session_id
            );
        }
    }
}
