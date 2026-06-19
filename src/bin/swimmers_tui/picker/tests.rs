mod picker {
    use super::super::picker_render::{
        initial_request_context_line, initial_request_hint, initial_request_input_render_model,
        initial_request_title, initial_request_voice_color, picker_filter_render_items,
        InitialRequestInputRenderModel, PickerFilterRenderItem,
    };
    use super::super::*;
    use swimmers::types::{LaunchPathMapping, RepoActionStatus};

    fn rect(x: u16, y: u16, width: u16) -> Rect {
        Rect {
            x,
            y,
            width,
            height: 1,
        }
    }

    fn field_rect() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        }
    }

    fn initial_request_test_layout(width: u16) -> InitialRequestLayout {
        InitialRequestLayout {
            frame: Rect {
                x: 0,
                y: 0,
                width: width + 2,
                height: 7,
            },
            content: Rect {
                x: 1,
                y: 1,
                width,
                height: 5,
            },
            input_y: 4,
        }
    }

    fn filter_test_layout() -> PickerLayout {
        PickerLayout {
            frame: rect(0, 0, 80),
            content: rect(1, 1, 78),
            back_button: None,
            close_button: rect(76, 1, 3),
            env_button: rect(2, 3, 9),
            group_buttons: vec![
                ("alpha".to_string(), rect(12, 3, 7)),
                ("beta".to_string(), rect(20, 3, 6)),
            ],
            all_button: rect(28, 3, 13),
            group_target_button: Some(rect(42, 3, 14)),
            tool_button: rect(60, 1, 7),
            launch_target_button: rect(48, 1, 11),
            exclude_button: rect(36, 1, 9),
            batch_button: rect(26, 1, 9),
            spawn_here_button: rect(2, 4, 76),
            first_entry_y: 5,
            visible_entry_rows: 1,
            visible_entries: vec![0],
        }
    }

    fn filter_test_picker() -> PickerState {
        PickerState::new(
            0,
            0,
            DirListResponse {
                path: "/tmp".to_string(),
                entries: Vec::new(),
                overlay_label: None,
                groups: vec!["alpha".to_string(), "beta".to_string()],
                launch_targets: Vec::new(),
                default_launch_target: None,
            },
            true,
            SpawnTool::Codex,
            None,
        )
    }

    fn top_action_at(layout: &PickerLayout, rect: Rect) -> Option<PickerAction> {
        picker_top_control_action_at(layout, rect.x, rect.y)
    }

    fn pointer_test_entry() -> DirEntry {
        DirEntry {
            name: "swimmers".to_string(),
            has_children: true,
            is_running: None,
            repo_dirty: Some(true),
            repo_action: None,
            group: None,
            groups: Vec::new(),
            full_path: None,
            has_restart: Some(true),
            open_url: Some("http://127.0.0.1:3210".to_string()),
        }
    }

    fn pointer_test_picker_with_entry(entry: DirEntry, batch_exclude_mode: bool) -> PickerState {
        let mut picker = PickerState::new(
            0,
            0,
            DirListResponse {
                path: "/tmp".to_string(),
                entries: vec![entry],
                overlay_label: None,
                groups: Vec::new(),
                launch_targets: Vec::new(),
                default_launch_target: None,
            },
            true,
            SpawnTool::Codex,
            None,
        );
        picker.batch_exclude_mode = batch_exclude_mode;
        picker
    }

    fn pointer_test_picker(batch_exclude_mode: bool) -> PickerState {
        pointer_test_picker_with_entry(pointer_test_entry(), batch_exclude_mode)
    }

    fn apply_response_entry(name: &str, full_path: &str) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            has_children: true,
            is_running: None,
            repo_dirty: None,
            repo_action: None,
            group: None,
            groups: Vec::new(),
            full_path: Some(full_path.to_string()),
            has_restart: None,
            open_url: None,
        }
    }

    fn apply_response_target(id: &str) -> LaunchTargetSummary {
        LaunchTargetSummary {
            id: id.to_string(),
            label: format!("{id} target"),
            kind: "remote".to_string(),
            base_url: None,
            auth_token_env: None,
            path_mappings: Vec::new(),
        }
    }

    fn mapped_response_target(id: &str) -> LaunchTargetSummary {
        LaunchTargetSummary {
            id: id.to_string(),
            label: format!("{id} target"),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://127.0.0.1:3210".to_string()),
            auth_token_env: None,
            path_mappings: vec![
                LaunchPathMapping {
                    local_prefix: "/Users/tester/repos".to_string(),
                    remote_prefix: "/srv/repos".to_string(),
                },
                LaunchPathMapping {
                    local_prefix: "/Users/tester/repos/opensource".to_string(),
                    remote_prefix: "/srv/opensource".to_string(),
                },
            ],
        }
    }

    fn apply_response_picker(entries: Vec<DirEntry>) -> PickerState {
        PickerState::new(
            0,
            0,
            DirListResponse {
                path: "/tmp".to_string(),
                entries,
                overlay_label: Some("old overlay".to_string()),
                groups: vec!["alpha".to_string(), "beta".to_string()],
                launch_targets: vec![apply_response_target("remote-a")],
                default_launch_target: Some("remote-a".to_string()),
            },
            true,
            SpawnTool::Codex,
            None,
        )
    }

    fn move_selection_entry(name: &str) -> DirEntry {
        apply_response_entry(name, &format!("/tmp/projects/{name}"))
    }

    fn move_selection_picker(names: &[&str]) -> PickerState {
        apply_response_picker(
            names
                .iter()
                .map(|name| move_selection_entry(name))
                .collect(),
        )
    }

    fn apply_response_dir_list(entries: Vec<DirEntry>) -> DirListResponse {
        DirListResponse {
            path: "/tmp/next".to_string(),
            entries,
            overlay_label: Some("new overlay".to_string()),
            groups: Vec::new(),
            launch_targets: Vec::new(),
            default_launch_target: None,
        }
    }

    #[test]
    fn apply_response_preserves_selection_by_normalized_path_and_retains_batch_exclusions() {
        let mut picker = apply_response_picker(vec![
            apply_response_entry("a", "/tmp/projects/a"),
            apply_response_entry("b", "/tmp/projects/b/"),
        ]);
        picker.selection = PickerSelection::Entry(1);
        picker.scroll = 5;
        picker.launch_target = Some("remote-a".to_string());
        picker.group_edit_target = Some("beta".to_string());
        picker.current_theme_color = Some(Color::Red);
        picker.entry_theme_colors = vec![Some(Color::Blue), Some(Color::Green)];
        picker
            .batch_excluded_paths
            .insert(normalize_path("/tmp/projects/b/"));
        picker
            .batch_excluded_paths
            .insert(normalize_path("/tmp/projects/missing"));

        let mut response = apply_response_dir_list(vec![
            apply_response_entry("renamed-b", "/tmp/projects/b"),
            apply_response_entry("c", "/tmp/projects/c"),
        ]);
        response.groups = vec!["alpha".to_string(), "beta".to_string()];
        response.launch_targets = vec![
            apply_response_target("remote-a"),
            apply_response_target("remote-b"),
        ];
        response.default_launch_target = Some("remote-b".to_string());
        picker.apply_response(response, true);

        assert_eq!(picker.current_path, "/tmp/next");
        assert_eq!(
            picker
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<Vec<_>>(),
            vec!["renamed-b", "c"]
        );
        assert_eq!(picker.overlay_label.as_deref(), Some("new overlay"));
        assert_eq!(picker.selection, PickerSelection::Entry(0));
        assert_eq!(picker.scroll, 1);
        assert_eq!(picker.launch_target.as_deref(), Some("remote-a"));
        assert_eq!(picker.launch_targets[0].id, "local");
        assert_eq!(picker.group_edit_target.as_deref(), Some("beta"));
        assert_eq!(
            picker.available_groups,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(picker.current_theme_color, None);
        assert!(picker.entry_theme_colors.is_empty());
        assert_eq!(picker.batch_excluded_paths.len(), 1);
        assert!(picker
            .batch_excluded_paths
            .contains(&normalize_path("/tmp/projects/b")));
    }

    #[test]
    fn apply_response_resets_selection_uses_default_launch_target_and_clears_batch_exclusions() {
        let mut picker = apply_response_picker(vec![apply_response_entry("a", "/tmp/projects/a")]);
        picker.selection = PickerSelection::Entry(0);
        picker.scroll = 3;
        picker.launch_target = Some("remote-a".to_string());
        picker.group_edit_target = Some("beta".to_string());
        picker
            .batch_excluded_paths
            .insert(normalize_path("/tmp/projects/a"));

        let mut response =
            apply_response_dir_list(vec![apply_response_entry("b", "/tmp/projects/b")]);
        response.groups = vec!["gamma".to_string(), "delta".to_string()];
        response.launch_targets = vec![apply_response_target("remote-b")];
        response.default_launch_target = Some("remote-b".to_string());
        picker.apply_response(response, false);

        assert_eq!(picker.selection, PickerSelection::SpawnHere);
        assert_eq!(picker.scroll, 0);
        assert_eq!(picker.launch_target.as_deref(), Some("remote-b"));
        assert_eq!(
            picker
                .launch_targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local", "remote-b"]
        );
        assert_eq!(picker.group_edit_target.as_deref(), Some("gamma"));
        assert!(picker.batch_excluded_paths.is_empty());
    }

    #[test]
    fn launch_target_preview_uses_longest_mapping_and_blocks_unmapped_remote_cwds() {
        let target = mapped_response_target("devbox");

        let preview =
            launch_target_preview_for_path("/Users/tester/repos/opensource/swimmers", &target);
        assert_eq!(preview.target_id, "devbox");
        assert_eq!(preview.target_label, "devbox target");
        assert_eq!(
            preview.remote_cwd.as_deref(),
            Some("/srv/opensource/swimmers")
        );
        assert_eq!(preview.blocked_reason, None);

        let blocker = launch_target_preview_for_path("/tmp/outside", &target);
        assert_eq!(blocker.remote_cwd, None);
        assert_eq!(blocker.blocked_reason, Some("unmapped cwd"));

        let unsupported = launch_target_preview_for_path(
            "/Users/tester/repos/swimmers",
            &apply_response_target("legacy"),
        );
        assert_eq!(unsupported.blocked_reason, Some("unsupported target"));
    }

    #[test]
    fn batch_launch_blockers_find_unmapped_remote_rows_only() {
        let mut picker = PickerState::new(
            0,
            0,
            DirListResponse {
                path: "/Users/tester/repos".to_string(),
                entries: vec![
                    apply_response_entry("swimmers", "/Users/tester/repos/opensource/swimmers"),
                    apply_response_entry("outside", "/tmp/outside"),
                ],
                overlay_label: None,
                groups: Vec::new(),
                launch_targets: vec![mapped_response_target("devbox")],
                default_launch_target: Some("devbox".to_string()),
            },
            true,
            SpawnTool::Codex,
            None,
        );

        let blockers = picker.batch_launch_blockers();
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].local_cwd, "/tmp/outside");
        assert_eq!(blockers[0].blocked_reason, Some("unmapped cwd"));

        picker.launch_target = Some("local".to_string());
        assert!(picker.batch_launch_blockers().is_empty());
    }

    #[test]
    fn apply_response_clamps_fallback_selection_snaps_search_and_clears_missing_groups() {
        let mut picker = apply_response_picker(vec![
            apply_response_entry("a", "/tmp/old/a"),
            apply_response_entry("b", "/tmp/old/b"),
            apply_response_entry("c", "/tmp/old/c"),
        ]);
        picker.selection = PickerSelection::Entry(2);
        picker.scroll = 9;
        picker.search = "match".to_string();
        picker.group_edit_target = Some("alpha".to_string());

        let response = apply_response_dir_list(vec![
            apply_response_entry("matchable", "/tmp/new/matchable"),
            apply_response_entry("plain", "/tmp/new/plain"),
        ]);
        picker.apply_response(response, true);

        assert_eq!(picker.selection, PickerSelection::Entry(0));
        assert_eq!(picker.scroll, 0);
        assert!(picker.available_groups.is_empty());
        assert_eq!(picker.group_edit_target, None);
    }

    #[test]
    fn apply_response_preserved_entry_falls_back_to_spawn_here_when_response_is_empty() {
        let mut picker = apply_response_picker(vec![apply_response_entry("a", "/tmp/projects/a")]);
        picker.selection = PickerSelection::Entry(0);
        picker.scroll = 4;
        picker
            .batch_excluded_paths
            .insert(normalize_path("/tmp/projects/a"));

        picker.apply_response(apply_response_dir_list(Vec::new()), true);

        assert_eq!(picker.selection, PickerSelection::SpawnHere);
        assert_eq!(picker.scroll, 0);
        assert!(picker.batch_excluded_paths.is_empty());
    }

    #[test]
    fn move_selection_keeps_empty_visible_spawn_here_as_noop() {
        let mut picker = move_selection_picker(&[]);
        picker.selection = PickerSelection::SpawnHere;
        picker.scroll = 7;

        picker.move_selection(1, 3);

        assert_eq!(picker.selection, PickerSelection::SpawnHere);
        assert_eq!(picker.scroll, 7);
    }

    #[test]
    fn move_selection_clamps_to_spawn_here_and_last_visible_entry() {
        let mut picker = move_selection_picker(&["alpha", "beta", "gamma"]);

        picker.move_selection(99, 3);

        assert_eq!(picker.selection, PickerSelection::Entry(2));

        picker.move_selection(-99, 3);

        assert_eq!(picker.selection, PickerSelection::SpawnHere);
    }

    #[test]
    fn move_selection_falls_back_to_spawn_here_when_selection_is_filtered_out() {
        let mut picker = move_selection_picker(&["alpha", "beta"]);
        picker.selection = PickerSelection::Entry(1);
        picker.scroll = 4;
        picker.search = "alpha".to_string();

        picker.move_selection(0, 3);

        assert_eq!(picker.selection, PickerSelection::SpawnHere);
        assert_eq!(picker.scroll, 0);
    }

    #[test]
    fn move_selection_maps_visible_position_to_raw_entry_and_scrolls_to_it() {
        let mut picker = move_selection_picker(&["alpha", "beta", "alpine", "gamma"]);
        picker.search = "al".to_string();
        picker.selection = PickerSelection::Entry(0);
        picker.scroll = 0;

        picker.move_selection(1, 1);

        assert_eq!(picker.selection, PickerSelection::Entry(2));
        assert_eq!(picker.scroll, 1);
    }

    #[test]
    fn picker_filter_render_plan_preserves_labels_positions_and_target() {
        let layout = filter_test_layout();
        let mut picker = filter_test_picker();
        picker.overlay_label = Some("Overlay".to_string());
        picker.group_edit_target = Some("beta".to_string());

        let items = picker_filter_render_items(&picker, &layout);

        assert_eq!(
            items,
            vec![
                PickerFilterRenderItem {
                    rect: layout.env_button,
                    label: "[overlay]".to_string(),
                    color: Color::White,
                },
                PickerFilterRenderItem {
                    rect: layout.group_buttons[0].1,
                    label: "[alpha]".to_string(),
                    color: Color::DarkGrey,
                },
                PickerFilterRenderItem {
                    rect: layout.group_buttons[1].1,
                    label: "[beta]".to_string(),
                    color: Color::DarkGrey,
                },
                PickerFilterRenderItem {
                    rect: layout.all_button,
                    label: "[all folders]".to_string(),
                    color: Color::DarkGrey,
                },
                PickerFilterRenderItem {
                    rect: layout.group_target_button.expect("target rect"),
                    label: "[target:beta]".to_string(),
                    color: Color::Yellow,
                },
            ]
        );
    }

    #[test]
    fn picker_filter_render_plan_truncates_labels_to_control_widths() {
        let mut layout = filter_test_layout();
        layout.env_button.width = 8;
        layout.group_buttons = vec![("frontend-platform".to_string(), rect(12, 3, 6))];
        layout.all_button.width = 5;
        layout.group_target_button = Some(rect(42, 3, 7));

        let mut picker = filter_test_picker();
        picker.overlay_label = Some("ExtremelyLongOverlay".to_string());
        picker.group_edit_target = Some("skillbox-prod-west".to_string());

        let items = picker_filter_render_items(&picker, &layout);

        assert_eq!(items[0].label, truncate_label("[extremelylongoverlay]", 8));
        assert_eq!(items[1].label, truncate_label("[frontend-platform]", 6));
        assert_eq!(items[2].label, truncate_label("[all folders]", 5));
        assert_eq!(
            items[3].label,
            truncate_label("[target:skillbox-prod-west]", 7)
        );
    }

    #[test]
    fn picker_filter_render_plan_uses_active_colors_for_each_mode() {
        let layout = filter_test_layout();
        let mut picker = filter_test_picker();

        let managed_items = picker_filter_render_items(&picker, &layout);
        assert_eq!(managed_items[0].color, Color::White);
        assert_eq!(managed_items[3].color, Color::DarkGrey);

        picker.current_group = Some("alpha".to_string());
        let group_items = picker_filter_render_items(&picker, &layout);
        assert_eq!(group_items[0].color, Color::DarkGrey);
        assert_eq!(group_items[1].color, Color::White);
        assert_eq!(group_items[2].color, Color::DarkGrey);
        assert_eq!(group_items[3].color, Color::DarkGrey);

        picker.current_group = None;
        picker.managed_only = false;
        let all_items = picker_filter_render_items(&picker, &layout);
        assert_eq!(all_items[0].color, Color::DarkGrey);
        assert_eq!(all_items[3].color, Color::White);
    }

    #[test]
    fn picker_filter_render_plan_requires_target_state_and_layout() {
        let mut layout = filter_test_layout();
        let mut picker = filter_test_picker();

        picker.group_edit_target = None;
        assert!(picker_filter_render_items(&picker, &layout)
            .iter()
            .all(|item| !item.label.starts_with("[target:")));

        picker.group_edit_target = Some("alpha".to_string());
        layout.group_target_button = None;
        assert!(picker_filter_render_items(&picker, &layout)
            .iter()
            .all(|item| !item.label.starts_with("[target:")));
    }

    #[test]
    fn picker_action_at_maps_all_top_controls_and_rect_edges() {
        let mut layout = filter_test_layout();
        layout.back_button = Some(rect(4, 2, 4));

        for (control, action) in [
            (layout.close_button, PickerAction::Close),
            (layout.tool_button, PickerAction::ToggleTool),
            (
                layout.launch_target_button,
                PickerAction::ToggleLaunchTarget,
            ),
            (layout.exclude_button, PickerAction::ToggleBatchExcludeMode),
            (layout.batch_button, PickerAction::BatchVisible),
            (layout.back_button.expect("back rect"), PickerAction::Up),
        ] {
            assert_eq!(top_action_at(&layout, control), Some(action.clone()));
            assert_eq!(
                picker_top_control_action_at(&layout, control.right() - 1, control.y),
                Some(action)
            );
        }

        assert_eq!(
            picker_top_control_action_at(
                &layout,
                layout.close_button.right(),
                layout.close_button.y
            ),
            None
        );
        assert_eq!(
            picker_top_control_action_at(
                &layout,
                layout.close_button.x,
                layout.close_button.bottom()
            ),
            None
        );

        let back_button = layout.back_button.expect("back rect");
        layout.back_button = None;
        assert_eq!(
            picker_top_control_action_at(&layout, back_button.x, back_button.y),
            None
        );
    }

    #[test]
    fn picker_action_at_preserves_top_control_precedence() {
        let overlap = rect(10, 1, 4);
        let away = rect(40, 10, 4);
        let mut layout = filter_test_layout();
        layout.close_button = overlap;
        layout.tool_button = overlap;
        layout.launch_target_button = overlap;
        layout.exclude_button = overlap;
        layout.batch_button = overlap;
        layout.back_button = Some(overlap);

        assert_eq!(top_action_at(&layout, overlap), Some(PickerAction::Close));

        layout.close_button = away;
        assert_eq!(
            top_action_at(&layout, overlap),
            Some(PickerAction::ToggleTool)
        );

        layout.tool_button = away;
        assert_eq!(
            top_action_at(&layout, overlap),
            Some(PickerAction::ToggleLaunchTarget)
        );

        layout.launch_target_button = away;
        assert_eq!(
            top_action_at(&layout, overlap),
            Some(PickerAction::ToggleBatchExcludeMode)
        );

        layout.exclude_button = away;
        assert_eq!(
            top_action_at(&layout, overlap),
            Some(PickerAction::BatchVisible)
        );

        layout.batch_button = away;
        assert_eq!(top_action_at(&layout, overlap), Some(PickerAction::Up));

        layout.back_button = None;
        assert_eq!(top_action_at(&layout, overlap), None);
    }

    #[test]
    fn picker_filter_hit_test_maps_controls_to_actions() {
        let layout = filter_test_layout();

        assert_eq!(
            picker_filter_action_at(&layout, 2, 3),
            Some(PickerAction::ToggleManaged(true))
        );
        assert_eq!(
            picker_filter_action_at(&layout, 12, 3),
            Some(PickerAction::ActivateGroup("alpha".to_string()))
        );
        assert_eq!(
            picker_filter_action_at(&layout, 20, 3),
            Some(PickerAction::ActivateGroup("beta".to_string()))
        );
        assert_eq!(
            picker_filter_action_at(&layout, 28, 3),
            Some(PickerAction::ToggleManaged(false))
        );
        assert_eq!(
            picker_filter_action_at(&layout, 42, 3),
            Some(PickerAction::CycleGroupEditTarget)
        );
        assert_eq!(
            picker_filter_action_at(&layout, 2, 4),
            Some(PickerAction::ActivateCurrentPath)
        );
    }

    #[test]
    fn picker_filter_hit_test_preserves_order_for_overlapping_controls() {
        let mut layout = filter_test_layout();
        layout.env_button = rect(10, 3, 8);
        layout.group_buttons = vec![("work".to_string(), rect(10, 3, 8))];
        layout.all_button = rect(10, 3, 8);
        layout.group_target_button = Some(rect(10, 3, 8));
        layout.spawn_here_button = rect(10, 3, 8);

        assert_eq!(
            picker_filter_action_at(&layout, 10, 3),
            Some(PickerAction::ToggleManaged(true))
        );

        layout.env_button = rect(2, 3, 8);
        assert_eq!(
            picker_filter_action_at(&layout, 10, 3),
            Some(PickerAction::ActivateGroup("work".to_string()))
        );

        layout.group_buttons.clear();
        assert_eq!(
            picker_filter_action_at(&layout, 10, 3),
            Some(PickerAction::ToggleManaged(false))
        );

        layout.all_button = rect(20, 3, 8);
        assert_eq!(
            picker_filter_action_at(&layout, 10, 3),
            Some(PickerAction::CycleGroupEditTarget)
        );

        layout.group_target_button = None;
        assert_eq!(
            picker_filter_action_at(&layout, 10, 3),
            Some(PickerAction::ActivateCurrentPath)
        );
    }

    #[test]
    fn picker_filter_hit_test_ignores_invalid_coordinates() {
        let layout = filter_test_layout();

        assert_eq!(picker_filter_action_at(&layout, 0, 0), None);
        assert_eq!(picker_filter_action_at(&layout, 2, 2), None);
        assert_eq!(picker_filter_action_at(&layout, 78, 4), None);
    }

    #[test]
    fn picker_entry_action_rects_ignore_offscreen_rows() {
        let mut picker = pointer_test_picker(false);
        let layout = picker_layout(&picker, field_rect());
        picker.scroll = 1;

        assert!(picker_entry_action_rects(&picker, &layout, 0, 0).is_empty());
        assert!(picker_entry_action_rects(&picker, &layout, 2, 0).is_empty());
    }

    #[test]
    fn picker_entry_action_rects_ignore_missing_entries() {
        let picker = pointer_test_picker(false);
        let layout = picker_layout(&picker, field_rect());

        assert!(picker_entry_action_rects(&picker, &layout, 0, 99).is_empty());
    }

    #[test]
    fn picker_entry_action_rects_return_empty_for_entries_without_actions() {
        let mut entry = pointer_test_entry();
        entry.repo_dirty = None;
        entry.has_restart = None;
        entry.open_url = None;
        let picker = pointer_test_picker_with_entry(entry, false);
        let layout = picker_layout(&picker, field_rect());

        assert!(picker_entry_action_rects(&picker, &layout, 0, 0).is_empty());
    }

    #[test]
    fn picker_entry_action_rects_keep_status_only_actions_unclickable() {
        let mut entry = pointer_test_entry();
        entry.repo_dirty = Some(false);
        entry.open_url = None;
        entry.repo_action = Some(RepoActionStatus {
            kind: RepoActionKind::Commit,
            state: RepoActionState::Succeeded,
            detail: None,
        });
        let picker = pointer_test_picker_with_entry(entry, false);
        let layout = picker_layout(&picker, field_rect());
        let row_y = layout.first_entry_y;
        let start_x = layout.content.right() - ("[done]".len() + 1 + "[restart]".len()) as u16;
        let restart_x = start_x + "[done]".len() as u16 + 1;

        assert_eq!(
            picker_entry_action_rects(&picker, &layout, 0, 0),
            vec![(
                Rect {
                    x: restart_x,
                    y: row_y,
                    width: "[restart]".len() as u16,
                    height: 1,
                },
                RepoActionKind::Restart,
            )]
        );
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, start_x, row_y),
            PickerAction::ActivateEntry(0)
        );
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, restart_x, row_y),
            PickerAction::StartRepoAction(0, RepoActionKind::Restart)
        );
    }

    #[test]
    fn picker_entry_action_rects_right_align_clickable_actions() {
        let picker = pointer_test_picker(false);
        let layout = picker_layout(&picker, field_rect());
        let row_y = layout.first_entry_y;
        let commit_width = "[commit]".len() as u16;
        let restart_width = "[restart]".len() as u16;
        let open_width = "[open]".len() as u16;
        let start_x = layout
            .content
            .right()
            .saturating_sub(commit_width + 1 + restart_width + 1 + open_width);

        let rects = picker_entry_action_rects(&picker, &layout, 0, 0);

        assert_eq!(
            rects,
            vec![
                (
                    Rect {
                        x: start_x,
                        y: row_y,
                        width: commit_width,
                        height: 1,
                    },
                    RepoActionKind::Commit,
                ),
                (
                    Rect {
                        x: start_x + commit_width + 1,
                        y: row_y,
                        width: restart_width,
                        height: 1,
                    },
                    RepoActionKind::Restart,
                ),
                (
                    Rect {
                        x: start_x + commit_width + 1 + restart_width + 1,
                        y: row_y,
                        width: open_width,
                        height: 1,
                    },
                    RepoActionKind::Open,
                ),
            ]
        );
        assert_eq!(
            rects.last().expect("open rect").0.right(),
            layout.content.right()
        );
    }

    #[test]
    fn picker_entry_action_rects_suppress_running_action_clicks() {
        let mut entry = pointer_test_entry();
        entry.repo_action = Some(RepoActionStatus {
            kind: RepoActionKind::Restart,
            state: RepoActionState::Running,
            detail: None,
        });
        let picker = pointer_test_picker_with_entry(entry, false);
        let layout = picker_layout(&picker, field_rect());
        let running_x = layout.content.right() - "[running]".len() as u16;

        assert!(picker_entry_action_rects(&picker, &layout, 0, 0).is_empty());
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, running_x, layout.first_entry_y),
            PickerAction::ActivateEntry(0)
        );
    }

    #[test]
    fn picker_entry_action_rects_coexist_with_batch_exclude_rects() {
        let picker = pointer_test_picker(true);
        let layout = picker_layout(&picker, field_rect());
        let exclude_rect = picker_batch_exclude_rect(&picker, &layout, 0, 0).expect("exclude rect");
        let action_rects = picker_entry_action_rects(&picker, &layout, 0, 0);
        let (commit_rect, commit_kind) = action_rects.first().copied().expect("commit rect");

        assert_eq!(exclude_rect.right() + 1, commit_rect.x);
        assert_eq!(
            action_rects.last().expect("open rect").0.right(),
            layout.content.right()
        );
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, exclude_rect.x, exclude_rect.y),
            PickerAction::ToggleBatchExclude(0)
        );
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, commit_rect.x, commit_rect.y),
            PickerAction::StartRepoAction(0, commit_kind)
        );
    }

    #[test]
    fn picker_entry_pointer_action_toggles_batch_exclude_target_first() {
        let picker = pointer_test_picker(true);
        let layout = picker_layout(&picker, field_rect());
        let exclude_rect = picker_batch_exclude_rect(&picker, &layout, 0, 0).expect("exclude rect");

        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, exclude_rect.x, exclude_rect.y),
            PickerAction::ToggleBatchExclude(0)
        );
    }

    #[test]
    fn picker_entry_pointer_action_starts_repo_action_target() {
        let picker = pointer_test_picker(false);
        let layout = picker_layout(&picker, field_rect());
        let (commit_rect, kind) = picker_entry_action_rects(&picker, &layout, 0, 0)
            .into_iter()
            .next()
            .expect("commit rect");

        assert_eq!(kind, RepoActionKind::Commit);
        assert_eq!(
            picker_entry_pointer_action(&picker, &layout, 0, 0, commit_rect.x, commit_rect.y),
            PickerAction::StartRepoAction(0, RepoActionKind::Commit)
        );
    }

    #[test]
    fn picker_entry_pointer_action_activates_entry_outside_row_targets() {
        let picker = pointer_test_picker(true);
        let layout = picker_layout(&picker, field_rect());

        assert_eq!(
            picker_entry_pointer_action(
                &picker,
                &layout,
                0,
                0,
                layout.content.x,
                layout.first_entry_y
            ),
            PickerAction::ActivateEntry(0)
        );
    }

    #[test]
    fn initial_request_title_switches_for_group_context() {
        assert_eq!(initial_request_title(None), "initial request");
        assert_eq!(
            initial_request_title(Some(("frontend", 3))),
            "send to school"
        );
    }

    #[test]
    fn initial_request_context_line_preserves_context_priority_and_truncation() {
        let narrow_layout = initial_request_test_layout(20);
        let wide_layout = initial_request_test_layout(56);
        let request = InitialRequestState::new("/fixture/projects/swimmers".to_string(), None);
        let batch_request =
            InitialRequestState::new_batch(vec!["/tmp/a".to_string(), "/tmp/b".to_string()], None);
        let remote_batch_request = InitialRequestState::new_batch(
            vec!["/tmp/a".to_string(), "/tmp/b".to_string()],
            Some("devbox".to_string()),
        );

        assert_eq!(
            initial_request_context_line(
                &request,
                &narrow_layout,
                Some(("school-of-long-name", 7))
            ),
            "school: school-of-l~"
        );
        assert_eq!(
            initial_request_context_line(&batch_request, &wide_layout, None),
            "batch: 2 included dirs"
        );
        assert_eq!(
            initial_request_context_line(&remote_batch_request, &wide_layout, None),
            "batch: 2 included dirs -> devbox"
        );
        assert_eq!(
            initial_request_context_line(&request, &narrow_layout, None),
            "cwd: .../swimmers"
        );
    }

    #[test]
    fn initial_request_hint_preserves_toggle_text_and_batch_pluralization() {
        let request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
        let batch_request =
            InitialRequestState::new_batch(vec!["/tmp/a".to_string(), "/tmp/b".to_string()], None);
        let voice_hint = toggle_hint();

        assert_eq!(
            initial_request_hint(&request, None),
            format!("enter creates hidden swimmer  {voice_hint}  esc cancels")
        );
        assert_eq!(
            initial_request_hint(&batch_request, None),
            format!("enter creates hidden swimmers  {voice_hint}  esc cancels")
        );
        assert_eq!(
            initial_request_hint(&request, Some(("frontend", 3))),
            format!("enter sends to school  {voice_hint}  esc cancels")
        );
    }

    #[test]
    fn initial_request_input_model_preserves_placeholder_tail_and_cursor() {
        let layout = initial_request_test_layout(10);
        let empty_request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
        let mut typed_request = empty_request.clone();
        typed_request.value = "abcdefghijk".to_string();

        assert_eq!(
            initial_request_input_render_model(&empty_request, &layout),
            InitialRequestInputRenderModel {
                visible: "type i~".to_string(),
                color: Color::DarkGrey,
                cursor_x: layout.content.x + 2,
            }
        );
        assert_eq!(
            initial_request_input_render_model(&typed_request, &layout),
            InitialRequestInputRenderModel {
                visible: "efghijk".to_string(),
                color: Color::White,
                cursor_x: layout.content.x + 9,
            }
        );
    }

    #[test]
    fn initial_request_voice_color_preserves_state_colors() {
        assert_eq!(
            initial_request_voice_color(&VoiceUiState::Transcribing),
            Color::Yellow
        );
        assert_eq!(
            initial_request_voice_color(&VoiceUiState::Recording),
            Color::Red
        );
        assert_eq!(
            initial_request_voice_color(&VoiceUiState::Failed("denied".to_string())),
            Color::Red
        );
        assert_eq!(
            initial_request_voice_color(&VoiceUiState::Unsupported),
            Color::DarkGrey
        );
        assert_eq!(
            initial_request_voice_color(&VoiceUiState::Ready),
            Color::Cyan
        );
    }
}
