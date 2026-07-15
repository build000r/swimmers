use super::*;

// ------------------------------------------------------------------
// Characterization tests for the Ghostty double-click handoff bug.
//
// Workgraph: local divide-and-conquer investigation
//   divide-and-conquer/2026-04-23T13-08-14Z/
// Describe packets:
//   - describe-swap-fallback.md       (TC-1..TC-5)
//   - describe-front-tab-empty.md     (TC-1..TC-5)
//   - describe-attach-race.md         (2 cases)
//   - describe-tmux-resolution.md     (4 cases)
//
// These tests characterize current behavior. Tests suffixed
// `_documents_bug` intentionally assert remaining *buggy* observable state
// so the behavior regresses loudly if someone changes it.
// ------------------------------------------------------------------

// Shared fake-binary builder to keep the new tests terse.
//
// osascript behavior:
//   - `-e "...get version..."`        -> prints `1.3.1`
//   - `-e "...selected tab..."`       -> prints tab_id_stdout (may be empty)
//                                         or exits non-zero if tab_id_err_exit
//   - any other argv (the script run) -> logs argv to `<log>` and prints
//                                         `created|<pane_prefix><N>` where
//                                         N is the sequential call count.
struct GhosttyFakes {
    _temp: tempfile::TempDir,
    fake_tmux: PathBuf,
    fake_osascript_dir: PathBuf,
    log_path: PathBuf,
}

fn write_ghostty_fakes(
    tab_id_stdout: &str,
    tab_id_err_exit: bool,
    pane_prefix: &str,
) -> GhosttyFakes {
    write_ghostty_fakes_with_status(tab_id_stdout, tab_id_err_exit, pane_prefix, "created")
}

fn write_ghostty_fakes_with_status(
    tab_id_stdout: &str,
    tab_id_err_exit: bool,
    pane_prefix: &str,
    result_status: &str,
) -> GhosttyFakes {
    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("tmux-bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();
    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/tmp/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let fake_osascript_dir = temp.path().join("osa-bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    let counter_path = temp.path().join("osa-count");
    let tab_err = if tab_id_err_exit { 1 } else { 0 };
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      if [ \"{tab_err}\" = \"1\" ]; then\n        printf 'ghostty tab query failed\\n' >&2\n        exit 1\n      fi\n      printf '{tab}\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nprintf '{status}|{prefix}%s\\n' \"$count\"\n",
                tab = tab_id_stdout,
                tab_err = tab_err,
                log = log_path.display(),
                counter = counter_path.display(),
                prefix = pane_prefix,
                status = result_status,
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    GhosttyFakes {
        _temp: temp,
        fake_tmux,
        fake_osascript_dir,
        log_path,
    }
}

// RAII wrapper that installs fake PATH + SWIMMERS_TMUX_BIN and restores them.
struct EnvSwap {
    original_path: Option<OsString>,
    original_tmux: Option<OsString>,
}

impl EnvSwap {
    fn install(fakes: &GhosttyFakes) -> Self {
        let original_path = std::env::var_os("PATH");
        let original_tmux = std::env::var_os(TMUX_BIN_ENV);
        let path_value = std::env::join_paths([fakes.fake_osascript_dir.as_path()]).unwrap();
        std::env::set_var("PATH", path_value);
        std::env::set_var(TMUX_BIN_ENV, &fakes.fake_tmux);
        Self {
            original_path,
            original_tmux,
        }
    }
}

impl Drop for EnvSwap {
    fn drop(&mut self) {
        match self.original_path.take() {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match self.original_tmux.take() {
            Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
            None => std::env::remove_var(TMUX_BIN_ENV),
        }
    }
}

// ------------------------------------------------------------------
// WG-S1: ghostty-swap-fallback (describe-swap-fallback.md)
// ------------------------------------------------------------------

// TC-1: Preview cache miss with title-prefix match — arg 8 is omitted.
#[tokio::test]
async fn swap_fallback_tc1_cache_miss_omits_known_preview_id() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "pane-tc1-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc1",
        "tmux-tc1",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    // argv: [script, sess, tmux, cwd, attach, display, prefix, mode]
    // Cache was empty, so no 8th index (index 8 would be the known preview id).
    assert_eq!(
        call.len(),
        8,
        "no known_preview_id arg expected on cache miss"
    );
    assert_eq!(call[7], GhosttyOpenMode::Swap.label());

    clear_ghostty_preview_term_cache();
}

// TC-2: Stale cache references a live but unlabeled terminal — arg 8 passed.
#[tokio::test]
async fn swap_fallback_tc2_stale_cache_passes_preview_id_arg() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-stale"));
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "pane-tc2-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc2",
        "tmux-tc2",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "fallback_created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(
        call.len(),
        9,
        "stale cache entry should be forwarded as arg 8"
    );
    assert_eq!(call[8], "term-stale");
    assert!(cached_ghostty_preview_term_id(Some("ghostty-tab-main")).is_none());

    clear_ghostty_preview_term_cache();
}

// TC-3 / TC-4: A stale known preview id that falls back to create-new is
// surfaced distinctly and does not overwrite the preview cache.
#[tokio::test]
async fn swap_fallback_tc3_tc4_stale_create_new_is_reported_and_not_cached() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-stale"));
    // Simulate the script choosing to createPreviewSplit instead of swap.
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "fresh-pane-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc3",
        "tmux-tc3",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(
        result.status, "fallback_created",
        "stale-id create-new fallback should be visible to callers"
    );
    assert_eq!(result.pane_id.as_deref(), Some("fresh-pane-1"));
    assert!(cached_ghostty_preview_term_id(Some("ghostty-tab-main")).is_none());

    clear_ghostty_preview_term_cache();
}

// TC-5: Successful swap baseline — cached id forwarded, cache updated.
#[tokio::test]
async fn swap_fallback_tc5_golden_path_forwards_cached_id_and_updates_cache() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-preview"));
    let fakes =
        write_ghostty_fakes_with_status("ghostty-tab-main", false, "new-preview-", "swapped");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc5",
        "tmux-tc5",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(call[8], "term-preview", "cached id must be forwarded");
    assert_eq!(result.pane_id.as_deref(), Some("new-preview-1"));
    assert_eq!(
        cached_ghostty_preview_term_id(Some("ghostty-tab-main")).as_deref(),
        Some("new-preview-1"),
    );

    clear_ghostty_preview_term_cache();
}

// ------------------------------------------------------------------
// WG-S4: front-tab-id empty/error path (describe-front-tab-empty.md)
// ------------------------------------------------------------------

// TC-1: query_front_ghostty_tab_id returns Ok(None) — cache lookup skipped,
// script invoked with known_preview_id=None even if an unrelated cache
// entry exists.
#[tokio::test]
async fn front_tab_empty_tc1_ok_empty_skips_cache_lookup() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    // Tab query returns empty string -> Ok(None).
    let fakes = write_ghostty_fakes("", false, "pane-ft1-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft1",
        "tmux-ft1",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(call.len(), 8, "no cache arg when tab id is empty");
    // Unrelated cache entry must not be written over, and no new entry
    // must be written under an empty key.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-known")).as_deref(),
        Some("term-prev"),
    );

    clear_ghostty_preview_term_cache();
}

// TC-2: query_front_ghostty_tab_id returns Err(...) — error absorbed,
// swap attempt proceeds.
#[tokio::test]
async fn front_tab_empty_tc2_err_absorbed_swap_proceeds() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    // Tab query exits non-zero -> Err absorbed by .unwrap_or(None).
    let fakes = write_ghostty_fakes("", true, "pane-ft2-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft2",
        "tmux-ft2",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(
        call.len(),
        8,
        "tab-id Err must not be forwarded as a cache arg"
    );

    clear_ghostty_preview_term_cache();
}

// TC-3 / TC-4: When pre-script AND post-script tab query both fail / return
// empty, the resulting pane id is NOT written into the cache, because
// remember_ghostty_preview_term_id short-circuits on an empty tab id.
// Unrelated entries remain intact.
#[tokio::test]
async fn front_tab_empty_tc3_tc4_no_stale_cache_write_when_tab_id_missing() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    let fakes = write_ghostty_fakes("", false, "pane-ft3-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft3",
        "tmux-ft3",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.pane_id.as_deref(), Some("pane-ft3-1"));
    // Cache under the empty key: none created.
    assert!(cached_ghostty_preview_term_id(Some("")).is_none());
    // Cache for an unrelated tab: untouched.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-known")).as_deref(),
        Some("term-prev"),
    );

    clear_ghostty_preview_term_cache();
}

// TC-5 (characterization variant): remember_ghostty_preview_term_id writes
// through cleanly when the resulting tab id is Some(non_empty), even if
// the pre-script tab query was missing. Purely tests the cache shape.
#[test]
fn front_tab_empty_tc5_post_script_recovery_writes_cache() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    // No entry pre-call.
    assert!(cached_ghostty_preview_term_id(Some("tab-recovered")).is_none());
    // Simulate post-script success path directly.
    remember_ghostty_preview_term_id(Some("tab-recovered"), Some("new-term-5"));
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-recovered")).as_deref(),
        Some("new-term-5"),
    );
    clear_ghostty_preview_term_cache();
}

// ------------------------------------------------------------------
// WG-S2: ghostty-attach-race (describe-attach-race.md)
// ------------------------------------------------------------------

// Case 1 + Case 2: the Rust layer has no retry or readiness gate on the
// attach command. A single script invocation returning `created|<id>` is
// treated as full success, and the pane id is cached — regardless of
// whether the shell actually accepted the attach keystrokes. The race
// itself lives in Ghostty's input buffer and cannot be unit-tested here;
// this test characterizes the absent retry/validation at the Rust layer.
#[tokio::test]
async fn attach_race_rust_layer_has_no_retry_or_readiness_probe_documents_bug() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "race-pane-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-race",
        "tmux-race",
        &crate::tmux_target::TmuxTarget::Default,
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    // Exactly ONE script run logged — no retry on potential attach drop.
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let script_invocations = log.lines().count();
    assert_eq!(
        script_invocations, 1,
        "bug: no retry/readiness probe after the script returns — the Rust \
             layer cannot detect a dropped attach command, so the pane id is \
             cached as success regardless of shell readiness"
    );
    // Cached as if successful.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("ghostty-tab-main")).as_deref(),
        Some("race-pane-1"),
    );

    clear_ghostty_preview_term_cache();
}
