use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::remote_sessions;
use crate::fleet_lens::{session_needs_attention, target_key, target_label};
use crate::session_labels::{repo_label_for_key, session_repo_key};
use crate::types::{
    ActionCueKind, RestState, SessionBatchMembership, SessionEnvironmentScope, SessionState,
    SessionSummary, StateConfidence, ThoughtState, TransportHealth,
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
pub struct OperatorAttentionInboxItem {
    pub session_id: String,
    pub repo_key: String,
    pub repo_label: String,
    pub target_key: String,
    pub target_label: String,
    pub pressure: OperatorPressure,
    pub remote: bool,
    pub degraded: bool,
    pub stale: bool,
    pub transport_health: TransportHealth,
    pub last_activity_at: String,
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
    pub inbox: Vec<OperatorAttentionInboxItem>,
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
    let mut builder = OperatorPressureResponseBuilder::new(sessions.len(), &batch_send_map);
    for session in sessions {
        builder.push(
            session,
            batch_send_ids_for_session(&batch_send_map, &session.session_id),
        );
    }

    builder.finish()
}

struct OperatorPressureResponseBuilder {
    summary: OperatorPressureSummary,
    repos: HashMap<String, OperatorPressureRepo>,
    sessions: Vec<OperatorPressureSession>,
    inbox: Vec<OperatorAttentionInboxItem>,
}

impl OperatorPressureResponseBuilder {
    fn new(session_count: usize, batch_send_map: &HashMap<String, Vec<String>>) -> Self {
        Self {
            summary: OperatorPressureSummary {
                max_score: 0,
                action_cues: 0,
                batch_send_groups: batch_send_group_count(batch_send_map),
            },
            repos: HashMap::new(),
            sessions: Vec::with_capacity(session_count),
            inbox: Vec::new(),
        }
    }

    fn push(&mut self, session: &SessionSummary, batch_send_session_ids: Vec<String>) {
        let pressure = operator_pressure_for_session(session);
        let repo_key = session_repo_key(session);
        let repo_label = repo_label_for_key(&repo_key);

        update_operator_pressure_summary(&mut self.summary, &pressure);
        update_operator_pressure_repo(&mut self.repos, session, &repo_key, &repo_label, &pressure);
        if let Some(item) =
            operator_attention_inbox_item(session, &repo_key, &repo_label, pressure.clone())
        {
            self.inbox.push(item);
        }
        self.sessions.push(operator_pressure_session(
            session,
            repo_key,
            repo_label,
            pressure,
            batch_send_session_ids,
        ));
    }

    fn finish(mut self) -> OperatorPressureResponse {
        let mut repos = self.repos.into_values().collect::<Vec<_>>();
        sort_operator_pressure_repos(&mut repos);
        sort_operator_pressure_sessions(&mut self.sessions);
        sort_operator_attention_inbox(&mut self.inbox);

        OperatorPressureResponse {
            sessions: self.sessions,
            repos,
            summary: self.summary,
            inbox: self.inbox,
        }
    }
}

fn batch_send_group_count(batch_send_map: &HashMap<String, Vec<String>>) -> usize {
    batch_send_map
        .values()
        .map(|ids| ids.join("\0"))
        .collect::<std::collections::HashSet<_>>()
        .len()
}

fn batch_send_ids_for_session(
    batch_send_map: &HashMap<String, Vec<String>>,
    session_id: &str,
) -> Vec<String> {
    batch_send_map.get(session_id).cloned().unwrap_or_default()
}

fn update_operator_pressure_summary(
    summary: &mut OperatorPressureSummary,
    pressure: &OperatorPressure,
) {
    summary.max_score = summary.max_score.max(pressure.score);
    summary.action_cues += pressure.action_cue_count;
}

fn update_operator_pressure_repo(
    repo_map: &mut HashMap<String, OperatorPressureRepo>,
    session: &SessionSummary,
    repo_key: &str,
    repo_label: &str,
    pressure: &OperatorPressure,
) {
    let repo = repo_map
        .entry(repo_key.to_string())
        .or_insert_with(|| operator_pressure_repo(repo_key, repo_label));
    repo.session_ids.push(session.session_id.clone());
    if pressure.score > repo.score {
        repo.score = pressure.score;
        repo.reason = pressure.reason.clone();
    }
}

fn operator_pressure_repo(repo_key: &str, repo_label: &str) -> OperatorPressureRepo {
    OperatorPressureRepo {
        repo_key: repo_key.to_string(),
        repo_label: repo_label.to_string(),
        score: 0,
        reason: "quiet".to_string(),
        session_ids: Vec::new(),
    }
}

fn operator_pressure_session(
    session: &SessionSummary,
    repo_key: String,
    repo_label: String,
    pressure: OperatorPressure,
    batch_send_session_ids: Vec<String>,
) -> OperatorPressureSession {
    OperatorPressureSession {
        session_id: session.session_id.clone(),
        repo_key,
        repo_label,
        pressure,
        batch_send_session_ids,
    }
}

fn operator_attention_inbox_item(
    session: &SessionSummary,
    repo_key: &str,
    repo_label: &str,
    pressure: OperatorPressure,
) -> Option<OperatorAttentionInboxItem> {
    if !session_is_attention_inbox_candidate(session) {
        return None;
    }
    Some(OperatorAttentionInboxItem {
        session_id: session.session_id.clone(),
        repo_key: repo_key.to_string(),
        repo_label: repo_label.to_string(),
        target_key: target_key(session),
        target_label: target_label(session),
        pressure,
        remote: session_is_remote(session),
        degraded: session_is_degraded(session),
        stale: session.is_stale,
        transport_health: session.transport_health,
        last_activity_at: session.last_activity_at.to_rfc3339(),
    })
}

fn session_is_attention_inbox_candidate(session: &SessionSummary) -> bool {
    session.session_id != "attention-group"
        && session.tmux_name != "swimmers-attention"
        && session.state != SessionState::Exited
        && session_needs_attention(session)
}

fn session_is_remote(session: &SessionSummary) -> bool {
    session.environment.scope == SessionEnvironmentScope::Remote
        || remote_sessions::split_remote_session_id(&session.session_id).is_some()
}

fn session_is_degraded(session: &SessionSummary) -> bool {
    session.is_stale || session.transport_health != TransportHealth::Healthy
}

fn sort_operator_pressure_repos(repos: &mut [OperatorPressureRepo]) {
    repos.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.repo_label.cmp(&right.repo_label))
    });
}

fn sort_operator_pressure_sessions(sessions: &mut [OperatorPressureSession]) {
    sessions.sort_by(|left, right| {
        right
            .pressure
            .score
            .cmp(&left.pressure.score)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
}

fn sort_operator_attention_inbox(items: &mut [OperatorAttentionInboxItem]) {
    items.sort_by(|left, right| {
        left.degraded
            .cmp(&right.degraded)
            .then_with(|| right.pressure.score.cmp(&left.pressure.score))
            .then_with(|| {
                pressure_reason_rank(right.pressure.reason_kind)
                    .cmp(&pressure_reason_rank(left.pressure.reason_kind))
            })
            .then_with(|| right.last_activity_at.cmp(&left.last_activity_at))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
}

fn pressure_reason_rank(kind: OperatorPressureReasonKind) -> u8 {
    match kind {
        OperatorPressureReasonKind::AwaitingUser => 5,
        OperatorPressureReasonKind::CommitReady => 4,
        OperatorPressureReasonKind::ValidationMissingAfterEdit
        | OperatorPressureReasonKind::DirtyCheckMissing
        | OperatorPressureReasonKind::NeedsInput => 3,
        OperatorPressureReasonKind::Error => 2,
        OperatorPressureReasonKind::Sleeping
        | OperatorPressureReasonKind::UntrustedState
        | OperatorPressureReasonKind::Stale
        | OperatorPressureReasonKind::Transport => 1,
        OperatorPressureReasonKind::Busy | OperatorPressureReasonKind::Idle => 0,
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
    pressure_glyph_for_action_cue(primary_cue)
        .unwrap_or_else(|| pressure_glyph_for_session_state(session))
}

fn pressure_glyph_for_action_cue(primary_cue: Option<ActionCueKind>) -> Option<&'static str> {
    match primary_cue {
        Some(ActionCueKind::AwaitingUser) => Some("!"),
        Some(ActionCueKind::CommitReady) => Some("$"),
        Some(ActionCueKind::ValidationMissingAfterEdit) => Some("v"),
        Some(ActionCueKind::DirtyCheckMissing) => Some("d"),
        None => None,
    }
}

fn pressure_glyph_for_session_state(session: &SessionSummary) -> &'static str {
    type PressureGlyphClassifier = fn(&SessionSummary) -> Option<&'static str>;
    let classifiers: [PressureGlyphClassifier; 4] = [
        pressure_glyph_for_attention_state,
        pressure_glyph_for_idle_input,
        pressure_glyph_for_error_state,
        pressure_glyph_for_commit_candidate,
    ];

    classifiers
        .into_iter()
        .find_map(|classify| classify(session))
        .unwrap_or("a")
}

fn pressure_glyph_for_attention_state(session: &SessionSummary) -> Option<&'static str> {
    (session.state == SessionState::Attention).then_some("!")
}

fn pressure_glyph_for_idle_input(session: &SessionSummary) -> Option<&'static str> {
    session_idle_agent_can_accept_operator_input(session).then_some("!")
}

fn pressure_glyph_for_error_state(session: &SessionSummary) -> Option<&'static str> {
    (session.state == SessionState::Error).then_some("x")
}

fn pressure_glyph_for_commit_candidate(session: &SessionSummary) -> Option<&'static str> {
    session.commit_candidate.then_some("$")
}

fn pressure_tone(
    session: &SessionSummary,
    score: u8,
    primary_cue: Option<ActionCueKind>,
) -> OperatorPressureTone {
    if pressure_tone_is_danger(session, score, primary_cue) {
        OperatorPressureTone::Danger
    } else if pressure_tone_is_warning(session, score) {
        OperatorPressureTone::Warning
    } else {
        pressure_tone_for_low_pressure_state(session)
    }
}

fn pressure_tone_is_danger(
    session: &SessionSummary,
    score: u8,
    primary_cue: Option<ActionCueKind>,
) -> bool {
    primary_cue.is_some() || session.state == SessionState::Error || score >= 70
}

fn pressure_tone_is_warning(session: &SessionSummary, score: u8) -> bool {
    score >= 35
        || session.commit_candidate
        || session.rest_state == RestState::Sleeping
        || session_idle_agent_can_accept_operator_input(session)
}

fn pressure_tone_for_low_pressure_state(session: &SessionSummary) -> OperatorPressureTone {
    if session.state == SessionState::Busy {
        OperatorPressureTone::Working
    } else {
        OperatorPressureTone::Quiet
    }
}

type BatchSendSession<'a> = (&'a SessionBatchMembership, &'a SessionSummary);

fn batch_send_session_map(sessions: &[SessionSummary]) -> HashMap<String, Vec<String>> {
    collect_batch_send_groups(sessions)
        .into_values()
        .filter_map(advertisable_batch_send_ids)
        .flat_map(batch_send_session_entries)
        .collect()
}

fn collect_batch_send_groups(
    sessions: &[SessionSummary],
) -> HashMap<String, Vec<BatchSendSession<'_>>> {
    let mut batches: HashMap<String, Vec<BatchSendSession<'_>>> = HashMap::new();
    for session in sessions {
        if let Some((batch, session)) = batch_send_group_member(session) {
            batches
                .entry(batch.id.clone())
                .or_default()
                .push((batch, session));
        }
    }
    batches
}

fn batch_send_group_member(session: &SessionSummary) -> Option<BatchSendSession<'_>> {
    let batch = session.batch.as_ref()?;
    batch_send_session_is_eligible(session).then_some((batch, session))
}

fn batch_send_session_is_eligible(session: &SessionSummary) -> bool {
    session_ready_for_operator_group_input(session)
}

fn advertisable_batch_send_ids(mut sessions: Vec<BatchSendSession<'_>>) -> Option<Vec<String>> {
    if !batch_send_group_has_single_scope(&sessions) {
        return None;
    }
    sessions.sort_by(|(left_batch, left_session), (right_batch, right_session)| {
        left_batch
            .index
            .cmp(&right_batch.index)
            .then_with(|| left_session.session_id.cmp(&right_session.session_id))
    });
    let ids = sessions
        .into_iter()
        .map(|(_, session)| session.session_id.clone())
        .collect::<Vec<_>>();
    (ids.len() >= 2).then_some(ids)
}

fn batch_send_group_has_single_scope(sessions: &[BatchSendSession<'_>]) -> bool {
    let mut scopes = sessions
        .iter()
        .map(|(_, session)| batch_send_scope_key(session))
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();
    scopes.len() <= 1
}

fn batch_send_scope_key(session: &SessionSummary) -> String {
    remote_target_id_for_group_send(session)
        .map(|target| format!("remote:{target}"))
        .unwrap_or_else(|| "local".to_string())
}

fn remote_target_id_for_group_send(session: &SessionSummary) -> Option<String> {
    if let Some((target_id, _)) = remote_sessions::split_remote_session_id(&session.session_id) {
        if session.environment.scope == SessionEnvironmentScope::Remote {
            let environment_target = session.environment.target_id.trim();
            if !environment_target.is_empty() && environment_target != "local" {
                return Some(environment_target.to_string());
            }
        }
        return Some(target_id.to_string());
    }
    None
}

fn batch_send_session_entries(ids: Vec<String>) -> Vec<(String, Vec<String>)> {
    ids.iter()
        .map(|id| (id.clone(), ids.clone()))
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::types::{
        ActionCue, ActionCueConfidence, ActionCueSource, ActionCueStatus, LaunchTargetSummary,
        SessionEnvironmentSummary, StateEvidence, ThoughtState,
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

    fn quiet_summary(id: &str) -> SessionSummary {
        let mut session = trusted_summary(id, SessionState::Idle);
        session.tool = None;
        session.rest_state = RestState::Active;
        session
    }

    fn assign_batch(
        session: &mut SessionSummary,
        id: &str,
        index: usize,
        total: usize,
        created_at: chrono::DateTime<Utc>,
    ) {
        session.batch = Some(SessionBatchMembership {
            id: id.to_string(),
            label: id.to_string(),
            index,
            total,
            created_at,
            prompt_excerpt: None,
        });
    }

    fn assert_reason_kind(session: SessionSummary, expected: OperatorPressureReasonKind) {
        assert_eq!(
            operator_pressure_for_session(&session).reason_kind,
            expected
        );
    }

    fn assert_pressure_tone(
        session: &SessionSummary,
        expected_score: u8,
        expected_tone: OperatorPressureTone,
    ) {
        let pressure = operator_pressure_for_session(session);
        assert_eq!(pressure.score, expected_score);
        assert_eq!(pressure.tone, expected_tone);
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
    fn cue_priority_keeps_commit_before_validation_and_dirty_checks() {
        let mut session = quiet_summary("priority-commit");
        session.action_cues = vec![
            cue(ActionCueKind::DirtyCheckMissing),
            cue(ActionCueKind::ValidationMissingAfterEdit),
            cue(ActionCueKind::CommitReady),
        ];

        let pressure = operator_pressure_for_session(&session);

        assert_eq!(
            pressure.reason_kind,
            OperatorPressureReasonKind::CommitReady
        );
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
    fn pressure_scores_keep_existing_weights_and_clamp() {
        let quiet = quiet_summary("quiet-score");
        assert_eq!(operator_pressure_for_session(&quiet).score, 1);

        let mut busy = trusted_summary("busy-score", SessionState::Busy);
        busy.tool = None;
        assert_eq!(operator_pressure_for_session(&busy).score, 12);

        let mut dirty_check = quiet_summary("dirty-score");
        dirty_check.action_cues = vec![cue(ActionCueKind::DirtyCheckMissing)];
        assert_eq!(operator_pressure_for_session(&dirty_check).score, 35);

        let mut clamped = quiet_summary("clamped-score");
        clamped.action_cues = vec![
            cue(ActionCueKind::AwaitingUser),
            cue(ActionCueKind::CommitReady),
            cue(ActionCueKind::ValidationMissingAfterEdit),
            cue(ActionCueKind::DirtyCheckMissing),
        ];
        assert_eq!(operator_pressure_for_session(&clamped).score, 99);
    }

    #[test]
    fn pressure_glyphs_keep_cue_mapping_and_session_fallback_priority() {
        for (kind, expected) in [
            (ActionCueKind::AwaitingUser, "!"),
            (ActionCueKind::CommitReady, "$"),
            (ActionCueKind::ValidationMissingAfterEdit, "v"),
            (ActionCueKind::DirtyCheckMissing, "d"),
        ] {
            let mut session = quiet_summary(kind.as_str());
            session.action_cues = vec![cue(kind)];
            assert_eq!(operator_pressure_for_session(&session).glyph, expected);
        }

        let mut attention = trusted_summary("attention-glyph", SessionState::Attention);
        attention.tool = None;
        attention.commit_candidate = true;
        assert_eq!(operator_pressure_for_session(&attention).glyph, "!");

        let idle_agent = trusted_summary("idle-agent-glyph", SessionState::Idle);
        assert_eq!(operator_pressure_for_session(&idle_agent).glyph, "!");

        let mut error = trusted_summary("error-glyph", SessionState::Error);
        error.tool = None;
        error.commit_candidate = true;
        assert_eq!(operator_pressure_for_session(&error).glyph, "x");

        let mut commit = quiet_summary("commit-glyph");
        commit.commit_candidate = true;
        assert_eq!(operator_pressure_for_session(&commit).glyph, "$");

        assert_eq!(
            operator_pressure_for_session(&quiet_summary("idle-glyph")).glyph,
            "a"
        );
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
    fn pressure_tone_keeps_existing_precedence_and_thresholds() {
        let mut action_cue = trusted_summary("action-cue", SessionState::Busy);
        action_cue.action_cues = vec![cue(ActionCueKind::DirtyCheckMissing)];
        assert_pressure_tone(&action_cue, 47, OperatorPressureTone::Danger);

        let mut error = trusted_summary("error-tone", SessionState::Error);
        error.tool = None;
        assert_pressure_tone(&error, 55, OperatorPressureTone::Danger);

        let mut high_score = summary("high-score-tone", SessionState::Idle);
        high_score.tool = None;
        high_score.commit_candidate = true;
        high_score.rest_state = RestState::Sleeping;
        assert_pressure_tone(&high_score, 75, OperatorPressureTone::Danger);

        let mut attention = trusted_summary("attention-tone", SessionState::Attention);
        attention.tool = None;
        assert_pressure_tone(&attention, 45, OperatorPressureTone::Warning);

        let mut sleeping = trusted_summary("sleeping-tone", SessionState::Idle);
        sleeping.tool = None;
        sleeping.rest_state = RestState::Sleeping;
        assert_pressure_tone(&sleeping, 35, OperatorPressureTone::Warning);

        let idle_input = trusted_summary("idle-input-tone", SessionState::Idle);
        assert_pressure_tone(&idle_input, 35, OperatorPressureTone::Warning);

        let mut busy = trusted_summary("busy-tone", SessionState::Busy);
        busy.tool = None;
        assert_pressure_tone(&busy, 12, OperatorPressureTone::Working);

        let mut deep_sleep = trusted_summary("deep-sleep-tone", SessionState::Idle);
        deep_sleep.tool = None;
        deep_sleep.rest_state = RestState::DeepSleep;
        assert_pressure_tone(&deep_sleep, 20, OperatorPressureTone::Quiet);

        let mut stale_transport = trusted_summary("stale-transport-tone", SessionState::Idle);
        stale_transport.tool = None;
        stale_transport.is_stale = true;
        stale_transport.transport_health = TransportHealth::Degraded;
        assert_pressure_tone(&stale_transport, 30, OperatorPressureTone::Quiet);

        let mut evidence_threshold = summary("evidence-threshold-tone", SessionState::Idle);
        evidence_threshold.tool = None;
        evidence_threshold.rest_state = RestState::DeepSleep;
        assert_pressure_tone(&evidence_threshold, 35, OperatorPressureTone::Warning);
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
    fn operator_pressure_response_handles_empty_sessions() {
        let response = build_operator_pressure_response(&[]);

        assert!(response.sessions.is_empty());
        assert!(response.repos.is_empty());
        assert!(response.inbox.is_empty());
        assert_eq!(
            response.summary,
            OperatorPressureSummary {
                max_score: 0,
                action_cues: 0,
                batch_send_groups: 0,
            }
        );
    }

    #[test]
    fn operator_pressure_response_keeps_busy_session_fields() {
        let mut busy = trusted_summary("busy-response", SessionState::Busy);
        busy.tool = None;

        let response = build_operator_pressure_response(&[busy]);

        assert_eq!(
            response.summary,
            OperatorPressureSummary {
                max_score: 12,
                action_cues: 0,
                batch_send_groups: 0,
            }
        );
        assert_eq!(response.repos.len(), 1);
        assert_eq!(response.repos[0].score, 12);
        assert_eq!(response.repos[0].reason, "busy");
        assert_eq!(
            response.repos[0].session_ids,
            vec!["busy-response".to_string()]
        );

        let session = &response.sessions[0];
        assert_eq!(session.session_id, "busy-response");
        assert_eq!(session.repo_key, "/tmp/repos/swimmers");
        assert_eq!(session.repo_label, "swimmers");
        assert!(session.batch_send_session_ids.is_empty());
        assert_eq!(session.pressure.score, 12);
        assert_eq!(
            session.pressure.reason_kind,
            OperatorPressureReasonKind::Busy
        );
        assert_eq!(session.pressure.reason, "busy");
        assert_eq!(session.pressure.glyph, "a");
        assert_eq!(session.pressure.tone, OperatorPressureTone::Working);
        assert!(!session.pressure.needs_input);
        assert!(!session.pressure.commit_ready);
    }

    #[test]
    fn operator_pressure_groups_mapped_remote_sessions_by_canonical_repo() {
        let mut local = trusted_summary("local-swimmers", SessionState::Idle);
        local.cwd = "/Users/b/repos/opensource/swimmers".to_string();
        local.environment = SessionEnvironmentSummary::local(local.cwd.clone());
        local.tool = None;

        let mut remote = trusted_summary("skillbox::remote-swimmers", SessionState::Busy);
        remote.cwd = "/srv/skillbox/repos/swimmers".to_string();
        remote.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "remote-swimmers",
            remote.cwd.clone(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        remote.tool = None;

        let response = build_operator_pressure_response(&[local, remote.clone()]);

        assert_eq!(response.repos.len(), 1);
        assert_eq!(
            response.repos[0].repo_key,
            "/Users/b/repos/opensource/swimmers"
        );
        assert_eq!(response.repos[0].repo_label, "opensource/swimmers");
        assert_eq!(
            response.repos[0].session_ids,
            vec![
                "local-swimmers".to_string(),
                "skillbox::remote-swimmers".to_string()
            ]
        );
        let remote_pressure = response
            .sessions
            .iter()
            .find(|session| session.session_id == "skillbox::remote-swimmers")
            .expect("remote pressure session");
        assert_eq!(
            remote_pressure.repo_key,
            "/Users/b/repos/opensource/swimmers"
        );
        assert_eq!(remote.cwd, "/srv/skillbox/repos/swimmers");
    }

    #[test]
    fn attention_inbox_includes_remote_attention_without_quiet_sessions() {
        let local = trusted_summary("local", SessionState::Attention);
        let mut remote = trusted_summary(
            &remote_sessions::namespace_session_id("skillbox", "remote"),
            SessionState::Idle,
        );
        remote.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "remote",
            "/srv/skillbox/repos/swimmers".to_string(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        remote.action_cues = vec![cue(ActionCueKind::AwaitingUser)];
        let quiet = quiet_summary("quiet");

        let response = build_operator_pressure_response(&[quiet, remote, local]);
        let ids = response
            .inbox
            .iter()
            .map(|item| item.session_id.clone())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains("local"));
        assert!(ids.contains(&remote_sessions::namespace_session_id("skillbox", "remote")));
        let remote_item = response
            .inbox
            .iter()
            .find(|item| item.remote)
            .expect("remote inbox item");
        assert_eq!(remote_item.target_key, "skillbox");
        assert_eq!(remote_item.target_label, "Skillbox devbox");
    }

    #[test]
    fn attention_inbox_sorts_degraded_remote_below_healthy_attention() {
        let healthy = trusted_summary("healthy", SessionState::Attention);
        let mut degraded = trusted_summary(
            &remote_sessions::namespace_session_id("skillbox", "stale"),
            SessionState::Attention,
        );
        degraded.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "stale",
            "/srv/skillbox/repos/swimmers".to_string(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        degraded = degraded.into_remote_poll_degraded(Some(Utc::now()));
        degraded.action_cues = vec![cue(ActionCueKind::AwaitingUser)];

        let response = build_operator_pressure_response(&[degraded, healthy]);

        assert_eq!(response.inbox[0].session_id, "healthy");
        assert!(response.inbox[1].remote);
        assert!(response.inbox[1].degraded);
        assert!(response.inbox[1].stale);
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
    fn batch_send_ids_sort_by_batch_index_then_session_id_for_every_member() {
        let created_at = Utc::now();
        let mut gamma = summary("gamma", SessionState::Attention);
        let mut beta = summary("beta", SessionState::Attention);
        let mut alpha = summary("alpha", SessionState::Attention);
        let mut zero = summary("zero", SessionState::Attention);
        assign_batch(&mut gamma, "batch-sort", 2, 4, created_at);
        assign_batch(&mut beta, "batch-sort", 1, 4, created_at);
        assign_batch(&mut alpha, "batch-sort", 1, 4, created_at);
        assign_batch(&mut zero, "batch-sort", 0, 4, created_at);

        let response = build_operator_pressure_response(&[gamma, beta, alpha, zero]);
        let expected = ["zero", "alpha", "beta", "gamma"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();

        assert_eq!(response.summary.batch_send_groups, 1);
        for session_id in ["zero", "alpha", "beta", "gamma"] {
            let session = response
                .sessions
                .iter()
                .find(|session| session.session_id == session_id)
                .expect("batch member");
            assert_eq!(session.batch_send_session_ids, expected, "{session_id}");
        }
    }

    #[test]
    fn batch_send_ids_drop_singleton_groups() {
        let created_at = Utc::now();
        let mut solo = summary("solo", SessionState::Attention);
        let mut pair_b = summary("pair-b", SessionState::Attention);
        let mut pair_a = summary("pair-a", SessionState::Attention);
        assign_batch(&mut solo, "batch-solo", 0, 1, created_at);
        assign_batch(&mut pair_b, "batch-pair", 1, 2, created_at);
        assign_batch(&mut pair_a, "batch-pair", 0, 2, created_at);

        let response = build_operator_pressure_response(&[solo, pair_b, pair_a]);
        let pair_ids = ["pair-a", "pair-b"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();

        assert_eq!(response.summary.batch_send_groups, 1);
        let solo = response
            .sessions
            .iter()
            .find(|session| session.session_id == "solo")
            .expect("solo session");
        assert!(solo.batch_send_session_ids.is_empty());
        for session_id in ["pair-a", "pair-b"] {
            let session = response
                .sessions
                .iter()
                .find(|session| session.session_id == session_id)
                .expect("pair session");
            assert_eq!(session.batch_send_session_ids, pair_ids, "{session_id}");
        }
    }

    #[test]
    fn batch_send_ids_allow_same_target_remote_sessions() {
        let created_at = Utc::now();
        let mut first = summary(
            &remote_sessions::namespace_session_id("jeremy-skillbox", "first"),
            SessionState::Attention,
        );
        let mut second = summary(
            &remote_sessions::namespace_session_id("jeremy-skillbox", "second"),
            SessionState::Attention,
        );
        assign_batch(&mut first, "batch-remote", 0, 2, created_at);
        assign_batch(&mut second, "batch-remote", 1, 2, created_at);

        let response = build_operator_pressure_response(&[first, second]);
        let expected = [
            remote_sessions::namespace_session_id("jeremy-skillbox", "first"),
            remote_sessions::namespace_session_id("jeremy-skillbox", "second"),
        ]
        .into_iter()
        .collect::<Vec<_>>();

        assert_eq!(response.summary.batch_send_groups, 1);
        for session in response.sessions {
            assert_eq!(
                session.batch_send_session_ids, expected,
                "{}",
                session.session_id
            );
        }
    }

    #[test]
    fn batch_send_ids_skip_mixed_local_remote_sessions() {
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
                "mixed local/remote group input must not be advertised for {}",
                session.session_id
            );
        }
    }

    #[test]
    fn batch_send_ids_skip_mixed_remote_targets() {
        let created_at = Utc::now();
        let mut first = summary(
            &remote_sessions::namespace_session_id("alpha", "first"),
            SessionState::Attention,
        );
        let mut second = summary(
            &remote_sessions::namespace_session_id("beta", "second"),
            SessionState::Attention,
        );
        assign_batch(&mut first, "batch-remote", 0, 2, created_at);
        assign_batch(&mut second, "batch-remote", 1, 2, created_at);

        let response = build_operator_pressure_response(&[first, second]);

        assert_eq!(response.summary.batch_send_groups, 0);
        for session in response.sessions {
            assert!(
                session.batch_send_session_ids.is_empty(),
                "mixed remote target group input must not be advertised for {}",
                session.session_id
            );
        }
    }
}
