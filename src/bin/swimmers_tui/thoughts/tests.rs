use super::*;

fn rect(x: u16, width: u16) -> Rect {
    Rect {
        x,
        y: header_filter_row(),
        width,
        height: 1,
    }
}

fn chip(cwd: &str, x: u16) -> ThoughtChipLayout {
    ThoughtChipLayout {
        rect: rect(x, 6),
        cwd: cwd.to_string(),
        label: cwd.to_string(),
        color: Color::Cyan,
    }
}

fn layout() -> HeaderFilterLayout {
    HeaderFilterLayout {
        filter_out_rect: Some(rect(2, 12)),
        clear_filters_rect: Some(rect(16, 15)),
        chips: vec![chip("/repo/a", 34), chip("/repo/b", 42)],
    }
}

fn repo_summary(cwd: &str, label: &str, count: usize, color: Color) -> ThoughtRepoSummary {
    ThoughtRepoSummary {
        cwd: cwd.to_string(),
        label: label.to_string(),
        count,
        color,
        last_seen: 0,
    }
}

fn filter_chip_labels(chips: &[FilterChipData]) -> Vec<&str> {
    chips.iter().map(|chip| chip.label.as_str()).collect()
}

fn thought_content() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 8,
    }
}

fn thought_entry(session_id: &str, label: &str, thought: &str) -> ThoughtPanelEntryView {
    ThoughtPanelEntryView {
        session_id: session_id.to_string(),
        label: label.to_string(),
        tmux_name: label.to_string(),
        cwd: format!("/repo/{label}"),
        target_label: "local".to_string(),
        batch: None,
        state: SessionState::Idle,
        current_command: None,
        tool: None,
        updated_at: None,
        rest_state: RestState::Active,
        color: Color::White,
        is_stale: false,
        transport_health: TransportHealth::Healthy,
        thought: thought.to_string(),
        mermaid_label: None,
        has_commit_candidate: false,
    }
}

#[test]
fn pwd_group_header_label_compacts_cross_host_targets() {
    let mut local = thought_entry("local", "local-agent", "ready");
    local.cwd = "/repo/swimmers".to_string();
    local.target_label = "local".to_string();
    let mut remote = thought_entry("remote", "remote-agent", "waiting");
    remote.cwd = "/repo/swimmers".to_string();
    remote.target_label = "Skillbox devbox".to_string();

    let groups = build_thought_groups(&[local, remote], ThoughtGroupBy::Pwd);

    assert_eq!(groups.len(), 1);
    assert_eq!(
        thought_group_header_label(ThoughtGroupBy::Pwd, &groups[0]),
        "swimmers L+Skillbox"
    );
    assert_eq!(
        thought_group_header_label(ThoughtGroupBy::Batch, &groups[0]),
        "swimmers"
    );
}

#[test]
fn thought_name_based_color_matches_shared_hsl_and_dark_terminal_adjustment() {
    let name = "repo-session";
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let seed = hasher.finish();
    let hue = (seed % 3600) as f64 / 10.0;
    let saturation = 0.50 + ((seed >> 16) % 200) as f64 / 1000.0;
    let lightness = 0.45 + ((seed >> 32) % 150) as f64 / 1000.0;
    let expected = rgb_color(adjust_for_dark_terminal(hsl_to_rgb(
        hue, saturation, lightness,
    )));

    assert_eq!(name_based_color(name), expected);
}

#[test]
fn thought_hsl_to_rgb_shared_helper_wraps_and_rounds() {
    assert_eq!(hsl_to_rgb(30.0, 1.0, 0.5), (255, 128, 0));
    assert_eq!(hsl_to_rgb(37.0, 0.0, 0.5), (128, 128, 128));
    assert_eq!(hsl_to_rgb(-120.0, 1.0, 0.5), hsl_to_rgb(240.0, 1.0, 0.5));
}

#[test]
fn header_filter_action_prioritizes_control_rects() {
    let filter = ThoughtFilter::default();
    let row = header_filter_row();

    assert_eq!(
        header_filter_action_for_layout(&layout(), &filter, 2, row),
        Some(ThoughtPanelAction::ToggleFilterOutMode)
    );
    assert_eq!(
        header_filter_action_for_layout(&layout(), &filter, 16, row),
        Some(ThoughtPanelAction::ClearFilters)
    );
}

#[test]
fn header_filter_action_prioritizes_controls_over_overlapping_chips() {
    let filter = ThoughtFilter::default();
    let row = header_filter_row();
    let layout = HeaderFilterLayout {
        filter_out_rect: Some(rect(2, 12)),
        clear_filters_rect: Some(rect(16, 15)),
        chips: vec![ThoughtChipLayout {
            rect: rect(2, 40),
            cwd: "/repo/a".to_string(),
            label: "1xa".to_string(),
            color: Color::Cyan,
        }],
    };

    assert_eq!(
        header_filter_action_for_layout(&layout, &filter, 2, row),
        Some(ThoughtPanelAction::ToggleFilterOutMode)
    );
    assert_eq!(
        header_filter_action_for_layout(&layout, &filter, 16, row),
        Some(ThoughtPanelAction::ClearFilters)
    );
}

#[test]
fn header_filter_action_maps_chip_modes() {
    let row = header_filter_row();

    assert_eq!(
        header_filter_action_for_layout(&layout(), &ThoughtFilter::default(), 34, row),
        Some(ThoughtPanelAction::FilterByCwd("/repo/a".to_string()))
    );

    let active_filter = ThoughtFilter {
        cwd: Some("/repo/a".to_string()),
        ..ThoughtFilter::default()
    };
    assert_eq!(
        header_filter_action_for_layout(&layout(), &active_filter, 34, row),
        Some(ThoughtPanelAction::OpenRepoInEditor("/repo/a".to_string()))
    );

    let filter_out = ThoughtFilter {
        filter_out_mode: true,
        ..ThoughtFilter::default()
    };
    assert_eq!(
        header_filter_action_for_layout(&layout(), &filter_out, 42, row),
        Some(ThoughtPanelAction::ToggleFilterOutCwd(
            "/repo/b".to_string()
        ))
    );
}

#[test]
fn header_filter_action_ignores_misses() {
    assert_eq!(
        header_filter_action_for_layout(
            &layout(),
            &ThoughtFilter::default(),
            34,
            header_filter_row() + 1
        ),
        None
    );
}

#[test]
fn header_filter_chips_include_exact_budget_and_reject_one_column_over() {
    let summaries = vec![
        repo_summary("/repo/a", "a", 1, Color::Cyan),
        repo_summary("/repo/bb", "bb", 2, Color::Yellow),
    ];
    let filter = ThoughtFilter::default();
    let exact_budget = display_width("1xa") + HEADER_FILTER_GAP + display_width("2xbb");

    let (chips, chips_width) = filter_chips_for_summaries(&summaries, &filter, exact_budget);

    assert_eq!(filter_chip_labels(&chips), vec!["1xa", "2xbb"]);
    assert_eq!(chips_width, exact_budget);

    let (chips, chips_width) = filter_chips_for_summaries(&summaries, &filter, exact_budget - 1);

    assert_eq!(filter_chip_labels(&chips), vec!["1xa"]);
    assert_eq!(chips_width, display_width("1xa"));
}

#[test]
fn header_filter_chips_stop_after_overflow_and_hide_later_chips() {
    let summaries = vec![
        repo_summary("/repo/a", "a", 1, Color::Cyan),
        repo_summary("/repo/overflow", "overflow", 2, Color::Yellow),
        repo_summary("/repo/c", "c", 3, Color::Magenta),
    ];
    let filter = ThoughtFilter::default();
    let budget_that_would_fit_a_and_c =
        display_width("1xa") + HEADER_FILTER_GAP + display_width("3xc");

    let (chips, chips_width) =
        filter_chips_for_summaries(&summaries, &filter, budget_that_would_fit_a_and_c);

    assert_eq!(filter_chip_labels(&chips), vec!["1xa"]);
    assert_eq!(chips_width, display_width("1xa"));
}

#[test]
fn header_filter_chips_mark_active_cwd_as_code_dot_and_dim_other_cwds() {
    let summaries = vec![
        repo_summary("/repo/active", "active", 3, Color::Yellow),
        repo_summary("/repo/other", "other", 2, Color::Cyan),
    ];
    let filter = ThoughtFilter {
        cwd: Some("/repo/active".to_string()),
        ..ThoughtFilter::default()
    };

    let (chips, chips_width) = filter_chips_for_summaries(&summaries, &filter, 80);

    assert_eq!(filter_chip_labels(&chips), vec!["code .", "2xother"]);
    assert_eq!(chips[0].color, Color::Yellow);
    assert_eq!(chips[1].color, Color::DarkGrey);
    assert_eq!(
        chips_width,
        display_width("code .") + HEADER_FILTER_GAP + display_width("2xother")
    );
}

#[test]
fn header_filter_chips_dim_excluded_cwd_only_in_filter_out_mode() {
    let summaries = vec![
        repo_summary("/repo/hidden", "hidden", 1, Color::Yellow),
        repo_summary("/repo/shown", "shown", 1, Color::Cyan),
    ];
    let mut filter = ThoughtFilter {
        filter_out_mode: true,
        ..ThoughtFilter::default()
    };
    filter.excluded_cwds.insert("/repo/hidden".to_string());

    let (chips, _) = filter_chips_for_summaries(&summaries, &filter, 80);

    assert_eq!(filter_chip_labels(&chips), vec!["1xhidden", "1xshown"]);
    assert_eq!(chips[0].color, Color::DarkGrey);
    assert_eq!(chips[1].color, Color::Cyan);
}

#[test]
fn build_flat_rows_selection_skips_empty_entry_rows() {
    assert_eq!(flat_row_selection(0, 3, false), FlatRowSelection::Skip);
}

#[test]
fn build_flat_rows_returns_empty_when_capacity_zero() {
    let entries = vec![thought_entry("new", "new", "newest thought")];

    assert!(build_flat_rows(&entries, thought_content(), 0).is_empty());
}

#[test]
fn build_flat_rows_partially_includes_newest_entry_when_it_exceeds_capacity() {
    let entries = vec![thought_entry("new", "new", "newest thought")];

    let rows = build_flat_rows(&entries, thought_content(), 1);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "new");
    assert_eq!(rows[0].line, "  newest thought");
}

#[test]
fn build_flat_rows_stops_before_older_entry_that_cannot_fit() {
    let entries = vec![
        thought_entry("old", "old", "older thought"),
        thought_entry("new", "new", "newest thought"),
    ];

    let rows = build_flat_rows(&entries, thought_content(), 3);

    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["new", "new"]
    );
}
