use super::*;

#[test]
fn enter_terminal_ui_enables_bracketed_paste_with_mouse_capture() {
    let mut output = Vec::new();

    enter_terminal_ui(&mut output).expect("enter terminal UI should write ANSI codes");

    assert_eq!(
        String::from_utf8(output).expect("terminal startup output should be valid utf-8"),
        EXPECTED_TERMINAL_ENTRY
    );
}

#[test]
fn leave_terminal_ui_disables_bracketed_paste_before_leaving_alt_screen() {
    let mut output = Vec::new();

    leave_terminal_ui(&mut output).expect("leave terminal UI should write ANSI codes");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
}

#[test]
fn cleanup_is_noop_when_renderer_is_inactive() {
    let mut renderer = test_renderer(80, 24);

    renderer.cleanup().expect("inactive cleanup should succeed");

    assert!(!renderer.terminal_state.raw_mode_enabled);
    assert!(!renderer.terminal_state.terminal_ui_active);
}

#[test]
fn cleanup_after_runtime_error_restores_terminal_in_reverse_order() {
    let mut terminal_state = TerminalState::default();
    let mut output = Vec::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    terminal_state
        .init_with(
            &mut output,
            {
                let events = Arc::clone(&events);
                move || {
                    events.lock().unwrap().push("enable_raw_mode");
                    Ok(())
                }
            },
            {
                let events = Arc::clone(&events);
                move |_writer| {
                    events.lock().unwrap().push("enter_terminal_ui");
                    Ok(())
                }
            },
        )
        .expect("terminal init should succeed");

    terminal_state
        .cleanup_with(
            &mut output,
            {
                let events = Arc::clone(&events);
                move |writer| {
                    events.lock().unwrap().push("leave_terminal_ui");
                    leave_terminal_ui(writer)
                }
            },
            {
                let events = Arc::clone(&events);
                move || {
                    events.lock().unwrap().push("disable_raw_mode");
                    Ok(())
                }
            },
        )
        .expect("cleanup should succeed after a runtime error");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "enable_raw_mode",
            "enter_terminal_ui",
            "leave_terminal_ui",
            "disable_raw_mode",
        ]
    );
}

#[test]
fn failed_init_still_runs_full_cleanup_once() {
    let mut terminal_state = TerminalState::default();
    let mut output = Vec::new();
    let leave_calls = TestCell::new(0usize);
    let disable_calls = TestCell::new(0usize);

    let err = terminal_state
        .init_with(
            &mut output,
            || Ok(()),
            |_writer| Err(io::Error::other("forced init failure")),
        )
        .expect_err("init should surface the forced failure");
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "forced init failure");

    terminal_state
        .cleanup_with(
            &mut output,
            |writer| {
                leave_calls.set(leave_calls.get() + 1);
                leave_terminal_ui(writer)
            },
            || {
                disable_calls.set(disable_calls.get() + 1);
                Ok(())
            },
        )
        .expect("cleanup should restore the terminal after init failure");

    terminal_state
        .cleanup_with(
            &mut output,
            |writer| {
                leave_calls.set(leave_calls.get() + 1);
                leave_terminal_ui(writer)
            },
            || {
                disable_calls.set(disable_calls.get() + 1);
                Ok(())
            },
        )
        .expect("second cleanup should be a no-op");

    assert_eq!(
        String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
        EXPECTED_TERMINAL_TEARDOWN
    );
    assert_eq!(leave_calls.get(), 1);
    assert_eq!(disable_calls.get(), 1);
    assert!(!terminal_state.raw_mode_enabled);
    assert!(!terminal_state.terminal_ui_active);
}
