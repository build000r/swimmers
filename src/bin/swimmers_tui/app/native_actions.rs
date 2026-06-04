use super::*;

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
        session_ids: focus
            .then_some(previous_session_ids.to_vec())
            .unwrap_or_default(),
        message: Some(err),
    }
}

fn should_show_attention_group_message(
    focus: bool,
    previous_session_ids: &[String],
    next_session_ids: &[String],
) -> bool {
    focus
        .then_some(true)
        .unwrap_or(previous_session_ids != next_session_ids)
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
}
