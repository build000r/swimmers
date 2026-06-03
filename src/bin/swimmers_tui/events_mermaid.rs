use super::*;

pub(super) fn handle_mermaid_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> bool {
    let (content_rect, is_text_tab, has_tabs) = {
        let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
            return true;
        };
        (
            viewer
                .content_rect
                .unwrap_or_else(|| mermaid_content_rect(layout.overview_field)),
            viewer.active_tab != DomainPlanTab::Schema,
            viewer.plan_tabs.is_some(),
        )
    };

    if has_tabs {
        if let Some(handled) = handle_mermaid_tab_key(app, key) {
            return handled;
        }
    }

    if is_text_tab {
        return handle_mermaid_text_key(app, key);
    }

    let (step_x, step_y) = {
        let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
            return true;
        };
        mermaid_pan_step(viewer, content_rect)
    };

    handle_mermaid_diagram_key(app, content_rect, step_x, step_y, key)
}

fn handle_mermaid_tab_key<C: TuiApi>(app: &mut App<C>, key: KeyEvent) -> Option<bool> {
    match key.code {
        KeyCode::Char('[') => {
            app.cycle_plan_tab(-1);
            Some(true)
        }
        KeyCode::Char(']') => {
            app.cycle_plan_tab(1);
            Some(true)
        }
        KeyCode::Char(c @ '1'..='9') => {
            let idx = (c as usize) - ('1' as usize);
            if let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode {
                if let Some(tabs) = &viewer.plan_tabs {
                    if let Some(&tab) = tabs.get(idx) {
                        app.switch_plan_tab(tab);
                    }
                }
            }
            Some(true)
        }
        _ => None,
    }
}

fn handle_mermaid_text_key<C: TuiApi>(app: &mut App<C>, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => false,
        KeyCode::Esc => {
            app.close_mermaid_viewer();
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.scroll_plan_text(-1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.scroll_plan_text(1);
            true
        }
        KeyCode::PageUp => {
            app.scroll_plan_text_page(-1);
            true
        }
        KeyCode::PageDown => {
            app.scroll_plan_text_page(1);
            true
        }
        KeyCode::Home => {
            if let Some(viewer) = app.mermaid_viewer_mut() {
                viewer.plan_text_scroll = 0;
            }
            true
        }
        KeyCode::End => {
            if let Some(viewer) = app.mermaid_viewer_mut() {
                viewer.plan_text_scroll = viewer.plan_text_lines.len().saturating_sub(1);
            }
            true
        }
        KeyCode::Char('o') => {
            app.open_mermaid_artifact();
            true
        }
        _ => true,
    }
}

fn handle_mermaid_diagram_key<C: TuiApi>(
    app: &mut App<C>,
    content_rect: Rect,
    step_x: f32,
    step_y: f32,
    key: KeyEvent,
) -> bool {
    match key.code {
        KeyCode::Char('q') => false,
        KeyCode::Esc => {
            if !app.clear_mermaid_focus() {
                app.close_mermaid_viewer();
            }
            true
        }
        KeyCode::Tab => {
            app.focus_next_mermaid_target(content_rect);
            true
        }
        KeyCode::BackTab => {
            app.focus_previous_mermaid_target(content_rect);
            true
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.pan_mermaid_viewer(-step_x, 0.0);
            true
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.pan_mermaid_viewer(step_x, 0.0);
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.pan_mermaid_viewer(0.0, -step_y);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.pan_mermaid_viewer(0.0, step_y);
            true
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.zoom_mermaid_viewer(MERMAID_KEYBOARD_ZOOM_STEP_PERCENT, None, content_rect);
            true
        }
        KeyCode::Char('-') => {
            app.zoom_mermaid_viewer(-MERMAID_KEYBOARD_ZOOM_STEP_PERCENT, None, content_rect);
            true
        }
        KeyCode::Char('o') => {
            app.open_mermaid_artifact();
            true
        }
        KeyCode::Char('0') => {
            app.reset_mermaid_viewer_fit();
            true
        }
        _ => true,
    }
}
