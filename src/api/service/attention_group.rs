use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::NativeOpenServiceError;
use crate::api::{remote_sessions, AppState};
use crate::native;
use crate::operator_pressure::session_ready_for_operator_group_input;
use crate::session_labels::session_repo_family_key;
use crate::types::{
    GhosttyOpenMode, NativeAttentionGroupOpenRequest, NativeAttentionGroupOpenResponse,
    NativeDesktopApp, SessionEnvironmentScope, SessionSummary,
};

const NATIVE_ATTENTION_GROUP_SESSION_ID: &str = "attention-group";
const NATIVE_ATTENTION_GROUP_TMUX_NAME: &str = "swimmers-attention";
const ATTENTION_PROJECT_FAMILY_SUFFIXES: &[&str] = &[
    "_server",
    "-server",
    "_backend",
    "-backend",
    "_frontend",
    "-frontend",
    "_client",
    "-client",
    "_web",
    "-web",
    "_api",
    "-api",
    "_core",
    "-core",
];

pub async fn open_native_attention_group_for_host(
    state: &Arc<AppState>,
    host: &str,
    request: NativeAttentionGroupOpenRequest,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    let app = *state.native_desktop_app.read().await;
    let ghostty_mode = *state.ghostty_open_mode.read().await;
    let status = native::support_for_host(host, app);
    let plan = native_attention_group_plan(state, &request).await;

    open_native_attention_group_plan(app, ghostty_mode, request, status.supported, plan).await
}

async fn native_attention_group_plan(
    state: &Arc<AppState>,
    request: &NativeAttentionGroupOpenRequest,
) -> AttentionGroupPlan {
    plan_attention_group_sessions(
        state.supervisor.list_sessions().await,
        request.max_sessions.unwrap_or(6),
        &request.current_session_ids,
        request.include_unnumbered_sessions,
    )
}

async fn open_native_attention_group_plan(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    request: NativeAttentionGroupOpenRequest,
    native_supported: bool,
    plan: AttentionGroupPlan,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    if plan.visible.is_empty() {
        return handle_empty_attention_group_plan(&request).await;
    }

    open_visible_native_attention_group(app, ghostty_mode, &request, native_supported, plan).await
}

async fn handle_empty_attention_group_plan(
    request: &NativeAttentionGroupOpenRequest,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    match empty_attention_group_plan_outcome(request) {
        EmptyAttentionGroupPlanOutcome::ClearNative => native::clear_native_attention_group()
            .await
            .map_err(|error| NativeOpenServiceError::Internal(error.to_string())),
        EmptyAttentionGroupPlanOutcome::NoAttentionSessions => {
            Err(NativeOpenServiceError::NoAttentionSessions)
        }
    }
}

fn empty_attention_group_plan_outcome(
    request: &NativeAttentionGroupOpenRequest,
) -> EmptyAttentionGroupPlanOutcome {
    if !request.focus && !request.current_session_ids.is_empty() {
        EmptyAttentionGroupPlanOutcome::ClearNative
    } else {
        EmptyAttentionGroupPlanOutcome::NoAttentionSessions
    }
}

async fn open_visible_native_attention_group(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    request: &NativeAttentionGroupOpenRequest,
    native_supported: bool,
    plan: AttentionGroupPlan,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    let response = native::open_native_attention_group(
        app,
        ghostty_mode,
        &plan.visible,
        request.focus && native_supported,
        request.layout.unwrap_or_default(),
    )
    .await
    .map_err(|error| NativeOpenServiceError::Internal(error.to_string()))?;

    Ok(response_with_attention_backlog(response, &plan))
}

fn response_with_attention_backlog(
    mut response: NativeAttentionGroupOpenResponse,
    plan: &AttentionGroupPlan,
) -> NativeAttentionGroupOpenResponse {
    response.backlog_session_ids = plan
        .backlog
        .iter()
        .map(|session| session.session_id.clone())
        .collect();
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyAttentionGroupPlanOutcome {
    ClearNative,
    NoAttentionSessions,
}

#[derive(Debug, Clone)]
struct AttentionGroupPlan {
    visible: Vec<SessionSummary>,
    backlog: Vec<SessionSummary>,
}

#[derive(Debug, Clone)]
struct AttentionCandidate {
    session: SessionSummary,
    repo: String,
    family: String,
    batch: Option<String>,
}

fn plan_attention_group_sessions(
    sessions: Vec<SessionSummary>,
    max_sessions: usize,
    current_session_ids: &[String],
    include_unnumbered_sessions: bool,
) -> AttentionGroupPlan {
    let limit = max_sessions.clamp(1, 6);
    let mut candidates = attention_group_candidates(sessions, include_unnumbered_sessions);
    if candidates.is_empty() {
        return AttentionGroupPlan {
            visible: Vec::new(),
            backlog: Vec::new(),
        };
    }

    let current_ids = current_session_ids.iter().collect::<HashSet<_>>();
    let mut visible =
        retain_current_attention_group_candidates(&mut candidates, current_session_ids, limit);
    fill_attention_group_candidates(&mut visible, &mut candidates, limit);
    sort_attention_backlog_candidates(&mut candidates, &visible);

    AttentionGroupPlan {
        visible: attention_sessions_from_candidates(visible),
        backlog: attention_backlog_sessions(candidates, &current_ids),
    }
}

fn attention_group_candidates(
    sessions: Vec<SessionSummary>,
    include_unnumbered_sessions: bool,
) -> Vec<AttentionCandidate> {
    sessions
        .into_iter()
        .filter(attention_group_session_is_eligible)
        .filter(|session| include_unnumbered_sessions || tmux_name_is_numbered(&session.tmux_name))
        .map(AttentionCandidate::from)
        .collect()
}

fn retain_current_attention_group_candidates(
    candidates: &mut Vec<AttentionCandidate>,
    current_session_ids: &[String],
    limit: usize,
) -> Vec<AttentionCandidate> {
    current_session_ids
        .iter()
        .filter_map(|session_id| remove_attention_candidate(candidates, session_id))
        .take(limit)
        .collect()
}

fn remove_attention_candidate(
    candidates: &mut Vec<AttentionCandidate>,
    session_id: &str,
) -> Option<AttentionCandidate> {
    candidates
        .iter()
        .position(|candidate| candidate.session.session_id == session_id)
        .map(|index| candidates.remove(index))
}

fn fill_attention_group_candidates(
    visible: &mut Vec<AttentionCandidate>,
    candidates: &mut Vec<AttentionCandidate>,
    limit: usize,
) {
    if visible.is_empty() {
        let anchor_index = best_attention_anchor_index(candidates);
        visible.push(candidates.remove(anchor_index));
    }

    while visible.len() < limit && !candidates.is_empty() {
        let next_index = best_attention_fill_index(visible, candidates);
        visible.push(candidates.remove(next_index));
    }
}

fn sort_attention_backlog_candidates(
    candidates: &mut [AttentionCandidate],
    visible: &[AttentionCandidate],
) {
    candidates.sort_by(|a, b| {
        best_adjacency_to_group(b, visible)
            .cmp(&best_adjacency_to_group(a, visible))
            .then_with(|| b.session.last_activity_at.cmp(&a.session.last_activity_at))
            .then_with(|| a.session.session_id.cmp(&b.session.session_id))
    });
}

fn attention_sessions_from_candidates(candidates: Vec<AttentionCandidate>) -> Vec<SessionSummary> {
    candidates
        .into_iter()
        .map(|candidate| candidate.session)
        .collect()
}

fn attention_backlog_sessions(
    candidates: Vec<AttentionCandidate>,
    current_ids: &HashSet<&String>,
) -> Vec<SessionSummary> {
    candidates
        .into_iter()
        .filter(|candidate| !current_ids.contains(&candidate.session.session_id))
        .map(|candidate| candidate.session)
        .collect()
}

fn attention_group_session_is_eligible(session: &SessionSummary) -> bool {
    session.session_id != NATIVE_ATTENTION_GROUP_SESSION_ID
        && session.tmux_name != NATIVE_ATTENTION_GROUP_TMUX_NAME
        && session.environment.scope != SessionEnvironmentScope::Remote
        && remote_sessions::split_remote_session_id(&session.session_id).is_none()
        && session_ready_for_operator_group_input(session)
}

fn tmux_name_is_numbered(tmux_name: &str) -> bool {
    !tmux_name.is_empty() && tmux_name.chars().all(|ch| ch.is_ascii_digit())
}

impl From<SessionSummary> for AttentionCandidate {
    fn from(session: SessionSummary) -> Self {
        let repo = session_repo_family_key(&session);
        let family = attention_project_family(&repo);
        let batch = session.batch.as_ref().map(|batch| batch.id.clone());
        Self {
            session,
            repo,
            family,
            batch,
        }
    }
}

fn best_attention_anchor_index(candidates: &[AttentionCandidate]) -> usize {
    candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            attention_anchor_score(a, candidates)
                .cmp(&attention_anchor_score(b, candidates))
                .then_with(|| a.session.last_activity_at.cmp(&b.session.last_activity_at))
                .then_with(|| b.session.session_id.cmp(&a.session.session_id))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn attention_anchor_score(
    candidate: &AttentionCandidate,
    candidates: &[AttentionCandidate],
) -> i32 {
    candidates
        .iter()
        .filter(|other| other.session.session_id != candidate.session.session_id)
        .map(|other| attention_adjacency_score(candidate, other))
        .sum()
}

fn best_attention_fill_index(
    visible: &[AttentionCandidate],
    candidates: &[AttentionCandidate],
) -> usize {
    best_attention_fill_choice(visible, candidates).unwrap_or(0)
}

fn best_attention_fill_choice(
    visible: &[AttentionCandidate],
    candidates: &[AttentionCandidate],
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, candidate)| attention_fill_rank(candidate, visible))
        .map(|(index, _)| index)
}

fn attention_fill_rank(
    candidate: &AttentionCandidate,
    visible: &[AttentionCandidate],
) -> (i32, DateTime<Utc>, Reverse<String>) {
    (
        best_adjacency_to_group(candidate, visible),
        candidate.session.last_activity_at,
        Reverse(candidate.session.session_id.clone()),
    )
}

fn best_adjacency_to_group(candidate: &AttentionCandidate, visible: &[AttentionCandidate]) -> i32 {
    visible
        .iter()
        .map(|visible| attention_adjacency_score(candidate, visible))
        .max()
        .unwrap_or(0)
}

fn attention_adjacency_score(a: &AttentionCandidate, b: &AttentionCandidate) -> i32 {
    if attention_candidates_are_same_session(a, b) {
        return attention_self_adjacency_score();
    }
    attention_relationship_score(a, b)
}

fn attention_candidates_are_same_session(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.session.session_id == b.session.session_id
}

fn attention_self_adjacency_score() -> i32 {
    0
}

fn attention_relationship_score(a: &AttentionCandidate, b: &AttentionCandidate) -> i32 {
    attention_weight_if(attention_repos_match(a, b), 100)
        + attention_weight_if(attention_families_match(a, b), 70)
        + attention_weight_if(attention_batches_match(a, b), 50)
        + attention_weight_if(attention_tools_match(a, b), 5)
}

fn attention_weight_if(matched: bool, weight: i32) -> i32 {
    if matched {
        weight
    } else {
        0
    }
}

fn attention_repos_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    !a.repo.is_empty() && a.repo == b.repo
}

fn attention_families_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    !a.family.is_empty() && a.family == b.family
}

fn attention_batches_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.batch.is_some() && a.batch == b.batch
}

fn attention_tools_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.session.tool.is_some() && a.session.tool == b.session.tool
}

fn attention_project_family(repo: &str) -> String {
    let family = repo.trim().to_ascii_lowercase();
    ATTENTION_PROJECT_FAMILY_SUFFIXES
        .iter()
        .find_map(|suffix| attention_project_family_without_suffix(&family, suffix))
        .unwrap_or(family)
}

fn attention_project_family_without_suffix(family: &str, suffix: &str) -> Option<String> {
    family
        .strip_suffix(suffix)
        .filter(|base| !base.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
fn select_attention_group_sessions(
    sessions: Vec<SessionSummary>,
    max_sessions: usize,
) -> Vec<SessionSummary> {
    plan_attention_group_sessions(sessions, max_sessions, &[], false).visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        LaunchTargetSummary, NativeAttentionGroupOpenRequest, NativeAttentionGroupOpenResponse,
        RestState, SessionBatchMembership, SessionEnvironmentSummary, SessionState, SessionSummary,
        StateEvidence, ThoughtSource, ThoughtState, TransportHealth,
    };
    use chrono::{Duration as ChronoDuration, Utc};

    fn summary(session_id: &str, tmux_name: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: tmux_name.to_string(),
            state,
            current_command: None,
            state_evidence: StateEvidence::new("test"),
            cwd: "/tmp/repos/swimmers".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
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
            environment: Default::default(),
        }
    }

    fn waiting_session(session_id: &str, cwd: &str, seconds_ago: i64) -> SessionSummary {
        let mut session = summary(session_id, session_id, SessionState::Idle);
        session.cwd = cwd.to_string();
        session.last_activity_at = Utc::now() - ChronoDuration::seconds(seconds_ago);
        session
    }

    fn numbered_waiting_session(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        seconds_ago: i64,
    ) -> SessionSummary {
        let mut session = waiting_session(session_id, cwd, seconds_ago);
        session.tmux_name = tmux_name.to_string();
        session
    }

    fn batch_session(mut session: SessionSummary, batch_id: &str) -> SessionSummary {
        session.batch = Some(SessionBatchMembership {
            id: batch_id.to_string(),
            label: batch_id.to_string(),
            index: 0,
            total: 2,
            created_at: Utc::now(),
            prompt_excerpt: None,
        });
        session
    }

    fn plan_ids(
        sessions: Vec<SessionSummary>,
        max_sessions: usize,
        current_session_ids: &[&str],
    ) -> Vec<String> {
        plan_ids_with_unnumbered(sessions, max_sessions, current_session_ids, false)
    }

    fn plan_ids_with_unnumbered(
        sessions: Vec<SessionSummary>,
        max_sessions: usize,
        current_session_ids: &[&str],
        include_unnumbered_sessions: bool,
    ) -> Vec<String> {
        let current = current_session_ids
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>();
        plan_attention_group_sessions(
            sessions,
            max_sessions,
            &current,
            include_unnumbered_sessions,
        )
        .visible
        .into_iter()
        .map(|session| session.session_id)
        .collect()
    }

    fn attention_group_request(
        focus: bool,
        current_session_ids: &[&str],
    ) -> NativeAttentionGroupOpenRequest {
        NativeAttentionGroupOpenRequest {
            max_sessions: None,
            current_session_ids: current_session_ids
                .iter()
                .map(|session_id| (*session_id).to_string())
                .collect(),
            include_unnumbered_sessions: false,
            layout: None,
            focus,
        }
    }

    #[test]
    fn attention_group_empty_focus_plan_reports_no_sessions() {
        let request = attention_group_request(true, &["visible-a"]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::NoAttentionSessions
        );
    }

    #[test]
    fn attention_group_empty_non_focus_current_group_requests_native_clear() {
        let request = attention_group_request(false, &["visible-a"]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::ClearNative
        );
    }

    #[test]
    fn attention_group_empty_non_focus_without_current_group_reports_no_sessions() {
        let request = attention_group_request(false, &[]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::NoAttentionSessions
        );
    }

    #[test]
    fn attention_group_response_populates_backlog_session_ids() {
        let visible = numbered_waiting_session("visible-a", "69", "/Users/b/repos/swimmers", 10);
        let backlog_a = numbered_waiting_session("backlog-a", "70", "/Users/b/repos/swimmers", 20);
        let backlog_b = numbered_waiting_session("backlog-b", "71", "/Users/b/repos/swimmers", 30);
        let plan = AttentionGroupPlan {
            visible: vec![visible],
            backlog: vec![backlog_a, backlog_b],
        };
        let response = NativeAttentionGroupOpenResponse {
            session_id: NATIVE_ATTENTION_GROUP_SESSION_ID.to_string(),
            tmux_name: NATIVE_ATTENTION_GROUP_TMUX_NAME.to_string(),
            session_count: 1,
            session_ids: vec!["visible-a".to_string()],
            backlog_session_ids: vec!["stale".to_string()],
            status: "refreshed".to_string(),
            focused: false,
            pane_id: None,
            attach_command: None,
        };

        let response = response_with_attention_backlog(response, &plan);

        assert_eq!(
            response.backlog_session_ids,
            vec!["backlog-a".to_string(), "backlog-b".to_string()]
        );
    }

    #[test]
    fn attention_group_selection_includes_idle_agent_sessions_without_sleep_snapshot() {
        let idle_agent = summary("sess-11", "11", SessionState::Idle);
        let managed_group = summary(
            NATIVE_ATTENTION_GROUP_SESSION_ID,
            NATIVE_ATTENTION_GROUP_TMUX_NAME,
            SessionState::Attention,
        );
        let mut shell = summary("shell", "shell", SessionState::Idle);
        shell.tool = None;
        let busy_agent = summary("busy", "busy", SessionState::Busy);

        let selected =
            select_attention_group_sessions(vec![shell, idle_agent, managed_group, busy_agent], 6);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].tmux_name, "11");
    }

    #[test]
    fn attention_group_selection_excludes_unnumbered_tmux_names_by_default() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);
        let named = summary("sess-named", "buildooor", SessionState::Attention);

        let selected = plan_ids(vec![wave, named, numbered], 6, &[]);

        assert_eq!(selected, vec!["sess-8"]);
    }

    #[test]
    fn attention_group_selection_requires_exact_numeric_tmux_names() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let padded_numeric = summary("sess-padded-9", " 9 ", SessionState::Attention);

        let selected = plan_ids(vec![padded_numeric, numbered], 6, &[]);

        assert_eq!(selected, vec!["sess-8"]);
    }

    #[test]
    fn attention_group_refresh_drops_current_unnumbered_tmux_names_by_default() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let current_wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);
        let next_numbered = summary("sess-9", "9", SessionState::Attention);

        let selected = plan_ids(
            vec![current_wave, numbered, next_numbered],
            2,
            &["sess-wave-01", "sess-8"],
        );

        assert_eq!(selected, vec!["sess-8", "sess-9"]);
    }

    #[test]
    fn attention_group_can_include_unnumbered_tmux_names_when_opted_in() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);

        let selected = plan_ids_with_unnumbered(vec![numbered, wave], 2, &["sess-wave-01"], true);

        assert_eq!(selected, vec!["sess-wave-01", "sess-8"]);
    }

    #[test]
    fn attention_queue_prefers_same_sweet_potato_project_over_newer_unrelated_sessions() {
        let sweet_a = numbered_waiting_session("sweet-a", "21", "/Users/b/repos/sweet-potato", 120);
        let sweet_b = numbered_waiting_session(
            "sweet-b",
            "22",
            "/Users/b/repos/sweet-potato/packages/api",
            90,
        );
        let newer_unrelated =
            numbered_waiting_session("newer", "23", "/Users/b/repos/buildooor", 1);

        let selected = plan_ids(vec![newer_unrelated, sweet_a, sweet_b], 2, &[]);

        assert_eq!(selected, vec!["sweet-b", "sweet-a"]);
    }

    #[test]
    fn attention_queue_treats_htma_and_htma_server_as_adjacent_siblings() {
        let htma = numbered_waiting_session("htma-ui", "31", "/Users/b/repos/htma", 80);
        let htma_server =
            numbered_waiting_session("htma-api", "32", "/Users/b/repos/htma_server", 70);
        let unrelated =
            numbered_waiting_session("newer-unrelated", "33", "/Users/b/repos/finalreceipts", 1);

        let selected = plan_ids(vec![unrelated, htma, htma_server], 2, &[]);

        assert_eq!(selected, vec!["htma-api", "htma-ui"]);
    }

    #[test]
    fn attention_queue_uses_batch_before_recency_tie_break() {
        let batch_a = batch_session(
            numbered_waiting_session("batch-a", "41", "/Users/b/repos/alpha", 90),
            "b1",
        );
        let batch_b = batch_session(
            numbered_waiting_session("batch-b", "42", "/Users/b/repos/beta", 80),
            "b1",
        );
        let newer_unrelated = numbered_waiting_session("newer", "43", "/Users/b/repos/gamma", 1);

        let selected = plan_ids(vec![newer_unrelated, batch_a, batch_b], 2, &[]);

        assert_eq!(selected, vec!["batch-b", "batch-a"]);
    }

    #[test]
    fn attention_adjacency_score_weights_only_present_matching_relationships() {
        let alpha_a = AttentionCandidate::from(batch_session(
            numbered_waiting_session("alpha-a", "44", "/Users/b/repos/alpha", 20),
            "b1",
        ));
        let alpha_b = AttentionCandidate::from(batch_session(
            numbered_waiting_session("alpha-b", "45", "/Users/b/repos/alpha", 10),
            "b1",
        ));

        assert_eq!(attention_adjacency_score(&alpha_a, &alpha_b), 225);
        assert_eq!(attention_adjacency_score(&alpha_a, &alpha_a), 0);

        let mut missing_a = numbered_waiting_session("missing-a", "46", "", 20);
        missing_a.tool = None;
        let mut missing_b = numbered_waiting_session("missing-b", "47", "", 10);
        missing_b.tool = None;

        assert_eq!(
            attention_adjacency_score(
                &AttentionCandidate::from(missing_a),
                &AttentionCandidate::from(missing_b)
            ),
            0
        );
    }

    #[test]
    fn attention_adjacency_treats_mapped_remote_cwd_as_same_repo() {
        let local = numbered_waiting_session(
            "local-swimmers",
            "61",
            "/Users/b/repos/opensource/swimmers",
            20,
        );
        let mut remote =
            numbered_waiting_session("remote-swimmers", "62", "/srv/skillbox/repos/swimmers", 10);
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
            "remote-swimmers",
            remote.cwd.clone(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );
        let unrelated =
            numbered_waiting_session("newer-unrelated", "63", "/Users/b/repos/buildooor", 1);

        let local = AttentionCandidate::from(local);
        let remote = AttentionCandidate::from(remote);
        let unrelated = AttentionCandidate::from(unrelated);

        assert!(
            attention_adjacency_score(&local, &remote)
                > attention_adjacency_score(&local, &unrelated)
        );
    }

    #[test]
    fn attention_queue_excludes_remote_environment_sessions_without_namespaced_ids() {
        let local = numbered_waiting_session(
            "local-swimmers",
            "61",
            "/Users/b/repos/opensource/swimmers",
            20,
        );
        let mut remote =
            numbered_waiting_session("remote-swimmers", "62", "/srv/skillbox/repos/swimmers", 10);
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
            "remote-swimmers",
            remote.cwd.clone(),
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );

        let selected = plan_ids(vec![local, remote], 2, &[]);

        assert_eq!(selected, vec!["local-swimmers"]);
    }

    #[test]
    fn attention_fill_prefers_best_adjacency_to_visible_group() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "48",
            "/Users/b/repos/alpha",
            60,
        ))];
        let unrelated_newer = AttentionCandidate::from(numbered_waiting_session(
            "unrelated-newer",
            "49",
            "/Users/b/repos/gamma",
            1,
        ));
        let same_repo_older = AttentionCandidate::from(numbered_waiting_session(
            "same-repo-older",
            "50",
            "/Users/b/repos/alpha",
            120,
        ));

        assert_eq!(
            best_attention_fill_index(&visible, &[unrelated_newer, same_repo_older]),
            1
        );
    }

    #[test]
    fn attention_fill_uses_recency_then_session_id_for_equal_scores() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "56",
            "/Users/b/repos/alpha",
            60,
        ))];
        let older = AttentionCandidate::from(numbered_waiting_session(
            "older-alpha",
            "57",
            "/Users/b/repos/alpha",
            120,
        ));
        let newer = AttentionCandidate::from(numbered_waiting_session(
            "newer-alpha",
            "58",
            "/Users/b/repos/alpha",
            30,
        ));

        assert_eq!(best_attention_fill_index(&visible, &[older, newer]), 1);

        let tied_at = Utc::now();
        let mut later_id = numbered_waiting_session("tie-b", "59", "/Users/b/repos/alpha", 30);
        later_id.last_activity_at = tied_at;
        let mut earlier_id = numbered_waiting_session("tie-a", "60", "/Users/b/repos/alpha", 30);
        earlier_id.last_activity_at = tied_at;

        assert_eq!(
            best_attention_fill_index(
                &visible,
                &[
                    AttentionCandidate::from(later_id),
                    AttentionCandidate::from(earlier_id)
                ]
            ),
            1
        );
    }

    #[test]
    fn attention_fill_returns_zero_when_candidates_are_empty() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "68",
            "/Users/b/repos/alpha",
            60,
        ))];

        assert_eq!(best_attention_fill_index(&visible, &[]), 0);
    }

    #[test]
    fn attention_queue_rotates_one_in_one_out_from_current_visible_set() {
        let visible_a =
            numbered_waiting_session("visible-a", "51", "/Users/b/repos/sweet-potato", 120);
        let mut resolved_b =
            numbered_waiting_session("visible-b", "52", "/Users/b/repos/sweet-potato", 110);
        resolved_b.thought_state = ThoughtState::Active;
        let visible_c =
            numbered_waiting_session("visible-c", "53", "/Users/b/repos/sweet-potato", 100);
        let next_d = numbered_waiting_session(
            "next-d",
            "54",
            "/Users/b/repos/sweet-potato/packages/api",
            90,
        );
        let unrelated_newer =
            numbered_waiting_session("unrelated", "55", "/Users/b/repos/buildooor", 1);

        let selected = plan_ids(
            vec![visible_a, resolved_b, visible_c, next_d, unrelated_newer],
            3,
            &["visible-a", "visible-b", "visible-c"],
        );

        assert_eq!(selected, vec!["visible-a", "visible-c", "next-d"]);
    }

    #[test]
    fn attention_queue_excludes_unsafe_sessions() {
        let ready = numbered_waiting_session("ready", "61", "/Users/b/repos/swimmers", 1);
        let mut stale = numbered_waiting_session("stale", "62", "/Users/b/repos/swimmers", 1);
        stale.is_stale = true;
        let mut unhealthy =
            numbered_waiting_session("unhealthy", "63", "/Users/b/repos/swimmers", 1);
        unhealthy.transport_health = TransportHealth::Disconnected;
        let mut unobserved =
            numbered_waiting_session("unobserved", "64", "/Users/b/repos/swimmers", 1);
        unobserved.state_evidence = StateEvidence::unobserved("test");
        let exited = summary("exited", "65", SessionState::Exited);
        let mut deep_sleep = numbered_waiting_session("deep", "66", "/Users/b/repos/swimmers", 1);
        deep_sleep.rest_state = RestState::DeepSleep;
        let remote = numbered_waiting_session("remote::sess", "67", "/Users/b/repos/swimmers", 1);
        let managed = summary(
            NATIVE_ATTENTION_GROUP_SESSION_ID,
            NATIVE_ATTENTION_GROUP_TMUX_NAME,
            SessionState::Attention,
        );

        let selected = plan_ids(
            vec![
                ready, stale, unhealthy, unobserved, exited, deep_sleep, remote, managed,
            ],
            6,
            &[],
        );

        assert_eq!(selected, vec!["ready"]);
    }

    #[test]
    fn attention_queue_excludes_remote_namespaced_sessions() {
        let ready = numbered_waiting_session("ready", "61", "/Users/b/repos/swimmers", 1);
        let remote_id = remote_sessions::namespace_session_id("skillbox", "remote-ready");
        let mut remote = numbered_waiting_session(&remote_id, "62", "/Users/b/repos/swimmers", 1);
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
            "remote-ready",
            "/srv/skillbox/repos/swimmers".to_string(),
            Some("/Users/b/repos/swimmers".to_string()),
            "remote_swimmers_api",
        );

        let selected = plan_ids(vec![remote, ready], 6, &[]);

        assert_eq!(selected, vec!["ready"]);
    }
}
