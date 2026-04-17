#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VoiceUiState {
    Unsupported,
    Ready,
    Recording,
    Transcribing,
    Failed(String),
}

impl VoiceUiState {
    pub(crate) fn status_line(&self) -> String {
        match self {
            Self::Unsupported => "voice: unavailable in this build".to_string(),
            Self::Ready => "voice: ready - ctrl-v records into the composer".to_string(),
            Self::Recording => "voice: recording - press ctrl-v again to stop".to_string(),
            Self::Transcribing => "voice: transcribing captured audio...".to_string(),
            Self::Failed(message) => format!("voice: {message}"),
        }
    }
}

pub(crate) fn default_ui_state() -> VoiceUiState {
    if cfg!(feature = "voice") {
        VoiceUiState::Ready
    } else {
        VoiceUiState::Unsupported
    }
}

pub(crate) fn toggle_hint() -> &'static str {
    if cfg!(feature = "voice") {
        "ctrl-v voice"
    } else {
        "ctrl-v voice unavailable"
    }
}

#[cfg(not(feature = "voice"))]
pub(crate) struct VoiceRecording;

#[cfg(not(feature = "voice"))]
pub(crate) fn start_recording() -> Result<VoiceRecording, String> {
    Err("voice support is not built; rebuild with `--features voice`".to_string())
}

#[cfg(not(feature = "voice"))]
impl VoiceRecording {
    pub(crate) fn finish(self) -> Result<String, String> {
        Err("voice support is not built; rebuild with `--features voice`".to_string())
    }

    pub(crate) fn cancel(self) {}
}

#[cfg(feature = "voice")]
mod enabled {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
    use whisper_rs::{
        FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    };

    const TARGET_SAMPLE_RATE: u32 = 16_000;
    const MIN_CAPTURE_SAMPLES: usize = 1_600;
    const SILENCE_THRESHOLD: f32 = 0.01;

    #[derive(Clone, Debug)]
    struct VoiceConfig {
        model_path: PathBuf,
        language: Option<String>,
    }

    pub(crate) struct VoiceRecording {
        capture: VoiceCapture,
    }

    struct VoiceCapture {
        stream: Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        sample_rate: u32,
        config: VoiceConfig,
    }

    pub(crate) fn start_recording() -> Result<VoiceRecording, String> {
        let config = VoiceConfig::from_env()?;
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no default input device available".to_string())?;
        let supported_config = device
            .default_input_config()
            .map_err(|err| format!("failed to read input device config: {err}"))?;
        let stream_config = supported_config.config();
        let sample_rate = stream_config.sample_rate;
        let channels = stream_config.channels as usize;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let error_handler = |err| tracing::warn!(error = %err, "voice input stream error");

        let stream = match supported_config.sample_format() {
            SampleFormat::I8 => build_input_stream::<i8, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::I16 => build_input_stream::<i16, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::I32 => build_input_stream::<i32, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::I64 => build_input_stream::<i64, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::U8 => build_input_stream::<u8, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::U16 => build_input_stream::<u16, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::U32 => build_input_stream::<u32, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::U64 => build_input_stream::<u64, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::F32 => build_input_stream::<f32, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            SampleFormat::F64 => build_input_stream::<f64, _>(
                &device,
                &stream_config,
                channels,
                Arc::clone(&samples),
                error_handler,
            )?,
            sample_format => {
                return Err(format!(
                    "unsupported microphone sample format: {sample_format:?}"
                ));
            }
        };

        stream
            .play()
            .map_err(|err| format!("failed to start microphone stream: {err}"))?;

        Ok(VoiceRecording {
            capture: VoiceCapture {
                stream,
                samples,
                sample_rate,
                config,
            },
        })
    }

    impl VoiceRecording {
        pub(crate) fn finish(self) -> Result<String, String> {
            let clip = self.capture.finish()?;
            transcribe_samples(&clip.config, &clip.samples, clip.sample_rate)
        }

        pub(crate) fn cancel(self) {}
    }

    struct RecordedClip {
        samples: Vec<f32>,
        sample_rate: u32,
        config: VoiceConfig,
    }

    impl VoiceCapture {
        fn finish(self) -> Result<RecordedClip, String> {
            drop(self.stream);
            let samples = match Arc::try_unwrap(self.samples) {
                Ok(mutex) => mutex
                    .into_inner()
                    .map_err(|_| "failed to finalize voice buffer".to_string())?,
                Err(arc) => arc
                    .lock()
                    .map_err(|_| "failed to lock voice buffer".to_string())?
                    .clone(),
            };
            if samples.len() < MIN_CAPTURE_SAMPLES {
                return Err("voice capture too short".to_string());
            }
            Ok(RecordedClip {
                samples,
                sample_rate: self.sample_rate,
                config: self.config,
            })
        }
    }

    impl VoiceConfig {
        fn from_env() -> Result<Self, String> {
            let model_path = env::var("SWIMMERS_VOICE_MODEL")
                .map(expand_home)
                .map(PathBuf::from)
                .map_err(|_| "SWIMMERS_VOICE_MODEL is not set".to_string())?;
            if !model_path.is_file() {
                return Err(format!("voice model not found at {}", model_path.display()));
            }
            let language = env::var("SWIMMERS_VOICE_LANGUAGE")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            Ok(Self {
                model_path,
                language,
            })
        }
    }

    fn expand_home(value: String) -> String {
        if let Some(rest) = value.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest).display().to_string();
            }
        }
        value
    }

    fn build_input_stream<T, E>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        samples: Arc<Mutex<Vec<f32>>>,
        error_handler: E,
    ) -> Result<Stream, String>
    where
        T: Sample + SizedSample,
        f32: FromSample<T>,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        let buffer = Arc::clone(&samples);
        device
            .build_input_stream(
                config,
                move |data: &[T], _| capture_input_data(data, channels, &buffer),
                error_handler,
                None,
            )
            .map_err(|err| format!("failed to build microphone stream: {err}"))
    }

    fn capture_input_data<T>(input: &[T], channels: usize, samples: &Arc<Mutex<Vec<f32>>>)
    where
        T: Sample + SizedSample,
        f32: FromSample<T>,
    {
        if channels == 0 {
            return;
        }
        let mut buffer = match samples.lock() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        buffer.reserve(input.len() / channels);
        for frame in input.chunks(channels) {
            let summed = frame
                .iter()
                .copied()
                .map(f32::from_sample)
                .fold(0.0, |acc, sample| acc + sample);
            buffer.push(summed / frame.len() as f32);
        }
    }

    fn transcribe_samples(
        config: &VoiceConfig,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        let trimmed = trim_silence(samples);
        if trimmed.len() < MIN_CAPTURE_SAMPLES {
            return Err("no speech detected in capture".to_string());
        }
        let prepared = resample(trimmed, sample_rate, TARGET_SAMPLE_RATE);
        let model_path = config.model_path.to_string_lossy().to_string();
        let context =
            WhisperContext::new_with_params(&model_path, WhisperContextParameters::default())
                .map_err(|err| format!("failed to load voice model: {err}"))?;
        let mut state = context
            .create_state()
            .map_err(|err| format!("failed to create whisper state: {err}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_no_timestamps(true);
        params.set_no_context(true);
        match config.language.as_deref() {
            Some("auto") | None => params.set_detect_language(true),
            Some(language) => params.set_language(Some(language)),
        }
        state
            .full(params, &prepared)
            .map_err(|err| format!("voice transcription failed: {err}"))?;
        normalize_transcript(&state)
    }

    fn normalize_transcript(state: &WhisperState) -> Result<String, String> {
        let transcript = state
            .as_iter()
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>()
            .join("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if transcript.is_empty() {
            Err("voice transcription was empty".to_string())
        } else {
            Ok(transcript)
        }
    }

    fn trim_silence(samples: &[f32]) -> Vec<f32> {
        let first = samples
            .iter()
            .position(|sample| sample.abs() >= SILENCE_THRESHOLD);
        let last = samples
            .iter()
            .rposition(|sample| sample.abs() >= SILENCE_THRESHOLD);
        match (first, last) {
            (Some(start), Some(end)) if end >= start => samples[start..=end].to_vec(),
            _ => Vec::new(),
        }
    }

    fn resample(samples: Vec<f32>, input_rate: u32, output_rate: u32) -> Vec<f32> {
        if input_rate == output_rate || samples.is_empty() {
            return samples;
        }
        let output_len =
            ((samples.len() as u64 * output_rate as u64) / input_rate as u64).max(1) as usize;
        let ratio = input_rate as f64 / output_rate as f64;
        let mut output = Vec::with_capacity(output_len);
        for index in 0..output_len {
            let position = index as f64 * ratio;
            let left_index = position.floor() as usize;
            let right_index = (left_index + 1).min(samples.len() - 1);
            let fraction = (position - left_index as f64) as f32;
            let left = samples[left_index];
            let right = samples[right_index];
            output.push(left + (right - left) * fraction);
        }
        output
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn resample_keeps_non_empty_signal() {
            let input = vec![0.0, 1.0, 0.0, -1.0, 0.0];
            let output = resample(input, 8_000, 16_000);
            assert!(!output.is_empty());
            assert!(output.iter().any(|sample| sample.abs() > 0.0));
        }

        #[test]
        fn trim_silence_strips_leading_and_trailing_quiet_frames() {
            let trimmed = trim_silence(&[0.0, 0.001, 0.02, 0.4, 0.01, 0.0]);
            assert_eq!(trimmed, vec![0.02, 0.4, 0.01]);
        }

        #[test]
        fn expand_home_resolves_user_prefix() {
            let Some(home) = dirs::home_dir() else {
                return;
            };
            let expanded = expand_home("~/models/test.bin".to_string());
            assert_eq!(Path::new(&expanded), home.join("models/test.bin"));
        }
    }
}

#[cfg(feature = "voice")]
pub(crate) use enabled::{start_recording, VoiceRecording};
