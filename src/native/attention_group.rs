use std::collections::VecDeque;
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::tmux_target::{exact_pane_target, exact_session_target, TmuxTarget};
use crate::types::{
    AttentionGroupLayout, GhosttyOpenMode, NativeAttentionGroupOpenResponse, NativeDesktopApp,
    SessionSummary,
};

pub(super) const ATTENTION_GROUP_SESSION_ID: &str = "attention-group";
pub(super) const ATTENTION_GROUP_TMUX_NAME: &str = "swimmers-attention";
const ATTENTION_GROUP_PANE_TITLE_PREFIX: &str = "swimmers-attention:";

pub async fn open_native_attention_group(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    sessions: &[SessionSummary],
    focus: bool,
    layout: AttentionGroupLayout,
) -> Result<NativeAttentionGroupOpenResponse> {
    if sessions.is_empty() {
        return Err(anyhow!("no sessions are waiting for operator input"));
    }

    let tmux_path = super::resolve_tmux_binary()?;
    sync_attention_group_tmux_session(&tmux_path, sessions, layout).await?;
    let attach_command = attention_group_attach_command();
    let open_result = if focus {
        Some(
            super::open_native_session(
                app,
                attention_group_ghostty_mode(app, ghostty_mode),
                ATTENTION_GROUP_SESSION_ID,
                ATTENTION_GROUP_TMUX_NAME,
                &TmuxTarget::Default,
                "",
            )
            .await?,
        )
    } else {
        None
    };

    Ok(NativeAttentionGroupOpenResponse {
        session_id: ATTENTION_GROUP_SESSION_ID.to_string(),
        tmux_name: ATTENTION_GROUP_TMUX_NAME.to_string(),
        session_count: sessions.len(),
        session_ids: sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect(),
        backlog_session_ids: Vec::new(),
        status: open_result
            .as_ref()
            .map(|result| result.status.clone())
            .unwrap_or_else(|| "refreshed".to_string()),
        focused: focus,
        pane_id: open_result.and_then(|result| result.pane_id),
        attach_command: Some(attach_command),
    })
}

pub async fn clear_native_attention_group() -> Result<NativeAttentionGroupOpenResponse> {
    let tmux_path = super::resolve_tmux_binary()?;
    let session_target = attention_group_session_target();
    let _ = super::run_tmux_status(&tmux_path, &["kill-session", "-t", &session_target]).await;

    Ok(NativeAttentionGroupOpenResponse {
        session_id: ATTENTION_GROUP_SESSION_ID.to_string(),
        tmux_name: ATTENTION_GROUP_TMUX_NAME.to_string(),
        session_count: 0,
        session_ids: Vec::new(),
        backlog_session_ids: Vec::new(),
        status: "cleared".to_string(),
        focused: false,
        pane_id: None,
        attach_command: Some(attention_group_attach_command()),
    })
}

pub fn attention_group_attach_command() -> String {
    format!("tmux attach -t {ATTENTION_GROUP_TMUX_NAME}")
}

fn attention_group_ghostty_mode(
    app: NativeDesktopApp,
    configured_mode: GhosttyOpenMode,
) -> GhosttyOpenMode {
    match app {
        NativeDesktopApp::Ghostty => GhosttyOpenMode::Window,
        NativeDesktopApp::Iterm => configured_mode,
    }
}

async fn sync_attention_group_tmux_session(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let session_target = attention_group_session_target();
    if super::run_tmux_status(tmux_path, &["has-session", "-t", &session_target])
        .await
        .is_err()
    {
        return rebuild_attention_group_tmux_session(tmux_path, sessions, layout).await;
    }

    replace_attention_group_tmux_panes(tmux_path, sessions, layout).await
}

async fn rebuild_attention_group_tmux_session(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let session_target = attention_group_session_target();
    let pane_target = attention_group_pane_target();
    let _ = super::run_tmux_status(tmux_path, &["kill-session", "-t", &session_target]).await;

    let plan = plan_attention_group_rebuild(sessions)?;
    create_attention_group_tmux_session(tmux_path, plan.first).await?;
    split_remaining_attention_group_panes(tmux_path, &pane_target, plan.remaining, layout).await?;
    tile_attention_group_tmux_session(tmux_path, &pane_target, layout).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct AttentionGroupRebuildPlan<'a> {
    first: &'a SessionSummary,
    remaining: &'a [SessionSummary],
}

fn plan_attention_group_rebuild(
    sessions: &[SessionSummary],
) -> Result<AttentionGroupRebuildPlan<'_>> {
    let (first, remaining) = sessions
        .split_first()
        .ok_or_else(|| anyhow!("no sessions are waiting for operator input"))?;
    Ok(AttentionGroupRebuildPlan { first, remaining })
}

async fn create_attention_group_tmux_session(
    tmux_path: &Path,
    first: &SessionSummary,
) -> Result<()> {
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            ATTENTION_GROUP_TMUX_NAME,
            "-n",
            "attention",
            &build_attention_group_attach_command(first, tmux_path),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to create tmux session {ATTENTION_GROUP_TMUX_NAME}: {err}"))?;
    if let Some(pane_id) = first_output_line(output.stdout.as_slice())? {
        set_attention_group_pane_title(tmux_path, &pane_id, &first.session_id).await?;
    }
    Ok(())
}

async fn split_remaining_attention_group_panes(
    tmux_path: &Path,
    pane_target: &str,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    for session in sessions {
        split_attention_group_pane(tmux_path, pane_target, session).await?;
        tile_attention_group_tmux_session(tmux_path, pane_target, layout).await?;
    }
    Ok(())
}

async fn replace_attention_group_tmux_panes(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let pane_target = attention_group_pane_target();
    let panes = list_attention_group_panes(tmux_path, &pane_target).await?;
    let desired = desired_attention_group_panes(sessions, tmux_path);

    if attention_group_panes_match_desired(&panes, &desired) {
        return Ok(());
    }

    let replacement = plan_attention_group_pane_replacement(panes, &desired);
    let changed = replacement.changes_panes();
    apply_attention_group_pane_replacement(tmux_path, &pane_target, replacement).await?;

    if changed {
        tile_attention_group_tmux_session(tmux_path, &pane_target, layout).await?;
    }
    Ok(())
}

struct DesiredAttentionGroupPane<'a> {
    session: &'a SessionSummary,
    command: String,
}

fn desired_attention_group_panes<'a>(
    sessions: &'a [SessionSummary],
    tmux_path: &Path,
) -> Vec<DesiredAttentionGroupPane<'a>> {
    sessions
        .iter()
        .map(|session| DesiredAttentionGroupPane {
            session,
            command: build_attention_group_attach_command(session, tmux_path),
        })
        .collect()
}

fn attention_group_panes_match_desired(
    panes: &[AttentionGroupPane],
    desired: &[DesiredAttentionGroupPane<'_>],
) -> bool {
    panes.len() == desired.len()
        && panes
            .iter()
            .zip(desired.iter())
            .all(|(pane, desired)| pane.start_command == desired.command)
}

struct AttentionGroupPaneReplacement<'a> {
    missing: Vec<&'a SessionSummary>,
    stale_panes: VecDeque<AttentionGroupPane>,
}

impl AttentionGroupPaneReplacement<'_> {
    fn changes_panes(&self) -> bool {
        !self.missing.is_empty() || !self.stale_panes.is_empty()
    }
}

fn plan_attention_group_pane_replacement<'a>(
    panes: Vec<AttentionGroupPane>,
    desired: &[DesiredAttentionGroupPane<'a>],
) -> AttentionGroupPaneReplacement<'a> {
    let mut available = panes;
    let mut missing = Vec::new();
    for desired_pane in desired {
        if let Some(index) = available
            .iter()
            .position(|pane| pane.start_command == desired_pane.command)
        {
            available.remove(index);
        } else {
            missing.push(desired_pane.session);
        }
    }

    AttentionGroupPaneReplacement {
        missing,
        stale_panes: VecDeque::from(available),
    }
}

async fn apply_attention_group_pane_replacement(
    tmux_path: &Path,
    pane_target: &str,
    replacement: AttentionGroupPaneReplacement<'_>,
) -> Result<()> {
    let mut stale_panes = replacement.stale_panes;
    replace_missing_attention_group_panes(
        tmux_path,
        pane_target,
        replacement.missing,
        &mut stale_panes,
    )
    .await?;
    clear_stale_attention_group_panes(tmux_path, stale_panes).await
}

async fn replace_missing_attention_group_panes(
    tmux_path: &Path,
    pane_target: &str,
    missing: Vec<&SessionSummary>,
    stale_panes: &mut VecDeque<AttentionGroupPane>,
) -> Result<()> {
    for session in missing {
        if let Some(pane) = stale_panes.pop_front() {
            respawn_attention_group_pane(tmux_path, &pane.pane_id, session).await?;
        } else {
            split_attention_group_pane(tmux_path, pane_target, session).await?;
        }
    }
    Ok(())
}

async fn clear_stale_attention_group_panes(
    tmux_path: &Path,
    stale_panes: VecDeque<AttentionGroupPane>,
) -> Result<()> {
    for pane in stale_panes {
        super::run_tmux_status(tmux_path, &["kill-pane", "-t", &pane.pane_id])
            .await
            .map_err(|err| anyhow!("failed to clear stale attention group pane: {err}"))?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct AttentionGroupPane {
    pane_id: String,
    start_command: String,
}

async fn list_attention_group_panes(
    tmux_path: &Path,
    pane_target: &str,
) -> Result<Vec<AttentionGroupPane>> {
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "list-panes",
            "-t",
            pane_target,
            "-F",
            "#{pane_id}\t#{pane_start_command}",
        ],
    )
    .await?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| anyhow!("tmux list-panes returned non-UTF-8 output: {err}"))?;
    let panes = stdout
        .lines()
        .filter_map(parse_attention_group_pane_line)
        .collect::<Vec<_>>();
    if panes.is_empty() {
        return Err(anyhow!("attention group has no panes"));
    }
    Ok(panes)
}

fn parse_attention_group_pane_line(line: &str) -> Option<AttentionGroupPane> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (pane_id, start_command) = trimmed
        .split_once('\t')
        .map(|(pane_id, command)| (pane_id.trim(), command.trim()))
        .unwrap_or((trimmed, ""));
    if pane_id.is_empty() {
        return None;
    }
    Some(AttentionGroupPane {
        pane_id: pane_id.to_string(),
        start_command: start_command.to_string(),
    })
}

async fn respawn_attention_group_pane(
    tmux_path: &Path,
    pane_id: &str,
    session: &SessionSummary,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "respawn-pane",
            "-k",
            "-t",
            pane_id,
            &build_attention_group_attach_command(session, tmux_path),
        ],
    )
    .await
    .map_err(|err| {
        anyhow!(
            "failed to refresh attention group pane for {}: {err}",
            session.tmux_name
        )
    })?;
    set_attention_group_pane_title(tmux_path, pane_id, &session.session_id).await
}

async fn split_attention_group_pane(
    tmux_path: &Path,
    pane_target: &str,
    session: &SessionSummary,
) -> Result<()> {
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "split-window",
            "-t",
            pane_target,
            "-P",
            "-F",
            "#{pane_id}",
            &build_attention_group_attach_command(session, tmux_path),
        ],
    )
    .await
    .map_err(|err| {
        anyhow!(
            "failed to add {} to attention group: {err}",
            session.tmux_name
        )
    })?;
    if let Some(pane_id) = first_output_line(output.stdout.as_slice())? {
        set_attention_group_pane_title(tmux_path, &pane_id, &session.session_id).await?;
    }
    Ok(())
}

async fn set_attention_group_pane_title(
    tmux_path: &Path,
    pane_id: &str,
    session_id: &str,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "select-pane",
            "-t",
            pane_id,
            "-T",
            &attention_group_pane_title(session_id),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to label attention group pane {pane_id}: {err}"))
}

fn first_output_line(stdout: &[u8]) -> Result<Option<String>> {
    let stdout = String::from_utf8(stdout.to_vec())
        .map_err(|err| anyhow!("tmux returned non-UTF-8 output: {err}"))?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned))
}

fn attention_group_pane_title(session_id: &str) -> String {
    let sanitized = sanitize_tmux_pane_title(session_id);
    format!("{ATTENTION_GROUP_PANE_TITLE_PREFIX}{sanitized}")
}

fn sanitize_tmux_pane_title(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\0' | '\n' | '\r' | '\t'))
        .collect()
}

async fn tile_attention_group_tmux_session(
    tmux_path: &Path,
    pane_target: &str,
    layout: AttentionGroupLayout,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "select-layout",
            "-t",
            pane_target,
            attention_group_tmux_layout(layout),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to tile attention group panes: {err}"))
}

fn attention_group_tmux_layout(layout: AttentionGroupLayout) -> &'static str {
    match layout {
        AttentionGroupLayout::Tiled => "tiled",
        AttentionGroupLayout::EvenHorizontal => "even-horizontal",
        AttentionGroupLayout::EvenVertical => "even-vertical",
        AttentionGroupLayout::MainHorizontal => "main-horizontal",
        AttentionGroupLayout::MainVertical => "main-vertical",
    }
}

pub(super) fn attention_group_session_target() -> String {
    exact_session_target(ATTENTION_GROUP_TMUX_NAME)
}

pub(super) fn attention_group_pane_target() -> String {
    exact_pane_target(ATTENTION_GROUP_TMUX_NAME)
}

pub(super) fn build_attention_group_attach_command(
    session: &SessionSummary,
    tmux_path: &Path,
) -> String {
    let target_words = session
        .tmux_target
        .shell_words()
        .into_iter()
        .map(|word| super::shell_quote_token(&word))
        .collect::<Vec<_>>()
        .join(" ");
    let target_words = if target_words.is_empty() {
        String::new()
    } else {
        format!(" {target_words}")
    };
    format!(
        "exec env TMUX= {}{} attach-session -t {}",
        super::shell_quote_token(&tmux_path.to_string_lossy()),
        target_words,
        super::shell_quote_token(&exact_session_target(&session.tmux_name))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session(session_id: &str, tmux_name: &str) -> SessionSummary {
        SessionSummary::placeholder(session_id, tmux_name, chrono::Utc::now())
    }

    fn test_pane(pane_id: &str, start_command: &str) -> AttentionGroupPane {
        AttentionGroupPane {
            pane_id: pane_id.to_string(),
            start_command: start_command.to_string(),
        }
    }

    #[test]
    fn build_attention_group_attach_command_unsets_nested_tmux_and_exact_targets() {
        let session = test_session("sess-1", "team session");
        let command = build_attention_group_attach_command(&session, Path::new("/tmp/tmux"));
        assert_eq!(
            command,
            "exec env TMUX= /tmp/tmux attach-session -t '=team session'"
        );
    }

    #[test]
    fn build_attention_group_attach_command_includes_isolated_tmux_target() {
        let mut session = test_session("sess-1", "team session");
        session.tmux_target = TmuxTarget::socket_name("tiktok");
        let command = build_attention_group_attach_command(&session, Path::new("/tmp/tmux"));
        assert_eq!(
            command,
            "exec env TMUX= /tmp/tmux -L tiktok attach-session -t '=team session'"
        );
    }

    #[test]
    fn attention_group_uses_session_target_for_kill_and_pane_target_for_layout_commands() {
        assert_eq!(attention_group_session_target(), "=swimmers-attention");
        assert_eq!(attention_group_pane_target(), "=swimmers-attention:");
    }

    #[test]
    fn plan_attention_group_rebuild_rejects_empty_sessions() {
        let err = plan_attention_group_rebuild(&[]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no sessions are waiting for operator input"
        );
    }

    #[test]
    fn plan_attention_group_rebuild_uses_single_session_as_first() {
        let sessions = vec![test_session("session-a", "tmux-a")];
        let plan = plan_attention_group_rebuild(&sessions).unwrap();

        assert_eq!(plan.first.session_id, "session-a");
        assert!(plan.remaining.is_empty());
    }

    #[test]
    fn plan_attention_group_rebuild_preserves_remaining_session_order() {
        let sessions = vec![
            test_session("session-a", "tmux-a"),
            test_session("session-b", "tmux-b"),
            test_session("session-c", "tmux-c"),
        ];
        let plan = plan_attention_group_rebuild(&sessions).unwrap();

        assert_eq!(plan.first.session_id, "session-a");
        assert_eq!(
            plan.remaining
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            ["session-b", "session-c"]
        );
    }

    #[test]
    fn desired_attention_group_panes_builds_commands_in_session_order() {
        let sessions = vec![
            test_session("session-a", "tmux-a"),
            test_session("session-b", "tmux b"),
        ];
        let tmux_path = Path::new("/tmp/tmux");
        let desired = desired_attention_group_panes(&sessions, tmux_path);

        assert_eq!(
            desired
                .iter()
                .map(|pane| pane.session.session_id.as_str())
                .collect::<Vec<_>>(),
            ["session-a", "session-b"]
        );
        assert_eq!(
            desired[0].command.as_str(),
            build_attention_group_attach_command(&sessions[0], tmux_path)
        );
        assert_eq!(
            desired[1].command.as_str(),
            build_attention_group_attach_command(&sessions[1], tmux_path)
        );
    }

    #[test]
    fn attention_group_panes_match_desired_requires_exact_command_order() {
        let sessions = vec![
            test_session("session-a", "tmux-a"),
            test_session("session-b", "tmux-b"),
        ];
        let desired = desired_attention_group_panes(&sessions, Path::new("/tmp/tmux"));
        let command_a = desired[0].command.clone();
        let command_b = desired[1].command.clone();

        let exact = vec![test_pane("%1", &command_a), test_pane("%2", &command_b)];
        assert!(attention_group_panes_match_desired(&exact, &desired));

        let reordered = vec![test_pane("%2", &command_b), test_pane("%1", &command_a)];
        assert!(!attention_group_panes_match_desired(&reordered, &desired));

        let extra = vec![
            test_pane("%1", &command_a),
            test_pane("%2", &command_b),
            test_pane("%3", "stale"),
        ];
        assert!(!attention_group_panes_match_desired(&extra, &desired));
    }

    #[test]
    fn plan_attention_group_pane_replacement_preserves_missing_and_stale_order() {
        let sessions = vec![
            test_session("session-a", "tmux-a"),
            test_session("session-b", "tmux-b"),
            test_session("session-c", "tmux-c"),
        ];
        let desired = desired_attention_group_panes(&sessions, Path::new("/tmp/tmux"));
        let panes = vec![
            test_pane("%old-a", "old-a"),
            test_pane("%b", &desired[1].command),
            test_pane("%old-b", "old-b"),
        ];
        let replacement = plan_attention_group_pane_replacement(panes, &desired);

        assert_eq!(
            replacement
                .missing
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            ["session-a", "session-c"]
        );
        assert_eq!(
            replacement
                .stale_panes
                .iter()
                .map(|pane| pane.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["%old-a", "%old-b"]
        );
        assert!(replacement.changes_panes());
    }

    #[test]
    fn plan_attention_group_pane_replacement_treats_reordered_commands_as_no_change() {
        let sessions = vec![
            test_session("session-a", "tmux-a"),
            test_session("session-b", "tmux-b"),
        ];
        let desired = desired_attention_group_panes(&sessions, Path::new("/tmp/tmux"));
        let panes = vec![
            test_pane("%b", &desired[1].command),
            test_pane("%a", &desired[0].command),
        ];

        let replacement = plan_attention_group_pane_replacement(panes, &desired);

        assert!(replacement.missing.is_empty());
        assert!(replacement.stale_panes.is_empty());
        assert!(!replacement.changes_panes());
    }
}
