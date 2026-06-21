use std::io::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::tmux_target::exact_pane_target;

use super::run_bounded_tmux_command;

const TMUX_SEND_KEYS_TIMEOUT: Duration = Duration::from_millis(500);
const TMUX_PASTE_BUFFER_TIMEOUT: Duration = Duration::from_secs(2);
const TMUX_AGENT_SUBMIT_DOUBLE_ENTER_DELAY: Duration = Duration::from_millis(75);

static NEXT_TMUX_SUBMIT_BUFFER_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn write_input_counts_as_activity(data: &[u8]) -> bool {
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0x1b
            && index + 2 < data.len()
            && data[index + 1] == b'['
            && matches!(data[index + 2], b'I' | b'O')
        {
            index += 3;
            continue;
        }

        return true;
    }

    false
}

pub(super) fn write_and_flush_input(
    writer: &mut Box<dyn std::io::Write + Send>,
    data: &[u8],
) -> io::Result<()> {
    writer.write_all(data)?;
    writer.flush()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TmuxInputChunk {
    Literal(String),
    Enter,
}

#[derive(Debug)]
pub(super) struct TmuxInputSendError {
    pub(super) delivered_chunks: usize,
    source: anyhow::Error,
}

impl std::fmt::Display for TmuxInputSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for TmuxInputSendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.source()
    }
}

pub(super) fn tmux_input_chunks(data: &[u8]) -> Option<Vec<TmuxInputChunk>> {
    let text = std::str::from_utf8(data).ok()?;
    let chunks = tmux_input_text_chunks(text)?;
    (!chunks.is_empty()).then_some(chunks)
}

fn tmux_input_text_chunks(text: &str) -> Option<Vec<TmuxInputChunk>> {
    let mut chunks = Vec::new();
    let mut literal = String::new();

    for ch in text.chars() {
        push_tmux_input_char(&mut chunks, &mut literal, ch)?;
    }

    flush_tmux_input_literal(&mut chunks, &mut literal);
    Some(chunks)
}

fn push_tmux_input_char(
    chunks: &mut Vec<TmuxInputChunk>,
    literal: &mut String,
    ch: char,
) -> Option<()> {
    if is_tmux_input_enter(ch) {
        flush_tmux_input_literal(chunks, literal);
        chunks.push(TmuxInputChunk::Enter);
        return Some(());
    }

    if is_rejected_tmux_input_control(ch) {
        return None;
    }

    literal.push(ch);
    Some(())
}

fn flush_tmux_input_literal(chunks: &mut Vec<TmuxInputChunk>, literal: &mut String) {
    if !literal.is_empty() {
        chunks.push(TmuxInputChunk::Literal(std::mem::take(literal)));
    }
}

fn is_tmux_input_enter(ch: char) -> bool {
    matches!(ch, '\r' | '\n')
}

fn is_rejected_tmux_input_control(ch: char) -> bool {
    ch == '\t' || ch.is_control()
}

pub(super) fn normalize_submit_line_text(text: &str) -> Option<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim_end_matches('\n').to_string();
    (!normalized.trim().is_empty()).then_some(normalized)
}

pub(super) fn submit_line_fallback_input(text: &str) -> Vec<u8> {
    let mut input = text.as_bytes().to_vec();
    input.extend_from_slice(b"\r\r");
    input
}

pub(super) async fn send_tmux_input_chunks(
    tmux_name: &str,
    chunks: &[TmuxInputChunk],
) -> Result<(), TmuxInputSendError> {
    let target = exact_pane_target(tmux_name);
    let _ = send_tmux_keys(&target, &["-X", "cancel"]).await;
    let mut delivered_chunks = 0;
    for chunk in chunks {
        let result = match chunk {
            // `--` ends send-keys option parsing so a literal beginning with
            // `-` (e.g. a user typing `-rf`, `--flag`, or `-N5`) is typed
            // verbatim instead of being swallowed as a send-keys flag -- tmux's
            // getopt does not stop after `-l`. Without it, `-N5` exits 0 having
            // typed nothing (a silent partial delivery falsely counted as ok),
            // and `-X` errors. Mirrors the set-buffer path's `--`.
            TmuxInputChunk::Literal(text) => send_tmux_keys(&target, &["-l", "--", text]).await,
            TmuxInputChunk::Enter => send_tmux_keys(&target, &["Enter"]).await,
        };
        match result {
            Ok(()) => delivered_chunks += 1,
            Err(source) => {
                return Err(TmuxInputSendError {
                    delivered_chunks,
                    source,
                });
            }
        }
    }
    Ok(())
}

pub(super) async fn send_tmux_submit_line(tmux_name: &str, text: &str) -> anyhow::Result<()> {
    let target = exact_pane_target(tmux_name);
    let _ = send_tmux_keys(&target, &["-X", "cancel"]).await;
    let buffer_name = next_tmux_submit_buffer_name();
    set_tmux_buffer(&buffer_name, text).await?;
    paste_tmux_buffer(&target, &buffer_name).await?;
    send_tmux_keys(&target, &["Enter"]).await?;
    tokio::time::sleep(TMUX_AGENT_SUBMIT_DOUBLE_ENTER_DELAY).await;
    send_tmux_keys(&target, &["Enter"]).await?;
    Ok(())
}

fn next_tmux_submit_buffer_name() -> String {
    let id = NEXT_TMUX_SUBMIT_BUFFER_ID.fetch_add(1, Ordering::Relaxed);
    format!("swimmers-submit-{}-{id}", std::process::id())
}

async fn set_tmux_buffer(buffer_name: &str, text: &str) -> anyhow::Result<()> {
    let output = run_bounded_tmux_command(
        "tmux",
        &["set-buffer", "-b", buffer_name, "--", text],
        TMUX_PASTE_BUFFER_TIMEOUT,
        "set-buffer",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux set-buffer exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(())
}

async fn paste_tmux_buffer(target: &str, buffer_name: &str) -> anyhow::Result<()> {
    let output = run_bounded_tmux_command(
        "tmux",
        &["paste-buffer", "-dpr", "-b", buffer_name, "-t", target],
        TMUX_PASTE_BUFFER_TIMEOUT,
        "paste-buffer",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux paste-buffer exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(())
}

async fn send_tmux_keys(target: &str, keys: &[&str]) -> anyhow::Result<()> {
    let mut args = vec!["send-keys", "-t", target];
    args.extend_from_slice(keys);
    let output =
        run_bounded_tmux_command("tmux", &args, TMUX_SEND_KEYS_TIMEOUT, "send-keys").await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux send-keys exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{tmux_input_chunks, TmuxInputChunk};

    fn literal(text: &str) -> TmuxInputChunk {
        TmuxInputChunk::Literal(text.to_string())
    }

    #[test]
    fn tmux_input_chunks_rejects_empty_input() {
        assert_eq!(tmux_input_chunks(b""), None);
    }

    #[test]
    fn tmux_input_chunks_splits_cr_as_enter() {
        assert_eq!(
            tmux_input_chunks(b"left\rright"),
            Some(vec![
                literal("left"),
                TmuxInputChunk::Enter,
                literal("right")
            ])
        );
    }

    #[test]
    fn tmux_input_chunks_splits_lf_as_enter() {
        assert_eq!(
            tmux_input_chunks(b"left\nright"),
            Some(vec![
                literal("left"),
                TmuxInputChunk::Enter,
                literal("right")
            ])
        );
    }

    #[test]
    fn tmux_input_chunks_keeps_crlf_as_two_enters() {
        assert_eq!(
            tmux_input_chunks(b"left\r\nright"),
            Some(vec![
                literal("left"),
                TmuxInputChunk::Enter,
                TmuxInputChunk::Enter,
                literal("right"),
            ])
        );
    }

    #[test]
    fn tmux_input_chunks_keeps_consecutive_enters() {
        assert_eq!(
            tmux_input_chunks(b"left\n\nright"),
            Some(vec![
                literal("left"),
                TmuxInputChunk::Enter,
                TmuxInputChunk::Enter,
                literal("right"),
            ])
        );
    }

    #[test]
    fn tmux_input_chunks_flushes_literal_before_enter() {
        assert_eq!(
            tmux_input_chunks(b"literal\r"),
            Some(vec![literal("literal"), TmuxInputChunk::Enter])
        );
    }

    #[test]
    fn tmux_input_chunks_coalesces_all_literal_input() {
        assert_eq!(
            tmux_input_chunks(b"one two three"),
            Some(vec![literal("one two three")])
        );
    }

    #[test]
    fn tmux_input_chunks_rejects_tabs() {
        assert_eq!(tmux_input_chunks(b"left\tright"), None);
    }

    #[test]
    fn tmux_input_chunks_rejects_other_controls() {
        assert_eq!(tmux_input_chunks(b"left\x00right"), None);
        assert_eq!(tmux_input_chunks(b"left\x1bright"), None);
        assert_eq!(tmux_input_chunks(b"left\x7fright"), None);
    }

    #[test]
    fn tmux_input_chunks_rejects_invalid_utf8() {
        assert_eq!(tmux_input_chunks(&[0xff]), None);
    }
}
