use super::*;

#[path = "events_mermaid.rs"]
mod events_mermaid;
#[path = "events_picker.rs"]
mod events_picker;

use events_mermaid::handle_mermaid_key;
use events_picker::{
    handle_picker_command_key, handle_picker_priority_key, handle_picker_search_key,
    handle_selection_key,
};

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
        include_unnumbered_sessions: bool,
        layout: AttentionGroupLayout,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>> {
        match self {
            Self::Embedded(client) => client.open_attention_group(
                max_sessions,
                current_session_ids,
                focus,
                include_unnumbered_sessions,
                layout,
            ),
            Self::External(client) => client.open_attention_group(
                max_sessions,
                current_session_ids,
                focus,
                include_unnumbered_sessions,
                layout,
            ),
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

pub(crate) fn build_embedded_client(
    runtime: &Runtime,
) -> (TuiClient, swimmers::startup::EmbeddedTuiShutdown) {
    // Mirror the standalone server's startup gate (main.rs `prepare_server_startup`):
    // load config WITH diagnostics, surface them, and refuse to start on any
    // config error or unsafe trust/bind pairing. `Config::from_env()` discards
    // these diagnostics, which would let a misconfigured auth setup (e.g.
    // AUTH_MODE=token without AUTH_TOKEN, an unknown AUTH_MODE, or a non-loopback
    // local_trust bind) silently run insecure under the embedded TUI. We enforce
    // BEFORE init_app_state_skeleton, exiting with the same EX_CONFIG code the
    // standalone server uses so the failure is indistinguishable to supervisors.
    let load = Config::from_env_report();
    swimmers::cli::print_config_diagnostics(&load.diagnostics);
    if let Err(msg) = swimmers::cli::enforce_startup_config(&load.config, &load.diagnostics) {
        eprintln!("swimmers: {msg}");
        std::process::exit(swimmers::cli::EXIT_CONFIG);
    }

    let config = Arc::new(load.config);
    let _guard = runtime.enter();
    let state = swimmers::startup::init_app_state_skeleton(config);
    let shutdown = swimmers::startup::spawn_deferred_init_for_embedded_tui(Arc::clone(&state));
    tracing::info!("embedded mode initialized with deferred startup");
    (
        TuiClient::Embedded(in_process::InProcessApi::new(state)),
        shutdown,
    )
}

pub(crate) fn initialize_tui_app() -> Result<(App<TuiClient>, Renderer), Box<dyn std::error::Error>>
{
    let _ = dotenvy::dotenv();
    swimmers::env_bootstrap::bootstrap_provider_env_from_shell();

    let runtime = Runtime::new()?;
    let (client, embedded_shutdown) = if external_mode_requested() {
        (build_external_client(&runtime)?, None)
    } else {
        let (client, shutdown) = build_embedded_client(&runtime);
        (client, Some(shutdown))
    };
    let mut renderer = Renderer::new()?;
    renderer.init()?;

    let mut app = App::new(runtime, client);
    if let Some(shutdown) = embedded_shutdown {
        app.set_embedded_shutdown(shutdown);
    }
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
    app.poll_pending_picker_repo_search();
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
    if is_embedded_ctrl_c_quit(app, key) {
        return false;
    }
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

fn is_embedded_ctrl_c_quit<C: TuiApi>(app: &App<C>, key: KeyEvent) -> bool {
    app.has_embedded_shutdown()
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn handle_modal_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    if app.show_help {
        app.show_help = false;
        if matches!(key.code, KeyCode::Char('q')) {
            return Some(false);
        }
        return Some(true);
    }

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
        KeyCode::Char('?') => {
            app.show_help = true;
            Some(true)
        }
        _ => None,
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
    let Some(action) = split_or_header_click_action(app, width, layout, mouse) else {
        return false;
    };
    dispatch_split_or_header_click(app, width, layout, mouse, action);
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SplitOrHeaderClickAction {
    SplitDrag,
    HeaderFilter,
    SpriteTheme,
    GhosttyMode,
    AttentionGroup,
    NativeStatus,
}

fn split_or_header_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    let hit_tests: [SplitOrHeaderHitTest<C>; 6] = [
        split_drag_click_action,
        header_filter_click_action,
        sprite_theme_click_action,
        ghostty_mode_click_action,
        attention_group_click_action,
        native_status_click_action,
    ];

    hit_tests
        .into_iter()
        .find_map(|hit_test| hit_test(app, width, layout, mouse))
}

type SplitOrHeaderHitTest<C> = fn(
    &App<C>,
    u16,
    WorkspaceLayout,
    crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction>;

fn split_drag_click_action<C: TuiApi>(
    _app: &App<C>,
    _width: u16,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    layout
        .split_hitbox
        .filter(|hitbox| hitbox.contains(mouse.column, mouse.row))
        .map(|_| SplitOrHeaderClickAction::SplitDrag)
}

fn header_filter_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    _layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    header_filter_action_at(app, width, mouse.column, mouse.row)
        .map(|_| SplitOrHeaderClickAction::HeaderFilter)
}

fn sprite_theme_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    _layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    app.sprite_theme_rect(width)
        .contains(mouse.column, mouse.row)
        .then_some(SplitOrHeaderClickAction::SpriteTheme)
}

fn ghostty_mode_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    _layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    app.ghostty_mode_rect(width)
        .filter(|rect| rect.contains(mouse.column, mouse.row))
        .map(|_| SplitOrHeaderClickAction::GhosttyMode)
}

fn attention_group_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    _layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    app.attention_group_rect(width)
        .filter(|rect| rect.contains(mouse.column, mouse.row))
        .map(|_| SplitOrHeaderClickAction::AttentionGroup)
}

fn native_status_click_action<C: TuiApi>(
    app: &App<C>,
    width: u16,
    _layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> Option<SplitOrHeaderClickAction> {
    app.native_status_rect(width)
        .filter(|rect| rect.contains(mouse.column, mouse.row))
        .map(|_| SplitOrHeaderClickAction::NativeStatus)
}

fn dispatch_split_or_header_click<C: TuiApi>(
    app: &mut App<C>,
    width: u16,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
    action: SplitOrHeaderClickAction,
) {
    match action {
        SplitOrHeaderClickAction::SplitDrag => {
            app.start_split_drag(layout, mouse.column);
        }
        SplitOrHeaderClickAction::HeaderFilter => {
            app.handle_header_filter_click(width, mouse.column, mouse.row);
        }
        SplitOrHeaderClickAction::SpriteTheme => {
            app.set_sprite_theme_from_click(mouse.column);
        }
        SplitOrHeaderClickAction::GhosttyMode => {
            app.toggle_ghostty_mode();
        }
        SplitOrHeaderClickAction::AttentionGroup => {
            app.open_attention_group();
        }
        SplitOrHeaderClickAction::NativeStatus => {
            app.toggle_native_app();
        }
    }
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
    dispatch_tui_event(app, renderer, layout, classify_tui_event(&event))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiEventAction<'a> {
    Key(KeyEvent),
    Paste(&'a str),
    Mouse(MouseEventAction),
    Resize(u16, u16),
    Ignore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MouseEventAction {
    LeftDown(crossterm::event::MouseEvent),
    LeftDrag(crossterm::event::MouseEvent),
    LeftUp,
    ScrollIn(crossterm::event::MouseEvent),
    ScrollOut(crossterm::event::MouseEvent),
    Ignore,
}

fn classify_tui_event(event: &Event) -> TuiEventAction<'_> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => TuiEventAction::Key(*key),
        Event::Paste(text) => TuiEventAction::Paste(text),
        Event::Mouse(mouse) => TuiEventAction::Mouse(classify_mouse_event(*mouse)),
        Event::Resize(width, height) => TuiEventAction::Resize(*width, *height),
        _ => TuiEventAction::Ignore,
    }
}

fn classify_mouse_event(mouse: crossterm::event::MouseEvent) -> MouseEventAction {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseEventAction::LeftDown(mouse),
        MouseEventKind::Drag(MouseButton::Left) => MouseEventAction::LeftDrag(mouse),
        MouseEventKind::Up(MouseButton::Left) => MouseEventAction::LeftUp,
        MouseEventKind::ScrollUp => MouseEventAction::ScrollIn(mouse),
        MouseEventKind::ScrollDown => MouseEventAction::ScrollOut(mouse),
        _ => MouseEventAction::Ignore,
    }
}

fn dispatch_tui_event<C: TuiApi>(
    app: &mut App<C>,
    renderer: &mut Renderer,
    layout: WorkspaceLayout,
    action: TuiEventAction<'_>,
) -> io::Result<bool> {
    match action {
        TuiEventAction::Key(key) => Ok(handle_key_event(app, layout, key)),
        TuiEventAction::Paste(text) => {
            app.handle_paste(text);
            Ok(true)
        }
        TuiEventAction::Mouse(mouse_action) => {
            dispatch_mouse_event(app, renderer, layout, mouse_action)
        }
        TuiEventAction::Resize(width, height) => {
            app.stop_split_drag();
            renderer.manual_resize(width, height)?;
            Ok(true)
        }
        TuiEventAction::Ignore => Ok(true),
    }
}

fn dispatch_mouse_event<C: TuiApi>(
    app: &mut App<C>,
    renderer: &Renderer,
    layout: WorkspaceLayout,
    action: MouseEventAction,
) -> io::Result<bool> {
    match action {
        MouseEventAction::LeftDown(mouse) => handle_left_mouse_down(app, renderer, layout, mouse),
        MouseEventAction::LeftDrag(mouse) => Ok(handle_left_mouse_drag(app, layout, mouse)),
        MouseEventAction::LeftUp => Ok(handle_left_mouse_up(app)),
        MouseEventAction::ScrollIn(mouse) => Ok(handle_mermaid_mouse_scroll(
            app,
            layout,
            mouse,
            MermaidZoomDirection::In,
        )),
        MouseEventAction::ScrollOut(mouse) => Ok(handle_mermaid_mouse_scroll(
            app,
            layout,
            mouse,
            MermaidZoomDirection::Out,
        )),
        MouseEventAction::Ignore => Ok(true),
    }
}

fn handle_left_mouse_down<C: TuiApi>(
    app: &mut App<C>,
    renderer: &Renderer,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> io::Result<bool> {
    handle_mouse_down(app, renderer, layout, mouse);
    Ok(true)
}

fn handle_left_mouse_drag<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> bool {
    if app.drag_split(layout, mouse.column) {
        return true;
    }
    if app.handle_mermaid_mouse_drag(layout.overview_field, mouse) {
        return true;
    }
    true
}

fn handle_left_mouse_up<C: TuiApi>(app: &mut App<C>) -> bool {
    app.stop_split_drag();
    app.handle_mermaid_mouse_up();
    true
}

fn handle_mermaid_mouse_scroll<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
    direction: MermaidZoomDirection,
) -> bool {
    let _ = app.handle_mermaid_scroll(layout.overview_field, mouse, direction);
    true
}
