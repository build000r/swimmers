use super::*;
use swimmers::types::LaunchPathMapping;

fn remote_target_mapping_alpha() -> LaunchTargetSummary {
    LaunchTargetSummary {
        id: "devbox".to_string(),
        label: "Devbox".to_string(),
        kind: "swimmers_api".to_string(),
        base_url: Some("http://127.0.0.1:3210".to_string()),
        auth_token_env: None,
        path_mappings: vec![LaunchPathMapping {
            local_prefix: TEST_REPO_ALPHA.to_string(),
            remote_prefix: "/srv/repos/alpha".to_string(),
        }],
    }
}

#[test]
fn render_picker_uses_current_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("buildooor");
    fs::create_dir_all(&repo_root).expect("create repo");
    write_repo_theme_file(&repo_root, "#B89875");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    let expected = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    assert_eq!(
        cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.content.y).fg,
        expected
    );
    assert_eq!(
        cell_at(
            &renderer,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        )
        .fg,
        expected
    );
}

#[test]
fn picker_theme_color_for_path_keeps_stored_theme_body_while_adjusting_display_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("skills");
    fs::create_dir_all(repo_root.join("src")).expect("create repo");
    write_repo_theme_file(&repo_root, "#3930B5");
    let colors_path = repo_root.join(".swimmers").join("colors.json");
    let original = fs::read_to_string(&colors_path).expect("read colors.json");
    let theme_id = repo_root.to_string_lossy().into_owned();
    let mut repo_themes = HashMap::new();

    let color = picker_theme_color_for_path(theme_id.as_str(), &mut repo_themes)
        .expect("theme color should resolve");

    assert_ne!(color, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(color);
    assert_eq!(
        repo_themes
            .get(theme_id.as_str())
            .expect("theme should be cached")
            .body,
        "#3930B5"
    );
    assert_eq!(
        fs::read_to_string(colors_path).expect("reread colors.json"),
        original
    );
}

#[test]
fn render_picker_adjusts_low_contrast_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("skills");
    fs::create_dir_all(repo_root.join("src")).expect("create repo");
    write_repo_theme_file(&repo_root, "#3930B5");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let expected = picker.current_theme_color.expect("current theme color");
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(expected);
    assert_eq!(picker.entry_theme_colors, vec![Some(expected)]);
    assert_eq!(
        cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.content.y + 1).fg,
        expected
    );
    assert_eq!(
        cell_at(
            &renderer,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        )
        .fg,
        expected
    );
    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
        expected
    );
}

#[test]
fn render_picker_uses_entry_repo_theme_color() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("swimmers");
    fs::create_dir_all(&repo_root).expect("create repo");
    write_repo_theme_file(&repo_root, "#4FA66A");

    let mut picker = PickerState::new(
        2,
        2,
        dir_response(
            temp.path().to_string_lossy().as_ref(),
            &[("swimmers", true)],
        ),
        true,
        SpawnTool::Codex,
        None,
    );
    let mut repo_themes = HashMap::new();
    picker.sync_theme_colors(&mut repo_themes);

    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert_eq!(
        cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
        Color::Rgb {
            r: 79,
            g: 166,
            b: 106,
        }
    );
}

fn action_summary(actions: Vec<ActionLabel>) -> Vec<(String, RepoActionKind, Color, bool)> {
    actions
        .into_iter()
        .map(|action| (action.text, action.kind, action.color, action.clickable))
        .collect()
}

fn picker_entry(name: &str) -> DirEntry {
    dir_response("/tmp", &[(name, true)])
        .entries
        .into_iter()
        .next()
        .expect("entry")
}

#[test]
fn picker_entry_actions_lists_available_repo_actions_in_row_order() {
    let mut entry = picker_entry("swimmers");
    entry.repo_dirty = Some(true);
    entry.has_restart = Some(true);
    entry.open_url = Some("http://127.0.0.1:3210".to_string());

    assert_eq!(
        action_summary(picker_entry_actions(&entry)),
        vec![
            (
                "[commit]".to_string(),
                RepoActionKind::Commit,
                Color::Green,
                true
            ),
            (
                "[restart]".to_string(),
                RepoActionKind::Restart,
                Color::Yellow,
                true
            ),
            (
                "[open]".to_string(),
                RepoActionKind::Open,
                Color::Cyan,
                true
            ),
        ]
    );
}

#[test]
fn picker_entry_actions_running_status_suppresses_other_actions() {
    let mut entry = picker_entry("swimmers");
    entry.repo_dirty = Some(true);
    entry.has_restart = Some(true);
    entry.open_url = Some("http://127.0.0.1:3210".to_string());
    entry.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Restart,
        state: RepoActionState::Running,
        detail: None,
    });

    assert_eq!(
        action_summary(picker_entry_actions(&entry)),
        vec![(
            "[running]".to_string(),
            RepoActionKind::Restart,
            Color::Yellow,
            false
        )]
    );
}

#[test]
fn picker_entry_actions_reports_commit_status_without_stale_clicks() {
    let mut failed = picker_entry("dirty");
    failed.repo_dirty = Some(true);
    failed.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Commit,
        state: RepoActionState::Failed,
        detail: None,
    });

    let mut done = picker_entry("clean");
    done.repo_dirty = Some(false);
    done.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Commit,
        state: RepoActionState::Succeeded,
        detail: None,
    });

    let mut dirty_after_success = picker_entry("dirty-again");
    dirty_after_success.repo_dirty = Some(true);
    dirty_after_success.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Commit,
        state: RepoActionState::Succeeded,
        detail: None,
    });

    assert_eq!(
        action_summary(picker_entry_actions(&failed)),
        vec![(
            "[failed]".to_string(),
            RepoActionKind::Commit,
            Color::Red,
            false
        )]
    );
    assert_eq!(
        action_summary(picker_entry_actions(&done)),
        vec![(
            "[done]".to_string(),
            RepoActionKind::Commit,
            Color::Green,
            false
        )]
    );
    assert_eq!(
        action_summary(picker_entry_actions(&dirty_after_success)),
        vec![(
            "[commit]".to_string(),
            RepoActionKind::Commit,
            Color::Green,
            true
        )]
    );
}

#[test]
fn picker_entry_actions_reports_restart_status_without_stale_clicks() {
    let mut failed = picker_entry("failed-service");
    failed.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Restart,
        state: RepoActionState::Failed,
        detail: None,
    });

    let mut done = picker_entry("done-service");
    done.repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Restart,
        state: RepoActionState::Succeeded,
        detail: None,
    });

    assert_eq!(
        action_summary(picker_entry_actions(&failed)),
        vec![(
            "[failed]".to_string(),
            RepoActionKind::Restart,
            Color::Red,
            false
        )]
    );
    assert_eq!(
        action_summary(picker_entry_actions(&done)),
        vec![(
            "[done]".to_string(),
            RepoActionKind::Restart,
            Color::Green,
            false
        )]
    );
}

#[test]
fn render_picker_draws_repo_action_badges_on_entry_row() {
    let mut response = dir_response("/tmp", &[("swimmers", true)]);
    response.entries[0].repo_dirty = Some(true);
    response.entries[0].has_restart = Some(true);
    response.entries[0].open_url = Some("http://127.0.0.1:3210".to_string());
    let picker = PickerState::new(2, 2, response, true, SpawnTool::Codex, None);
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    let row = row_text(&renderer, layout.first_entry_y);
    assert!(
        row.contains("swimmers"),
        "row should include entry label: {row}"
    );
    assert!(
        row.contains("[commit]"),
        "row should include commit badge: {row}"
    );
    assert!(
        row.contains("[restart]"),
        "row should include restart badge: {row}"
    );
    assert!(
        row.contains("[open]"),
        "row should include open badge: {row}"
    );

    let (commit_x, commit_y) =
        find_text_position(&renderer, "[commit]").expect("commit badge position");
    let (restart_x, restart_y) =
        find_text_position(&renderer, "[restart]").expect("restart badge position");
    let (open_x, open_y) = find_text_position(&renderer, "[open]").expect("open badge position");
    assert_eq!(commit_y, layout.first_entry_y);
    assert_eq!(restart_y, layout.first_entry_y);
    assert_eq!(open_y, layout.first_entry_y);
    assert_eq!(cell_at(&renderer, commit_x, commit_y).fg, Color::Green);
    assert_eq!(cell_at(&renderer, restart_x, restart_y).fg, Color::Yellow);
    assert_eq!(cell_at(&renderer, open_x, open_y).fg, Color::Cyan);
}

#[test]
fn picker_repo_action_badges_hit_test_in_row_order() {
    let mut response = dir_response("/tmp", &[("swimmers", true)]);
    response.entries[0].repo_dirty = Some(true);
    response.entries[0].has_restart = Some(true);
    response.entries[0].open_url = Some("http://127.0.0.1:3210".to_string());
    let picker = PickerState::new(2, 2, response, true, SpawnTool::Codex, None);
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    let (commit_x, row_y) = find_text_position(&renderer, "[commit]").expect("commit badge");
    let (restart_x, _) = find_text_position(&renderer, "[restart]").expect("restart badge");
    let (open_x, _) = find_text_position(&renderer, "[open]").expect("open badge");

    assert_eq!(
        picker_action_at(&picker, &layout, commit_x, row_y),
        Some(PickerAction::StartRepoAction(0, RepoActionKind::Commit))
    );
    assert_eq!(
        picker_action_at(&picker, &layout, restart_x, row_y),
        Some(PickerAction::StartRepoAction(0, RepoActionKind::Restart))
    );
    assert_eq!(
        picker_action_at(&picker, &layout, open_x, row_y),
        Some(PickerAction::StartRepoAction(0, RepoActionKind::Open))
    );
    assert_eq!(
        picker_action_at(&picker, &layout, commit_x + "[commit]".len() as u16, row_y),
        Some(PickerAction::ActivateEntry(0))
    );
}

#[test]
fn picker_disabled_repo_action_status_hit_tests_as_entry_activation() {
    let mut response = dir_response("/tmp", &[("swimmers", true)]);
    response.entries[0].repo_dirty = Some(false);
    response.entries[0].repo_action = Some(RepoActionStatus {
        kind: RepoActionKind::Commit,
        state: RepoActionState::Succeeded,
        detail: None,
    });
    let picker = PickerState::new(2, 2, response, true, SpawnTool::Codex, None);
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    let (done_x, done_y) = find_text_position(&renderer, "[done]").expect("done badge");

    assert_eq!(
        picker_action_at(&picker, &layout, done_x, done_y),
        Some(PickerAction::ActivateEntry(0))
    );
}

#[test]
fn render_picker_draws_empty_state_for_empty_directory() {
    let picker = PickerState::new(
        2,
        2,
        dir_response("/tmp/empty", &[]),
        true,
        SpawnTool::Codex,
        None,
    );
    let field = test_field();
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert!(
        row_text(&renderer, layout.first_entry_y).contains("empty"),
        "empty directories should render an empty row"
    );
}

#[test]
fn render_initial_request_draws_placeholder_and_ready_voice_state() {
    let request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
    let field = test_field();
    let layout = initial_request_layout(field);
    let mut renderer = test_renderer(100, 30);

    render_initial_request(&mut renderer, &request, &VoiceUiState::Ready, field, None);

    let cursor_x = layout.content.x + 2;
    let (status_x, status_y) = find_text_position(&renderer, "voice: ready").expect("voice status");
    let input_row = row_text(&renderer, layout.input_y);
    assert!(
        input_row.contains("|ype initial request"),
        "empty composer should show cursor over placeholder text: {input_row}"
    );
    assert_eq!(
        cell_at(&renderer, cursor_x, layout.input_y).fg,
        Color::Yellow
    );
    assert_eq!(
        cell_at(&renderer, cursor_x + 1, layout.input_y).fg,
        Color::DarkGrey
    );
    assert_eq!(status_y, layout.content.y + 4);
    assert_eq!(cell_at(&renderer, status_x, status_y).fg, Color::Cyan);
}

#[test]
fn render_initial_request_draws_typed_value_and_failed_voice_state() {
    let mut request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
    request.value = "Ask Codex to harden the picker".to_string();
    let field = test_field();
    let layout = initial_request_layout(field);
    let mut renderer = test_renderer(100, 30);

    render_initial_request(
        &mut renderer,
        &request,
        &VoiceUiState::Failed("microphone denied".to_string()),
        field,
        None,
    );

    let (value_x, value_y) =
        find_text_position(&renderer, "Ask Codex to harden the picker").expect("typed value");
    let (status_x, status_y) =
        find_text_position(&renderer, "voice: microphone denied").expect("voice failure");
    assert_eq!(value_y, layout.input_y);
    assert_eq!(cell_at(&renderer, value_x, value_y).fg, Color::White);
    assert_eq!(status_y, layout.content.y + 4);
    assert_eq!(cell_at(&renderer, status_x, status_y).fg, Color::Red);
}

#[test]
fn sleeping_entities_fill_bottom_row_by_sleepiness() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-new", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:20:00Z"),
            sleeping_session("sess-mid", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:10:00Z"),
            sleeping_session("sess-old", "9", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-old", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-mid", field),
        sleep_grid_rect(field, 1)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-new", field),
        sleep_grid_rect(field, 2)
    );
}

#[test]
fn sleeping_entities_use_tmux_name_tiebreaker() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-b", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-a", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-a", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-b", field),
        sleep_grid_rect(field, 1)
    );
}

#[test]
fn existing_entity_relocates_into_sleep_grid_when_it_falls_asleep() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    app.entities
        .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

    app.merge_sessions(
        vec![sleeping_session(
            "sess-1",
            "dev",
            TEST_REPO_DEV,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-1", field),
        sleep_grid_rect(field, 0)
    );
}

#[test]
fn sleeping_entities_stay_fixed_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            sleeping_session("sess-a", "7", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
            sleeping_session("sess-b", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:10:00Z"),
        ],
        field,
    );
    for entity in &mut app.entities {
        entity.vx = 1.0;
        entity.vy = 1.0;
    }

    app.tick(field);

    assert_eq!(
        entity_rect_for(&app, "sess-a", field),
        sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-b", field),
        sleep_grid_rect(field, 1)
    );
}

#[test]
fn drowsy_sprite_uses_fish_motion_profile() {
    assert_eq!(SpriteKind::Drowsy.speed_scale(), 0.5);
    assert!(drowsy_frame(0)[1].contains("><-"));
}

#[test]
fn jelly_sprites_match_entity_width_for_all_states() {
    let kinds = [
        SpriteKind::Active,
        SpriteKind::Busy,
        SpriteKind::Drowsy,
        SpriteKind::Sleeping,
        SpriteKind::DeepSleep,
        SpriteKind::Attention,
        SpriteKind::Error,
        SpriteKind::Exited,
    ];
    for kind in kinds {
        for tick in [0u64, 4u64] {
            let frame = kind.frame_with_theme(tick, SpriteTheme::Jelly);
            for (row, line) in frame.iter().enumerate() {
                let count = line.chars().count();
                assert_eq!(
                    count, ENTITY_WIDTH as usize,
                    "jelly sprite row {row} for {kind:?} tick {tick} must be exactly {} chars, got {count}: {line:?}",
                    ENTITY_WIDTH
                );
            }
        }
    }
}

#[test]
fn deep_sleep_jelly_sprite_is_saggier_than_active() {
    let active_bottom = ball_active_frame(0)[3]
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    let deep_sleep_bottom = ball_deep_sleep_frame(0)[3]
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    assert!(
        deep_sleep_bottom > active_bottom,
        "deep sleep bottom row should be saggier (more glyphs) than active; deep_sleep={deep_sleep_bottom}, active={active_bottom}",
    );
}

#[test]
fn balls_theme_sags_sleepier_sessions_and_clamps_to_floor() {
    let api = MockApi::new();
    let layout = test_layout(120, 40);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![
            attention_session(
                "sess-attn",
                "1",
                TEST_REPO_SWIMMERS,
                RestState::Active,
                "2026-03-08T12:30:00Z",
            ),
            sleeping_session(
                "sess-sleep",
                "2",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:10:00Z",
            ),
            deep_sleep_session("sess-deep", "3", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        layout.overview_field,
    );
    let visible = app.visible_entities();
    let slots = balls_theme_slots(&visible, layout.overview_field);
    let attention = slots
        .iter()
        .find(|slot| slot.entity.session.session_id == "sess-attn")
        .expect("attention slot");
    let deep_sleep = slots
        .iter()
        .find(|slot| slot.entity.session.session_id == "sess-deep")
        .expect("deep sleep slot");

    assert!(
        deep_sleep.ball_y > attention.ball_y,
        "older sleepier sessions should sag lower than attention seekers"
    );
    assert!(deep_sleep.on_floor, "deep sleep should clamp to the floor");
}

#[test]
fn balls_theme_slots_keep_natural_tmux_order_across_state_changes() {
    let api = MockApi::new();
    let layout = test_layout(140, 40);
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            session_summary("sess-alpha-10", "alpha-10", TEST_REPO_SWIMMERS),
            attention_session(
                "sess-10",
                "10",
                TEST_REPO_SWIMMERS,
                RestState::Active,
                "2026-03-08T12:30:00Z",
            ),
            session_summary("sess-alpha-2", "alpha-2", TEST_REPO_SWIMMERS),
            deep_sleep_session("sess-2", "2", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z"),
        ],
        layout.overview_field,
    );

    let visible = app.visible_entities();
    let slots = balls_theme_slots(&visible, layout.overview_field);
    let slot_order = |slots: &[BallsThemeSlot<'_>]| {
        let mut ordered = slots.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|slot| slot.ball_x);
        ordered
            .into_iter()
            .map(|slot| slot.entity.session.tmux_name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        slot_order(&slots),
        vec!["2", "10", "alpha-2", "alpha-10"],
        "ball slots should read left-to-right in natural tmux-name order"
    );
    let original_x_by_name = slots
        .iter()
        .map(|slot| (slot.entity.session.tmux_name.clone(), slot.ball_x))
        .collect::<HashMap<_, _>>();

    app.merge_sessions(
        vec![
            attention_session(
                "sess-alpha-10",
                "alpha-10",
                TEST_REPO_SWIMMERS,
                RestState::Active,
                "2026-03-08T12:00:00Z",
            ),
            deep_sleep_session("sess-10", "10", TEST_REPO_SWIMMERS, "2026-03-08T12:10:00Z"),
            sleeping_session(
                "sess-alpha-2",
                "alpha-2",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:20:00Z",
            ),
            attention_session(
                "sess-2",
                "2",
                TEST_REPO_SWIMMERS,
                RestState::Active,
                "2026-03-08T12:30:00Z",
            ),
        ],
        layout.overview_field,
    );

    let visible = app.visible_entities();
    let slots_after_state_change = balls_theme_slots(&visible, layout.overview_field);
    assert_eq!(
        slot_order(&slots_after_state_change),
        vec!["2", "10", "alpha-2", "alpha-10"],
        "state and last-activity changes must not swap ball slots"
    );
    for slot in slots_after_state_change {
        assert_eq!(
            original_x_by_name.get(&slot.entity.session.tmux_name),
            Some(&slot.ball_x),
            "{} should keep its horizontal ball slot",
            slot.entity.session.tmux_name
        );
    }
}

#[test]
fn balls_theme_handles_very_narrow_fields() {
    let field = Rect {
        x: 7,
        y: 3,
        width: 4,
        height: 12,
    };
    let entity = SessionEntity::new(
        session_summary("sess-narrow", "1", TEST_REPO_SWIMMERS),
        field,
    );
    let visible = vec![&entity];

    let slots = balls_theme_slots(&visible, field);

    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].ball_x, field.x);
}

#[test]
fn balls_theme_renders_floor_and_attention_marker() {
    let api = MockApi::new();
    let layout = test_layout(120, 40);
    let mut app = make_app(api);
    let theme_id = "/tmp/swimmers".to_string();
    app.repo_themes.insert(
        theme_id.clone(),
        repo_theme_with_sprite("#B89875", Some("balls")),
    );
    let mut attention = attention_session(
        "sess-attn",
        "7",
        TEST_REPO_SWIMMERS,
        RestState::Active,
        "2026-03-08T12:30:00Z",
    );
    attention.repo_theme_id = Some(theme_id.clone());
    let mut deep_sleep =
        deep_sleep_session("sess-deep", "8", TEST_REPO_SWIMMERS, "2026-03-08T12:00:00Z");
    deep_sleep.repo_theme_id = Some(theme_id);
    app.merge_sessions(vec![attention, deep_sleep], layout.overview_field);
    let mut renderer = test_renderer(120, 40);

    app.render(&mut renderer, layout);

    assert!(
        find_text_position(&renderer, "!").is_some(),
        "attention swimmers should render as tight attention-seeking balls"
    );
    assert_eq!(
        cell_at(
            &renderer,
            layout.overview_field.x,
            layout.overview_field.bottom().saturating_sub(1)
        )
        .ch,
        '_'
    );
}

#[test]
fn toggle_sprite_theme_cycles_between_auto_and_overrides() {
    let api = MockApi::new();
    let mut app = make_app(api);
    assert_eq!(app.sprite_theme_override, None);
    app.toggle_sprite_theme();
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Fish));
    app.toggle_sprite_theme();
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Balls));
    app.toggle_sprite_theme();
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Jelly));
    app.toggle_sprite_theme();
    assert_eq!(app.sprite_theme_override, None);
}

#[test]
fn sprite_theme_click_selects_matching_segment() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let rect = app.sprite_theme_rect(120);
    let auto_width = display_width("[auto]");
    let fish_width = display_width("[fish]");
    let balls_width = display_width("[balls]");

    app.set_sprite_theme_from_click(rect.x + auto_width + 1);
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Fish));

    app.set_sprite_theme_from_click(rect.x + auto_width + 1 + fish_width + 1);
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Balls));

    app.set_sprite_theme_from_click(rect.x + auto_width + 1 + fish_width + 1 + balls_width + 1);
    assert_eq!(app.sprite_theme_override, Some(SpriteTheme::Jelly));

    app.set_sprite_theme_from_click(rect.x);
    assert_eq!(app.sprite_theme_override, None);
}

#[test]
fn effective_sprite_theme_prefers_override_then_repo_then_default() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let theme_id = "/tmp/swimmers".to_string();
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.repo_theme_id = Some(theme_id.clone());
    app.repo_themes
        .insert(theme_id, repo_theme_with_sprite("#B89875", Some("jelly")));

    assert_eq!(
        app.effective_sprite_theme_for_session(&session),
        SpriteTheme::Jelly
    );

    app.sprite_theme_override = Some(SpriteTheme::Fish);
    assert_eq!(
        app.effective_sprite_theme_for_session(&session),
        SpriteTheme::Fish
    );

    app.sprite_theme_override = None;
    session.repo_theme_id = None;
    assert_eq!(
        app.effective_sprite_theme_for_session(&session),
        SpriteTheme::Balls
    );
}

#[test]
fn drowsy_entities_bob_in_place_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.thought_state = ThoughtState::Holding;
    entity.session.rest_state = RestState::Drowsy;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let rect = entity_rect_for(&app, "sess-1", field);
    assert_eq!(rect.x, 30);
    assert_ne!(rect.y, 8);
    assert!((rect.y as i32 - 8).abs() <= 3);
}

#[test]
fn deep_sleep_entities_stay_fixed_after_tick() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![
            deep_sleep_session(
                "sess-deep-a",
                "7",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:00:00Z",
            ),
            deep_sleep_session(
                "sess-deep-b",
                "8",
                TEST_REPO_SWIMMERS,
                "2026-03-08T12:10:00Z",
            ),
        ],
        field,
    );
    for entity in &mut app.entities {
        entity.vx = 1.0;
        entity.vy = 1.0;
    }

    app.tick(field);

    assert_eq!(
        entity_rect_for(&app, "sess-deep-a", field),
        deep_sleep_grid_rect(field, 0)
    );
    assert_eq!(
        entity_rect_for(&app, "sess-deep-b", field),
        deep_sleep_grid_rect(field, 1)
    );
}

#[test]
fn active_entities_swim_in_place_with_bob() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.thought_state = ThoughtState::Active;
    entity.session.rest_state = RestState::Active;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let moved = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-1")
        .expect("entity should exist");
    assert_eq!(moved.screen_rect(field).x, 30);
    assert_ne!(moved.screen_rect(field).y, 8);
    assert!((moved.screen_rect(field).y as i32 - 8).abs() <= 3);
}

#[test]
fn busy_entities_hold_horizontal_position() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
    entity.session.state = SessionState::Busy;
    entity.bob_phase = 0.0;
    entity.vx = 1.0;
    entity.vy = 0.0;
    app.entities.push(entity);

    for _ in 0..16 {
        app.tick(field);
    }

    let rect = entity_rect_for(&app, "sess-1", field);
    assert_eq!(rect.x, 30);
    assert_ne!(rect.y, 8);
    assert!((rect.y as i32 - 8).abs() <= 3);
}

#[test]
fn busy_sleeping_entities_keep_busy_motion() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    let mut session = sleeping_session(
        "sess-busy-sleeping",
        "busy-waiting",
        TEST_REPO_DEV,
        "2026-03-08T12:00:00Z",
    );
    session.state = SessionState::Busy;

    app.merge_sessions(vec![session], field);
    for entity in &mut app.entities {
        entity.vx = 1.0;
        entity.vy = 1.0;
    }

    app.tick(field);

    let entity = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-busy-sleeping")
        .expect("entity should exist");
    assert_eq!(entity.sprite_kind(), SpriteKind::Busy);
    assert_eq!(entity.rest_anchor(), RestAnchor::FreeSwim);
}

#[test]
fn truncate_label_adds_trailing_tilde() {
    assert_eq!(truncate_label("abcdefghijkl", 6), "abcde~");
    assert_eq!(truncate_label("abc", 6), "abc");
}

#[test]
fn shorten_path_keeps_tail() {
    assert_eq!(shorten_path("/a/b/c/d/e", 8), ".../d/e");
    assert_eq!(shorten_path("/short", 20), "/short");
}

#[test]
fn intersects_detects_overlap() {
    let a = Rect {
        x: 0,
        y: 0,
        width: 5,
        height: 5,
    };
    let b = Rect {
        x: 4,
        y: 2,
        width: 5,
        height: 3,
    };
    let c = Rect {
        x: 5,
        y: 5,
        width: 2,
        height: 2,
    };
    assert!(intersects(a, b));
    assert!(!intersects(a, c));
}

#[test]
fn empty_field_click_opens_picker_with_managed_order() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(
        TEST_REPOS_ROOT,
        &[("opensource", true), ("swimmers", true)],
    )));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    assert!(app.picker.is_none());

    poll_until_interaction(&mut app);

    let picker = app.picker.as_ref().expect("picker should open");
    assert!(picker.managed_only);
    assert_eq!(picker.base_path, TEST_REPOS_ROOT);
    assert_eq!(
        picker
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["opensource", "swimmers"]
    );
    assert_eq!(api.list_calls(), vec![(None, true)]);
}

#[test]
fn opening_picker_loads_repo_search_entries() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_repo_dirs(Ok(repo_search_response(&["/Users/tester/hard/pcbcd"])));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    poll_until_interaction(&mut app);
    poll_until_picker_repo_search(&mut app);

    let picker = app.picker.as_mut().expect("picker should open");
    picker.search = "pcbcd".to_string();
    let visible = picker.visible_entries();
    assert_eq!(visible.len(), 1);
    assert_eq!(
        picker.path_for_entry(visible[0]).as_deref(),
        Some("/Users/tester/hard/pcbcd")
    );
    assert_eq!(api.list_repo_dirs_calls(), 1);
}

#[test]
fn opening_picker_does_not_wait_for_slow_repo_search() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_repo_dirs(Ok(repo_search_response(&["/Users/tester/hard/pcbcd"])));
    api.set_list_repo_dirs_delay(Duration::from_millis(150));
    let field = test_field();
    let mut app = make_app(api.clone());

    let started = Instant::now();
    app.handle_field_click(10, 10, field);
    poll_until_interaction(&mut app);
    let visible_elapsed = started.elapsed();

    assert!(
        visible_elapsed < Duration::from_millis(100),
        "picker should become visible after list_dirs, not wait for slow repo search; elapsed {visible_elapsed:?}"
    );
    let picker = app.picker.as_ref().expect("picker should open");
    assert!(
        picker.repo_search_entries.is_empty(),
        "repo search entries should arrive after the initial picker frame"
    );
    assert!(
        app.pending_picker_repo_search.is_some(),
        "repo search should continue in the background"
    );

    poll_until_picker_repo_search(&mut app);

    let picker = app.picker.as_ref().expect("picker should stay open");
    assert_eq!(
        picker
            .repo_search_entries
            .first()
            .and_then(|entry| entry.full_path.as_deref()),
        Some("/Users/tester/hard/pcbcd")
    );
    assert_eq!(api.list_repo_dirs_calls(), 1);
}

#[test]
fn navigating_into_folder_opens_initial_request_composer() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_dirs(Ok(dir_response(TEST_REPO_OPENSOURCE, &[("skills", false)])));

    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    app.activate_picker_entry(0, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    app.activate_picker_entry(0, field);

    assert_eq!(
        api.list_calls(),
        vec![(None, true), (Some(TEST_REPO_OPENSOURCE.to_string()), true),]
    );
    assert_eq!(
        api.create_calls(),
        Vec::<(String, SpawnTool, Option<String>)>::new()
    );
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some(TEST_REPO_SKILLS)
    );
    assert!(app.picker.is_some());
}

#[test]
fn spawn_here_opens_initial_request_for_current_path() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPO_OPENSOURCE, &[("skills", true)]),
        true,
        SpawnTool::Codex,
        None,
    ));

    app.spawn_session_from_picker(field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some(TEST_REPO_OPENSOURCE)
    );
}

#[test]
fn remote_picker_activation_blocks_unmapped_entry_before_composer() {
    let api = MockApi::new();
    let field = test_field();
    let mut response = dir_response(TEST_REPOS_ROOT, &[("alpha", false), ("beta", false)]);
    response.launch_targets = vec![LaunchTargetSummary::local(), remote_target_mapping_alpha()];
    response.default_launch_target = Some("devbox".to_string());
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        response,
        true,
        SpawnTool::Codex,
        None,
    ));

    app.activate_picker_entry(1, field);

    assert!(app.initial_request.is_none());
    assert!(app
        .visible_message()
        .is_some_and(|message| message.contains("unmapped cwd")));
    assert!(api.create_calls().is_empty());

    app.activate_picker_entry(0, field);

    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|request| request.cwd.as_str()),
        Some(TEST_REPO_ALPHA)
    );
}

#[test]
fn toggling_to_all_reloads_same_path_without_reordering() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
    api.push_list_dirs(Ok(dir_response(
        TEST_REPOS_ROOT,
        &[("Alpha", true), ("beta", true), ("zzz-old", true)],
    )));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    app.picker_set_managed_only(false);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    let picker = app.picker.as_ref().expect("picker should stay open");
    assert!(!picker.managed_only);
    assert_eq!(
        picker
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "beta", "zzz-old"]
    );
    assert_eq!(
        api.list_calls(),
        vec![(None, true), (Some(TEST_REPOS_ROOT.to_string()), false),]
    );
}

#[test]
fn picker_search_filters_without_changing_browsing_scope() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let mut picker = PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.current_group = Some("work".to_string());
    app.picker = Some(picker);

    app.picker_search_push('b');

    let picker = app.picker.as_ref().expect("picker");
    assert_eq!(picker.search, "b");
    assert!(picker.managed_only);
    assert_eq!(picker.current_group.as_deref(), Some("work"));
    assert_eq!(picker.visible_entries(), vec![1]);
    assert!(api.list_calls().is_empty());
}

#[test]
fn picker_visible_entries_with_empty_search_returns_local_entries_only() {
    let mut picker = PickerState::new(
        0,
        0,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.set_repo_search_entries(repo_search_response(&["/Users/tester/hard/pcbcd"]).entries);

    assert_eq!(picker.visible_entries(), vec![0, 1]);
}

#[test]
fn picker_search_includes_repo_cwd_when_not_in_current_entries() {
    let mut picker = PickerState::new(
        0,
        0,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.set_repo_search_entries(
        repo_search_response(&[
            "/Users/tester/repos/opensource/swimmers",
            "/Users/tester/hard/pcbcd",
        ])
        .entries,
    );
    picker.search = "swim".to_string();

    let visible = picker.visible_entries();
    assert_eq!(visible.len(), 1);
    assert_eq!(
        picker.path_for_entry(visible[0]).as_deref(),
        Some("/Users/tester/repos/opensource/swimmers")
    );
}

#[test]
fn picker_search_deduplicates_repo_cwd_already_visible() {
    let mut response = dir_response(TEST_REPOS_ROOT, &[("swimmers", true)]);
    response.entries[0].full_path = Some("/Users/tester/repos/opensource/swimmers".to_string());
    let mut picker = PickerState::new(0, 0, response, true, SpawnTool::Codex, None);
    picker.set_repo_search_entries(
        repo_search_response(&["/Users/tester/repos/opensource/swimmers"]).entries,
    );
    picker.search = "swim".to_string();

    assert_eq!(picker.visible_entries(), vec![0]);
}

#[test]
fn picker_parent_path_returns_none_at_root() {
    let picker = PickerState::new(
        0,
        0,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );

    assert_eq!(picker.parent_path(), None);
}

#[test]
fn picker_parent_path_normalizes_current_path_before_parent() {
    let mut picker = PickerState::new(
        0,
        0,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.current_path = "/Users/tester/repos/opensource/swimmers/".to_string();

    assert_eq!(
        picker.parent_path().as_deref(),
        Some("/Users/tester/repos/opensource")
    );
}

#[test]
fn picker_parent_path_maps_empty_parent_to_root() {
    let mut picker = PickerState::new(
        0,
        0,
        dir_response("base", &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.current_path = "child".to_string();

    assert_eq!(picker.parent_path().as_deref(), Some("/"));
}

#[test]
fn activating_repo_search_result_opens_initial_request_for_cwd() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut picker = PickerState::new(
        0,
        0,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.set_repo_search_entries(
        repo_search_response(&["/Users/tester/repos/opensource/swimmers"]).entries,
    );
    picker.search = "swim".to_string();
    picker.selection = PickerSelection::Entry(picker.visible_entries()[0]);
    app.picker = Some(picker);

    app.picker_activate_selection(layout.overview_field);

    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some("/Users/tester/repos/opensource/swimmers")
    );
}

#[test]
fn picker_group_target_cycles_across_available_groups() {
    let mut picker = PickerState::new(
        10,
        10,
        dir_response_with_groups(
            TEST_REPOS_ROOT,
            &[("alpha", true)],
            &["frontend", "backend"],
        ),
        true,
        SpawnTool::Codex,
        None,
    );

    assert_eq!(picker.group_edit_target.as_deref(), Some("frontend"));
    assert_eq!(picker.cycle_group_edit_target().as_deref(), Some("backend"));
    assert_eq!(
        picker.cycle_group_edit_target().as_deref(),
        Some("frontend")
    );
}

#[test]
fn picker_group_update_plan_deltas_for_add_remove_and_move() {
    let mut picker = picker_with_grouped_swimmers(&["frontend", "backend"], None);

    assert_group_delta(&picker, PickerGroupUpdateMode::Add, &["frontend"], &[]);
    assert_group_delta(&picker, PickerGroupUpdateMode::Remove, &[], &["frontend"]);
    assert_group_delta(
        &picker,
        PickerGroupUpdateMode::Move,
        &["frontend"],
        &["backend"],
    );

    picker.group_edit_target = Some("backend".to_string());
    assert_group_delta(
        &picker,
        PickerGroupUpdateMode::Move,
        &["backend"],
        &["frontend"],
    );
}

#[test]
fn picker_group_move_plan_includes_current_group_once_when_missing_from_memberships() {
    let mut picker = picker_with_grouped_swimmers(&["frontend", "backend"], Some("work"));

    assert_group_delta(
        &picker,
        PickerGroupUpdateMode::Move,
        &["frontend"],
        &["backend", "work"],
    );

    picker.entries[0].groups.push("work".to_string());
    assert_group_delta(
        &picker,
        PickerGroupUpdateMode::Move,
        &["frontend"],
        &["backend", "work"],
    );
}

#[test]
fn picker_add_selected_entry_to_group_target_updates_api_and_reloads() {
    let api = MockApi::new();
    api.push_update_dir_group_memberships(Ok(DirGroupMembershipUpdateResponse {
        path: TEST_REPO_SWIMMERS.to_string(),
        groups: vec!["frontend".to_string()],
        available_groups: vec!["frontend".to_string(), "backend".to_string()],
    }));
    api.push_list_dirs(Ok(dir_response_with_groups(
        TEST_REPOS_ROOT,
        &[("swimmers", false)],
        &["frontend", "backend"],
    )));
    let mut app = make_app(api.clone());
    let mut picker = PickerState::new(
        10,
        10,
        dir_response_with_groups(
            TEST_REPOS_ROOT,
            &[("swimmers", false)],
            &["frontend", "backend"],
        ),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.selection = PickerSelection::Entry(0);
    app.picker = Some(picker);

    app.picker_add_selected_to_group_target();
    poll_until_interaction(&mut app);

    assert_eq!(
        api.update_dir_group_memberships_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            vec!["frontend".to_string()],
            Vec::<String>::new()
        )]
    );
    assert_eq!(
        api.list_calls(),
        vec![(Some(TEST_REPOS_ROOT.to_string()), true)]
    );
}

fn picker_with_grouped_swimmers(groups: &[&str], current_group: Option<&str>) -> PickerState {
    let mut picker = PickerState::new(
        10,
        10,
        dir_response_with_groups(
            TEST_REPOS_ROOT,
            &[("swimmers", false)],
            &["frontend", "backend", "work"],
        ),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.entries[0].groups = groups.iter().map(|group| (*group).to_string()).collect();
    picker.current_group = current_group.map(str::to_string);
    picker.selection = PickerSelection::Entry(0);
    picker
}

fn assert_group_delta(
    picker: &PickerState,
    mode: PickerGroupUpdateMode,
    add: &[&str],
    remove: &[&str],
) {
    let plan = picker_group_update_plan(picker, mode).expect("group update plan");
    assert_eq!(plan.delta.add, strings(add));
    assert_eq!(plan.delta.remove, strings(remove));
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn picker_batch_visible_opens_composer_for_filtered_entries() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    ));
    app.picker_search_push('b');

    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('B'))));

    let request = app.initial_request.as_ref().expect("composer");
    assert_eq!(request.value, "");
    assert_eq!(
        request.batch_dirs.as_deref(),
        Some(&[TEST_REPO_BETA.to_string()][..])
    );
    assert!(api.create_calls().is_empty());
    assert!(api.create_batch_calls().is_empty());
}

#[test]
fn picker_batch_exclusion_removes_dirs_from_batch_composer() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(
            TEST_REPOS_ROOT,
            &[("alpha", true), ("beta", true), ("gamma", true)],
        ),
        true,
        SpawnTool::Codex,
        None,
    ));

    app.handle_picker_action(PickerAction::ToggleBatchExcludeMode, field);
    app.handle_picker_action(PickerAction::ToggleBatchExclude(1), field);
    app.open_batch_initial_request_for_visible_entries();

    let request = app.initial_request.as_ref().expect("composer");
    assert_eq!(
        request.batch_dirs.as_deref(),
        Some(&[TEST_REPO_ALPHA.to_string(), TEST_REPO_GAMMA.to_string()][..])
    );
    assert!(api.create_calls().is_empty());
    assert!(api.create_batch_calls().is_empty());
}

#[test]
fn remote_batch_blocks_unmapped_dirs_before_composer_and_allows_local_override() {
    let api = MockApi::new();
    let mut response = dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]);
    response.launch_targets = vec![LaunchTargetSummary::local(), remote_target_mapping_alpha()];
    response.default_launch_target = Some("devbox".to_string());
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        response,
        true,
        SpawnTool::Codex,
        None,
    ));

    app.open_batch_initial_request_for_visible_entries();

    assert!(app.initial_request.is_none());
    assert!(app
        .visible_message()
        .is_some_and(|message| message.contains("remote batch")));
    assert!(api.create_batch_calls().is_empty());

    app.picker.as_mut().expect("picker").launch_target = Some("local".to_string());
    app.open_batch_initial_request_for_visible_entries();

    let request = app.initial_request.as_ref().expect("composer");
    assert_eq!(
        request.batch_dirs.as_deref(),
        Some(&[TEST_REPO_ALPHA.to_string(), TEST_REPO_BETA.to_string()][..])
    );
}

#[test]
fn picker_batch_exclude_badge_toggles_without_activating_entry() {
    let field = test_field();
    let mut picker = PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.batch_exclude_mode = true;
    let layout = picker_layout(&picker, field);
    let out_x = layout.content.right().saturating_sub("[out]".len() as u16);

    assert!(matches!(
        picker_action_at(&picker, &layout, out_x, layout.first_entry_y),
        Some(PickerAction::ToggleBatchExclude(0))
    ));
}

#[test]
fn picker_keyboard_exclusion_toggles_selected_entry_out_of_batch() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    ));

    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('X'))));
    {
        let picker = app.picker.as_mut().expect("picker");
        assert!(picker.batch_exclude_mode);
        picker.selection = PickerSelection::Entry(1);
    }
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char(' '))));

    let picker = app.picker.as_ref().expect("picker");
    assert!(picker.batch_entry_is_excluded(1));
    assert_eq!(
        picker.batch_dirs_for_visible_entries(),
        vec![TEST_REPO_ALPHA.to_string()]
    );
}

#[test]
fn render_picker_shows_batch_count_and_out_in_chips() {
    let field = test_field();
    let mut picker = PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.batch_exclude_mode = true;
    picker.toggle_batch_exclusion(1);
    let layout = picker_layout(&picker, field);
    let mut renderer = test_renderer(100, 30);

    render_picker(&mut renderer, &picker, field);

    assert!(row_text(&renderer, layout.batch_button.y).contains("[batch 1]"));
    assert!(row_text(&renderer, layout.first_entry_y).contains("[out]"));
    assert!(row_text(&renderer, layout.first_entry_y + 1).contains("[in]"));
}

#[test]
fn dir_list_failure_blocks_spawn_and_shows_error() {
    let api = MockApi::new();
    api.push_list_dirs(Err("Permission denied".to_string()));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    assert!(app.picker.is_none());

    poll_until_interaction(&mut app);

    assert!(app.picker.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("Permission denied")
    );
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
}
