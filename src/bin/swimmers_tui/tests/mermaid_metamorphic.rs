use super::*;

proptest::proptest! {
    #[test]
    fn mermaid_mr_fit_is_canonical_after_zoom_and_pan_sequences(
        source in mermaid_flowchart_source_strategy(),
        width in 100u16..160,
        height in 24u16..52,
        ops in mermaid_metamorphic_ops_strategy(),
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        let baseline = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        apply_mermaid_metamorphic_ops(&mut app, layout, &ops);
        app.reset_mermaid_viewer_fit();
        let after_fit = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        proptest::prop_assert_eq!(after_fit, baseline);
    }

    #[test]
    fn mermaid_mr_pan_round_trip_restores_viewport(
        source in mermaid_flowchart_source_strategy(),
        width in 110u16..180,
        height in 28u16..56,
        x_ratio_percent in -90i16..=90,
        y_ratio_percent in -90i16..=90,
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        let content_rect = mermaid_content_rect_for_layout(layout);

        app.zoom_mermaid_viewer(MERMAID_KEYBOARD_ZOOM_STEP_PERCENT, None, content_rect);
        let baseline = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        let (dx, dy) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                let (left, right, up, down) = mermaid_pan_headroom(viewer, content_rect);
                (
                    mermaid_safe_pan_distance(x_ratio_percent, left, right),
                    mermaid_safe_pan_distance(y_ratio_percent, up, down),
                )
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        proptest::prop_assume!(dx.abs() > 0.5 || dy.abs() > 0.5);

        app.pan_mermaid_viewer(dx, dy);
        let after_pan = render_mermaid_snapshot(&mut app, &mut renderer, layout);
        proptest::prop_assume!(after_pan != baseline);

        app.pan_mermaid_viewer(-dx, -dy);
        let round_trip = render_mermaid_snapshot(&mut app, &mut renderer, layout);

        proptest::prop_assert_eq!(round_trip, baseline);
    }

    #[test]
    fn mermaid_mr_pointer_zoom_keeps_anchor_stable(
        source in mermaid_anchorable_source_strategy(),
        width in 120u16..180,
        height in 28u16..56,
        anchor_pick in 0usize..8,
    ) {
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(&source, width, height);
        app.render(&mut renderer, layout);

        let (source_index, anchor_x, anchor_y) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                let content_rect = viewer.content_rect.expect("content rect");
                let eligible = viewer
                    .cached_semantic_lines
                    .iter()
                    .filter(|line| {
                        line.x > content_rect.x.saturating_add(1)
                            && line.y > content_rect.y
                            && line.x.saturating_add(display_width(&line.text))
                                < content_rect.right().saturating_sub(1)
                            && line.y < content_rect.bottom().saturating_sub(1)
                    })
                    .collect::<Vec<_>>();
                proptest::prop_assume!(!eligible.is_empty());
                let anchor = eligible[anchor_pick % eligible.len()];
                (
                    anchor.source_index,
                    anchor.x.saturating_add(display_width(&anchor.text) / 2),
                    anchor.y,
                )
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

        let content_rect = mermaid_content_rect_for_layout(layout);
        app.zoom_mermaid_viewer(
            MERMAID_SCROLL_ZOOM_STEP_PERCENT,
            Some((anchor_x, anchor_y)),
            content_rect,
        );
        app.render(&mut renderer, layout);

        let anchored_line = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.source_index == source_index),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        proptest::prop_assume!(anchored_line.is_some());
        let anchored_line = anchored_line.expect("anchored line");
        let anchored_center_x = anchored_line
            .x
            .saturating_add(display_width(&anchored_line.text) / 2);

        proptest::prop_assert!(
            (anchored_center_x as i32 - anchor_x as i32).abs() <= 2,
            "expected x anchor to stay stable: before={anchor_x}, after={}",
            anchored_center_x
        );
        proptest::prop_assert!(
            (anchored_line.y as i32 - anchor_y as i32).abs() <= 1,
            "expected y anchor to stay stable: before={anchor_y}, after={}",
            anchored_line.y
        );
    }
}
