use std::collections::HashSet;
use std::fs;
use std::path::Path;
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

fn normalize_skill_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if is_valid_skill_name_token(trimmed) {
        Some(trimmed.to_ascii_lowercase())
    } else {
        None
    }
}

fn is_valid_skill_name_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_skill_name_byte)
}

fn is_skill_name_byte(byte: u8) -> bool {
    matches!(byte, 65..=90 | 97..=122 | 48..=57 | 45 | 95 | 46 | 47)
}

fn is_probable_skill_name(raw: &str) -> bool {
    let Some(normalized) = normalize_skill_name(raw) else {
        return false;
    };

    if is_builtin_skill_name(&normalized) {
        return true;
    }

    installed_skill_decision(&normalized).accepts()
}

fn is_builtin_skill_name(normalized: &str) -> bool {
    matches!(normalized, "commit" | "describe" | "domain-planner" | "gog")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstalledSkillDecision {
    CompleteRegistry { contains: bool },
    PartialRegistry { contains: bool },
    Unavailable,
}

impl InstalledSkillDecision {
    fn accepts(self) -> bool {
        match self {
            Self::CompleteRegistry { contains } => contains,
            Self::PartialRegistry { contains } => contains,
            Self::Unavailable => false,
        }
    }
}

fn installed_skill_decision(normalized: &str) -> InstalledSkillDecision {
    static INSTALLED_SKILLS: OnceLock<Option<HashSet<String>>> = OnceLock::new();

    match INSTALLED_SKILLS.get_or_init(load_installed_skill_names) {
        Some(installed) if installed.len() >= 5 => InstalledSkillDecision::CompleteRegistry {
            contains: installed.contains(normalized),
        },
        Some(installed) => InstalledSkillDecision::PartialRegistry {
            contains: installed.contains(normalized),
        },
        None => InstalledSkillDecision::Unavailable,
    }
}

fn load_installed_skill_names() -> Option<HashSet<String>> {
    let home = std::env::var("HOME").ok()?;
    non_empty_skill_names(load_installed_skill_names_from_home(Path::new(&home)))
}

fn load_installed_skill_names_from_home(home: &Path) -> HashSet<String> {
    let mut names = HashSet::new();

    for rel_root in [".codex/skills", ".claude/skills"] {
        collect_installed_skill_names(&home.join(rel_root), &mut names);
    }

    names
}

fn collect_installed_skill_names(root: &Path, names: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        if let Some(name) = installed_skill_dir_name(entry) {
            names.insert(name);
        }
    }
}

fn installed_skill_dir_name(entry: fs::DirEntry) -> Option<String> {
    let file_type = entry.file_type().ok()?;
    let path = entry.path();
    let is_skill_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
    if !is_skill_dir {
        return None;
    }

    normalize_skill_name(&entry.file_name().to_string_lossy())
}

fn non_empty_skill_names(names: HashSet<String>) -> Option<HashSet<String>> {
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
    fn load_installed_skill_names_from_home_returns_empty_for_missing_roots() {
        let temp = tempfile::tempdir().expect("tempdir");

        assert!(load_installed_skill_names_from_home(temp.path()).is_empty());
        assert!(non_empty_skill_names(HashSet::new()).is_none());
    }

    #[test]
    fn load_installed_skill_names_from_home_collects_valid_skill_dirs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex = temp.path().join(".codex/skills");
        let claude = temp.path().join(".claude/skills");
        std::fs::create_dir_all(codex.join("Commit")).expect("codex skill");
        std::fs::create_dir_all(codex.join("bad skill")).expect("invalid skill");
        std::fs::create_dir_all(claude.join("domain-planner")).expect("claude skill");
        std::fs::write(codex.join("describe"), "not a directory").expect("file entry");

        let names = load_installed_skill_names_from_home(temp.path());

        assert!(names.contains("commit"));
        assert!(names.contains("domain-planner"));
        assert!(!names.contains("bad skill"));
        assert!(!names.contains("describe"));
        assert_eq!(
            non_empty_skill_names(names.clone())
                .expect("non-empty names")
                .len(),
            names.len()
        );
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
