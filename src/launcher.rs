use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::types::SpawnTool;

pub const SWIMMERS_GROK_BIN_ENV: &str = "SWIMMERS_GROK_BIN";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnToolLauncher {
    tool: SpawnTool,
    program_override: Option<OsString>,
}

impl SpawnToolLauncher {
    pub fn from_env(tool: SpawnTool) -> Self {
        let program_override = env_override_for_tool(tool);
        Self {
            tool,
            program_override,
        }
    }

    #[cfg(test)]
    pub fn with_program_override(tool: SpawnTool, program_override: Option<OsString>) -> Self {
        Self {
            tool,
            program_override,
        }
    }

    pub fn process_program(&self) -> OsString {
        self.program_override
            .clone()
            .unwrap_or_else(|| OsString::from(self.tool.command()))
    }

    pub fn shell_program(&self) -> String {
        match &self.program_override {
            Some(program) => shell_single_quote(&program.to_string_lossy()),
            None => self.tool.command().to_string(),
        }
    }
}

fn env_override_for_tool(tool: SpawnTool) -> Option<OsString> {
    match tool {
        SpawnTool::Grok => non_empty_env(SWIMMERS_GROK_BIN_ENV),
        SpawnTool::Claude | SpawnTool::Codex => None,
    }
}

fn non_empty_env(key: &str) -> Option<OsString> {
    let value = std::env::var_os(key)?;
    (!value.is_empty()).then_some(value)
}

pub fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn prepare_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("prompt directory is a symlink: {}", path.display()),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("prompt path is not a directory: {}", path.display()),
        ));
    }
    set_private_dir_permissions(path)
}

pub fn create_private_file(path: &Path) -> io::Result<fs::File> {
    create_private_file_impl(path)
}

pub fn write_private_file(path: &Path, contents: &str) -> io::Result<()> {
    let mut file = create_private_file(path)?;
    file.write_all(contents.as_bytes())
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_private_file_impl(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file_impl(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_uses_default_program_without_override() {
        let launcher = SpawnToolLauncher::with_program_override(SpawnTool::Grok, None);

        assert_eq!(launcher.process_program(), OsString::from("grok"));
        assert_eq!(launcher.shell_program(), "grok");
    }

    #[test]
    fn launcher_shell_quotes_override_but_process_keeps_raw_program() {
        let launcher = SpawnToolLauncher::with_program_override(
            SpawnTool::Grok,
            Some(OsString::from("/tmp/agent bins/grok's wrapper")),
        );

        assert_eq!(
            launcher.process_program(),
            OsString::from("/tmp/agent bins/grok's wrapper")
        );
        assert_eq!(
            launcher.shell_program(),
            "'/tmp/agent bins/grok'\\''s wrapper'"
        );
    }
}
