use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use thiserror::Error;
use uuid::Uuid;

use crate::launcher::SpawnToolLauncher;
use crate::types::{
    AuthorizedProviderResumeLaunchReceipt, AuthorizedProviderResumeReceipt, LaunchReceipt,
    ProviderResumeCaptureSource, ProviderResumeContractError, ProviderResumeProvider, SpawnTool,
};

const NEW_CONVERSATION_ARG: &str = "--session-id";
const RESUME_ARG: &str = "--resume";

#[derive(Debug, Error)]
pub(crate) enum GrokCliProviderError {
    #[error("Grok executable must not be empty")]
    EmptyProgram,
    #[error(
        "Grok executable is not valid UTF-8 and cannot be represented in provider resume argv"
    )]
    NonUtf8Program,
    #[error("Grok launch cwd must be absolute: {0}")]
    RelativeCwd(PathBuf),
    #[error("Grok launch arguments cannot override provider identity or cwd")]
    ConflictingControlArgument,
    #[error("failed to spawn Grok {operation}")]
    Spawn {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("Grok {operation} rejected the conversation identity (exit code {code:?})")]
    Rejected {
        operation: &'static str,
        code: Option<i32>,
    },
    #[error("Grok launch receipt cwd mismatch: expected {expected}, got {actual:?}")]
    ReceiptCwdMismatch {
        expected: String,
        actual: Option<String>,
    },
    #[error("Grok resume receipt has the wrong provider or capture source")]
    InvalidResumeReceipt,
    #[error("Grok resume receipt argv does not match its conversation identity")]
    InvalidResumeArgv,
    #[error("observed Grok launch argv does not match the prepared preassigned launch")]
    ObservedArgvMismatch,
    #[error("observed Grok launch cwd mismatch: expected {expected}, got {actual}")]
    ObservedCwdMismatch { expected: String, actual: String },
    #[error(transparent)]
    ReceiptContract(#[from] ProviderResumeContractError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GrokCliProvider {
    program: OsString,
    program_arg: String,
    display_program: String,
}

impl GrokCliProvider {
    pub(crate) fn from_env() -> Result<Self, GrokCliProviderError> {
        let launcher = SpawnToolLauncher::from_env(SpawnTool::Grok);
        Self::from_launcher(launcher)
    }

    pub(crate) fn with_program(program: impl Into<OsString>) -> Result<Self, GrokCliProviderError> {
        let program = program.into();
        if program.is_empty() {
            return Err(GrokCliProviderError::EmptyProgram);
        }
        let program_arg = program
            .to_str()
            .ok_or(GrokCliProviderError::NonUtf8Program)?
            .to_string();
        let display_program = display_shell_arg(&program);
        Ok(Self {
            program,
            program_arg,
            display_program,
        })
    }

    fn from_launcher(launcher: SpawnToolLauncher) -> Result<Self, GrokCliProviderError> {
        let program = launcher.process_program();
        if program.is_empty() {
            return Err(GrokCliProviderError::EmptyProgram);
        }
        let program_arg = program
            .to_str()
            .ok_or(GrokCliProviderError::NonUtf8Program)?
            .to_string();
        Ok(Self {
            program,
            program_arg,
            display_program: launcher.shell_program(),
        })
    }

    pub(crate) fn prepare_new_conversation(
        &self,
        cwd: impl AsRef<Path>,
    ) -> Result<PreparedGrokLaunch, GrokCliProviderError> {
        self.prepare_launch(cwd, std::iter::empty::<OsString>())
    }

    pub(crate) fn prepare_launch<I>(
        &self,
        cwd: impl AsRef<Path>,
        trailing_args: I,
    ) -> Result<PreparedGrokLaunch, GrokCliProviderError>
    where
        I: IntoIterator<Item = OsString>,
    {
        let cwd = cwd.as_ref();
        if !cwd.is_absolute() {
            return Err(GrokCliProviderError::RelativeCwd(cwd.to_path_buf()));
        }
        let trailing_args: Vec<_> = trailing_args.into_iter().collect();
        if trailing_args.iter().any(|arg| is_control_argument(arg)) {
            return Err(GrokCliProviderError::ConflictingControlArgument);
        }

        let conversation_id = Uuid::new_v4().to_string();
        Ok(PreparedGrokLaunch {
            program: self.program.clone(),
            program_arg: self.program_arg.clone(),
            display_program: self.display_program.clone(),
            cwd: cwd.to_path_buf(),
            conversation_id,
            trailing_args,
        })
    }

    pub(crate) fn execute_to_exit(
        &self,
        launch: &PreparedGrokLaunch,
    ) -> Result<SuccessfulGrokLaunch, GrokCliProviderError> {
        let status = launch
            .command()
            .status()
            .map_err(|source| GrokCliProviderError::Spawn {
                operation: "launch",
                source,
            })?;
        self.verify_observed_launch(
            launch,
            GrokLaunchObservation::exited(launch.argv(), launch.cwd.clone(), status),
        )
    }

    pub(crate) fn verify_observed_launch(
        &self,
        launch: &PreparedGrokLaunch,
        observation: GrokLaunchObservation,
    ) -> Result<SuccessfulGrokLaunch, GrokCliProviderError> {
        launch.verify_provider(self)?;
        if observation.argv != launch.argv() {
            return Err(GrokCliProviderError::ObservedArgvMismatch);
        }
        if observation.cwd != launch.cwd {
            return Err(GrokCliProviderError::ObservedCwdMismatch {
                expected: launch.cwd.to_string_lossy().into_owned(),
                actual: observation.cwd.to_string_lossy().into_owned(),
            });
        }
        if let ObservedGrokLaunchState::Exited(status) = observation.state {
            verify_success("launch", status)?;
        }
        Ok(SuccessfulGrokLaunch {
            launch: launch.clone(),
        })
    }

    pub(crate) fn resume_to_exit(
        &self,
        receipt: &AuthorizedProviderResumeReceipt,
        cwd: impl AsRef<Path>,
    ) -> Result<(), GrokCliProviderError> {
        let conversation_id = receipt
            .conversation_id()
            .ok_or(GrokCliProviderError::InvalidResumeReceipt)?;
        if receipt.provider() != ProviderResumeProvider::Grok
            || receipt.capture_source() != ProviderResumeCaptureSource::Preassigned
            || !receipt.is_resumable()
        {
            return Err(GrokCliProviderError::InvalidResumeReceipt);
        }

        let expected_argv = [self.program_arg.as_str(), RESUME_ARG, conversation_id];
        let actual_argv = receipt
            .resume_argv()
            .ok_or(GrokCliProviderError::InvalidResumeReceipt)?;
        if actual_argv
            .iter()
            .map(String::as_str)
            .ne(expected_argv.into_iter())
        {
            return Err(GrokCliProviderError::InvalidResumeArgv);
        }

        let status = Command::new(&self.program)
            .arg(RESUME_ARG)
            .arg(conversation_id)
            .current_dir(cwd)
            .status()
            .map_err(|source| GrokCliProviderError::Spawn {
                operation: "resume",
                source,
            })?;
        verify_success("resume", status)
    }
}

#[derive(Debug)]
pub(crate) struct GrokLaunchObservation {
    argv: Vec<OsString>,
    cwd: PathBuf,
    state: ObservedGrokLaunchState,
}

impl GrokLaunchObservation {
    pub(crate) fn running(argv: Vec<OsString>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            argv,
            cwd: cwd.into(),
            state: ObservedGrokLaunchState::Running,
        }
    }

    pub(crate) fn exited(argv: Vec<OsString>, cwd: impl Into<PathBuf>, status: ExitStatus) -> Self {
        Self {
            argv,
            cwd: cwd.into(),
            state: ObservedGrokLaunchState::Exited(status),
        }
    }
}

#[derive(Debug)]
enum ObservedGrokLaunchState {
    Running,
    Exited(ExitStatus),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedGrokLaunch {
    program: OsString,
    program_arg: String,
    display_program: String,
    cwd: PathBuf,
    conversation_id: String,
    trailing_args: Vec<OsString>,
}

impl PreparedGrokLaunch {
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub(crate) fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub(crate) fn argv(&self) -> Vec<OsString> {
        std::iter::once(self.program.clone())
            .chain([
                OsString::from(NEW_CONVERSATION_ARG),
                OsString::from(&self.conversation_id),
            ])
            .chain(self.trailing_args.iter().cloned())
            .collect()
    }

    pub(crate) fn display_command(&self) -> String {
        self.argv()
            .iter()
            .map(|arg| display_shell_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .arg(NEW_CONVERSATION_ARG)
            .arg(&self.conversation_id)
            .args(&self.trailing_args)
            .current_dir(&self.cwd);
        command
    }

    fn verify_provider(&self, provider: &GrokCliProvider) -> Result<(), GrokCliProviderError> {
        if self.program != provider.program
            || self.program_arg != provider.program_arg
            || self.display_program != provider.display_program
        {
            return Err(GrokCliProviderError::InvalidResumeArgv);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct SuccessfulGrokLaunch {
    launch: PreparedGrokLaunch,
}

impl SuccessfulGrokLaunch {
    pub(crate) fn bind_receipt(
        self,
        launch_receipt: LaunchReceipt,
    ) -> Result<AuthorizedProviderResumeLaunchReceipt, GrokCliProviderError> {
        let expected_cwd = self.launch.cwd.to_string_lossy().into_owned();
        if launch_receipt.local_cwd.as_deref() != Some(expected_cwd.as_str()) {
            return Err(GrokCliProviderError::ReceiptCwdMismatch {
                expected: expected_cwd,
                actual: launch_receipt.local_cwd,
            });
        }

        let resume_argv = vec![
            self.launch.program_arg.clone(),
            RESUME_ARG.to_string(),
            self.launch.conversation_id.clone(),
        ];
        let resume_command = format!(
            "{} {RESUME_ARG} {}",
            self.launch.display_program, self.launch.conversation_id
        );
        let provider_receipt = AuthorizedProviderResumeReceipt::resumable(
            ProviderResumeProvider::Grok,
            self.launch.conversation_id,
            resume_argv,
            resume_command,
            ProviderResumeCaptureSource::Preassigned,
        )?;

        Ok(AuthorizedProviderResumeLaunchReceipt::new(
            launch_receipt,
            provider_receipt,
        ))
    }
}

fn verify_success(operation: &'static str, status: ExitStatus) -> Result<(), GrokCliProviderError> {
    if status.success() {
        Ok(())
    } else {
        Err(GrokCliProviderError::Rejected {
            operation,
            code: status.code(),
        })
    }
}

fn is_control_argument(argument: &OsStr) -> bool {
    let argument = argument.to_string_lossy();
    matches!(
        argument.as_ref(),
        "--session-id"
            | "-s"
            | "--resume"
            | "-r"
            | "--continue"
            | "-c"
            | "--fork-session"
            | "--cwd"
    ) || argument.starts_with("--session-id=")
        || argument.starts_with("--resume=")
        || argument.starts_with("--cwd=")
}

fn display_shell_arg(argument: &OsStr) -> String {
    let argument = argument.to_string_lossy();
    if !argument.is_empty()
        && argument
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-./:".contains(character))
    {
        argument.into_owned()
    } else {
        crate::launcher::shell_single_quote(&argument)
    }
}

#[cfg(test)]
#[path = "tests/grok_cli.rs"]
mod tests;
