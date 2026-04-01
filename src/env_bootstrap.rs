use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHELL_CAPTURE_START: &str = "__SWIMMERS_ENV_CAPTURE_START__";
const SHELL_CAPTURE_END: &str = "__SWIMMERS_ENV_CAPTURE_END__";
const PROVIDER_ENV_VARS: &[&str] = &["OPENROUTER_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

pub fn bootstrap_provider_env_from_shell() {
    let Some(shell) = detect_interactive_shell() else {
        return;
    };

    for key in PROVIDER_ENV_VARS {
        if env_value_present(key) {
            continue;
        }

        let Some(value) = read_env_var_from_interactive_shell(&shell, key) else {
            continue;
        };

        if !value.trim().is_empty() {
            env::set_var(key, value);
        }
    }
}

fn env_value_present(key: &str) -> bool {
    env::var(key)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn detect_interactive_shell() -> Option<PathBuf> {
    env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| fallback_shell("/bin/zsh"))
        .or_else(|| fallback_shell("/bin/bash"))
}

fn fallback_shell(path: &str) -> Option<PathBuf> {
    let shell = PathBuf::from(path);
    shell.is_file().then_some(shell)
}

fn read_env_var_from_interactive_shell(shell: &Path, key: &str) -> Option<String> {
    let script = format!(
        "printf '%s\\n' '{SHELL_CAPTURE_START}'; printf '%s\\n' \"${{{key}:-}}\"; printf '%s\\n' '{SHELL_CAPTURE_END}'"
    );
    let output = Command::new(shell).arg("-ic").arg(script).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_marked_value(&stdout)
}

fn parse_marked_value(output: &str) -> Option<String> {
    let mut in_capture = false;
    for line in output.lines() {
        if line == SHELL_CAPTURE_START {
            in_capture = true;
            continue;
        }
        if line == SHELL_CAPTURE_END {
            return None;
        }
        if in_capture {
            return Some(line.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_marked_value_returns_captured_line() {
        let output = format!("noise\n{SHELL_CAPTURE_START}\nsk-test\n{SHELL_CAPTURE_END}\n");
        assert_eq!(parse_marked_value(&output).as_deref(), Some("sk-test"));
    }

    #[test]
    fn parse_marked_value_returns_empty_string_when_capture_is_blank() {
        let output = format!("{SHELL_CAPTURE_START}\n\n{SHELL_CAPTURE_END}\n");
        assert_eq!(parse_marked_value(&output).as_deref(), Some(""));
    }

    #[test]
    fn parse_marked_value_ignores_missing_capture() {
        assert_eq!(parse_marked_value("no markers"), None);
    }
}
