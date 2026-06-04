use std::collections::{HashMap, HashSet};

use crate::persistence::file_store::{PersistedSession, ThoughtSnapshot};
use crate::thought::loop_runner::SessionInfo;
use crate::types::{SessionState, SessionSummary};

pub(super) fn thought_snapshot_for_summary<'a>(
    summary: &SessionSummary,
    active_pane_session_id: Option<&str>,
    thought_snapshots: &'a HashMap<String, ThoughtSnapshot>,
) -> Option<&'a ThoughtSnapshot> {
    thought_snapshots
        .get(&summary.session_id)
        .or_else(|| active_pane_session_id.and_then(|session_id| thought_snapshots.get(session_id)))
}

fn summary_requires_active_pane_lookup(
    summary: &SessionSummary,
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
) -> bool {
    !thought_snapshots.contains_key(&summary.session_id)
        && !summary.tmux_name.is_empty()
        && summary.state != SessionState::Exited
}

pub(super) fn merge_summary_with_thought_snapshot(
    summary: &mut SessionSummary,
    thought_data: &ThoughtSnapshot,
) {
    if summary.thought.is_none() {
        summary.thought = thought_data.thought.clone();
    }
    summary.thought_state = thought_data.thought_state;
    summary.thought_source = thought_data.thought_source;
    summary.thought_updated_at = Some(thought_data.updated_at);
    summary.rest_state = thought_data.rest_state;
    summary.commit_candidate = thought_data.commit_candidate;
    summary.action_cues = thought_data.action_cues.clone();
    summary.objective_changed_at = thought_data.objective_changed_at;
    if thought_data.token_count > 0 || summary.token_count == 0 {
        summary.token_count = thought_data.token_count;
    }
    if thought_data.context_limit > 0 {
        summary.context_limit = thought_data.context_limit;
    }
}

pub(super) fn persisted_session_from_summary(
    summary: &SessionSummary,
    thought_data: Option<&ThoughtSnapshot>,
) -> PersistedSession {
    PersistedSession {
        session_id: summary.session_id.clone(),
        tmux_name: summary.tmux_name.clone(),
        state: summary.state,
        tool: summary.tool.clone(),
        token_count: summary.token_count,
        context_limit: summary.context_limit,
        thought: summary.thought.clone(),
        thought_state: summary.thought_state,
        thought_source: summary.thought_source,
        thought_updated_at: summary.thought_updated_at,
        rest_state: summary.rest_state,
        commit_candidate: summary.commit_candidate,
        action_cues: thought_data
            .map(|snapshot| snapshot.action_cues.clone())
            .unwrap_or_else(|| summary.action_cues.clone()),
        objective_changed_at: summary.objective_changed_at,
        last_skill: summary.last_skill.clone(),
        objective_fingerprint: thought_data
            .and_then(|snapshot| snapshot.objective_fingerprint.clone()),
        batch: summary.batch.clone(),
        cwd: summary.cwd.clone(),
        last_activity_at: summary.last_activity_at,
    }
}

pub(super) fn active_pane_session_id_for_summary(
    summary: &SessionSummary,
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
    active_pane_session_ids: &HashMap<String, String>,
) -> Option<String> {
    if !summary_requires_active_pane_lookup(summary, thought_snapshots) {
        return None;
    }

    active_pane_session_ids.get(&summary.tmux_name).cloned()
}

pub(super) fn merge_thought_snapshots_into_summaries(
    summaries: &mut [SessionSummary],
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
    active_pane_session_ids: &HashMap<String, String>,
) {
    for summary in summaries {
        merge_matching_thought_snapshot_into_summary(
            summary,
            thought_snapshots,
            active_pane_session_ids,
        );
    }
}

fn merge_matching_thought_snapshot_into_summary(
    summary: &mut SessionSummary,
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
    active_pane_session_ids: &HashMap<String, String>,
) {
    let active_pane_session_id =
        active_pane_session_id_for_summary(summary, thought_snapshots, active_pane_session_ids);
    if let Some(thought_data) = thought_snapshot_for_summary(
        summary,
        active_pane_session_id.as_deref(),
        thought_snapshots,
    ) {
        merge_summary_with_thought_snapshot(summary, thought_data);
    }
}

pub(super) fn session_info_from_summary(
    summary: SessionSummary,
    replay_text: String,
    thought_data: Option<&ThoughtSnapshot>,
) -> SessionInfo {
    let state = summary.state;
    let mut info = SessionInfo {
        session_id: summary.session_id,
        state,
        exited: state == SessionState::Exited,
        tool: summary.tool,
        cwd: summary.cwd,
        replay_text,
        thought: summary.thought,
        thought_state: summary.thought_state,
        thought_source: summary.thought_source,
        rest_state: summary.rest_state,
        commit_candidate: summary.commit_candidate,
        action_cues: summary.action_cues,
        objective_fingerprint: None,
        thought_updated_at: summary.thought_updated_at,
        token_count: summary.token_count,
        context_limit: summary.context_limit,
        last_activity_at: summary.last_activity_at,
    };

    if let Some(thought_data) = thought_data {
        apply_thought_snapshot_to_session_info(&mut info, thought_data);
    }

    info
}

fn apply_thought_snapshot_to_session_info(info: &mut SessionInfo, thought_data: &ThoughtSnapshot) {
    if let Some(thought) = &thought_data.thought {
        info.thought = Some(thought.clone());
    }
    info.thought_state = thought_data.thought_state;
    info.thought_source = thought_data.thought_source;
    info.rest_state = thought_data.rest_state;
    info.commit_candidate = thought_data.commit_candidate;
    info.action_cues = thought_data.action_cues.clone();
    info.thought_updated_at = Some(thought_data.updated_at);
    info.objective_fingerprint = thought_data.objective_fingerprint.clone();
    info.token_count = thought_data.token_count;
    info.context_limit = thought_data.context_limit;
}

pub(super) fn tmux_names_requiring_active_pane_lookup<'a, I>(
    summaries: I,
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
) -> HashSet<String>
where
    I: IntoIterator<Item = &'a SessionSummary>,
{
    if thought_snapshots.is_empty() {
        return HashSet::new();
    }

    summaries
        .into_iter()
        .filter(|summary| summary_requires_active_pane_lookup(summary, thought_snapshots))
        .map(|summary| summary.tmux_name.clone())
        .collect()
}
