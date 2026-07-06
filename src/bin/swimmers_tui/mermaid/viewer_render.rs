use super::*;

fn render_wrapped_lines(renderer: &mut Renderer, rect: Rect, text: &str, color: Color) {
    for (offset, line) in wrap_text(text, rect.width as usize).into_iter().enumerate() {
        let y = rect.y.saturating_add(offset as u16);
        if y >= rect.bottom() {
            break;
        }
        renderer.draw_text(
            rect.x,
            y,
            &truncate_label(&line, rect.width as usize),
            color,
        );
    }
}

fn render_plan_text_content(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &mut MermaidViewerState,
) {
    if viewer.plan_text_content.is_none() {
        render_wrapped_lines(
            renderer,
            content_rect,
            "loading artifact file...",
            Color::DarkGrey,
        );
        return;
    }

    let width = content_rect.width as usize;
    if width == 0 {
        return;
    }

    refresh_plan_text_lines_if_needed(viewer, content_rect.width);

    let visible_height = content_rect.height as usize;
    let total_lines = viewer.plan_text_lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    viewer.plan_text_scroll = viewer.plan_text_scroll.min(max_scroll);

    render_visible_plan_text_lines(renderer, content_rect, viewer, visible_height, width);
    render_plan_text_scroll_indicator(renderer, content_rect, viewer, visible_height, total_lines);
}

fn refresh_plan_text_lines_if_needed(viewer: &mut MermaidViewerState, width: u16) {
    if !viewer.plan_text_lines.is_empty() && viewer.plan_text_cached_width == width {
        return;
    }
    let Some(content) = viewer.plan_text_content.as_deref() else {
        return;
    };

    viewer.plan_text_lines = wrapped_plan_text_lines(content, width as usize);
    let original_rows = viewer.plan_text_lines.len();
    if mermaid_truncate_lines_with_marker(&mut viewer.plan_text_lines, MERMAID_RENDER_MAX_ROWS) {
        tracing::warn!(
            session_id = %viewer.session_id,
            rows = original_rows,
            cap_rows = MERMAID_RENDER_MAX_ROWS,
            "Mermaid plan text exceeded row cap; truncating"
        );
    }
    viewer.plan_text_cached_width = width;
}

fn wrapped_plan_text_lines(content: &str, width: usize) -> Vec<String> {
    content
        .lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![String::new()]
            } else {
                wrap_text(line, width)
            }
        })
        .collect()
}

fn render_visible_plan_text_lines(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &MermaidViewerState,
    visible_height: usize,
    width: usize,
) {
    for (offset, line) in viewer
        .plan_text_lines
        .iter()
        .skip(viewer.plan_text_scroll)
        .take(visible_height)
        .enumerate()
    {
        renderer.draw_text(
            content_rect.x,
            content_rect.y + offset as u16,
            &truncate_label(line, width),
            plan_text_line_color(line),
        );
    }
}

fn plan_text_line_color(line: &str) -> Color {
    let heading = usize::from(line.starts_with('#'));
    let list = usize::from(line.starts_with("- ")) + usize::from(line.starts_with("  - "));
    let table = usize::from(line.starts_with("| ")) + usize::from(line.starts_with("|-"));
    [Color::White, Color::DarkCyan, Color::Green, Color::Cyan]
        [heading * 3 + (1 - heading) * (list * 2 + (1 - list) * table)]
}

fn render_plan_text_scroll_indicator(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &MermaidViewerState,
    visible_height: usize,
    total_lines: usize,
) {
    if total_lines > visible_height {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let pct = plan_text_scroll_percent(viewer.plan_text_scroll, max_scroll);
        let indicator = format!("{}/{} ({}%)", viewer.plan_text_scroll + 1, total_lines, pct);
        let indicator_x = content_rect
            .right()
            .saturating_sub(display_width(&indicator));
        renderer.draw_text(
            indicator_x,
            content_rect.bottom().saturating_sub(1),
            &indicator,
            Color::DarkGrey,
        );
    }
}

fn plan_text_scroll_percent(scroll: usize, max_scroll: usize) -> usize {
    (scroll * 100).checked_div(max_scroll).unwrap_or(100)
}

fn render_mermaid_viewer_header(
    renderer: &mut Renderer,
    field: Rect,
    content_rect: Rect,
    viewer: &mut MermaidViewerState,
) {
    let after_back = render_mermaid_header_back_button(renderer, field, viewer);
    if render_mermaid_header_plan_tabs(renderer, field, viewer, after_back) {
        return;
    }
    render_mermaid_header_status(renderer, field, content_rect, viewer, after_back);
}

fn render_mermaid_header_back_button(
    renderer: &mut Renderer,
    field: Rect,
    viewer: &mut MermaidViewerState,
) -> u16 {
    viewer.back_rect = Some(Rect {
        x: field.x,
        y: field.y,
        width: display_width(MERMAID_BACK_LABEL),
        height: 1,
    });
    renderer.draw_text(field.x, field.y, MERMAID_BACK_LABEL, Color::Cyan);

    field
        .x
        .saturating_add(display_width(MERMAID_BACK_LABEL) + 1)
}

fn render_mermaid_header_plan_tabs(
    renderer: &mut Renderer,
    field: Rect,
    viewer: &mut MermaidViewerState,
    after_back: u16,
) -> bool {
    let Some(tabs) = viewer.plan_tabs.as_deref() else {
        return false;
    };

    viewer.tab_rects.clear();
    let tab_x = render_mermaid_header_plan_tab_labels(
        renderer,
        field,
        tabs,
        viewer.active_tab,
        &mut viewer.tab_rects,
        after_back,
    );
    render_mermaid_header_plan_tab_tmux_suffix(
        renderer,
        field,
        &viewer.tmux_name,
        &viewer.tmux_target,
        tab_x,
    );
    true
}

fn render_mermaid_header_plan_tab_labels(
    renderer: &mut Renderer,
    field: Rect,
    tabs: &[DomainPlanTab],
    active_tab: DomainPlanTab,
    tab_rects: &mut Vec<(DomainPlanTab, Rect)>,
    mut tab_x: u16,
) -> u16 {
    for &tab in tabs {
        let label = format!("[{}]", tab.label());
        let label_width = display_width(&label);
        if tab_x + label_width >= field.right() {
            break;
        }
        renderer.draw_text(
            tab_x,
            field.y,
            &label,
            mermaid_header_plan_tab_color(tab, active_tab),
        );
        tab_rects.push((
            tab,
            Rect {
                x: tab_x,
                y: field.y,
                width: label_width,
                height: 1,
            },
        ));
        tab_x = tab_x.saturating_add(label_width + 1);
    }
    tab_x
}

fn mermaid_header_plan_tab_color(tab: DomainPlanTab, active_tab: DomainPlanTab) -> Color {
    if tab == active_tab {
        Color::Cyan
    } else {
        Color::DarkGrey
    }
}

fn render_mermaid_header_plan_tab_tmux_suffix(
    renderer: &mut Renderer,
    field: Rect,
    tmux_name: &str,
    tmux_target: &swimmers::tmux_target::TmuxTarget,
    tab_x: u16,
) {
    let name_label = if tmux_target.is_default() {
        format!("| {tmux_name}")
    } else {
        format!("| {} {tmux_name}", tmux_target.display_label())
    };
    if tab_x + display_width(&name_label) < field.right() {
        renderer.draw_text(tab_x, field.y, &name_label, Color::DarkGrey);
    }
}

fn render_mermaid_header_status(
    renderer: &mut Renderer,
    field: Rect,
    content_rect: Rect,
    viewer: &MermaidViewerState,
    status_x: u16,
) {
    let status_width = field.right().saturating_sub(status_x) as usize;
    let status = mermaid_header_status_line(viewer, content_rect, status_width);
    renderer.draw_text(
        status_x,
        field.y,
        &truncate_label(&status, status_width),
        Color::DarkGrey,
    );
}

fn mermaid_header_status_line(
    viewer: &MermaidViewerState,
    content_rect: Rect,
    status_width: usize,
) -> String {
    let view_state = mermaid_view_state_for_view(viewer, content_rect);
    let detail_label = mermaid_status_detail_label(view_state);
    let zoom_label = mermaid_zoom_status_label(viewer.zoom);
    let mut status = format!(
        "{} | {} | {} | {} | o open",
        viewer.tmux_name,
        detail_label,
        shorten_path(
            viewer.display_path(),
            mermaid_header_status_path_budget(viewer, &detail_label, &zoom_label, status_width)
        ),
        zoom_label,
    );
    mermaid_header_append_focus_status(&mut status, viewer.focus_status.as_deref());
    status
}

fn mermaid_header_status_path_budget(
    viewer: &MermaidViewerState,
    detail_label: &str,
    zoom_label: &str,
    status_width: usize,
) -> usize {
    status_width.saturating_sub(mermaid_header_status_fixed_width(
        viewer,
        detail_label,
        zoom_label,
    ))
}

fn mermaid_header_status_fixed_width(
    viewer: &MermaidViewerState,
    detail_label: &str,
    zoom_label: &str,
) -> usize {
    usize::from(display_width(&viewer.tmux_name))
        + usize::from(display_width(" | "))
        + usize::from(display_width(detail_label))
        + usize::from(display_width(" | "))
        + usize::from(display_width(" | "))
        + usize::from(display_width(zoom_label))
        + usize::from(display_width(" | o open"))
        + mermaid_header_focus_status_width(viewer.focus_status.as_deref())
}

fn mermaid_header_focus_status_width(focus_status: Option<&str>) -> usize {
    focus_status
        .map(|status| usize::from(display_width(" | ")) + usize::from(display_width(status)))
        .unwrap_or(0)
}

fn mermaid_header_append_focus_status(status: &mut String, focus_status: Option<&str>) {
    if let Some(focus_status) = focus_status {
        status.push_str(" | ");
        status.push_str(focus_status);
    }
}

pub(super) fn render_mermaid_cached_background(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &MermaidViewerState,
) {
    if viewer.cached_background_cells.len() == viewer.cached_lines.len() {
        render_mermaid_cached_background_rows(
            renderer,
            content_rect,
            &viewer.cached_background_cells,
        );
        return;
    }

    render_mermaid_cached_line_fallback(renderer, content_rect, &viewer.cached_lines);
}

fn render_mermaid_cached_background_rows(
    renderer: &mut Renderer,
    content_rect: Rect,
    rows: &[Vec<Cell>],
) {
    for (row_offset, row) in rows.iter().enumerate() {
        let y = content_rect.y + row_offset as u16;
        if y >= content_rect.bottom() {
            break;
        }
        render_mermaid_cached_background_row(renderer, content_rect.x, y, row);
    }
}

fn render_mermaid_cached_background_row(
    renderer: &mut Renderer,
    content_x: u16,
    y: u16,
    row: &[Cell],
) {
    for (column_offset, cell) in row.iter().enumerate() {
        if cell.ch == ' ' {
            continue;
        }
        renderer.draw_char(content_x + column_offset as u16, y, cell.ch, cell.fg);
    }
}

fn render_mermaid_cached_line_fallback(
    renderer: &mut Renderer,
    content_rect: Rect,
    lines: &[String],
) {
    for (offset, line) in lines.iter().enumerate() {
        let y = content_rect.y + offset as u16;
        if y >= content_rect.bottom() {
            break;
        }
        renderer.draw_text(content_rect.x, y, line, MERMAID_CONNECTOR_COLOR);
    }
}

fn render_mermaid_cached_semantic_lines(renderer: &mut Renderer, viewer: &MermaidViewerState) {
    for line in &viewer.cached_semantic_lines {
        let color = if Some(line.source_index) == viewer.focused_source_index {
            MERMAID_FOCUS_COLOR
        } else {
            line.color
        };
        renderer.draw_text(line.x, line.y, &line.text, color);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MermaidViewerBodyState {
    PlanText,
    TooSmall,
    Unsupported(String),
    ArtifactError(String),
    Diagram,
}

pub(super) fn mermaid_viewer_body_state(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> MermaidViewerBodyState {
    if viewer.active_tab != DomainPlanTab::Schema {
        return MermaidViewerBodyState::PlanText;
    }
    if content_rect.width < MERMAID_VIEW_MIN_WIDTH || content_rect.height < MERMAID_VIEW_MIN_HEIGHT
    {
        return MermaidViewerBodyState::TooSmall;
    }
    if let Some(reason) = viewer.unsupported_reason.as_deref() {
        return MermaidViewerBodyState::Unsupported(reason.to_string());
    }
    if let Some(error) = viewer.artifact_error.as_deref() {
        return MermaidViewerBodyState::ArtifactError(error.to_string());
    }
    MermaidViewerBodyState::Diagram
}

fn render_mermaid_viewer_body(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &mut MermaidViewerState,
) {
    match mermaid_viewer_body_state(viewer, content_rect) {
        MermaidViewerBodyState::PlanText => {
            render_plan_text_content(renderer, content_rect, viewer)
        }
        MermaidViewerBodyState::TooSmall => {
            render_wrapped_lines(
                renderer,
                content_rect,
                "Mermaid view too small",
                Color::DarkGrey,
            );
        }
        MermaidViewerBodyState::Unsupported(reason) => {
            render_wrapped_lines(renderer, content_rect, &reason, Color::DarkGrey);
        }
        MermaidViewerBodyState::ArtifactError(error) => {
            render_wrapped_lines(renderer, content_rect, &error, Color::Red);
        }
        MermaidViewerBodyState::Diagram => {
            render_mermaid_diagram_body(renderer, content_rect, viewer);
        }
    }
}

fn render_mermaid_diagram_body(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &mut MermaidViewerState,
) {
    if render_mermaid_viewport_cache_error(renderer, content_rect, viewer) {
        return;
    }
    viewer.render_error = None;
    mermaid_apply_render_line_cap(viewer, content_rect);

    render_mermaid_cached_background(renderer, content_rect, viewer);
    render_mermaid_cached_semantic_lines(renderer, viewer);
}

fn render_mermaid_viewport_cache_error(
    renderer: &mut Renderer,
    content_rect: Rect,
    viewer: &mut MermaidViewerState,
) -> bool {
    let Err(err) = ensure_mermaid_viewport_cache(viewer, content_rect) else {
        return false;
    };

    tracing::warn!(
        session_id = %viewer.session_id,
        error = %err,
        "Mermaid viewport render failed; rendering wrapped error text"
    );
    viewer.render_error = Some(err);
    if let Some(error) = viewer.render_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
    }
    true
}

pub(crate) fn render_mermaid_viewer(
    renderer: &mut Renderer,
    field: Rect,
    viewer: &mut MermaidViewerState,
) {
    renderer.fill_rect(field, ' ', Color::Reset);

    let content_rect = mermaid_content_rect(field);
    viewer.content_rect = Some(content_rect);
    render_mermaid_viewer_header(renderer, field, content_rect, viewer);

    render_mermaid_viewer_body(renderer, content_rect, viewer);
}
