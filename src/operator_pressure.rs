use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::remote_sessions;
use crate::types::{
    ActionCueKind, RestState, SessionBatchMembership, SessionState, SessionSummary,
    StateConfidence, ThoughtState, TransportHealth,
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
    let signals = OperatorPressureSignals::collect(session);
    let reason_kind = pressure_reason_kind(session, signals.primary_cue);
    let score = pressure_score(&signals);
    OperatorPressure {
        score,
        reason: pressure_reason_label(reason_kind).to_string(),
        reason_kind,
        glyph: pressure_glyph(session, signals.primary_cue).to_string(),
        tone: pressure_tone(session, score, signals.primary_cue),
        needs_input: signals.needs_input,
        launch_ready: signals.needs_input,
        commit_ready: signals.commit_ready,
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
        && session_has_operator_input_signal(session)
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

#[derive(Debug, Clone, Copy)]
struct OperatorPressureSignals {
    primary_cue: Option<ActionCueKind>,
    needs_input: bool,
    commit_ready: bool,
    awaiting_user_cue: bool,
    commit_ready_cue: bool,
    validation_missing_after_edit_cue: bool,
    dirty_check_missing_cue: bool,
    attention_state: bool,
    busy_state: bool,
    error_state: bool,
    sleeping: bool,
    idle_agent_input_ready: bool,
    deep_sleep: bool,
    commit_candidate: bool,
    unverified_state_evidence: bool,
    stale: bool,
    transport_unhealthy: bool,
}

impl OperatorPressureSignals {
    fn collect(session: &SessionSummary) -> Self {
        let primary_cue = primary_action_cue_kind(session);
        let commit_ready_cue = has_action_cue(session, ActionCueKind::CommitReady);
        let commit_candidate = session.commit_candidate;
        Self {
            primary_cue,
            needs_input: session_has_operator_input_signal(session),
            commit_ready: commit_ready_cue || commit_candidate,
            awaiting_user_cue: has_action_cue(session, ActionCueKind::AwaitingUser),
            commit_ready_cue,
            validation_missing_after_edit_cue: has_action_cue(
                session,
                ActionCueKind::ValidationMissingAfterEdit,
            ),
            dirty_check_missing_cue: has_action_cue(session, ActionCueKind::DirtyCheckMissing),
            attention_state: session.state == SessionState::Attention,
            busy_state: session.state == SessionState::Busy,
            error_state: session.state == SessionState::Error,
            sleeping: session.rest_state == RestState::Sleeping,
            idle_agent_input_ready: session_idle_agent_can_accept_operator_input(session),
            deep_sleep: session.rest_state == RestState::DeepSleep,
            commit_candidate,
            unverified_state_evidence: state_evidence_is_unverified(session),
            stale: session.is_stale,
            transport_unhealthy: session.transport_health != TransportHealth::Healthy,
        }
    }
}

fn pressure_score(signals: &OperatorPressureSignals) -> u8 {
    pressure_score_raw(signals).clamp(1, 99) as u8
}

fn pressure_score_raw(signals: &OperatorPressureSignals) -> u16 {
    pressure_contribution(signals.awaiting_user_cue, 55)
        + pressure_contribution(signals.commit_ready_cue, 45)
        + pressure_contribution(signals.validation_missing_after_edit_cue, 40)
        + pressure_contribution(signals.dirty_check_missing_cue, 35)
        + pressure_contribution(signals.attention_state, 45)
        + pressure_contribution(signals.busy_state, 12)
        + pressure_contribution(signals.error_state, 55)
        + pressure_contribution(signals.sleeping, 35)
        + pressure_contribution(signals.idle_agent_input_ready, 35)
        + pressure_contribution(signals.deep_sleep, 20)
        + pressure_contribution(signals.commit_candidate, 25)
        + pressure_contribution(signals.unverified_state_evidence, 15)
        + pressure_contribution(signals.stale, 10)
        + pressure_contribution(signals.transport_unhealthy, 20)
}

fn pressure_contribution(present: bool, weight: u16) -> u16 {
    if present {
        weight
    } else {
        0
    }
}

fn has_action_cue(session: &SessionSummary, kind: ActionCueKind) -> bool {
    session.action_cues.iter().any(|cue| cue.kind == kind)
}

fn session_has_operator_input_signal(session: &SessionSummary) -> bool {
    has_action_cue(session, ActionCueKind::AwaitingUser)
        || session.rest_state == RestState::Sleeping
        || session.state == SessionState::Attention
        || session_idle_agent_can_accept_operator_input(session)
}

fn session_idle_agent_can_accept_operator_input(session: &SessionSummary) -> bool {
    session.state == SessionState::Idle
        && session.current_command.is_none()
        && session.thought_state != ThoughtState::Active
        && session
            .tool
            .as_deref()
            .is_some_and(agent_tool_accepts_input)
}

fn agent_tool_accepts_input(tool: &str) -> bool {
    matches!(
        tool,
        "Codex" | "Claude Code" | "Amp" | "OpenCode" | "Aider" | "Goose" | "Cline" | "Cursor"
    )
}

fn state_evidence_is_unverified(session: &SessionSummary) -> bool {
    session.state_evidence.observed_at.is_none()
        || session.state_evidence.confidence != StateConfidence::High
}

fn pressure_reason_kind(
    session: &SessionSummary,
    primary_cue: Option<ActionCueKind>,
) -> OperatorPressureReasonKind {
    pressure_reason_kind_for_action_cue(primary_cue)
        .unwrap_or_else(|| pressure_reason_kind_for_session(session))
}

fn pressure_reason_kind_for_action_cue(
    primary_cue: Option<ActionCueKind>,
) -> Option<OperatorPressureReasonKind> {
    match primary_cue {
        Some(ActionCueKind::AwaitingUser) => Some(OperatorPressureReasonKind::AwaitingUser),
        Some(ActionCueKind::CommitReady) => Some(OperatorPressureReasonKind::CommitReady),
        Some(ActionCueKind::ValidationMissingAfterEdit) => {
            Some(OperatorPressureReasonKind::ValidationMissingAfterEdit)
        }
        Some(ActionCueKind::DirtyCheckMissing) => {
            Some(OperatorPressureReasonKind::DirtyCheckMissing)
        }
        None => None,
    }
}

fn pressure_reason_kind_for_session(session: &SessionSummary) -> OperatorPressureReasonKind {
    type PressureReasonClassifier = fn(&SessionSummary) -> Option<OperatorPressureReasonKind>;
    let classifiers: [PressureReasonClassifier; 9] = [
        pressure_reason_kind_for_attention_state,
        pressure_reason_kind_for_error_state,
        pressure_reason_kind_for_commit_candidate,
        pressure_reason_kind_for_sleep_state,
        pressure_reason_kind_for_idle_input,
        pressure_reason_kind_for_state_evidence,
        pressure_reason_kind_for_staleness,
        pressure_reason_kind_for_transport,
        pressure_reason_kind_for_busy_state,
    ];

    classifiers
        .into_iter()
        .find_map(|classify| classify(session))
        .unwrap_or(OperatorPressureReasonKind::Idle)
}

fn pressure_reason_kind_for_attention_state(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    (session.state == SessionState::Attention).then_some(OperatorPressureReasonKind::NeedsInput)
}

fn pressure_reason_kind_for_error_state(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    (session.state == SessionState::Error).then_some(OperatorPressureReasonKind::Error)
}

fn pressure_reason_kind_for_commit_candidate(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    session
        .commit_candidate
        .then_some(OperatorPressureReasonKind::CommitReady)
}

fn pressure_reason_kind_for_sleep_state(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    (session.rest_state == RestState::Sleeping).then_some(OperatorPressureReasonKind::Sleeping)
}

fn pressure_reason_kind_for_idle_input(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    session_idle_agent_can_accept_operator_input(session)
        .then_some(OperatorPressureReasonKind::NeedsInput)
}

fn pressure_reason_kind_for_state_evidence(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    state_evidence_is_unverified(session).then_some(OperatorPressureReasonKind::UntrustedState)
}

fn pressure_reason_kind_for_staleness(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    session
        .is_stale
        .then_some(OperatorPressureReasonKind::Stale)
}

fn pressure_reason_kind_for_transport(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    (session.transport_health != TransportHealth::Healthy)
        .then_some(OperatorPressureReasonKind::Transport)
}

fn pressure_reason_kind_for_busy_state(
    session: &SessionSummary,
) -> Option<OperatorPressureReasonKind> {
    (session.state == SessionState::Busy).then_some(OperatorPressureReasonKind::Busy)
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
        None if session_idle_agent_can_accept_operator_input(session) => "!",
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
    } else if score >= 35
        || session.commit_candidate
        || session.rest_state == RestState::Sleeping
        || session_idle_agent_can_accept_operator_input(session)
    {
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
        ThoughtState,
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
        SessionSummary::live(
            id,
            id,
            state,
            None,
            StateEvidence::new("test"),
            "/tmp/repos/swimmers",
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        )
    }

    fn trusted_summary(id: &str, state: SessionState) -> SessionSummary {
        let mut session = summary(id, state);
        session.state_evidence = StateEvidence::new("osc133_command");
        session
    }

    fn assert_reason_kind(session: SessionSummary, expected: OperatorPressureReasonKind) {
        assert_eq!(
            operator_pressure_for_session(&session).reason_kind,
            expected
        );
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
    fn idle_agent_without_active_thought_snapshot_is_group_input_ready() {
        let session = summary("idle-codex", SessionState::Idle);

        let pressure = operator_pressure_for_session(&session);

        assert!(session_ready_for_operator_group_input(&session));
        assert!(pressure.needs_input);
        assert_eq!(pressure.reason_kind, OperatorPressureReasonKind::NeedsInput);
        assert_eq!(pressure.glyph, "!");
        assert_eq!(pressure.tone, OperatorPressureTone::Warning);
    }

    #[test]
    fn active_thought_snapshot_prevents_idle_agent_group_input_fallback() {
        let mut session = summary("thinking-codex", SessionState::Idle);
        session.thought_state = ThoughtState::Active;

        let pressure = operator_pressure_for_session(&session);

        assert!(!session_ready_for_operator_group_input(&session));
        assert!(!pressure.needs_input);
    }

    #[test]
    fn pressure_reason_fallbacks_cover_session_evidence_and_transport_order() {
        let mut commit_ready = trusted_summary("commit", SessionState::Idle);
        commit_ready.tool = None;
        commit_ready.commit_candidate = true;

        let mut sleeping = trusted_summary("sleeping", SessionState::Idle);
        sleeping.tool = None;
        sleeping.rest_state = RestState::Sleeping;

        let mut idle_input = trusted_summary("idle-input", SessionState::Idle);
        idle_input.rest_state = RestState::Active;

        let mut untrusted = summary("untrusted", SessionState::Idle);
        untrusted.tool = None;

        let mut stale = trusted_summary("stale", SessionState::Idle);
        stale.tool = None;
        stale.is_stale = true;

        let mut transport = trusted_summary("transport", SessionState::Idle);
        transport.tool = None;
        transport.transport_health = TransportHealth::Degraded;

        let mut quiet_idle = trusted_summary("idle", SessionState::Idle);
        quiet_idle.tool = None;

        for (session, expected) in [
            (
                trusted_summary("attention", SessionState::Attention),
                OperatorPressureReasonKind::NeedsInput,
            ),
            (
                trusted_summary("error", SessionState::Error),
                OperatorPressureReasonKind::Error,
            ),
            (commit_ready, OperatorPressureReasonKind::CommitReady),
            (sleeping, OperatorPressureReasonKind::Sleeping),
            (idle_input, OperatorPressureReasonKind::NeedsInput),
            (untrusted, OperatorPressureReasonKind::UntrustedState),
            (stale, OperatorPressureReasonKind::Stale),
            (transport, OperatorPressureReasonKind::Transport),
            (
                trusted_summary("busy", SessionState::Busy),
                OperatorPressureReasonKind::Busy,
            ),
            (quiet_idle, OperatorPressureReasonKind::Idle),
        ] {
            assert_reason_kind(session, expected);
        }
    }

    #[test]
    fn pressure_reason_fallbacks_keep_commit_before_sleeping() {
        let mut session = trusted_summary("commit-sleeping", SessionState::Idle);
        session.tool = None;
        session.commit_candidate = true;
        session.rest_state = RestState::Sleeping;

        assert_reason_kind(session, OperatorPressureReasonKind::CommitReady);
    }

    #[test]
    fn stale_summary_helper_disables_operator_group_input() {
        let session =
            summary("stale-codex", SessionState::Idle).into_remote_poll_degraded(Some(Utc::now()));

        assert!(!session_ready_for_operator_group_input(&session));
    }

    #[test]
    fn idle_non_agent_shell_is_not_group_input_ready() {
        let mut session = summary("shell", SessionState::Idle);
        session.tool = None;

        assert!(!session_ready_for_operator_group_input(&session));
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
