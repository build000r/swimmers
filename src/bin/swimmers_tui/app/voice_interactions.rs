use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FinishVoiceRecordingPlan {
    WaitForPendingInteraction,
    StartTranscription { generation: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToggleVoiceRecordingPlan {
    ShowMessage(&'static str),
    FinishRecording,
    StartRecording,
}

fn toggle_voice_recording_message(
    has_initial_request: bool,
    is_transcribing: bool,
) -> Option<&'static str> {
    missing_initial_request_voice_message(has_initial_request)
        .or_else(|| transcribing_voice_message(is_transcribing))
}

fn missing_initial_request_voice_message(has_initial_request: bool) -> Option<&'static str> {
    (!has_initial_request).then_some("open an initial request first")
}

fn transcribing_voice_message(is_transcribing: bool) -> Option<&'static str> {
    is_transcribing.then_some("wait for voice transcription to finish")
}

fn toggle_voice_recording_action(has_recording: bool) -> ToggleVoiceRecordingPlan {
    if has_recording {
        ToggleVoiceRecordingPlan::FinishRecording
    } else {
        ToggleVoiceRecordingPlan::StartRecording
    }
}

pub(super) fn plan_toggle_voice_recording(
    has_initial_request: bool,
    is_transcribing: bool,
    has_recording: bool,
) -> ToggleVoiceRecordingPlan {
    toggle_voice_recording_message(has_initial_request, is_transcribing)
        .map(ToggleVoiceRecordingPlan::ShowMessage)
        .unwrap_or_else(|| toggle_voice_recording_action(has_recording))
}

pub(super) fn plan_finish_voice_recording(
    has_pending_interaction: bool,
    generation: u64,
) -> FinishVoiceRecordingPlan {
    if has_pending_interaction {
        FinishVoiceRecordingPlan::WaitForPendingInteraction
    } else {
        FinishVoiceRecordingPlan::StartTranscription { generation }
    }
}

async fn send_voice_transcription_result(
    recording: VoiceRecording,
    generation: u64,
    tx: oneshot::Sender<PendingInteractionResult>,
) {
    let response = voice_transcription_task_response(spawn_voice_recording_finish(recording).await);
    let _ = tx.send(voice_transcription_result(generation, response));
}

fn spawn_voice_recording_finish(
    recording: VoiceRecording,
) -> tokio::task::JoinHandle<Result<String, String>> {
    tokio::task::spawn_blocking(move || recording.finish())
}

fn voice_transcription_task_response(
    response: Result<Result<String, String>, tokio::task::JoinError>,
) -> Result<String, String> {
    response.unwrap_or_else(|err| Err(format!("voice task failed: {err}")))
}

fn voice_transcription_result(
    generation: u64,
    response: Result<String, String>,
) -> PendingInteractionResult {
    PendingInteractionResult::VoiceTranscription {
        generation,
        response,
    }
}

impl<C: TuiApi> App<C> {
    pub(crate) fn toggle_voice_recording(&mut self) {
        match plan_toggle_voice_recording(
            self.initial_request.is_some(),
            matches!(self.voice_state, VoiceUiState::Transcribing),
            self.voice_recording.is_some(),
        ) {
            ToggleVoiceRecordingPlan::ShowMessage(message) => self.set_message(message),
            ToggleVoiceRecordingPlan::FinishRecording => self.finish_current_voice_recording(),
            ToggleVoiceRecordingPlan::StartRecording => self.start_voice_recording(),
        }
    }

    fn finish_current_voice_recording(&mut self) {
        if let Some(recording) = self.voice_recording.take() {
            self.finish_voice_recording(recording);
        }
    }

    fn start_voice_recording(&mut self) {
        match start_recording() {
            Ok(recording) => {
                self.voice_recording = Some(recording);
                self.voice_state = VoiceUiState::Recording;
                self.set_message("voice recording started");
            }
            Err(err) => {
                self.voice_state = VoiceUiState::Failed(err.clone());
                self.set_message(err);
            }
        }
    }

    fn finish_voice_recording(&mut self, recording: VoiceRecording) {
        match plan_finish_voice_recording(
            self.pending_interaction.is_some(),
            self.initial_request_generation,
        ) {
            FinishVoiceRecordingPlan::WaitForPendingInteraction => {
                self.defer_voice_recording_finish(recording);
            }
            FinishVoiceRecordingPlan::StartTranscription { generation } => {
                self.start_voice_transcription(recording, generation);
            }
        }
    }

    fn defer_voice_recording_finish(&mut self, recording: VoiceRecording) {
        self.voice_recording = Some(recording);
        self.set_message("wait for the current action to finish");
    }

    fn start_voice_transcription(&mut self, recording: VoiceRecording, generation: u64) {
        let (tx, rx) = oneshot::channel();
        self.pending_interaction = Some(rx);
        self.voice_state = VoiceUiState::Transcribing;
        self.set_message("transcribing voice capture...");
        self.runtime
            .spawn(send_voice_transcription_result(recording, generation, tx));
    }

    pub(super) fn cancel_voice_recording(&mut self) {
        if let Some(recording) = self.voice_recording.take() {
            recording.cancel();
        }
        self.voice_state = default_ui_state();
    }

    pub(super) fn apply_voice_transcription_result(
        &mut self,
        generation: u64,
        response: Result<String, String>,
    ) {
        match response {
            Ok(transcript) => self.insert_voice_transcript(generation, transcript),
            Err(err) => {
                self.voice_state = VoiceUiState::Failed(err.clone());
                self.set_message(err);
            }
        }
    }

    fn insert_voice_transcript(&mut self, generation: u64, transcript: String) {
        self.voice_state = default_ui_state();
        if generation != self.initial_request_generation {
            self.set_message("voice transcript finished after the composer changed");
            return;
        }
        let Some(initial_request) = &mut self.initial_request else {
            self.set_message("voice transcript finished after the composer closed");
            return;
        };

        if !initial_request.value.trim().is_empty() {
            initial_request.value.push('\n');
        }
        initial_request.value.push_str(transcript.trim());
        self.set_message("voice transcript inserted");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_task_response_preserves_success_payload() {
        let result = voice_transcription_result(
            17,
            voice_transcription_task_response(Ok(Ok("ship it".to_string()))),
        );

        let PendingInteractionResult::VoiceTranscription {
            generation,
            response,
        } = result
        else {
            panic!("expected voice transcription result");
        };
        assert_eq!(generation, 17);
        assert_eq!(response, Ok("ship it".to_string()));
    }

    #[test]
    fn voice_task_response_preserves_recording_error() {
        assert_eq!(
            voice_transcription_task_response(Ok(Err("voice capture too short".to_string()))),
            Err("voice capture too short".to_string())
        );
    }

    #[test]
    fn voice_task_response_converts_join_error() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let err = runtime.block_on(async {
            tokio::task::spawn_blocking(|| panic!("voice worker panic"))
                .await
                .expect_err("blocking task should panic")
        });

        let message =
            voice_transcription_task_response(Err(err)).expect_err("join error should convert");
        assert!(message.starts_with("voice task failed: "));
        assert!(message.contains("panicked"));
    }
}
