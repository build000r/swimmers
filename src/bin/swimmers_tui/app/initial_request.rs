use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum InitialRequestSubmission {
    GroupInput {
        session_ids: Vec<String>,
        text: String,
    },
    Batch {
        dirs: Vec<String>,
        launch_target: Option<String>,
        text: String,
    },
    Single {
        cwd: String,
        launch_target: Option<String>,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitialRequestKeyAction {
    Close,
    Submit,
    ToggleVoice,
    Backspace,
    InsertChar(char),
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteRoute {
    ThoughtConfig,
    InitialRequest,
    Ignore,
}

fn classify_initial_request_key(key: KeyEvent) -> InitialRequestKeyAction {
    match key.code {
        KeyCode::Esc => InitialRequestKeyAction::Close,
        KeyCode::Enter => InitialRequestKeyAction::Submit,
        KeyCode::Char('v') if key.modifiers == KeyModifiers::CONTROL => {
            InitialRequestKeyAction::ToggleVoice
        }
        KeyCode::Backspace => InitialRequestKeyAction::Backspace,
        KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            InitialRequestKeyAction::InsertChar(ch)
        }
        _ => InitialRequestKeyAction::Ignore,
    }
}

fn route_paste(thought_config_open: bool, initial_request_open: bool) -> PasteRoute {
    match (thought_config_open, initial_request_open) {
        (true, _) => PasteRoute::ThoughtConfig,
        (false, true) => PasteRoute::InitialRequest,
        _ => PasteRoute::Ignore,
    }
}

fn plan_initial_request_submission(
    initial_request: Option<&InitialRequestState>,
    group_input_targets: Option<&GroupInputTargets>,
) -> Result<InitialRequestSubmission, &'static str> {
    let Some(state) = initial_request else {
        return Err("enter an initial request");
    };
    let Some(text) = state.trimmed_value() else {
        return Err("enter an initial request");
    };

    if let Some(targets) = group_input_targets {
        return Ok(InitialRequestSubmission::GroupInput {
            session_ids: targets.session_ids.clone(),
            text,
        });
    }

    if let Some(dirs) = state.batch_dirs.clone() {
        return Ok(InitialRequestSubmission::Batch {
            dirs,
            launch_target: state.launch_target.clone(),
            text,
        });
    }

    Ok(InitialRequestSubmission::Single {
        cwd: state.cwd.clone(),
        launch_target: state.launch_target.clone(),
        text,
    })
}

impl<C: TuiApi> App<C> {
    pub(crate) fn handle_initial_request_key(&mut self, key: KeyEvent, field: Rect) {
        match classify_initial_request_key(key) {
            InitialRequestKeyAction::Close => self.close_initial_request(),
            InitialRequestKeyAction::Submit => self.submit_initial_request(field),
            InitialRequestKeyAction::ToggleVoice => self.toggle_voice_recording(),
            InitialRequestKeyAction::Backspace => {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.pop();
                }
            }
            InitialRequestKeyAction::InsertChar(ch) => {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.push(ch);
                }
            }
            InitialRequestKeyAction::Ignore => {}
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        match route_paste(
            self.thought_config_editor.is_some(),
            self.initial_request.is_some(),
        ) {
            PasteRoute::ThoughtConfig => self.handle_thought_config_paste(text),
            PasteRoute::InitialRequest => {
                let Some(initial_request) = &mut self.initial_request else {
                    return;
                };
                initial_request.value.push_str(text);
            }
            PasteRoute::Ignore => {}
        }
    }

    pub(crate) fn submit_initial_request(&mut self, field: Rect) {
        if let Some(message) = self.initial_request_submission_blocker() {
            self.set_message(message);
            return;
        }

        match plan_initial_request_submission(
            self.initial_request.as_ref(),
            self.group_input_targets.as_ref(),
        ) {
            Ok(InitialRequestSubmission::GroupInput { session_ids, text }) => {
                self.send_group_input(session_ids, text);
            }
            Ok(InitialRequestSubmission::Batch {
                dirs,
                launch_target,
                text,
            }) => {
                self.spawn_sessions_batch(dirs, launch_target, Some(text), field);
            }
            Ok(InitialRequestSubmission::Single {
                cwd,
                launch_target,
                text,
            }) => {
                self.spawn_session(&cwd, launch_target, Some(text), field);
            }
            Err(message) => self.set_message(message),
        }
    }

    fn initial_request_submission_blocker(&self) -> Option<&'static str> {
        if self.voice_recording.is_some() {
            return Some("stop voice recording before creating a swimmer");
        }
        matches!(self.voice_state, VoiceUiState::Transcribing)
            .then_some("wait for voice transcription to finish")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_request_key_classifier_routes_control_actions() {
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            InitialRequestKeyAction::Close
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            InitialRequestKeyAction::Submit
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            InitialRequestKeyAction::ToggleVoice
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            InitialRequestKeyAction::Backspace
        );
    }

    #[test]
    fn initial_request_key_classifier_routes_text_and_ignored_keys() {
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            InitialRequestKeyAction::InsertChar('a')
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            InitialRequestKeyAction::InsertChar('A')
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            InitialRequestKeyAction::Ignore
        );
        assert_eq!(
            classify_initial_request_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            InitialRequestKeyAction::Ignore
        );
    }

    #[test]
    fn paste_routing_prioritizes_thought_config_before_initial_request() {
        assert_eq!(route_paste(true, true), PasteRoute::ThoughtConfig);
        assert_eq!(route_paste(true, false), PasteRoute::ThoughtConfig);
        assert_eq!(route_paste(false, true), PasteRoute::InitialRequest);
        assert_eq!(route_paste(false, false), PasteRoute::Ignore);
    }

    #[test]
    fn initial_request_submission_requires_nonblank_text() {
        assert_eq!(
            plan_initial_request_submission(None, None),
            Err("enter an initial request")
        );

        let mut request = InitialRequestState::new("/repo".to_string(), None);
        request.value = " \n\t ".to_string();
        assert_eq!(
            plan_initial_request_submission(Some(&request), None),
            Err("enter an initial request")
        );
    }

    #[test]
    fn initial_request_submission_routes_to_group_input() {
        let mut request = InitialRequestState::new("/repo".to_string(), Some("main".to_string()));
        request.value = "  please review  ".to_string();
        let targets = GroupInputTargets {
            session_ids: vec!["s1".to_string(), "s2".to_string()],
            label: "school".to_string(),
        };

        assert_eq!(
            plan_initial_request_submission(Some(&request), Some(&targets)),
            Ok(InitialRequestSubmission::GroupInput {
                session_ids: vec!["s1".to_string(), "s2".to_string()],
                text: "please review".to_string(),
            })
        );
    }

    #[test]
    fn initial_request_submission_routes_to_batch_create() {
        let mut request = InitialRequestState::new_batch(
            vec!["/repo/a".to_string(), "/repo/b".to_string()],
            Some("feature".to_string()),
        );
        request.value = "\nship it\n".to_string();

        assert_eq!(
            plan_initial_request_submission(Some(&request), None),
            Ok(InitialRequestSubmission::Batch {
                dirs: vec!["/repo/a".to_string(), "/repo/b".to_string()],
                launch_target: Some("feature".to_string()),
                text: "ship it".to_string(),
            })
        );
    }

    #[test]
    fn initial_request_submission_routes_to_single_create() {
        let mut request = InitialRequestState::new("/repo".to_string(), Some("main".to_string()));
        request.value = "start here".to_string();

        assert_eq!(
            plan_initial_request_submission(Some(&request), None),
            Ok(InitialRequestSubmission::Single {
                cwd: "/repo".to_string(),
                launch_target: Some("main".to_string()),
                text: "start here".to_string(),
            })
        );
    }
}
