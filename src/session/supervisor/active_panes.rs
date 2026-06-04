use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::warn;

use crate::session::actor::run_bounded_tmux_command;

const ACTIVE_PANE_LOOKUP_TIMEOUT: Duration = Duration::from_millis(500);
const ACTIVE_PANE_LOOKUP_WARN_THRESHOLD: Duration = Duration::from_millis(200);
const TMUX_LIST_PANES_FIELD_SEPARATOR: char = '\x1f';

fn format_tmux_active_pane_session_id(tmux_name: &str, pane_selector: &str) -> String {
    format!("tmux:{tmux_name}:{pane_selector}")
}

/// Runs `tmux list-panes -a` once and returns every session's active-pane id.
/// Callers that only care about specific tmux session names should pair this
/// with [`filter_active_panes_to_requested`]; keeping the query unfiltered
/// lets the supervisor share one tmux call across callers within the cache
/// TTL window.
pub(super) async fn query_all_active_pane_session_ids() -> anyhow::Result<HashMap<String, String>> {
    let started = Instant::now();
    let pane_format = format!(
        "#{{session_name}}{sep}#{{window_active}}{sep}#{{pane_active}}{sep}#{{window_index}}.#{{pane_index}}:#{{pane_id}}",
        sep = TMUX_LIST_PANES_FIELD_SEPARATOR
    );
    let output = run_bounded_tmux_command(
        "tmux",
        &["list-panes", "-a", "-F", pane_format.as_str()],
        ACTIVE_PANE_LOOKUP_TIMEOUT,
        "list-panes",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("tmux list-panes failed: {}", stderr.trim()));
    }

    let elapsed = started.elapsed();
    if elapsed >= ACTIVE_PANE_LOOKUP_WARN_THRESHOLD {
        warn!(
            phase = "tmux_list_panes",
            elapsed_ms = elapsed.as_millis() as u64,
            "tmux active pane lookup completed slowly"
        );
    }

    Ok(parse_active_pane_session_ids(&output.stdout))
}

#[derive(Debug, PartialEq, Eq)]
struct ActivePaneFields<'a> {
    session_name: &'a str,
    window_active: &'a str,
    pane_active: &'a str,
    pane_selector: &'a str,
}

impl<'a> ActivePaneFields<'a> {
    fn active_pane_selector(&self) -> Option<(&'a str, &'a str)> {
        if self.session_name.is_empty()
            || self.window_active != "1"
            || self.pane_active != "1"
            || self.pane_selector.is_empty()
        {
            return None;
        }

        Some((self.session_name, self.pane_selector))
    }
}

fn parse_active_pane_fields(line: &str) -> Option<ActivePaneFields<'_>> {
    let mut fields = line.splitn(4, TMUX_LIST_PANES_FIELD_SEPARATOR);
    Some(ActivePaneFields {
        session_name: fields.next()?,
        window_active: fields.next()?,
        pane_active: fields.next()?,
        pane_selector: fields.next()?,
    })
}

fn active_pane_selector_from_line(line: &str) -> Option<(&str, &str)> {
    parse_active_pane_fields(line)?.active_pane_selector()
}

fn parse_active_pane_session_ids(stdout: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(active_pane_selector_from_line)
        .map(|(session_name, pane_selector)| {
            (
                session_name.to_string(),
                format_tmux_active_pane_session_id(session_name, pane_selector),
            )
        })
        .collect()
}

pub(super) fn filter_active_panes_to_requested(
    all: &HashMap<String, String>,
    tmux_names: &HashSet<String>,
) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(tmux_names.len().min(all.len()));
    for name in tmux_names {
        if let Some(id) = all.get(name) {
            out.insert(name.clone(), id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use super::*;

    fn write_executable(path: &std::path::Path, contents: &str) {
        std::fs::write(path, contents).expect("write executable");
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(path, perms).expect("set executable");
    }

    fn prepend_test_path(dir: &std::path::Path, original_path: Option<&std::ffi::OsStr>) {
        let mut paths = vec![dir.to_path_buf()];
        if let Some(original_path) = original_path {
            paths.extend(std::env::split_paths(original_path));
        }
        let joined = std::env::join_paths(paths).expect("join PATH");
        std::env::set_var("PATH", joined);
    }

    fn restore_test_path(original_path: Option<std::ffi::OsString>) {
        if let Some(value) = original_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[tokio::test]
    async fn query_tmux_active_pane_session_ids_uses_list_panes_and_supports_numeric_names() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        let command_file = dir.path().join("tmux-command.txt");
        write_executable(
            &bin_dir.join("tmux"),
            &format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" > \"{}\"\nsep=$(printf '\\037')\nprintf '0%s1%s1%s0.0:%%1\\n' \"$sep\" \"$sep\" \"$sep\"\nprintf 'work%s0%s1%s1.0:%%9\\n' \"$sep\" \"$sep\" \"$sep\"\nprintf 'work%s1%s1%s1.1:%%2\\n' \"$sep\" \"$sep\" \"$sep\"\n",
                command_file.display()
            ),
        );
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());

        let requested = HashSet::from_iter(["0".to_string(), "work".to_string()]);
        let all = query_all_active_pane_session_ids()
            .await
            .expect("active pane session ids");
        let pane_ids = filter_active_panes_to_requested(&all, &requested);
        assert_eq!(pane_ids.get("0").map(String::as_str), Some("tmux:0:0.0:%1"));
        assert_eq!(
            pane_ids.get("work").map(String::as_str),
            Some("tmux:work:1.1:%2")
        );
        assert_eq!(
            std::fs::read_to_string(&command_file).expect("command file"),
            "list-panes\n"
        );

        restore_test_path(original_path);
    }

    #[test]
    fn parse_active_pane_fields_splits_unit_separator_fields_only() {
        let fields = parse_active_pane_fields("work\tspace\x1f1\x1f0\x1f1.1:%2")
            .expect("active pane fields");

        assert_eq!(
            fields,
            ActivePaneFields {
                session_name: "work\tspace",
                window_active: "1",
                pane_active: "0",
                pane_selector: "1.1:%2",
            }
        );
    }

    #[test]
    fn parse_active_pane_session_ids_preserves_tabs_in_session_names() {
        let stdout = b"work\tspace\x1f1\x1f1\x1f1.1:%2\nother\x1f1\x1f0\x1f1.0:%1\n";

        let panes = parse_active_pane_session_ids(stdout);

        assert_eq!(
            panes.get("work\tspace").map(String::as_str),
            Some("tmux:work\tspace:1.1:%2")
        );
        assert!(!panes.contains_key("other"));
    }

    #[test]
    fn parse_active_pane_session_ids_filters_inactive_windows_and_panes() {
        let stdout = b"window-off\x1f0\x1f1\x1f1.0:%1\npane-off\x1f1\x1f0\x1f1.1:%2\nactive\x1f1\x1f1\x1f2.0:%3\n";

        let panes = parse_active_pane_session_ids(stdout);

        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes.get("active").map(String::as_str),
            Some("tmux:active:2.0:%3")
        );
        assert!(!panes.contains_key("window-off"));
        assert!(!panes.contains_key("pane-off"));
    }

    #[test]
    fn parse_active_pane_session_ids_filters_empty_pane_selectors() {
        let stdout = b"missing-selector\x1f1\x1f1\x1f\nactive\x1f1\x1f1\x1f2.0:%3\n";

        let panes = parse_active_pane_session_ids(stdout);

        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes.get("active").map(String::as_str),
            Some("tmux:active:2.0:%3")
        );
        assert!(!panes.contains_key("missing-selector"));
    }
}
