use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;

pub(crate) fn detect_skill_from_input_line(line: &str) -> Option<String> {
    extract_skill_from_xml_block(line)
        .or_else(|| extract_skill_from_dollar_token(line))
        .or_else(|| extract_skill_from_slash_token(line))
        .or_else(|| extract_skill_from_using_marker(line))
}

pub(crate) fn drain_completed_input_lines(buffer: &mut String, data: &[u8]) -> Vec<String> {
    let mut completed = Vec::new();
    if data.is_empty() {
        return completed;
    }

    let text = String::from_utf8_lossy(data);
    for ch in text.chars() {
        match ch {
            '\r' | '\n' => {
                let line = buffer.trim().to_string();
                buffer.clear();
                if !line.is_empty() {
                    completed.push(line);
                }
            }
            // Ctrl+C/Ctrl+D should discard any partially typed command line.
            '\u{3}' | '\u{4}' => {
                buffer.clear();
            }
            '\u{8}' | '\u{7f}' => {
                buffer.pop();
            }
            _ if ch.is_control() => {}
            _ => {
                buffer.push(ch);
                if buffer.len() > 8_192 {
                    buffer.clear();
                }
            }
        }
    }

    completed
}

fn extract_skill_from_xml_block(text: &str) -> Option<String> {
    static SKILL_XML_RE: OnceLock<Regex> = OnceLock::new();
    let re = SKILL_XML_RE.get_or_init(|| {
        Regex::new(
            r"(?is)<skill\b[^>]*>.*?<name>\s*([A-Za-z][A-Za-z0-9._/-]{0,63})\s*</name>.*?</skill>",
        )
        .expect("valid skill xml regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_dollar_token(text: &str) -> Option<String> {
    static DOLLAR_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = DOLLAR_SKILL_RE.get_or_init(|| {
        Regex::new(r"\$([A-Za-z][A-Za-z0-9_-]{0,63})").expect("valid dollar skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_slash_token(text: &str) -> Option<String> {
    static SLASH_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = SLASH_SKILL_RE.get_or_init(|| {
        Regex::new(r#"^\s*/([A-Za-z][A-Za-z0-9._-]{0,63})(?:\s|$)"#)
            .expect("valid slash skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter(|value| !is_common_filesystem_root_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn extract_skill_from_using_marker(text: &str) -> Option<String> {
    static USING_SKILL_RE: OnceLock<Regex> = OnceLock::new();
    let re = USING_SKILL_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\busing\s+(?:the\s+)?skill\s+[`"']?([A-Za-z][A-Za-z0-9._/-]{0,63})[`"']?(?:\s+skill)?\b"#,
        )
        .expect("valid using skill regex")
    });

    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
        .filter(|value| is_probable_skill_name(value))
        .filter_map(normalize_skill_name)
        .last()
}

fn normalize_skill_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    is_valid_skill_name_token(trimmed).then(|| trimmed.to_ascii_lowercase())
}

fn is_valid_skill_name_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_skill_name_byte)
}

fn is_skill_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-_./".contains(&byte)
}

fn is_probable_skill_name(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }

    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if is_builtin_skill_name(&normalized) {
        return true;
    }

    if let Some(installed) = installed_skill_names() {
        // Only enforce strict membership when the discovered registry looks
        // complete enough to trust; tiny registries are often partial.
        if installed.len() >= 5 {
            return installed.contains(&normalized);
        }
        if installed.contains(&normalized) {
            return true;
        }
    }

    false
}

fn is_builtin_skill_name(normalized: &str) -> bool {
    matches!(normalized, "commit" | "describe" | "domain-planner" | "gog")
}

fn installed_skill_names() -> Option<&'static HashSet<String>> {
    static INSTALLED_SKILLS: OnceLock<Option<HashSet<String>>> = OnceLock::new();
    INSTALLED_SKILLS
        .get_or_init(load_installed_skill_names)
        .as_ref()
}

fn load_installed_skill_names() -> Option<HashSet<String>> {
    let home = std::env::var("HOME").ok()?;
    let mut names = HashSet::new();

    for rel_root in [".codex/skills", ".claude/skills"] {
        let root = PathBuf::from(&home).join(rel_root);
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let is_skill_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
            if !is_skill_dir {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(normalized) = normalize_skill_name(&name) {
                names.insert(normalized);
            }
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn is_common_filesystem_root_name(raw: &str) -> bool {
    matches!(
        raw.to_ascii_lowercase().as_str(),
        "bin"
            | "dev"
            | "etc"
            | "home"
            | "lib"
            | "lib64"
            | "mnt"
            | "opt"
            | "private"
            | "proc"
            | "sbin"
            | "sys"
            | "tmp"
            | "usr"
            | "users"
            | "var"
            | "volumes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_skill_name_rejects_blank_and_invalid_values() {
        assert_eq!(normalize_skill_name("  "), None);
        assert_eq!(normalize_skill_name("bad!skill"), None);
        assert_eq!(normalize_skill_name(" Commit "), Some("commit".to_string()));
    }

    #[test]
    fn normalize_skill_name_preserves_existing_character_policy() {
        let cases = [
            ("skill-name", Some("skill-name")),
            ("skill_name", Some("skill_name")),
            ("skill.name", Some("skill.name")),
            ("path/to-skill", Some("path/to-skill")),
            ("UPPER/Case_1", Some("upper/case_1")),
            ("bad skill", None),
            ("skill:bad", None),
            ("skíll", None),
        ];

        for (raw, expected) in cases {
            assert_eq!(normalize_skill_name(raw).as_deref(), expected, "{raw}");
        }
    }

    #[test]
    fn detect_skill_prefers_explicit_skill_block() {
        let line = r#"send <skill><name>describe</name></skill> and $fallback"#;
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("describe".to_string())
        );
    }

    #[test]
    fn detect_skill_falls_back_to_dollar_token() {
        let line = "please run $domain-planner for this slice";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("domain-planner".to_string())
        );
    }

    #[test]
    fn detect_skill_records_full_commit_name() {
        let line = "$commit";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("commit".to_string())
        );
    }

    #[test]
    fn detect_skill_ignores_short_partial_dollar_tokens() {
        assert_eq!(detect_skill_from_input_line("$c"), None);
        assert_eq!(detect_skill_from_input_line("$com"), None);
        assert_eq!(detect_skill_from_input_line("$comm"), None);
    }

    #[test]
    fn detect_skill_falls_back_to_slash_token() {
        let line = "/describe";
        assert_eq!(
            detect_skill_from_input_line(line),
            Some("describe".to_string())
        );
    }

    #[test]
    fn detect_skill_ignores_common_root_path_slash_token() {
        let line = "/tmp";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_common_shell_env_vars() {
        let line = "echo $HOME && echo $PATH";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_unknown_dollar_token() {
        let line = "please run $notarealskillzzzzz";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn detect_skill_ignores_generic_using_phrase_without_skill_keyword() {
        let line = "using decision heuristics for this pass";
        assert_eq!(detect_skill_from_input_line(line), None);
    }

    #[test]
    fn completed_lines_drop_partial_skill_on_ctrl_c_carriage_return() {
        let mut buffer = String::new();
        assert!(drain_completed_input_lines(&mut buffer, b"$c").is_empty());
        assert_eq!(buffer, "$c");

        let lines = drain_completed_input_lines(&mut buffer, b"\x03\r");
        assert!(lines.is_empty());
        assert!(buffer.is_empty());
    }

    #[test]
    fn completed_lines_emit_full_skill_after_chunked_input() {
        let mut buffer = String::new();
        assert!(drain_completed_input_lines(&mut buffer, b"$com").is_empty());
        let lines = drain_completed_input_lines(&mut buffer, b"mit\r");
        assert_eq!(lines, vec!["$commit".to_string()]);
    }
}
