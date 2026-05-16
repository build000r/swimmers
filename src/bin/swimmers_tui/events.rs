use super::*;

pub(crate) enum TuiClient {
    Embedded(in_process::InProcessApi),
    External(ApiClient),
}

impl TuiApi for TuiClient {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        match self {
            Self::Embedded(client) => client.fetch_sessions(),
            Self::External(client) => client.fetch_sessions(),
        }
    }

    fn fetch_sessions_for_initial_frame(
        &self,
    ) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        match self {
            Self::Embedded(client) => client.fetch_sessions_for_initial_frame(),
            Self::External(client) => client.fetch_sessions_for_initial_frame(),
        }
    }

    fn fetch_backend_health(&self) -> BoxFuture<'_, Result<BackendHealthResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_backend_health(),
            Self::External(client) => client.fetch_backend_health(),
        }
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_thought_config(),
            Self::External(client) => client.fetch_thought_config(),
        }
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        match self {
            Self::Embedded(client) => client.update_thought_config(config),
            Self::External(client) => client.update_thought_config(config),
        }
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        match self {
            Self::Embedded(client) => client.test_thought_config(config),
            Self::External(client) => client.test_thought_config(config),
        }
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        match self {
            Self::Embedded(client) => client.refresh_openrouter_candidates(),
            Self::External(client) => client.refresh_openrouter_candidates(),
        }
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_mermaid_artifact(session_id),
            Self::External(client) => client.fetch_mermaid_artifact(session_id),
        }
    }

    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_session_skills(session_id),
            Self::External(client) => client.fetch_session_skills(session_id),
        }
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_plan_file(session_id, name),
            Self::External(client) => client.fetch_plan_file(session_id, name),
        }
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        match self {
            Self::Embedded(client) => client.fetch_native_status(),
            Self::External(client) => client.fetch_native_status(),
        }
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        match self {
            Self::Embedded(client) => client.set_native_app(app),
            Self::External(client) => client.set_native_app(app),
        }
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        match self {
            Self::Embedded(client) => client.set_native_mode(mode),
            Self::External(client) => client.set_native_mode(mode),
        }
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        match self {
            Self::Embedded(client) => client.publish_selection(session_id),
            Self::External(client) => client.publish_selection(session_id),
        }
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        match self {
            Self::Embedded(client) => client.open_session(session_id),
            Self::External(client) => client.open_session(session_id),
        }
    }

    fn open_attention_group(
        &self,
        max_sessions: usize,
        current_session_ids: Vec<String>,
        focus: bool,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>> {
        match self {
            Self::Embedded(client) => {
                client.open_attention_group(max_sessions, current_session_ids, focus)
            }
            Self::External(client) => {
                client.open_attention_group(max_sessions, current_session_ids, focus)
            }
        }
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        match self {
            Self::Embedded(client) => client.list_dirs(path, managed_only, group),
            Self::External(client) => client.list_dirs(path, managed_only, group),
        }
    }

    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>> {
        match self {
            Self::Embedded(client) => client.list_repo_dirs(),
            Self::External(client) => client.list_repo_dirs(),
        }
    }

    fn update_dir_group_memberships(
        &self,
        path: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> BoxFuture<'_, Result<DirGroupMembershipUpdateResponse, String>> {
        match self {
            Self::Embedded(client) => client.update_dir_group_memberships(path, add, remove),
            Self::External(client) => client.update_dir_group_memberships(path, add, remove),
        }
    }

    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>> {
        match self {
            Self::Embedded(client) => client.start_repo_action(path, kind),
            Self::External(client) => client.start_repo_action(path, kind),
        }
    }

    fn fetch_overlay_plans(&self) -> BoxFuture<'_, Result<Vec<PlanPanelEntry>, String>> {
        match self {
            Self::Embedded(client) => client.fetch_overlay_plans(),
            Self::External(client) => client.fetch_overlay_plans(),
        }
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        match self {
            Self::Embedded(client) => {
                client.create_session(cwd, spawn_tool, launch_target, initial_request)
            }
            Self::External(client) => {
                client.create_session(cwd, spawn_tool, launch_target, initial_request)
            }
        }
    }

    fn adopt_session(
        &self,
        tmux_name: &str,
        session_id: Option<&str>,
    ) -> BoxFuture<'_, Result<AdoptSessionResponse, String>> {
        match self {
            Self::Embedded(client) => client.adopt_session(tmux_name, session_id),
            Self::External(client) => client.adopt_session(tmux_name, session_id),
        }
    }

    fn create_sessions_batch(
        &self,
        dirs: Vec<String>,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionsBatchResponse, String>> {
        match self {
            Self::Embedded(client) => {
                client.create_sessions_batch(dirs, spawn_tool, launch_target, initial_request)
            }
            Self::External(client) => {
                client.create_sessions_batch(dirs, spawn_tool, launch_target, initial_request)
            }
        }
    }

    fn send_group_input(
        &self,
        session_ids: Vec<String>,
        text: String,
    ) -> BoxFuture<'_, Result<SessionGroupInputResponse, String>> {
        match self {
            Self::Embedded(client) => client.send_group_input(session_ids, text),
            Self::External(client) => client.send_group_input(session_ids, text),
        }
    }
}

fn external_mode_requested() -> bool {
    std::env::var_os("SWIMMERS_TUI_URL").is_some()
}

fn is_loopback_base_url(base_url: &str) -> Result<bool, String> {
    let parsed = reqwest::Url::parse(base_url)
        .map_err(|err| format!("invalid SWIMMERS_TUI_URL `{base_url}`: {err}"))?;

    Ok(match parsed.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false),
        None => false,
    })
}

fn build_external_client(runtime: &Runtime) -> Result<TuiClient, io::Error> {
    let client = ApiClient::from_env().map_err(io::Error::other)?;
    if is_loopback_base_url(&client.base_url).map_err(io::Error::other)? {
        let handle = runtime
            .block_on(lifecycle::ensure_server(
                &client.base_url,
                Duration::from_secs(10),
            ))
            .map_err(io::Error::other)?;
        tracing::info!(?handle, url = %client.base_url, "external mode backend ready");
    } else {
        tracing::info!(url = %client.base_url, "external mode with remote backend URL");
    }
    runtime
        .block_on(client.preflight_startup_access())
        .map_err(io::Error::other)?;
    Ok(TuiClient::External(client))
}

pub(crate) fn build_embedded_client(runtime: &Runtime) -> (TuiClient, tokio::task::JoinHandle<()>) {
    let config = Arc::new(Config::from_env());
    let _guard = runtime.enter();
    let state = swimmers::startup::init_app_state_skeleton(config);
    let deferred_init = swimmers::startup::spawn_deferred_init(Arc::clone(&state));
    tracing::info!("embedded mode initialized with deferred startup");
    (
        TuiClient::Embedded(in_process::InProcessApi::new(state)),
        deferred_init,
    )
}

pub(crate) fn initialize_tui_app() -> Result<(App<TuiClient>, Renderer), Box<dyn std::error::Error>>
{
    let _ = dotenvy::dotenv();
    swimmers::env_bootstrap::bootstrap_provider_env_from_shell();

    let runtime = Runtime::new()?;
    let client = if external_mode_requested() {
        build_external_client(&runtime)?
    } else {
        let (client, _deferred_init) = build_embedded_client(&runtime);
        client
    };
    let mut renderer = Renderer::new()?;
    renderer.init()?;

    let mut app = App::new(runtime, client);
    let initial_layout = app.layout_for_terminal(renderer.width(), renderer.height());
    app.refresh_initial_frame(initial_layout);

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
    app.maybe_refresh_picker();
    app.maybe_refresh_plans();
    if app.should_refresh() && app.pending_refresh.is_none() {
        app.spawn_background_refresh(false);
    }
    let tank_field = build_skill_panel(app, layout.overview_field).tank_field;
    app.tick(tank_field);
    app.render(renderer, layout);
    layout
}

pub(crate) fn handle_key_event<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> bool {
    if let Some(handled) = handle_modal_key(app, layout, key) {
        return handled;
    }
    if let Some(handled) = handle_picker_priority_key(app, layout, key) {
        return handled;
    }
    if let Some(handled) = handle_picker_search_key(app, key) {
        return handled;
    }
    handle_workspace_key(app, layout, key)
}

fn handle_modal_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    if app.thought_config_editor.is_some() {
        app.handle_thought_config_key(key, layout);
        return Some(true);
    }

    if app.initial_request.is_some() {
        app.handle_initial_request_key(key, layout.overview_field);
        return Some(true);
    }

    if matches!(app.fish_bowl_mode, FishBowlMode::Mermaid(_)) {
        return Some(handle_mermaid_key(app, layout, key));
    }

    None
}

fn handle_picker_priority_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    app.picker.as_ref()?;

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('B')) {
        app.open_batch_initial_request_for_visible_entries();
        return Some(true);
    }

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('X')) {
        app.handle_picker_action(PickerAction::ToggleBatchExcludeMode, layout.overview_field);
        return Some(true);
    }

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('G')) {
        app.picker_cycle_group_edit_target();
        return Some(true);
    }

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('+') | KeyCode::Char('=')) {
        app.picker_add_selected_to_group_target();
        return Some(true);
    }

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('-')) {
        app.picker_remove_selected_from_group_target();
        return Some(true);
    }

    if app.picker.is_some() && matches!(key.code, KeyCode::Char('M')) {
        app.picker_move_selected_to_group_target();
        return Some(true);
    }

    if app
        .picker
        .as_ref()
        .map(|picker| picker.batch_exclude_mode)
        .unwrap_or(false)
        && matches!(key.code, KeyCode::Char(' '))
    {
        if let Some(index) = app
            .picker
            .as_ref()
            .and_then(|picker| match picker.selection {
                PickerSelection::Entry(index) => Some(index),
                PickerSelection::SpawnHere => None,
            })
        {
            app.handle_picker_action(
                PickerAction::ToggleBatchExclude(index),
                layout.overview_field,
            );
        }
        return Some(true);
    }

    None
}

fn handle_picker_search_key<C: TuiApi>(app: &mut App<C>, key: KeyEvent) -> Option<bool> {
    if app.picker.is_some() {
        let no_mods = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        if no_mods {
            if let KeyCode::Char(c) = key.code {
                if !c.is_control() {
                    app.picker_search_push(c);
                    return Some(true);
                }
            }
        }
    }

    None
}

fn handle_workspace_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> bool {
    if let Some(handled) = handle_quit_or_escape_key(app, key) {
        return handled;
    }
    if let Some(handled) = handle_selection_key(app, layout, key) {
        return handled;
    }
    if let Some(handled) = handle_picker_command_key(app, layout, key) {
        return handled;
    }
    if let Some(handled) = handle_global_toggle_key(app, layout, key) {
        return handled;
    }
    true
}

fn handle_quit_or_escape_key<C: TuiApi>(app: &mut App<C>, key: KeyEvent) -> Option<bool> {
    match key.code {
        KeyCode::Char('q') => Some(false),
        KeyCode::Esc => {
            if app.picker_search_clear() {
                return Some(true);
            }
            if app.picker.is_some() {
                app.close_picker();
                Some(true)
            } else {
                Some(false)
            }
        }
        _ => None,
    }
}

fn handle_selection_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    match key.code {
        KeyCode::Backspace => {
            if app.picker_search_pop() {
                return Some(true);
            }
            if app.picker.is_some() {
                app.picker_up();
            } else {
                app.move_selection(-1, layout.overview_field);
            }
            Some(true)
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if app.picker.is_some() {
                app.picker_up();
            } else {
                app.move_selection(-1, layout.overview_field);
            }
            Some(true)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1, layout.overview_field);
            Some(true)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1, layout.overview_field);
            Some(true)
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char('o') => {
            if app.picker.is_some() {
                app.picker_activate_selection(layout.overview_field);
            } else {
                app.open_selected();
            }
            Some(true)
        }
        _ => None,
    }
}

fn handle_picker_command_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    match key.code {
        KeyCode::Char('e') => {
            app.picker_set_managed_only(true);
            Some(true)
        }
        KeyCode::Char('a') => {
            app.picker_set_managed_only(false);
            Some(true)
        }
        KeyCode::Char('c') if app.picker.is_some() => {
            app.picker_start_action_for_selection(RepoActionKind::Commit);
            Some(true)
        }
        KeyCode::Char('R') if app.picker.is_some() => {
            app.picker_start_action_for_selection(RepoActionKind::Restart);
            Some(true)
        }
        KeyCode::Char('O') if app.picker.is_some() => {
            app.picker_open_url_for_selection();
            Some(true)
        }
        KeyCode::Char('r') => {
            if let Some((path, managed_only, group)) = app.picker.as_ref().map(|picker| {
                (
                    picker.current_path.clone(),
                    picker.managed_only,
                    picker.current_group.clone(),
                )
            }) {
                app.picker_reload(Some(path), managed_only, group);
            } else {
                app.manual_refresh(layout);
            }
            Some(true)
        }
        _ => None,
    }
}

fn handle_global_toggle_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    match key.code {
        KeyCode::Char('m') => {
            app.toggle_ghostty_mode();
            Some(true)
        }
        KeyCode::Tab => {
            app.toggle_thought_group_by();
            Some(true)
        }
        KeyCode::Char('>') => {
            app.toggle_thought_show_all();
            Some(true)
        }
        KeyCode::Char('t') => {
            app.open_thought_config_editor();
            Some(true)
        }
        KeyCode::Char('n') => {
            app.toggle_native_app();
            Some(true)
        }
        KeyCode::Char('A') => {
            app.reattach_selected_tmux_session(layout.overview_field);
            Some(true)
        }
        KeyCode::Char('s') => {
            app.toggle_sprite_theme();
            Some(true)
        }
        _ => None,
    }
}

fn handle_mermaid_key<C: TuiApi>(app: &mut App<C>, layout: WorkspaceLayout, key: KeyEvent) -> bool {
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
        .sprite_theme_rect(width)
        .contains(mouse.column, mouse.row)
    {
        app.set_sprite_theme_from_click(mouse.column);
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
        .attention_group_rect(width)
        .map(|rect| rect.contains(mouse.column, mouse.row))
        .unwrap_or(false)
    {
        app.open_attention_group();
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
