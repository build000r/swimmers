use super::*;

impl<C: TuiApi> App<C> {
    pub(super) fn apply_attention_group_result(
        &mut self,
        focus: bool,
        response: Result<NativeAttentionGroupOpenResponse, String>,
    ) {
        match response {
            Ok(response) => {
                let previous = self.attention_group_session_ids.clone();
                self.attention_group_session_ids = response.session_ids.clone();
                if focus || previous != response.session_ids {
                    let mut message = format!(
                        "{} attention group: {} sessions",
                        response.status, response.session_count
                    );
                    if focus && !response.focused {
                        if let Some(command) = response.attach_command.as_deref() {
                            message.push_str(" | ");
                            message.push_str(command);
                        }
                    }
                    self.set_message(message);
                }
            }
            Err(err) => {
                if !focus {
                    self.attention_group_session_ids.clear();
                }
                self.set_message(err);
            }
        }
    }

    pub(crate) fn open_repo_in_editor(&mut self, cwd: &str) {
        let repo_label = path_tail_label(cwd).unwrap_or_else(|| cwd.to_string());
        match ProcessCommand::new("code")
            .arg(".")
            .current_dir(cwd)
            .spawn()
        {
            Ok(_) => self.set_message(format!("code . -> {repo_label}")),
            Err(err) => self.set_message(format!("failed to run code .: {err}")),
        }
    }
}
