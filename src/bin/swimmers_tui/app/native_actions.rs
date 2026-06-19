use super::*;
use swimmers::types::SessionEnvironmentScope;

const EDITOR_COMMAND: &str = "code";
const EDITOR_ARG: &str = ".";

impl<C: TuiApi> App<C> {
    pub(super) fn apply_attention_group_result(
        &mut self,
        focus: bool,
        response: Result<NativeAttentionGroupOpenResponse, String>,
    ) {
        let plan = plan_attention_group_result(focus, &self.attention_group_session_ids, response);
        self.attention_group_session_ids = plan.session_ids;
        if let Some(message) = plan.message {
            self.set_message(message);
        }
    }

    pub(crate) fn open_repo_in_editor(&mut self, cwd: &str) {
        let plan = plan_open_repo_in_editor(cwd);
        let result = spawn_open_repo_in_editor(&plan);
        self.set_message(editor_spawn_message(result, plan.success_message));
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AttentionGroupResultPlan {
    session_ids: Vec<String>,
    message: Option<String>,
}

fn plan_attention_group_result(
    focus: bool,
    previous_session_ids: &[String],
    response: Result<NativeAttentionGroupOpenResponse, String>,
) -> AttentionGroupResultPlan {
    response
        .map(|response| plan_attention_group_success(focus, previous_session_ids, response))
        .unwrap_or_else(|err| plan_attention_group_error(focus, previous_session_ids, err))
}

fn plan_attention_group_success(
    focus: bool,
    previous_session_ids: &[String],
    response: NativeAttentionGroupOpenResponse,
) -> AttentionGroupResultPlan {
    let message =
        should_show_attention_group_message(focus, previous_session_ids, &response.session_ids)
            .then_some(attention_group_success_message(focus, &response));

    AttentionGroupResultPlan {
        session_ids: response.session_ids,
        message,
    }
}

fn plan_attention_group_error(
    focus: bool,
    previous_session_ids: &[String],
    err: String,
) -> AttentionGroupResultPlan {
    AttentionGroupResultPlan {
        session_ids: if focus {
            previous_session_ids.to_vec()
        } else {
            Vec::new()
        },
        message: Some(err),
    }
}

fn should_show_attention_group_message(
    focus: bool,
    previous_session_ids: &[String],
    next_session_ids: &[String],
) -> bool {
    focus || previous_session_ids != next_session_ids
}

fn attention_group_success_message(
    focus: bool,
    response: &NativeAttentionGroupOpenResponse,
) -> String {
    format!(
        "{} attention group: {} sessions{}",
        response.status,
        response.session_count,
        attention_group_attach_suffix(focus, response.focused, response.attach_command.as_deref())
    )
}

fn attention_group_attach_suffix(
    focus: bool,
    focused: bool,
    attach_command: Option<&str>,
) -> String {
    focus
        .then_some(())
        .filter(|_| !focused)
        .and(attach_command)
        .map(prepend_attention_attach_command)
        .unwrap_or_default()
}

fn prepend_attention_attach_command(command: &str) -> String {
    format!(" | {command}")
}

pub(super) fn remote_native_handoff_message(session: &SessionSummary) -> Option<String> {
    if session.environment.scope != SessionEnvironmentScope::Remote {
        return None;
    }

    let target = first_non_empty([
        Some(session.environment.display_host.as_str()),
        Some(session.environment.target_label.as_str()),
        Some(session.environment.target_id.as_str()),
    ])
    .unwrap_or("remote target");
    let mode = backend_mode_label(first_non_empty([session
        .environment
        .launch_source
        .as_deref()]));
    let remote_session_id = first_non_empty([
        session.environment.remote_session_id.as_deref(),
        Some(session.session_id.as_str()),
        Some(session.tmux_name.as_str()),
    ])
    .unwrap_or("selected session");
    let cwd = first_non_empty([
        session.environment.remote_cwd.as_deref(),
        session.environment.canonical_cwd.as_deref(),
        Some(session.cwd.as_str()),
    ]);
    let target_id = session.environment.target_id.trim();
    let target_suffix = if target_id.is_empty() || target_id == target {
        String::new()
    } else {
        format!(" ({target_id})")
    };
    let cwd_suffix = cwd.map(|cwd| format!(" @ {cwd}")).unwrap_or_default();

    Some(format!(
        "remote handoff: local native open cannot open this remote terminal; open Swimmers on {target}{target_suffix} for {remote_session_id}{cwd_suffix} via {mode}"
    ))
}

fn first_non_empty<'a, const N: usize>(values: [Option<&'a str>; N]) -> Option<&'a str> {
    values
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
}

fn backend_mode_label(value: Option<&str>) -> String {
    match value.unwrap_or("remote").trim() {
        "remote_swimmers_api" => "remote Swimmers API".to_string(),
        "" => "remote".to_string(),
        other => other.replace(['_', '-'], " "),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OpenRepoInEditorPlan<'a> {
    command: &'static str,
    arg: &'static str,
    cwd: &'a str,
    success_message: String,
}

fn plan_open_repo_in_editor(cwd: &str) -> OpenRepoInEditorPlan<'_> {
    OpenRepoInEditorPlan {
        command: EDITOR_COMMAND,
        arg: EDITOR_ARG,
        cwd,
        success_message: editor_success_message(cwd),
    }
}

fn spawn_open_repo_in_editor(plan: &OpenRepoInEditorPlan<'_>) -> std::io::Result<()> {
    ProcessCommand::new(plan.command)
        .arg(plan.arg)
        .current_dir(plan.cwd)
        .spawn()
        .map(drop)
}

fn editor_success_message(cwd: &str) -> String {
    format!("code . -> {}", editor_repo_label(cwd))
}

fn editor_repo_label(cwd: &str) -> String {
    path_tail_label(cwd).unwrap_or(cwd.to_string())
}

fn editor_spawn_message(result: std::io::Result<()>, success_message: String) -> String {
    result
        .map(|()| success_message)
        .unwrap_or_else(|err| editor_error_message(&err))
}

fn editor_error_message(err: &std::io::Error) -> String {
    format!("failed to run code .: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn attention_response(
        session_ids: Vec<String>,
        status: &str,
        focused: bool,
        attach_command: Option<&str>,
    ) -> NativeAttentionGroupOpenResponse {
        NativeAttentionGroupOpenResponse {
            session_id: "attention-group".to_string(),
            tmux_name: "swimmers-attention".to_string(),
            session_count: session_ids.len(),
            session_ids,
            backlog_session_ids: Vec::new(),
            status: status.to_string(),
            focused,
            pane_id: None,
            attach_command: attach_command.map(str::to_string),
        }
    }

    #[test]
    fn attention_group_success_plan_reports_changed_sessions() {
        let plan = plan_attention_group_result(
            false,
            &ids(&["sess-old"]),
            Ok(attention_response(
                ids(&["sess-1", "sess-2"]),
                "refreshed",
                true,
                None,
            )),
        );

        assert_eq!(plan.session_ids, ids(&["sess-1", "sess-2"]));
        assert_eq!(
            plan.message.as_deref(),
            Some("refreshed attention group: 2 sessions")
        );
    }

    #[test]
    fn attention_group_success_plan_suppresses_unchanged_nonfocus_message() {
        let previous = ids(&["sess-1"]);
        let plan = plan_attention_group_result(
            false,
            &previous,
            Ok(attention_response(
                previous.clone(),
                "refreshed",
                true,
                None,
            )),
        );

        assert_eq!(plan.session_ids, previous);
        assert_eq!(plan.message, None);
    }

    #[test]
    fn attention_group_focus_plan_appends_attach_command_when_focus_is_unavailable() {
        let plan = plan_attention_group_result(
            true,
            &ids(&["sess-1"]),
            Ok(attention_response(
                ids(&["sess-1"]),
                "swapped",
                false,
                Some("tmux attach -t swimmers-attention"),
            )),
        );

        assert_eq!(plan.session_ids, ids(&["sess-1"]));
        assert_eq!(
            plan.message.as_deref(),
            Some("swapped attention group: 1 sessions | tmux attach -t swimmers-attention")
        );
    }

    #[test]
    fn attention_group_error_plan_clears_nonfocus_sessions_and_reports_error() {
        let plan =
            plan_attention_group_result(false, &ids(&["sess-1"]), Err("native failed".to_string()));

        assert_eq!(plan.session_ids, Vec::<String>::new());
        assert_eq!(plan.message.as_deref(), Some("native failed"));
    }

    #[test]
    fn attention_group_error_plan_keeps_focus_sessions_and_reports_error() {
        let previous = ids(&["sess-1", "sess-2"]);
        let plan = plan_attention_group_result(true, &previous, Err("native failed".to_string()));

        assert_eq!(plan.session_ids, previous);
        assert_eq!(plan.message.as_deref(), Some("native failed"));
    }

    #[test]
    fn open_repo_editor_plan_preserves_code_dot_spawn_spec_and_success_message() {
        let plan = plan_open_repo_in_editor("/tmp/swimmers");

        assert_eq!(plan.command, "code");
        assert_eq!(plan.arg, ".");
        assert_eq!(plan.cwd, "/tmp/swimmers");
        assert_eq!(plan.success_message, "code . -> swimmers");
    }

    #[test]
    fn open_repo_editor_error_message_preserves_original_text() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing code");

        assert_eq!(
            editor_spawn_message(Err(err), "code . -> swimmers".to_string()),
            "failed to run code .: missing code"
        );
    }

    #[test]
    fn remote_native_handoff_message_describes_target_without_secret_fields() {
        let target = LaunchTargetSummary {
            id: " skillbox ".to_string(),
            label: "Skillbox devbox".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://100.64.1.2:3210/?token=secret".to_string()),
            auth_token_env: Some("SWIMMERS_TOKEN".to_string()),
            path_mappings: Vec::new(),
        };
        let mut session = SessionSummary {
            session_id: "skillbox::sess-7".to_string(),
            tmux_name: "[Skillbox devbox] 7".to_string(),
            state: SessionState::Idle,
            current_command: None,
            state_evidence: swimmers::types::StateEvidence::new("osc133_prompt"),
            cwd: "/Users/b/repos/swimmers".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: swimmers::types::ThoughtState::Holding,
            thought_source: swimmers::types::ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
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
        };
        session.environment = swimmers::types::SessionEnvironmentSummary::remote(
            &target,
            "sess-7",
            "/srv/skillbox/repos/swimmers",
            Some("/Users/b/repos/swimmers".to_string()),
            "remote_swimmers_api",
        );

        let message = remote_native_handoff_message(&session).expect("remote handoff");

        assert_eq!(
            message,
            "remote handoff: local native open cannot open this remote terminal; open Swimmers on Skillbox devbox (skillbox) for sess-7 @ /srv/skillbox/repos/swimmers via remote Swimmers API"
        );
        assert!(!message.contains("secret"));
        assert!(!message.contains("SWIMMERS_TOKEN"));
    }
}
