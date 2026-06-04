use super::*;
use futures::stream::StreamExt;
use swimmers::openrouter_models::should_rotate_openrouter_model;

pub(crate) struct ThoughtConfigActionOutcome {
    pub(crate) message: String,
    pub(crate) updated_config: Option<ThoughtConfig>,
    pub(crate) openrouter_candidates: Option<Vec<String>>,
    pub(crate) close_editor: bool,
    pub(crate) refresh_sessions: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenRouterRotationProbe {
    candidate: String,
    config: ThoughtConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThoughtConfigPasteAction {
    WaitForPendingAction,
    AppendToModel,
    Ignore,
}

fn route_thought_config_paste(
    has_pending_action: bool,
    focus: Option<ThoughtConfigEditorField>,
) -> ThoughtConfigPasteAction {
    if has_pending_action {
        return ThoughtConfigPasteAction::WaitForPendingAction;
    }

    match focus {
        Some(ThoughtConfigEditorField::Model) => ThoughtConfigPasteAction::AppendToModel,
        _ => ThoughtConfigPasteAction::Ignore,
    }
}

impl<C: TuiApi> App<C> {
    pub(super) fn apply_open_thought_config_result(
        &mut self,
        response: Result<ThoughtConfigResponse, String>,
    ) {
        match response {
            Ok(response) => {
                self.daemon_defaults_status =
                    DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref());
                self.thought_config_editor = Some(ThoughtConfigEditorState::new(
                    response.config,
                    response.daemon_defaults,
                ));
            }
            Err(err) => self.set_message(err),
        }
    }

    pub(crate) fn apply_thought_config_action_outcome(
        &mut self,
        outcome: ThoughtConfigActionOutcome,
    ) {
        let ThoughtConfigActionOutcome {
            message,
            updated_config,
            openrouter_candidates,
            close_editor,
            refresh_sessions,
        } = outcome;

        self.apply_thought_config_editor_action_updates(updated_config, openrouter_candidates);
        if close_editor {
            self.close_thought_config_editor();
        }
        if refresh_sessions {
            self.pending_refresh = None;
            self.spawn_background_refresh(false);
        }
        self.set_message(message);
    }

    fn apply_thought_config_editor_action_updates(
        &mut self,
        updated_config: Option<ThoughtConfig>,
        openrouter_candidates: Option<Vec<String>>,
    ) {
        let Some(editor) = &mut self.thought_config_editor else {
            return;
        };
        if let Some(candidates) = openrouter_candidates {
            editor.replace_openrouter_model_presets(candidates);
        }
        if let Some(config) = updated_config {
            editor.config = config;
        }
    }

    pub(crate) fn open_thought_config_editor(&mut self) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading thought config...");
        self.runtime.spawn(async move {
            let response = client.fetch_thought_config().await;
            let _ = tx.send(PendingInteractionResult::OpenThoughtConfig { response });
        });
    }

    pub(crate) fn close_thought_config_editor(&mut self) {
        self.thought_config_editor = None;
    }

    pub(crate) fn handle_thought_config_key(&mut self, key: KeyEvent, layout: WorkspaceLayout) {
        if self.pending_interaction.is_some() {
            self.handle_pending_thought_config_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc => self.close_thought_config_editor(),
            KeyCode::Up | KeyCode::BackTab => self.move_thought_config_focus(-1),
            KeyCode::Down | KeyCode::Tab => self.move_thought_config_focus(1),
            KeyCode::Left => self.adjust_thought_config_field(-1),
            KeyCode::Right | KeyCode::Char(' ') => self.adjust_thought_config_field(1),
            KeyCode::Backspace => self.pop_thought_config_model_char(),
            KeyCode::Enter => self.activate_thought_config_field(layout),
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_thought_config_model_char(ch);
            }
            _ => {}
        }
    }

    fn handle_pending_thought_config_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.close_thought_config_editor();
        } else {
            self.set_message("wait for the current action to finish");
        }
    }

    fn move_thought_config_focus(&mut self, delta: isize) {
        if let Some(editor) = &mut self.thought_config_editor {
            editor.move_focus(delta);
        }
    }

    fn focused_thought_config_model_mut(&mut self) -> Option<&mut String> {
        self.thought_config_editor
            .as_mut()
            .filter(|editor| editor.focus == ThoughtConfigEditorField::Model)
            .map(|editor| &mut editor.config.model)
    }

    fn pop_thought_config_model_char(&mut self) {
        let _ = self
            .focused_thought_config_model_mut()
            .and_then(|model| model.pop());
    }

    fn push_thought_config_model_char(&mut self, ch: char) {
        self.focused_thought_config_model_mut()
            .into_iter()
            .for_each(|model| model.push(ch));
    }

    pub(crate) fn handle_thought_config_paste(&mut self, text: &str) {
        match route_thought_config_paste(
            self.pending_interaction.is_some(),
            self.thought_config_editor
                .as_ref()
                .map(|editor| editor.focus),
        ) {
            ThoughtConfigPasteAction::WaitForPendingAction => {
                self.set_message("wait for the current action to finish");
            }
            ThoughtConfigPasteAction::AppendToModel => {
                let Some(editor) = &mut self.thought_config_editor else {
                    return;
                };
                editor.config.model.push_str(text);
            }
            ThoughtConfigPasteAction::Ignore => {}
        }
    }

    fn adjust_thought_config_field(&mut self, delta: isize) {
        let Some(editor) = &mut self.thought_config_editor else {
            return;
        };
        match editor.focus {
            ThoughtConfigEditorField::Enabled => editor.config.enabled = !editor.config.enabled,
            ThoughtConfigEditorField::Backend => editor.cycle_backend(delta),
            ThoughtConfigEditorField::Model => {
                let _ = editor.cycle_model_preset(delta);
            }
            ThoughtConfigEditorField::Test
            | ThoughtConfigEditorField::Save
            | ThoughtConfigEditorField::Cancel => {}
        }
    }

    fn activate_thought_config_field(&mut self, layout: WorkspaceLayout) {
        let Some(focus) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.focus)
        else {
            return;
        };
        match focus {
            ThoughtConfigEditorField::Enabled | ThoughtConfigEditorField::Backend => {
                self.adjust_thought_config_field(1)
            }
            ThoughtConfigEditorField::Model => {}
            ThoughtConfigEditorField::Test => self.test_thought_config(),
            ThoughtConfigEditorField::Save => self.submit_thought_config(layout),
            ThoughtConfigEditorField::Cancel => self.close_thought_config_editor(),
        }
    }

    fn test_thought_config(&mut self) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("testing thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_test_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::TestThoughtConfig { outcome });
        });
    }

    pub(crate) fn submit_thought_config(&mut self, _layout: WorkspaceLayout) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("saving thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_save_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::SaveThoughtConfig { outcome });
        });
    }

    async fn run_thought_config_test_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        let target = Self::thought_config_target_summary(&config);
        match client.test_thought_config(config.clone()).await {
            Ok(test) if test.ok => ThoughtConfigActionOutcome {
                message: format!("test ok: {target}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
            Ok(test) => Self::try_openrouter_rotation(
                client,
                &config,
                daemon_defaults,
                false,
                target.clone(),
                test.message.clone(),
            )
            .await
            .unwrap_or_else(|| ThoughtConfigActionOutcome {
                message: format!("test failed: {target} | {}", test.message),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            }),
            Err(err) => ThoughtConfigActionOutcome {
                message: format!("test error: {target} | {err}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
        }
    }

    pub(crate) async fn run_thought_config_save_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        match client.update_thought_config(config).await {
            Ok(saved) => {
                let save_summary = Self::thought_config_target_summary(&saved);
                Self::thought_config_save_test_outcome(client, saved, daemon_defaults, save_summary)
                    .await
            }
            Err(err) => Self::thought_config_save_failed_outcome(err),
        }
    }

    async fn thought_config_save_test_outcome(
        client: Arc<C>,
        saved: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
        save_summary: String,
    ) -> ThoughtConfigActionOutcome {
        match client.test_thought_config(saved.clone()).await {
            Ok(test) if test.ok => Self::thought_config_save_complete_outcome(format!(
                "saved {save_summary} | test ok"
            )),
            Ok(test) => Self::try_openrouter_rotation(
                Arc::clone(&client),
                &saved,
                daemon_defaults,
                true,
                save_summary.clone(),
                test.message.clone(),
            )
            .await
            .unwrap_or_else(|| {
                Self::thought_config_save_complete_outcome(format!(
                    "saved {save_summary} | {}",
                    test.message
                ))
            }),
            Err(err) => Self::thought_config_save_complete_outcome(format!(
                "saved {save_summary} | test error: {err}"
            )),
        }
    }

    fn thought_config_save_complete_outcome(message: String) -> ThoughtConfigActionOutcome {
        ThoughtConfigActionOutcome {
            message,
            updated_config: None,
            openrouter_candidates: None,
            close_editor: true,
            refresh_sessions: true,
        }
    }

    fn thought_config_save_failed_outcome(message: String) -> ThoughtConfigActionOutcome {
        ThoughtConfigActionOutcome {
            message,
            updated_config: None,
            openrouter_candidates: None,
            close_editor: false,
            refresh_sessions: false,
        }
    }

    async fn try_openrouter_rotation(
        client: Arc<C>,
        config: &ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
        persist: bool,
        target: String,
        failure_message: String,
    ) -> Option<ThoughtConfigActionOutcome> {
        if !Self::should_attempt_openrouter_rotation(
            config,
            daemon_defaults.as_ref(),
            &failure_message,
        ) {
            return None;
        }

        let candidates = client.refresh_openrouter_candidates().await.ok()?;
        let probe =
            Self::find_working_openrouter_rotation(client.clone(), config, &candidates).await?;
        if persist {
            return Some(
                Self::persist_openrouter_rotation(client, target, candidates, probe).await,
            );
        }

        Some(Self::openrouter_rotation_test_outcome(
            target, candidates, probe,
        ))
    }

    fn should_attempt_openrouter_rotation(
        config: &ThoughtConfig,
        daemon_defaults: Option<&DaemonDefaults>,
        failure_message: &str,
    ) -> bool {
        Self::is_effective_openrouter_backend(config, daemon_defaults)
            && should_rotate_openrouter_model(failure_message)
    }

    async fn find_working_openrouter_rotation(
        client: Arc<C>,
        config: &ThoughtConfig,
        candidates: &[String],
    ) -> Option<OpenRouterRotationProbe> {
        let probes = Self::openrouter_rotation_probes(config, candidates);
        Self::first_working_openrouter_rotation(client, probes).await
    }

    async fn first_working_openrouter_rotation(
        client: Arc<C>,
        probes: Vec<OpenRouterRotationProbe>,
    ) -> Option<OpenRouterRotationProbe> {
        let working_probes = futures::stream::iter(probes)
            .filter_map(|probe| Self::test_openrouter_rotation_probe(&client, probe))
            .fuse();
        futures::pin_mut!(working_probes);
        working_probes.next().await
    }

    async fn test_openrouter_rotation_probe(
        client: &Arc<C>,
        probe: OpenRouterRotationProbe,
    ) -> Option<OpenRouterRotationProbe> {
        client
            .test_thought_config(probe.config.clone())
            .await
            .ok()
            .filter(|test| test.ok)
            .map(|_| probe)
    }

    fn openrouter_rotation_probes(
        config: &ThoughtConfig,
        candidates: &[String],
    ) -> Vec<OpenRouterRotationProbe> {
        let current_model = config.model.trim();
        candidates
            .iter()
            .filter(|candidate| !candidate.eq_ignore_ascii_case(current_model))
            .map(|candidate| {
                let mut rotated = config.clone();
                rotated.model = candidate.clone();
                OpenRouterRotationProbe {
                    candidate: candidate.clone(),
                    config: rotated,
                }
            })
            .collect()
    }

    async fn persist_openrouter_rotation(
        client: Arc<C>,
        target: String,
        candidates: Vec<String>,
        probe: OpenRouterRotationProbe,
    ) -> ThoughtConfigActionOutcome {
        let save_result = client.update_thought_config(probe.config).await;
        Self::openrouter_rotation_save_outcome(&target, &probe.candidate, candidates, save_result)
    }

    fn openrouter_rotation_test_outcome(
        target: String,
        candidates: Vec<String>,
        probe: OpenRouterRotationProbe,
    ) -> ThoughtConfigActionOutcome {
        ThoughtConfigActionOutcome {
            message: format!(
                "test failed: {target} | rotated to {} after OpenRouter catalog refresh | test ok",
                probe.candidate
            ),
            updated_config: Some(probe.config),
            openrouter_candidates: Some(candidates),
            close_editor: false,
            refresh_sessions: false,
        }
    }

    fn openrouter_rotation_save_outcome(
        target: &str,
        candidate: &str,
        candidates: Vec<String>,
        save_result: Result<ThoughtConfig, String>,
    ) -> ThoughtConfigActionOutcome {
        let message = match save_result {
            Ok(_) => format!(
                "saved {target} | rotated to {candidate} after OpenRouter catalog refresh | test ok"
            ),
            Err(err) => {
                format!("saved {target} | rotated probe found {candidate}, but save failed: {err}")
            }
        };
        ThoughtConfigActionOutcome {
            message,
            updated_config: None,
            openrouter_candidates: Some(candidates),
            close_editor: true,
            refresh_sessions: true,
        }
    }

    fn is_effective_openrouter_backend(
        config: &ThoughtConfig,
        daemon_defaults: Option<&DaemonDefaults>,
    ) -> bool {
        if Self::is_openrouter_backend(&config.backend) {
            return true;
        }
        Self::uses_openrouter_daemon_default(config, daemon_defaults)
    }

    fn is_openrouter_backend(backend: &str) -> bool {
        backend.eq_ignore_ascii_case("openrouter")
    }

    fn uses_openrouter_daemon_default(
        config: &ThoughtConfig,
        daemon_defaults: Option<&DaemonDefaults>,
    ) -> bool {
        config.backend.trim().is_empty()
            && daemon_defaults
                .is_some_and(|defaults| Self::is_openrouter_backend(&defaults.backend))
    }

    pub(crate) fn thought_config_target_summary(config: &ThoughtConfig) -> String {
        format!(
            "{} / {}",
            Self::thought_config_backend_summary(config),
            Self::thought_config_model_summary(config)
        )
    }

    fn thought_config_backend_summary(config: &ThoughtConfig) -> &str {
        if config.backend.trim().is_empty() {
            "auto"
        } else {
            config.backend.as_str()
        }
    }

    fn thought_config_model_summary(config: &ThoughtConfig) -> &str {
        if config.model.trim().is_empty() {
            "daemon default"
        } else {
            config.model.as_str()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thought_config_with_backend(backend: &str) -> ThoughtConfig {
        ThoughtConfig {
            backend: backend.to_string(),
            ..ThoughtConfig::default()
        }
    }

    fn daemon_defaults_with_backend(backend: &str) -> DaemonDefaults {
        DaemonDefaults {
            backend: backend.to_string(),
            ..DaemonDefaults::default()
        }
    }

    #[test]
    fn paste_routing_waits_when_action_is_pending() {
        assert_eq!(
            route_thought_config_paste(true, Some(ThoughtConfigEditorField::Model)),
            ThoughtConfigPasteAction::WaitForPendingAction
        );
    }

    #[test]
    fn paste_routing_appends_only_to_model_field() {
        assert_eq!(
            route_thought_config_paste(false, Some(ThoughtConfigEditorField::Model)),
            ThoughtConfigPasteAction::AppendToModel
        );
        assert_eq!(
            route_thought_config_paste(false, Some(ThoughtConfigEditorField::Backend)),
            ThoughtConfigPasteAction::Ignore
        );
        assert_eq!(
            route_thought_config_paste(false, None),
            ThoughtConfigPasteAction::Ignore
        );
    }

    #[test]
    fn effective_openrouter_backend_detects_explicit_backend_case_insensitively() {
        let config = thought_config_with_backend("OpenRouter");
        let defaults = daemon_defaults_with_backend("grok");

        assert!(App::<ApiClient>::is_effective_openrouter_backend(
            &config,
            Some(&defaults)
        ));
    }

    #[test]
    fn effective_openrouter_backend_uses_openrouter_daemon_default_for_blank_backend() {
        let config = thought_config_with_backend("  ");
        let defaults = daemon_defaults_with_backend("OPENROUTER");

        assert!(App::<ApiClient>::is_effective_openrouter_backend(
            &config,
            Some(&defaults)
        ));
    }

    #[test]
    fn effective_openrouter_backend_rejects_blank_backend_without_openrouter_default() {
        let config = thought_config_with_backend("");
        let grok_defaults = daemon_defaults_with_backend("grok");

        assert!(!App::<ApiClient>::is_effective_openrouter_backend(
            &config,
            Some(&grok_defaults)
        ));
        assert!(!App::<ApiClient>::is_effective_openrouter_backend(
            &config, None
        ));
    }

    #[test]
    fn effective_openrouter_backend_rejects_nonblank_non_openrouter_override() {
        let config = thought_config_with_backend("grok");
        let defaults = daemon_defaults_with_backend("openrouter");

        assert!(!App::<ApiClient>::is_effective_openrouter_backend(
            &config,
            Some(&defaults)
        ));
    }

    #[test]
    fn openrouter_rotation_attempt_requires_openrouter_backend_and_rotatable_failure() {
        let config = thought_config_with_backend("openrouter");
        let defaults = daemon_defaults_with_backend("grok");

        assert!(App::<ApiClient>::should_attempt_openrouter_rotation(
            &config,
            Some(&defaults),
            "probe failed: old/expired:free is not a valid model ID"
        ));
        assert!(!App::<ApiClient>::should_attempt_openrouter_rotation(
            &thought_config_with_backend("grok"),
            Some(&defaults),
            "probe failed: old/expired:free is not a valid model ID"
        ));
        assert!(!App::<ApiClient>::should_attempt_openrouter_rotation(
            &config,
            Some(&defaults),
            "probe failed: request timed out"
        ));
    }

    #[test]
    fn openrouter_rotation_probes_skip_current_model_case_insensitively() {
        let mut config = thought_config_with_backend("openrouter");
        config.model = " old/expired:free ".to_string();
        let candidates = vec![
            "OLD/EXPIRED:FREE".to_string(),
            "openrouter/free".to_string(),
            "google/gemma-3-4b-it:free".to_string(),
        ];

        let probes = App::<ApiClient>::openrouter_rotation_probes(&config, &candidates);

        assert_eq!(
            probes
                .iter()
                .map(|probe| probe.candidate.as_str())
                .collect::<Vec<_>>(),
            vec!["openrouter/free", "google/gemma-3-4b-it:free"]
        );
        assert_eq!(probes[0].config.backend, "openrouter");
        assert_eq!(probes[0].config.model, "openrouter/free");
        assert_eq!(probes[1].config.model, "google/gemma-3-4b-it:free");
    }

    #[test]
    fn openrouter_rotation_test_outcome_preserves_message_and_editor_update() {
        let candidates = vec!["openrouter/free".to_string()];
        let probe = OpenRouterRotationProbe {
            candidate: "openrouter/free".to_string(),
            config: ThoughtConfig {
                backend: "openrouter".to_string(),
                model: "openrouter/free".to_string(),
                ..ThoughtConfig::default()
            },
        };

        let outcome = App::<ApiClient>::openrouter_rotation_test_outcome(
            "openrouter / old/expired:free".to_string(),
            candidates.clone(),
            probe,
        );

        assert_eq!(
            outcome.message,
            "test failed: openrouter / old/expired:free | rotated to openrouter/free after OpenRouter catalog refresh | test ok"
        );
        assert_eq!(
            outcome
                .updated_config
                .as_ref()
                .map(|config| config.model.as_str()),
            Some("openrouter/free")
        );
        assert_eq!(outcome.openrouter_candidates, Some(candidates));
        assert!(!outcome.close_editor);
        assert!(!outcome.refresh_sessions);
    }

    #[test]
    fn openrouter_rotation_save_outcome_preserves_success_and_failure_messages() {
        let candidates = vec!["openrouter/free".to_string()];
        let target = "openrouter / old/expired:free";

        let success = App::<ApiClient>::openrouter_rotation_save_outcome(
            target,
            "openrouter/free",
            candidates.clone(),
            Ok(ThoughtConfig::default()),
        );
        assert_eq!(
            success.message,
            "saved openrouter / old/expired:free | rotated to openrouter/free after OpenRouter catalog refresh | test ok"
        );
        assert_eq!(success.openrouter_candidates, Some(candidates.clone()));
        assert!(success.close_editor);
        assert!(success.refresh_sessions);
        assert!(success.updated_config.is_none());

        let failure = App::<ApiClient>::openrouter_rotation_save_outcome(
            target,
            "openrouter/free",
            candidates.clone(),
            Err("disk full".to_string()),
        );
        assert_eq!(
            failure.message,
            "saved openrouter / old/expired:free | rotated probe found openrouter/free, but save failed: disk full"
        );
        assert_eq!(failure.openrouter_candidates, Some(candidates));
        assert!(failure.close_editor);
        assert!(failure.refresh_sessions);
        assert!(failure.updated_config.is_none());
    }
}
