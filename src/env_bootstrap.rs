use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHELL_CAPTURE_START: &str = "__SWIMMERS_ENV_CAPTURE_START__";
const SHELL_CAPTURE_END: &str = "__SWIMMERS_ENV_CAPTURE_END__";
const FALLBACK_SHELLS: &[&str] = &["/bin/zsh", "/bin/bash"];
const PROVIDER_ENV_VARS: &[&str] = &["OPENROUTER_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

pub fn bootstrap_provider_env_from_shell() {
    let Some(shell) = detect_interactive_shell() else {
        return;
    };

    assign_missing_provider_env_from_shell(&shell);
}

fn assign_missing_provider_env_from_shell(shell: &Path) {
    for key in missing_provider_env_vars() {
        assign_provider_env_from_shell(shell, key);
    }
}

fn env_value_present(key: &str) -> bool {
    env::var(key)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn detect_interactive_shell() -> Option<PathBuf> {
    select_interactive_shell(
        env::var_os("SHELL").map(PathBuf::from),
        fallback_shell_candidates(),
        Path::is_file,
    )
}

fn fallback_shell_candidates() -> impl Iterator<Item = PathBuf> {
    FALLBACK_SHELLS.iter().map(PathBuf::from)
}

fn select_interactive_shell<I, F>(
    shell_env: Option<PathBuf>,
    fallback_shells: I,
    is_file: F,
) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
    F: Fn(&Path) -> bool,
{
    if let Some(shell) = shell_env.filter(|path| is_file(path)) {
        return Some(shell);
    }

    fallback_shells.into_iter().find(|path| is_file(path))
}

fn missing_provider_env_vars() -> impl Iterator<Item = &'static str> {
    PROVIDER_ENV_VARS
        .iter()
        .copied()
        .filter(|key| !env_value_present(key))
}

fn assign_provider_env_from_shell(shell: &Path, key: &str) {
    if let Some(value) = provider_assignment_from_shell(shell, key) {
        env::set_var(key, value);
    }
}

fn provider_assignment_from_shell(shell: &Path, key: &str) -> Option<String> {
    assignable_env_value(read_env_var_from_interactive_shell(shell, key))
}

fn assignable_env_value(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
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

    #[test]
    fn assignable_env_value_keeps_non_blank_values() {
        assert_eq!(
            assignable_env_value(Some("sk-test".to_string())).as_deref(),
            Some("sk-test")
        );
    }

    #[test]
    fn assignable_env_value_rejects_blank_values() {
        assert_eq!(assignable_env_value(Some(" \t ".to_string())), None);
    }

    #[test]
    fn assignable_env_value_preserves_value_whitespace_when_non_blank() {
        assert_eq!(
            assignable_env_value(Some(" sk-test ".to_string())).as_deref(),
            Some(" sk-test ")
        );
    }

    #[test]
    fn select_interactive_shell_prefers_valid_shell_env() {
        let shell_env = PathBuf::from("/custom/shell");
        let selected = select_interactive_shell(
            Some(shell_env.clone()),
            [
                PathBuf::from("/fallback/zsh"),
                PathBuf::from("/fallback/bash"),
            ],
            |path| path == shell_env,
        );

        assert_eq!(selected.as_deref(), Some(shell_env.as_path()));
    }

    #[test]
    fn select_interactive_shell_rejects_missing_shell_env() {
        let fallback = PathBuf::from("/fallback/zsh");
        let selected = select_interactive_shell(
            Some(PathBuf::from("/custom/missing")),
            [fallback.clone()],
            |path| path == fallback,
        );

        assert_eq!(selected.as_deref(), Some(fallback.as_path()));
    }

    #[test]
    fn select_interactive_shell_rejects_non_file_shell_env() {
        let fallback = PathBuf::from("/fallback/bash");
        let selected = select_interactive_shell(
            Some(PathBuf::from("/custom/directory")),
            [fallback.clone()],
            |path| path == fallback,
        );

        assert_eq!(selected.as_deref(), Some(fallback.as_path()));
    }

    #[test]
    fn select_interactive_shell_uses_first_available_fallback() {
        let zsh = PathBuf::from("/fallback/zsh");
        let bash = PathBuf::from("/fallback/bash");
        let selected = select_interactive_shell(None, [zsh.clone(), bash], |path| path == zsh);

        assert_eq!(selected.as_deref(), Some(zsh.as_path()));
    }

    #[test]
    fn select_interactive_shell_tries_later_fallbacks_when_first_is_missing() {
        let bash = PathBuf::from("/fallback/bash");
        let selected = select_interactive_shell(
            None,
            [PathBuf::from("/fallback/zsh"), bash.clone()],
            |path| path == bash,
        );

        assert_eq!(selected.as_deref(), Some(bash.as_path()));
    }

    #[test]
    fn select_interactive_shell_returns_none_without_valid_candidates() {
        let selected = select_interactive_shell(
            Some(PathBuf::from("/custom/missing")),
            [
                PathBuf::from("/fallback/zsh"),
                PathBuf::from("/fallback/bash"),
            ],
            |_| false,
        );

        assert_eq!(selected, None);
    }
}
