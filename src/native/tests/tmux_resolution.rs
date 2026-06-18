use super::*;

// ------------------------------------------------------------------
// WG-S3: native-tmux-resolution (describe-tmux-resolution.md)
// ------------------------------------------------------------------

// Case 1: SWIMMERS_TMUX_BIN set to a relative path -> error names env var
// and "not absolute".
#[test]
fn tmux_resolution_case1_env_override_non_absolute_errors() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let original = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, "tmux");

    let err = resolve_tmux_binary().expect_err("relative env path must error");

    match original {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    let message = format!("{err:#}");
    assert!(
        message.contains(TMUX_BIN_ENV),
        "error names env var: {message}"
    );
    assert!(
        message.contains("not absolute"),
        "error cites non-absolute path: {message}"
    );
}

// Case 2: SWIMMERS_TMUX_BIN set to an absolute path that does not exist.
#[test]
fn tmux_resolution_case2_env_override_missing_file_errors() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let temp = tempdir().unwrap();
    let ghost_path = temp.path().join("nonexistent-tmux");
    let ghost_str = ghost_path.to_string_lossy().to_string();
    let original = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, &ghost_path);

    let err = resolve_tmux_binary().expect_err("missing-file env path must error");

    match original {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    let message = format!("{err:#}");
    assert!(
        message.contains(TMUX_BIN_ENV),
        "error names env var: {message}"
    );
    assert!(
        message.contains(&ghost_str) || message.contains("binary not found"),
        "error cites missing path: {message}"
    );
}

// Case 3: find_binary_in_path_os returns None when PATH has no tmux.
// Exercises the PATH-lookup tier without requiring full fallback absence.
#[test]
fn tmux_resolution_case3_path_without_tmux_returns_none() {
    let temp = tempdir().unwrap();
    let empty_dir = temp.path().join("empty");
    std::fs::create_dir_all(&empty_dir).unwrap();
    let path_os = std::env::join_paths([empty_dir.as_path()]).unwrap();
    assert!(find_binary_in_path_os("tmux", &path_os).is_none());
}

// Case 4: find_binary_in_path_os finds tmux on PATH — documents that a
// PATH hit returns before any fallback iteration would run.
#[test]
fn tmux_resolution_case4_path_beats_fallbacks() {
    let temp = tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let tmux_path = bin_dir.join("tmux");
    std::fs::write(&tmux_path, b"#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&tmux_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux_path, perms).unwrap();

    let path_os = std::env::join_paths([bin_dir.as_path()]).unwrap();
    let found = find_binary_in_path_os("tmux", &path_os).expect("PATH hit must resolve");
    assert_eq!(found, tmux_path);
    // None of the hardcoded fallback paths point inside the tempdir, so
    // the resolver would never have reached them.
    assert!(
        !TMUX_BIN_FALLBACKS
            .iter()
            .any(|candidate| found.starts_with(candidate)),
        "PATH-resolved tmux must not coincide with a hardcoded fallback"
    );
}
