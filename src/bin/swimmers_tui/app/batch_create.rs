use super::*;

struct BatchCreateSuccess {
    session: SessionSummary,
    repo_theme: Option<RepoTheme>,
}

struct BatchCreatePartition {
    total: usize,
    successes: Vec<BatchCreateSuccess>,
    first_error: Option<String>,
}

struct AppliedBatchCreate {
    success_count: usize,
    last_tmux_name: Option<String>,
    last_session_id: Option<String>,
}

struct BatchCreateCompletion {
    total: usize,
    success_count: usize,
    last_tmux_name: Option<String>,
    last_session_id: Option<String>,
    first_error: Option<String>,
}

impl BatchCreateCompletion {
    fn should_close_picker(&self) -> bool {
        self.success_count > 0
    }

    fn should_show_batch_thoughts(&self) -> bool {
        self.success_count > 1
    }

    fn message(self) -> String {
        if self.success_count == 0 {
            return batch_create_failure_message(self.first_error);
        }

        if self.success_count != self.total {
            return batch_create_partial_success_message(
                self.success_count,
                self.total,
                self.first_error,
            );
        }

        batch_create_success_message(self.success_count, self.last_tmux_name)
    }
}

fn batch_create_failure_message(first_error: Option<String>) -> String {
    first_error.unwrap_or_else(|| "batch create failed".to_string())
}

fn batch_create_partial_success_message(
    success_count: usize,
    total: usize,
    first_error: Option<String>,
) -> String {
    format!(
        "created {success_count}/{total}; {}",
        first_error.unwrap_or_else(|| "some sessions failed".to_string())
    )
}

fn batch_create_success_message(success_count: usize, last_tmux_name: Option<String>) -> String {
    match last_tmux_name {
        Some(name) if success_count == 1 => format!("created {name}"),
        _ => format!("created {success_count} sessions"),
    }
}

fn partition_batch_create_results(response: CreateSessionsBatchResponse) -> BatchCreatePartition {
    let total = response.results.len();
    let mut successes = Vec::new();
    let mut first_error = None;

    for result in response.results {
        match (result.ok, result.session) {
            (true, Some(session)) => successes.push(BatchCreateSuccess {
                session,
                repo_theme: result.repo_theme,
            }),
            (false, _) if first_error.is_none() => {
                first_error = Some(batch_create_error_message(&result.cwd, result.error));
            }
            _ => {}
        }
    }

    BatchCreatePartition {
        total,
        successes,
        first_error,
    }
}

fn batch_create_error_message(cwd: &str, error: Option<ErrorResponse>) -> String {
    let detail = error
        .and_then(|error| error.message)
        .unwrap_or_else(|| "unknown error".to_string());
    format!("{}: {detail}", shorten_path(cwd, 32))
}

impl<C: TuiApi> App<C> {
    pub(super) fn apply_batch_create_response(
        &mut self,
        field: Rect,
        response: CreateSessionsBatchResponse,
    ) {
        let partition = partition_batch_create_results(response);
        let completion = self.apply_batch_create_successes(field, partition);

        self.apply_batch_create_selection(completion.last_session_id.clone());
        self.finish_batch_create(completion);
    }

    fn apply_batch_create_successes(
        &mut self,
        field: Rect,
        partition: BatchCreatePartition,
    ) -> BatchCreateCompletion {
        let mut applied = AppliedBatchCreate {
            success_count: 0,
            last_tmux_name: None,
            last_session_id: None,
        };

        for success in partition.successes {
            applied.last_tmux_name = Some(success.session.tmux_name.clone());
            applied.last_session_id = Some(success.session.session_id.clone());
            self.remember_repo_theme(&success.session, success.repo_theme);
            self.upsert_session(success.session, field);
            applied.success_count += 1;
        }

        BatchCreateCompletion {
            total: partition.total,
            success_count: applied.success_count,
            last_tmux_name: applied.last_tmux_name,
            last_session_id: applied.last_session_id,
            first_error: partition.first_error,
        }
    }

    fn apply_batch_create_selection(&mut self, selected_session_id: Option<String>) {
        let Some(session_id) = selected_session_id else {
            return;
        };

        self.selected_id = Some(session_id);
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    fn finish_batch_create(&mut self, completion: BatchCreateCompletion) {
        if completion.should_close_picker() {
            self.close_picker();
        }
        if completion.should_show_batch_thoughts() {
            self.thought_group_by = ThoughtGroupBy::Batch;
            self.thought_show_all = true;
        }
        self.set_message(completion.message());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(
        total: usize,
        success_count: usize,
        last_tmux_name: Option<&str>,
        first_error: Option<&str>,
    ) -> BatchCreateCompletion {
        BatchCreateCompletion {
            total,
            success_count,
            last_tmux_name: last_tmux_name.map(str::to_string),
            last_session_id: None,
            first_error: first_error.map(str::to_string),
        }
    }

    #[test]
    fn batch_create_completion_message_reports_zero_success_error() {
        assert_eq!(
            completion(2, 0, None, Some("/tmp/app: denied")).message(),
            "/tmp/app: denied"
        );
    }

    #[test]
    fn batch_create_completion_message_uses_fallback_for_zero_success_without_error() {
        assert_eq!(
            completion(1, 0, None, None).message(),
            "batch create failed"
        );
    }

    #[test]
    fn batch_create_completion_message_reports_partial_success_with_error() {
        assert_eq!(
            completion(3, 2, Some("third"), Some("/tmp/app: denied")).message(),
            "created 2/3; /tmp/app: denied"
        );
    }

    #[test]
    fn batch_create_completion_message_reports_partial_success_fallback() {
        assert_eq!(
            completion(3, 2, Some("third"), None).message(),
            "created 2/3; some sessions failed"
        );
    }

    #[test]
    fn batch_create_completion_message_reports_single_success_by_name() {
        assert_eq!(
            completion(1, 1, Some("swimmers"), None).message(),
            "created swimmers"
        );
    }

    #[test]
    fn batch_create_completion_message_reports_multi_success_count() {
        assert_eq!(
            completion(2, 2, Some("second"), None).message(),
            "created 2 sessions"
        );
    }

    #[test]
    fn batch_create_error_message_uses_short_path_and_unknown_fallback() {
        assert_eq!(
            batch_create_error_message(
                "/Users/tester/projects/very-long-repo-name",
                Some(ErrorResponse {
                    code: "spawn_failed".to_string(),
                    message: Some("permission denied".to_string()),
                }),
            ),
            ".../projects/very-long-repo-name: permission denied"
        );
        assert_eq!(
            batch_create_error_message("/tmp/app", None),
            "/tmp/app: unknown error"
        );
    }
}
