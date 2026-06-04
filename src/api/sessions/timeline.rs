use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::sync::Arc;

use crate::api::session_git_diff::read_git_diff_for_summary;
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    LaunchTargetSummary, MermaidArtifactResponse, SessionGitDiffResponse, SessionSummary,
    SessionTimelineEvent, SessionTimelinePinned, SessionTimelinePinnedItem,
    SessionTimelineResponse,
};

use super::error_response;
use super::fetch_mermaid_artifact_response;
use super::pane_tail::{request_pane_tail, PaneTailError};
use super::structured_context::{
    agent_context_unavailable, append_context_events, context_limit_for_agent_context,
    read_agent_context_for_summary,
};

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/timeline
// ---------------------------------------------------------------------------

pub(super) async fn get_timeline(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_timeline_response(&state, &session_id).await
}

async fn fetch_timeline_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match timeline_route(session_id) {
        Ok(TimelineRoute::Remote {
            target,
            remote_session_id,
        }) => fetch_remote_timeline_response(&target, remote_session_id).await,
        Ok(TimelineRoute::Local) => fetch_local_timeline_response(state, session_id).await,
        Err(err) => err.into_response(),
    }
}

enum TimelineRoute<'a> {
    Remote {
        target: LaunchTargetSummary,
        remote_session_id: &'a str,
    },
    Local,
}

fn timeline_route(
    session_id: &str,
) -> Result<TimelineRoute<'_>, remote_sessions::RemoteSessionError> {
    Ok(match remote_sessions::denamespace_for_target(session_id)? {
        Some((target, remote_session_id)) => TimelineRoute::Remote {
            target,
            remote_session_id,
        },
        None => TimelineRoute::Local,
    })
}

async fn fetch_remote_timeline_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Response {
    match remote_sessions::fetch_remote_timeline(target, remote_session_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn fetch_local_timeline_response(state: &Arc<AppState>, session_id: &str) -> Response {
    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
        Err(err) => {
            tracing::error!("timeline summary lookup failed: {err}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            );
        }
    };

    let response = build_timeline_response(state, summary).await;
    (StatusCode::OK, Json(response)).into_response()
}

async fn build_timeline_response(
    state: &Arc<AppState>,
    summary: SessionSummary,
) -> SessionTimelineResponse {
    let session_id = summary.session_id.clone();
    let cwd = summary.cwd.clone();
    let tool = summary.tool.clone();
    let context = read_agent_context_for_summary(summary.clone())
        .await
        .unwrap_or_else(|err| {
            tracing::warn!("timeline context read failed: {err}");
            agent_context_unavailable(
                session_id.clone(),
                tool.clone(),
                cwd.clone(),
                summary.token_count,
                context_limit_for_agent_context(&tool, summary.context_limit),
                "structured context could not be read",
            )
        });
    let git_diff = read_git_diff_for_summary(summary.clone()).await;
    let pane_tail = request_pane_tail(state, &session_id).await;
    let artifact = fetch_mermaid_artifact_response(state, &session_id)
        .await
        .ok();

    let mut builder = TimelineBuilder::default();
    let mut pinned = SessionTimelinePinned::default();

    append_context_events(&mut builder, &mut pinned, &context);
    append_git_diff_event(&mut builder, &mut pinned, &git_diff);
    append_pane_tail_event(&mut builder, &mut pinned, pane_tail);
    append_artifact_event(&mut builder, &mut pinned, artifact.as_ref());

    SessionTimelineResponse {
        session_id,
        available: true,
        cwd,
        tool,
        events: builder.events,
        pinned,
        message: None,
    }
}

#[derive(Default)]
pub(super) struct TimelineBuilder {
    next_order: u64,
    pub(super) events: Vec<SessionTimelineEvent>,
}

impl TimelineBuilder {
    pub(super) fn push(
        &mut self,
        id: impl Into<String>,
        kind: impl Into<String>,
        source: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> String {
        self.next_order += 1;
        let id = id.into();
        self.events.push(SessionTimelineEvent {
            id: id.clone(),
            kind: kind.into(),
            source: source.into(),
            title: title.into(),
            summary: summary.into(),
            timestamp: None,
            order: Some(self.next_order),
            detail,
        });
        id
    }
}

pub(super) fn pinned_item(
    title: impl Into<String>,
    summary: impl Into<String>,
    source: impl Into<String>,
    event_id: impl Into<String>,
) -> SessionTimelinePinnedItem {
    SessionTimelinePinnedItem {
        title: title.into(),
        summary: summary.into(),
        source: source.into(),
        event_id: Some(event_id.into()),
    }
}

pub(super) fn timeline_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\r', "").trim().to_string();
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut excerpt = normalized.chars().take(max_chars).collect::<String>();
    excerpt.push_str("...");
    excerpt
}

fn append_git_diff_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    git_diff: &SessionGitDiffResponse,
) {
    let summary = git_diff_timeline_summary(git_diff);
    let detail = git_diff_timeline_detail(git_diff);
    let event_id = builder.push(
        "git-diff",
        "diff",
        "git-diff",
        "Diffs",
        timeline_excerpt(&summary, 180),
        detail,
    );
    pinned.diff = Some(pinned_item("Diffs", summary, "git-diff", event_id));
}

pub(super) fn git_diff_timeline_summary(git_diff: &SessionGitDiffResponse) -> String {
    if !git_diff.available {
        return git_diff
            .message
            .clone()
            .unwrap_or_else(|| "git diff unavailable".to_string());
    }

    if git_diff_has_no_changes(git_diff) {
        return "clean".to_string();
    }

    if git_diff.truncated {
        "dirty, truncated".to_string()
    } else {
        "dirty".to_string()
    }
}

pub(super) fn git_diff_has_no_changes(git_diff: &SessionGitDiffResponse) -> bool {
    [
        git_diff.status_short.as_str(),
        git_diff.unstaged_diff.as_str(),
        git_diff.staged_diff.as_str(),
    ]
    .into_iter()
    .all(str_is_blank)
}

fn str_is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

pub(super) fn git_diff_timeline_detail(git_diff: &SessionGitDiffResponse) -> Option<String> {
    let detail = [
        git_diff.status_short.as_str(),
        git_diff.staged_diff.as_str(),
        git_diff.unstaged_diff.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n");

    (!detail.is_empty()).then(|| timeline_excerpt(&detail, 1200))
}

fn append_pane_tail_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    pane_tail: Result<String, PaneTailError>,
) {
    let (summary, detail) = match pane_tail {
        Ok(text) => {
            let line_count = text.trim_end().lines().count();
            let summary = if line_count == 0 {
                "empty".to_string()
            } else {
                format!("{line_count} lines")
            };
            (
                summary,
                (!text.trim().is_empty()).then(|| timeline_excerpt(&text, 1200)),
            )
        }
        Err(err) => (err.message().to_string(), None),
    };
    let event_id = builder.push(
        "pane-tail",
        "pane_tail",
        "pane-tail",
        "Recent output",
        summary.clone(),
        detail,
    );
    pinned.pane_tail = Some(pinned_item("Recent output", summary, "pane-tail", event_id));
}

pub(super) fn append_artifact_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    artifact: Option<&MermaidArtifactResponse>,
) {
    let content = artifact_timeline_content(artifact);
    let event_id = builder.push(
        "artifact",
        "artifact",
        "mermaid-artifact",
        "Artifacts",
        timeline_excerpt(&content.summary, 180),
        content.detail.map(|detail| timeline_excerpt(&detail, 1200)),
    );
    pinned.artifact = Some(pinned_item(
        "Artifacts",
        content.summary,
        "mermaid-artifact",
        event_id,
    ));
}

struct ArtifactTimelineContent {
    summary: String,
    detail: Option<String>,
}

fn artifact_timeline_content(
    artifact: Option<&MermaidArtifactResponse>,
) -> ArtifactTimelineContent {
    match artifact {
        Some(artifact) if artifact.available => available_artifact_timeline_content(artifact),
        Some(artifact) => unavailable_artifact_timeline_content(artifact),
        None => artifact_unavailable_timeline_content(),
    }
}

fn available_artifact_timeline_content(
    artifact: &MermaidArtifactResponse,
) -> ArtifactTimelineContent {
    ArtifactTimelineContent {
        summary: available_artifact_summary(artifact),
        detail: artifact.source.clone(),
    }
}

fn available_artifact_summary(artifact: &MermaidArtifactResponse) -> String {
    let plan_count = artifact.plan_files.as_ref().map_or(0, Vec::len);
    if plan_count > 0 {
        format!("{plan_count} plan files")
    } else {
        artifact
            .path
            .clone()
            .unwrap_or_else(|| "artifact available".to_string())
    }
}

fn unavailable_artifact_timeline_content(
    artifact: &MermaidArtifactResponse,
) -> ArtifactTimelineContent {
    ArtifactTimelineContent {
        summary: artifact
            .error
            .clone()
            .unwrap_or_else(|| "artifact unavailable".to_string()),
        detail: None,
    }
}

fn artifact_unavailable_timeline_content() -> ArtifactTimelineContent {
    ArtifactTimelineContent {
        summary: "artifact unavailable".to_string(),
        detail: None,
    }
}
