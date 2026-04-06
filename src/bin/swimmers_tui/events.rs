use super::*;

pub(crate) fn should_background_startup_refresh(client: &ApiClient) -> bool {
    client.targets_local_backend()
}

pub(crate) fn initialize_tui_app() -> Result<(App<ApiClient>, Renderer), Box<dyn std::error::Error>>
{
    let _ = dotenvy::dotenv();
    swimmers::env_bootstrap::bootstrap_provider_env_from_shell();

    let runtime = Runtime::new()?;
    let client = ApiClient::from_env().map_err(io::Error::other)?;
    let background_startup_refresh = should_background_startup_refresh(&client);
    if !background_startup_refresh {
        runtime
            .block_on(client.preflight_startup_access())
            .map_err(io::Error::other)?;
    }
    let startup_base_url = client.base_url.clone();
    let mut renderer = Renderer::new()?;
    renderer.init()?;

    let mut app = App::new(runtime, client);
    let initial_layout = app.layout_for_terminal(renderer.width(), renderer.height());
    if background_startup_refresh {
        app.set_message(format!("connecting to swimmers API at {startup_base_url}"));
        app.spawn_background_refresh(false);
    } else {
        app.refresh(initial_layout);
    }

    Ok((app, renderer))
}

pub(crate) fn prepare_frame<C: TuiApi>(
    app: &mut App<C>,
    renderer: &mut Renderer,
) -> WorkspaceLayout {
    let layout = app.layout_for_terminal(renderer.width(), renderer.height());
    if layout.split_divider.is_none() {
        app.stop_split_drag();
    }
    app.trim_thought_log(layout.thought_entry_capacity());
    app.poll_pending_selection_publication();
    app.poll_pending_interaction();
    app.poll_refresh(layout);
    if app.should_refresh() && app.pending_refresh.is_none() {
        app.spawn_background_refresh(false);
    }
    app.tick(layout.overview_field);
    app.render(renderer, layout);
    layout
}

pub(crate) fn handle_key_event<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> bool {
    if app.thought_config_editor.is_some() {
        app.handle_thought_config_key(key, layout);
        return true;
    }

    if app.initial_request.is_some() {
        app.handle_initial_request_key(key, layout.overview_field);
        return true;
    }

    if let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode {
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(layout.overview_field));
        let is_text_tab = viewer.active_tab != DomainPlanTab::Schema;
        let has_tabs = viewer.plan_tabs.is_some();

        // Tab navigation keys (available in both schema and text modes)
        if has_tabs {
            match key.code {
                KeyCode::Char('[') => {
                    app.cycle_plan_tab(-1);
                    return true;
                }
                KeyCode::Char(']') => {
                    app.cycle_plan_tab(1);
                    return true;
                }
                KeyCode::Char(c @ '1'..='7') => {
                    let idx = (c as usize) - ('1' as usize);
                    if let FishBowlMode::Mermaid(v) = &app.fish_bowl_mode {
                        if let Some(tabs) = &v.plan_tabs {
                            if let Some(&tab) = tabs.get(idx) {
                                app.switch_plan_tab(tab);
                                return true;
                            }
                        }
                    }
                    return true;
                }
                _ => {}
            }
        }

        // Text tab scrolling keys
        if is_text_tab {
            return match key.code {
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
            };
        }

        // Schema tab: original mermaid diagram keys
        let (step_x, step_y) = mermaid_pan_step(viewer, content_rect);
        return match key.code {
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
        };
    }

    match key.code {
        KeyCode::Char('q') => false,
        KeyCode::Esc => {
            if app.picker.is_some() {
                app.close_picker();
                true
            } else {
                false
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
            if app.picker.is_some() {
                app.picker_up();
            } else {
                app.move_selection(-1, layout.overview_field);
            }
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1, layout.overview_field);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1, layout.overview_field);
            true
        }
        KeyCode::Char('m') => {
            app.toggle_ghostty_mode();
            true
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char('o') => {
            if app.picker.is_some() {
                app.picker_activate_selection(layout.overview_field);
            } else {
                app.open_selected();
            }
            true
        }
        KeyCode::Char('e') => {
            app.picker_set_managed_only(true);
            true
        }
        KeyCode::Char('a') => {
            app.picker_set_managed_only(false);
            true
        }
        KeyCode::Char('r') => {
            if let Some((path, managed_only)) = app
                .picker
                .as_ref()
                .map(|picker| (picker.current_path.clone(), picker.managed_only))
            {
                app.picker_reload(Some(path), managed_only);
            } else {
                app.manual_refresh(layout);
            }
            true
        }
        KeyCode::Char('t') => {
            app.open_thought_config_editor();
            true
        }
        KeyCode::Char('n') => {
            app.toggle_native_app();
            true
        }
        _ => true,
    }
}

pub(crate) fn handle_mouse_down<C: TuiApi>(
    app: &mut App<C>,
    renderer: &Renderer,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) {
    if app.thought_config_editor.is_some() {
        return;
    }
    if app.initial_request.is_some() {
        return;
    }
    if handle_split_or_header_click(app, renderer.width(), layout, mouse) {
        return;
    }
    if app.handle_mermaid_mouse_down(layout.overview_field, mouse) {
        return;
    }
    handle_workspace_click(app, layout, mouse);
}

pub(crate) fn handle_split_or_header_click<C: TuiApi>(
    app: &mut App<C>,
    width: u16,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> bool {
    if layout
        .split_hitbox
        .map(|hitbox| hitbox.contains(mouse.column, mouse.row))
        .unwrap_or(false)
    {
        app.start_split_drag(layout, mouse.column);
        return true;
    }
    if header_filter_action_at(app, width, mouse.column, mouse.row).is_some() {
        app.handle_header_filter_click(width, mouse.column, mouse.row);
        return true;
    }
    if app
        .ghostty_mode_rect(width)
        .map(|rect| rect.contains(mouse.column, mouse.row))
        .unwrap_or(false)
    {
        app.toggle_ghostty_mode();
        return true;
    }
    if app
        .native_status_rect(width)
        .map(|rect| rect.contains(mouse.column, mouse.row))
        .unwrap_or(false)
    {
        app.toggle_native_app();
        return true;
    }
    false
}

pub(crate) fn handle_workspace_click<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) {
    if let Some(thought_box) = layout.thought_box {
        if thought_box.contains(mouse.column, mouse.row) {
            if let Some(thought_content) = layout.thought_content {
                app.handle_thought_click(
                    mouse.column,
                    mouse.row,
                    thought_content,
                    layout.thought_entry_capacity(),
                );
            }
            return;
        }
    }
    if layout.overview_field.contains(mouse.column, mouse.row) {
        app.handle_field_click(mouse.column, mouse.row, layout.overview_field);
    }
}

pub(crate) fn handle_tui_event<C: TuiApi>(
    app: &mut App<C>,
    renderer: &mut Renderer,
    layout: WorkspaceLayout,
    event: Event,
) -> io::Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            Ok(handle_key_event(app, layout, key))
        }
        Event::Paste(text) => {
            app.handle_paste(&text);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) => {
            handle_mouse_down(app, renderer, layout, mouse);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) => {
            if app.drag_split(layout, mouse.column) {
                return Ok(true);
            }
            if app.handle_mermaid_mouse_drag(layout.overview_field, mouse) {
                return Ok(true);
            }
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) => {
            app.stop_split_drag();
            app.handle_mermaid_mouse_up();
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::ScrollUp) => {
            let _ =
                app.handle_mermaid_scroll(layout.overview_field, mouse, MermaidZoomDirection::In);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::ScrollDown) => {
            let _ =
                app.handle_mermaid_scroll(layout.overview_field, mouse, MermaidZoomDirection::Out);
            Ok(true)
        }
        Event::Resize(width, height) => {
            app.stop_split_drag();
            renderer.manual_resize(width, height)?;
            Ok(true)
        }
        _ => Ok(true),
    }
}
