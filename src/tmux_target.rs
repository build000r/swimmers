use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum TmuxTarget {
    #[default]
    Default,
    SocketName(String),
    SocketPath(PathBuf),
}

impl TmuxTarget {
    pub fn socket_name(name: impl Into<String>) -> Self {
        Self::SocketName(name.into())
    }

    pub fn socket_path(path: impl Into<PathBuf>) -> Self {
        Self::SocketPath(path.into())
    }

    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Default => Ok(()),
            Self::SocketName(name) => validate_socket_name(name),
            Self::SocketPath(path) => validate_socket_path(path),
        }
    }

    pub fn command_args(&self, args: &[&str]) -> Vec<String> {
        let mut out = Vec::with_capacity(args.len() + 2);
        self.push_prefix_args(&mut out);
        out.extend(args.iter().map(|arg| (*arg).to_string()));
        out
    }

    pub fn apply_to_command_builder(&self, command: &mut CommandBuilder) {
        match self {
            Self::Default => {}
            Self::SocketName(name) => command.args(["-L", name.as_str()]),
            Self::SocketPath(path) => {
                command.arg("-S");
                command.arg(path.to_string_lossy().as_ref());
            }
        }
    }

    pub fn shell_words(&self) -> Vec<String> {
        match self {
            Self::Default => Vec::new(),
            Self::SocketName(name) => vec!["-L".to_string(), name.clone()],
            Self::SocketPath(path) => vec!["-S".to_string(), path.to_string_lossy().into_owned()],
        }
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::SocketName(name) => format!("-L {name}"),
            Self::SocketPath(path) => format!("-S {}", path.display()),
        }
    }

    fn push_prefix_args(&self, out: &mut Vec<String>) {
        match self {
            Self::Default => {}
            Self::SocketName(name) => {
                out.push("-L".to_string());
                out.push(name.clone());
            }
            Self::SocketPath(path) => {
                out.push("-S".to_string());
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
}

pub fn validate_socket_name(name: &str) -> anyhow::Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("tmux socket name must not be empty"));
    }
    if trimmed != name {
        return Err(anyhow::anyhow!(
            "tmux socket name must not contain surrounding whitespace"
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(anyhow::anyhow!(
            "tmux socket name must be a name, not a path; use socket_path for explicit paths"
        ));
    }
    Ok(())
}

pub fn validate_socket_path(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("tmux socket path must not be empty"));
    }
    Ok(())
}

pub fn exact_session_target(tmux_name: &str) -> String {
    format!("={tmux_name}")
}

pub fn exact_pane_target(tmux_name: &str) -> String {
    format!("={tmux_name}:")
}

#[cfg(test)]
mod tests {
    use super::{exact_pane_target, exact_session_target, TmuxTarget};

    #[test]
    fn exact_session_target_qualifies_numeric_names() {
        assert_eq!(exact_session_target("0"), "=0");
        assert_eq!(exact_session_target("workspace"), "=workspace");
    }

    #[test]
    fn exact_pane_target_qualifies_numeric_names() {
        assert_eq!(exact_pane_target("0"), "=0:");
        assert_eq!(exact_pane_target("workspace"), "=workspace:");
    }

    #[test]
    fn tmux_target_default_leaves_command_args_unchanged() {
        assert_eq!(
            TmuxTarget::Default.command_args(&["list-sessions"]),
            vec!["list-sessions".to_string()]
        );
    }

    #[test]
    fn tmux_target_socket_name_prefixes_tmux_args() {
        assert_eq!(
            TmuxTarget::socket_name("tiktok").command_args(&["list-sessions"]),
            vec!["-L", "tiktok", "list-sessions"]
        );
    }

    #[test]
    fn tmux_target_socket_path_prefixes_tmux_args() {
        assert_eq!(
            TmuxTarget::socket_path("/tmp/tmux-critical").command_args(&["list-sessions"]),
            vec!["-S", "/tmp/tmux-critical", "list-sessions"]
        );
    }

    #[test]
    fn tmux_target_rejects_blank_or_path_like_socket_name() {
        assert!(TmuxTarget::socket_name("").validate().is_err());
        assert!(TmuxTarget::socket_name(" /tmp ").validate().is_err());
        assert!(TmuxTarget::socket_name("/tmp/tmux").validate().is_err());
    }
}
