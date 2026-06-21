use super::*;
use std::thread;
use std::time::Duration;

fn test_entries(values: Vec<Value>) -> Vec<JsonlEntry> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let raw = value.to_string();
            JsonlEntry {
                value,
                raw,
                byte_start: index as u64 * 100,
                byte_end: index as u64 * 100 + 50,
            }
        })
        .collect()
}

#[test]
fn parse_jsonl_lines_skips_bad() {
    let buf = b"{\"type\":\"user\"}\nnot json\n{\"type\":\"assistant\"}\n";
    let entries = parse_jsonl_lines(buf);
    assert_eq!(entries.len(), 2);
}

#[test]
fn parse_jsonl_entries_keeps_incomplete_tail_unconsumed() {
    let buf = b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n{\"type\":\"event_msg\"";
    let (entries, consumed_offset) = parse_jsonl_entries_and_offset(buf, 10);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].byte_start, 10);
    assert_eq!(
            entries[0].byte_end,
            10 + b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n"
                .len() as u64
        );
    assert_eq!(consumed_offset, entries[0].byte_end);
}

#[test]
fn parse_jsonl_entries_consumes_complete_malformed_lines() {
    let buf = b"not json\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"second\"}}\n";
    let (entries, consumed_offset) = parse_jsonl_entries_and_offset(buf, 3);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].byte_start, 3 + b"not json\n".len() as u64);
    assert_eq!(consumed_offset, 3 + buf.len() as u64);
}

#[test]
fn parse_jsonl_entries_holds_valid_but_unterminated_tail() {
    // A valid-JSON final segment with no trailing newline may be a torn write,
    // so it must NOT be consumed; the next read re-reads it once the newline
    // lands (swimmers-nb7g).
    let complete = b"{\"type\":\"event_msg\"}\n";
    let tail = b"{\"type\":\"user\"}"; // valid JSON, but not newline-terminated yet
    let mut buf = Vec::new();
    buf.extend_from_slice(complete);
    buf.extend_from_slice(tail);

    let (entries, consumed_offset) = parse_jsonl_entries_and_offset(&buf, 0);

    assert_eq!(
        entries.len(),
        1,
        "only the newline-terminated line is consumed"
    );
    assert_eq!(entries[0].raw, "{\"type\":\"event_msg\"}");
    assert_eq!(consumed_offset, complete.len() as u64);
}

#[test]
fn log_read_start_caps_incremental_window_to_bootstrap_max() {
    // A normal incremental read resumes at previous_size.
    assert_eq!(log_read_start(LogReadPhase::Incremental, 100, 200), 100);

    // If the file grew by more than BOOTSTRAP_MAX since the last read, only the
    // trailing window is read so the allocation stays bounded (swimmers-nb7g).
    let previous = 10;
    let current = BOOTSTRAP_MAX * 3 + previous;
    assert_eq!(
        log_read_start(LogReadPhase::Incremental, previous, current),
        current - BOOTSTRAP_MAX
    );

    // Bootstrap behavior is unchanged.
    assert_eq!(
        log_read_start(LogReadPhase::Bootstrap, previous, current),
        current - BOOTSTRAP_MAX
    );
}

#[test]
fn parse_jsonl_entries_consumes_blank_and_crlf_lines() {
    let first_blank = b"\n";
    let second_blank = b"\r\n";
    let json_line = b"{\"type\":\"event_msg\"}\r\n";
    let trailing_blank = b"\n";
    let mut buf = Vec::new();
    buf.extend_from_slice(first_blank);
    buf.extend_from_slice(second_blank);
    buf.extend_from_slice(json_line);
    buf.extend_from_slice(trailing_blank);

    let (entries, consumed_offset) = parse_jsonl_entries_and_offset(&buf, 5);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw, "{\"type\":\"event_msg\"}");
    assert_eq!(
        entries[0].byte_start,
        5 + first_blank.len() as u64 + second_blank.len() as u64
    );
    assert_eq!(
        entries[0].byte_end,
        entries[0].byte_start + json_line.len() as u64
    );
    assert_eq!(consumed_offset, 5 + buf.len() as u64);
}

#[test]
fn parse_jsonl_entries_leaves_incomplete_malformed_tail_unconsumed_after_blank() {
    let complete = b"\n{\"type\":\"event_msg\"}\n";
    let tail = b"not json";
    let mut buf = Vec::new();
    buf.extend_from_slice(complete);
    buf.extend_from_slice(tail);

    let (entries, consumed_offset) = parse_jsonl_entries_and_offset(&buf, 7);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].byte_start, 7 + b"\n".len() as u64);
    assert_eq!(consumed_offset, 7 + complete.len() as u64);
}

#[test]
fn truncate_works() {
    assert_eq!(truncate("hello", 3), "hel");
    assert_eq!(truncate("hi", 10), "hi");
}

#[test]
fn basename_extracts() {
    assert_eq!(basename("/foo/bar/baz.rs"), "baz.rs");
    assert_eq!(basename("baz.rs"), "baz.rs");
}

#[test]
fn cap_actions_limits() {
    let mut actions: Vec<AgentAction> = (0..15)
        .map(|i| AgentAction {
            tool: format!("t{i}"),
            detail: None,
        })
        .collect();
    cap_actions(&mut actions, 10);
    assert_eq!(actions.len(), 10);
    assert_eq!(actions[0].tool, "t5");
}

#[test]
fn context_reader_for_known_tools() {
    assert!(context_reader_for("Claude Code", "/tmp", &[]).is_some());
    assert!(context_reader_for("Codex", "/tmp", &[]).is_some());
    assert!(context_reader_for("Unknown", "/tmp", &[]).is_none());
}

#[test]
fn claude_user_message_text_requires_user_entry_and_role() {
    let user_msg = serde_json::json!({
        "role": "user",
        "content": "ship the fix"
    });
    let assistant_msg = serde_json::json!({
        "role": "assistant",
        "content": "not a user task"
    });

    assert_eq!(
        claude_user_message_text("user", Some(&user_msg)),
        Some("ship the fix")
    );
    assert_eq!(claude_user_message_text("assistant", Some(&user_msg)), None);
    assert_eq!(claude_user_message_text("user", Some(&assistant_msg)), None);
    assert_eq!(claude_user_message_text("user", None), None);
}

#[test]
fn claude_user_message_text_uses_first_text_block_with_text() {
    let msg = serde_json::json!({
        "role": "user",
        "content": [
            { "type": "image", "text": "ignored image text" },
            { "type": "text", "content": "ignored content field" },
            { "type": "text", "text": "first text task" },
            { "type": "text", "text": "second text task" }
        ]
    });

    assert_eq!(
        claude_user_message_text("user", Some(&msg)),
        Some("first text task")
    );
}

#[test]
fn claude_file_matches_cwd_skips_bad_lines_and_preserves_legacy_no_cwd() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let legacy = tmp.path().join("legacy.jsonl");
    fs::write(&legacy, "\nnot json\n{\"type\":\"user\"}\n").expect("legacy jsonl");

    assert!(claude_file_matches_cwd(&legacy, "/tmp/project"));
    assert!(!claude_file_matches_cwd(
        &tmp.path().join("missing.jsonl"),
        "/tmp/project"
    ));
}

#[test]
fn claude_file_matches_cwd_rejects_mismatch_and_scans_only_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mismatch = tmp.path().join("mismatch.jsonl");
    fs::write(&mismatch, "{\"cwd\":\"/tmp/other\"}\n").expect("mismatch jsonl");
    assert!(!claude_file_matches_cwd(&mismatch, "/tmp/project"));

    let late_cwd = tmp.path().join("late-cwd.jsonl");
    let mut lines = (0..64)
        .map(|_| "{\"type\":\"user\"}")
        .collect::<Vec<_>>()
        .join("\n");
    lines.push_str("\n{\"cwd\":\"/tmp/other\"}\n");
    fs::write(&late_cwd, lines).expect("late cwd jsonl");
    assert!(claude_file_matches_cwd(&late_cwd, "/tmp/project"));
}

#[test]
fn plan_log_read_classifies_unchanged_incremental_bootstrap_and_truncated() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("session.jsonl");
    fs::write(&path, b"0123456789").expect("session");

    assert!(plan_log_read(Some(&path), 10, true, path.clone()).is_none());

    let incremental =
        plan_log_read(Some(&path), 5, true, path.clone()).expect("incremental read plan");
    assert_eq!(incremental.start, 5);
    assert_eq!(incremental.phase, LogReadPhase::Incremental);
    assert!(!incremental.reset_reader);

    let bootstrap = plan_log_read(None, 0, false, path.clone()).expect("bootstrap read plan");
    assert_eq!(bootstrap.start, 0);
    assert_eq!(bootstrap.phase, LogReadPhase::Bootstrap);
    assert!(bootstrap.reset_reader);

    let truncated =
        plan_log_read(Some(&path), 20, true, path.clone()).expect("truncated read plan");
    assert_eq!(truncated.start, 0);
    assert_eq!(truncated.phase, LogReadPhase::Bootstrap);
    assert!(truncated.reset_reader);
}

#[test]
fn codex_reader_matches_cwd_with_large_session_meta_line() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("rollout-large-meta.jsonl");
    let large_instructions = "x".repeat(4096);
    fs::write(
            &path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"/tmp/project\",\"base_instructions\":{{\"text\":\"{}\"}}}}}}\n",
                large_instructions
            ),
        )
        .expect("write rollout");

    let reader = CodexReader::new("/tmp/project", &[]);
    assert!(reader.matches_cwd(&path));
}

#[test]
fn codex_reader_discovery_skips_excluded_non_rollout_and_uses_reverse_order() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let older_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("03")
        .join("16");
    let newer_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("03")
        .join("17");
    fs::create_dir_all(&older_dir).expect("older sessions dir");
    fs::create_dir_all(&newer_dir).expect("newer sessions dir");

    let older_match = older_dir.join("rollout-z.jsonl");
    fs::write(
        &older_match,
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
    )
    .expect("older match");

    let wrong_cwd = newer_dir.join("rollout-c.jsonl");
    fs::write(
        &wrong_cwd,
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/other\"}}\n",
    )
    .expect("wrong cwd");
    let excluded = newer_dir.join("rollout-b.jsonl");
    fs::write(
        &excluded,
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
    )
    .expect("excluded");
    let selected = newer_dir.join("rollout-a.jsonl");
    fs::write(
        &selected,
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
    )
    .expect("selected");
    fs::write(
        newer_dir.join("session-newest.jsonl"),
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
    )
    .expect("non rollout");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let reader = CodexReader::new("/tmp/project", &[excluded]);
    assert_eq!(reader.discover_file(), Some(selected));

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn codex_file_matches_cwd_rejects_malformed_and_non_meta_first_lines() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let empty = tmp.path().join("rollout-empty.jsonl");
    fs::write(&empty, "").expect("empty");
    let malformed = tmp.path().join("rollout-malformed.jsonl");
    fs::write(&malformed, "{not json}\n").expect("malformed");
    let non_meta = tmp.path().join("rollout-non-meta.jsonl");
    fs::write(
        &non_meta,
        "{\"type\":\"event_msg\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
    )
    .expect("non meta");
    let missing_cwd = tmp.path().join("rollout-missing-cwd.jsonl");
    fs::write(&missing_cwd, "{\"type\":\"session_meta\",\"payload\":{}}\n").expect("missing cwd");

    assert!(!codex_file_matches_cwd(&empty, "/tmp/project"));
    assert!(!codex_file_matches_cwd(&malformed, "/tmp/project"));
    assert!(!codex_file_matches_cwd(&non_meta, "/tmp/project"));
    assert!(!codex_file_matches_cwd(&missing_cwd, "/tmp/project"));
}

#[test]
fn codex_reader_consumes_token_count_event_and_context_window() {
    let mut reader = CodexReader::new("/tmp", &[]);
    let entries = test_entries(vec![serde_json::json!({
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "total_token_usage": { "input_tokens": 99_735_u64 }
            },
            "model_context_window": 258_400_u64
        }
    })]);

    reader.parse_entries(&entries);

    assert_eq!(reader.token_count, 99_735);
    assert_eq!(reader.context_limit, 258_400);
}

#[test]
fn codex_reader_keeps_previous_context_limit_when_event_lacks_window() {
    let mut reader = CodexReader::new("/tmp", &[]);
    let default_limit = reader.context_limit;
    let entries = test_entries(vec![serde_json::json!({
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "total_token_usage": { "input_tokens": 12_345_u64 }
            }
        }
    })]);

    reader.parse_entries(&entries);

    assert_eq!(reader.token_count, 12_345);
    assert_eq!(reader.context_limit, default_limit);
}

#[test]
fn codex_reader_captures_last_reasoning_summary_text() {
    let mut reader = CodexReader::new("/tmp", &[]);
    let entries = test_entries(vec![serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "reasoning",
            "summary": [
                { "type": "other", "text": "ignored" },
                { "type": "summary_text", "text": "first summary" },
                { "type": "summary_text" },
                { "type": "summary_text", "text": "final summary" }
            ]
        }
    })]);

    reader.parse_entries(&entries);

    let current_tool = reader
        .current_tool
        .expect("summary should set thinking tool");
    assert_eq!(current_tool.tool, "thinking");
    assert_eq!(current_tool.detail.as_deref(), Some("final summary"));
}

#[test]
fn codex_reader_ignores_non_reasoning_summary_payloads() {
    let mut reader = CodexReader::new("/tmp", &[]);
    let entries = test_entries(vec![
        serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "wrong entry type" }]
            }
        }),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "summary": [{ "type": "summary_text", "text": "wrong payload type" }]
            }
        }),
    ]);

    reader.parse_entries(&entries);

    assert!(reader.current_tool.is_none());
}

#[test]
fn codex_reader_truncates_reasoning_summary_thinking_detail() {
    let mut reader = CodexReader::new("/tmp", &[]);
    let long_summary = "x".repeat(120);
    let entries = test_entries(vec![serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": long_summary }]
        }
    })]);

    reader.parse_entries(&entries);

    let current_tool = reader
        .current_tool
        .expect("summary should set thinking tool");
    let expected = "x".repeat(100);
    assert_eq!(current_tool.detail.as_deref(), Some(expected.as_str()));
}

#[test]
fn claude_reader_discovery_filters_slug_collision_by_exact_cwd() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");

    let cwd_a = "/tmp/a-b/c";
    let cwd_b = "/tmp/a/b-c";
    let slug_a = cwd_a.replace('/', "-");
    let slug_b = cwd_b.replace('/', "-");
    assert_eq!(slug_a, slug_b, "test requires slug collision");

    let project_dir = tmp.path().join(".claude").join("projects").join(slug_a);
    fs::create_dir_all(&project_dir).expect("mkdir");

    let file_a = project_dir.join("session-a.jsonl");
    fs::write(
            &file_a,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_A\"}}}}\n",
                cwd_a
            ),
        )
        .expect("write file a");
    thread::sleep(Duration::from_millis(50));

    let file_b = project_dir.join("session-b.jsonl");
    fs::write(
            &file_b,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_B\"}}}}\n",
                cwd_b
            ),
        )
        .expect("write file b");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let reader = ClaudeCodeReader::new(cwd_a, &[]);
    let discovered = reader.discover_file();
    assert_eq!(discovered, Some(file_a));

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn claude_reader_discovery_uses_jsonl_exclusions_and_newest_mtime() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = "/tmp/project-discovery";
    let slug = cwd.replace('/', "-");
    let project_dir = tmp.path().join(".claude").join("projects").join(slug);
    fs::create_dir_all(&project_dir).expect("project dir");

    let old = project_dir.join("old.jsonl");
    fs::write(
            &old,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"old\"}}}}\n",
                cwd
            ),
        )
        .expect("old jsonl");
    thread::sleep(Duration::from_millis(50));

    let next = project_dir.join("next.jsonl");
    fs::write(
            &next,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"next\"}}}}\n",
                cwd
            ),
        )
        .expect("next jsonl");
    thread::sleep(Duration::from_millis(50));

    let excluded = project_dir.join("excluded.jsonl");
    fs::write(
            &excluded,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"excluded\"}}}}\n",
                cwd
            ),
        )
        .expect("excluded jsonl");
    thread::sleep(Duration::from_millis(50));

    let txt = project_dir.join("newest.txt");
    fs::write(
            &txt,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"txt\"}}}}\n",
                cwd
            ),
        )
        .expect("txt file");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let reader = ClaudeCodeReader::new(cwd, &[excluded]);
    assert_eq!(reader.discover_file(), Some(next));

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn claude_reader_read_bootstraps_and_then_reads_incremental_updates() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = "/tmp/project-alpha";
    let slug = cwd.replace('/', "-");
    let project_dir = tmp.path().join(".claude").join("projects").join(slug);
    fs::create_dir_all(&project_dir).expect("project dir");
    let session_file = project_dir.join("session.jsonl");
    fs::write(
            &session_file,
            format!(
                concat!(
                    "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"investigate startup\"}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input_tokens\":321}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"exec\",\"input\":{{\"cmd\":\"ls\"}}}}]}}}}\n"
                ),
                cwd = cwd
            ),
        )
        .expect("session file");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let mut reader = ClaudeCodeReader::new(cwd, &[]);
    let first = reader.read().expect("bootstrap snapshot");
    assert_eq!(first.user_task.as_deref(), Some("investigate startup"));
    assert_eq!(first.user_turns.len(), 1);
    assert_eq!(first.user_turns[0].text, "investigate startup");
    assert_eq!(first.user_turns[0].source, "Claude Code");
    assert!(
        first
            .transcript_records
            .iter()
            .any(|record| record.kind == "assistant_message"),
        "assistant records should remain in the post-turn transcript source"
    );
    assert_eq!(first.token_count, 321);
    assert_eq!(
        first.current_tool.as_ref().map(|tool| tool.tool.as_str()),
        Some("exec")
    );
    assert!(reader.read().is_none(), "no new data should yield None");

    fs::write(
            &session_file,
            format!(
                concat!(
                    "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"investigate startup\"}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input_tokens\":321}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"exec\",\"input\":{{\"cmd\":\"ls\"}}}}]}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"done reading logs\"}}]}}}}\n"
                ),
                cwd = cwd
            ),
        )
        .expect("append assistant line");

    let second = reader.read().expect("incremental snapshot");
    assert_eq!(second.user_task.as_deref(), Some("investigate startup"));
    assert_eq!(second.user_turns.len(), 1);
    assert!(
        second
            .recent_actions
            .iter()
            .any(|action| action.tool == "said"),
        "incremental assistant text should be recorded"
    );

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn read_range_returns_empty_when_end_le_start() {
    // Regression: previously this underflowed `(end - start) as usize` for
    // reversed ranges (which the readers can pass when a JSONL file gets
    // truncated in place between ticks), producing a panic in debug builds
    // and a multi-exabyte allocation request in release builds.
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("payload.jsonl");
    fs::write(&path, b"hello world").expect("write");

    let buf = read_range(&path, 5, 5).expect("eq range");
    assert!(buf.is_empty());

    let buf = read_range(&path, 9, 3).expect("reversed range");
    assert!(buf.is_empty());
}

#[test]
fn claude_reader_recovers_when_file_is_truncated_between_reads() {
    // Regression: previously a JSONL file truncated in place between ticks
    // (log rotation, agent rewrote the file) would feed `read_range` a
    // reversed byte range and panic the reader.
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = "/tmp/project-truncate";
    let slug = cwd.replace('/', "-");
    let project_dir = tmp.path().join(".claude").join("projects").join(slug);
    fs::create_dir_all(&project_dir).expect("project dir");
    let session_file = project_dir.join("session.jsonl");

    let initial = format!(
            concat!(
                "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"first task\"}}}}\n",
                "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input_tokens\":111}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"exec\",\"input\":{{\"cmd\":\"ls\"}}}}]}}}}\n",
                "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"long enough first text\"}}]}}}}\n"
            ),
            cwd = cwd
        );
    fs::write(&session_file, &initial).expect("session file");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let mut reader = ClaudeCodeReader::new(cwd, &[]);
    let first = reader.read().expect("bootstrap snapshot");
    assert_eq!(first.user_task.as_deref(), Some("first task"));
    assert_eq!(first.token_count, 111);

    // Truncate the file in place to a strictly shorter, valid payload.
    let shorter = format!(
            "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"new task\"}}}}\n",
            cwd = cwd
        );
    assert!(
        shorter.len() < initial.len(),
        "test requires shorter post-truncation payload"
    );
    fs::write(&session_file, &shorter).expect("truncate file");

    // Must not panic and must reflect the new, post-truncation state.
    let after = reader.read().expect("post-truncation snapshot");
    assert_eq!(after.user_task.as_deref(), Some("new task"));
    // token_count was reset on truncation; the new payload has no usage.
    assert_eq!(after.token_count, 0);

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn codex_reader_read_discovers_matching_rollout_and_tracks_incremental_usage() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("03")
        .join("16");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let other = sessions_dir.join("rollout-other.jsonl");
    fs::write(
        &other,
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/other\"}}\n",
    )
    .expect("other rollout");

    let target = sessions_dir.join("rollout-target.jsonl");
    fs::write(
            &target,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fix websocket bug\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"cmd\\\":\\\"git status\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":555}},\"model_context_window\":258400}}\n"
            ),
        )
        .expect("target rollout");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let mut reader = CodexReader::new("/tmp/project", &[]);
    let first = reader.read().expect("bootstrap snapshot");
    assert_eq!(first.user_task.as_deref(), Some("fix websocket bug"));
    assert_eq!(first.user_turns.len(), 1);
    assert_eq!(first.user_turns[0].text, "fix websocket bug");
    assert_eq!(first.user_turns[0].source, "Codex");
    assert!(
        first
            .transcript_records
            .iter()
            .any(|record| record.kind == "function_call"),
        "tool records should remain transcript records but not turns"
    );
    assert_eq!(first.token_count, 555);
    assert_eq!(first.context_limit, 258_400);
    assert_eq!(
        first.current_tool.as_ref().map(|tool| tool.tool.as_str()),
        Some("exec")
    );

    fs::write(
            &target,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fix websocket bug\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"cmd\\\":\\\"git status\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":555}},\"model_context_window\":258400}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"tighten the retry path\"}}\n"
            ),
        )
        .expect("updated rollout");

    let second = reader.read().expect("incremental snapshot");
    assert_eq!(second.user_task.as_deref(), Some("tighten the retry path"));
    assert_eq!(second.user_turns.len(), 2);
    assert_eq!(
        second.user_turns.last().map(|turn| turn.text.as_str()),
        Some("tighten the retry path")
    );
    assert!(
        reader.read().is_none(),
        "steady state should not re-emit snapshot"
    );

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn codex_reader_does_not_advance_past_partial_jsonl_tail() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("03")
        .join("16");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let target = sessions_dir.join("rollout-partial.jsonl");
    let prefix = concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n"
    );
    let partial =
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"second\"";
    fs::write(&target, format!("{prefix}{partial}")).expect("partial rollout");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let mut reader = CodexReader::new("/tmp/project", &[]);
    let first = reader.read().expect("first snapshot");
    assert_eq!(first.user_task.as_deref(), Some("first"));

    fs::write(&target, format!("{}{}{}\n", prefix, partial, "}}")).expect("complete rollout");
    let second = reader.read().expect("completed tail snapshot");
    assert_eq!(second.user_task.as_deref(), Some("second"));

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn codex_reader_rediscover_after_claimed_file_is_deleted() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("03")
        .join("16");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let first_path = sessions_dir.join("rollout-a.jsonl");
    fs::write(
            &first_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n"
            ),
        )
        .expect("first rollout");

    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", tmp.path());

    let mut reader = CodexReader::new("/tmp/project", &[]);
    assert_eq!(
        reader
            .read()
            .and_then(|snapshot| snapshot.user_task)
            .as_deref(),
        Some("first")
    );
    fs::remove_file(&first_path).expect("remove first rollout");
    let second_path = sessions_dir.join("rollout-b.jsonl");
    fs::write(
            &second_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"second\"}}\n"
            ),
        )
        .expect("second rollout");

    assert_eq!(
        reader
            .read()
            .and_then(|snapshot| snapshot.user_task)
            .as_deref(),
        Some("second")
    );

    if let Some(prev) = previous_home {
        std::env::set_var("HOME", prev);
    } else {
        std::env::remove_var("HOME");
    }
}
