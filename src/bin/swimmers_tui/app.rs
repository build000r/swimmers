use super::*;
use swimmers::openrouter_models::should_rotate_openrouter_model;

// Once the school is large, the O(n^2) pairwise collision pass gets expensive.
// 50 entities is where we start seeing frame-time spikes on laptops, so we cap
// checks per entity above that point and distribute checks across frames.
const COLLISION_THROTTLE_ENTITY_THRESHOLD: usize = 50;
const COLLISION_THROTTLE_PAIR_BUDGET: usize = 24;
const API_FAILURE_BANNER_THRESHOLD: u8 = 3;
const API_STALE_BANNER_TEXT: &str = "API disconnected - showing stale data";
const ATTENTION_GROUP_SESSION_ID: &str = "attention-group";
const ATTENTION_GROUP_TMUX_NAME: &str = "swimmers-attention";
const ATTENTION_GROUP_LABEL: &str = "[attention group]";
const ATTENTION_GROUP_MAX_SESSIONS: usize = 6;
const ATTENTION_GROUP_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

fn sprite_theme_option_width(theme: Option<SpriteTheme>) -> u16 {
    let label = format!("[{}]", SpriteTheme::override_label(theme));
    display_width(&label)
}

fn attention_group_session_is_eligible(session: &SessionSummary) -> bool {
    session.session_id != ATTENTION_GROUP_SESSION_ID
        && session.tmux_name != ATTENTION_GROUP_TMUX_NAME
        && swimmers::api::remote_sessions::split_remote_session_id(&session.session_id).is_none()
        && session_ready_for_operator_group_input(session)
}

pub(crate) struct RefreshResult {
    pub(crate) sessions: Result<Vec<SessionSummary>, String>,
    pub(crate) mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
    pub(crate) session_skills: Vec<(String, Result<SessionSkillListResponse, String>)>,
    pub(crate) native_status: Option<Result<NativeDesktopStatusResponse, String>>,
    pub(crate) daemon_defaults_status: Option<DaemonDefaultsStatus>,
    pub(crate) show_success_message: bool,
    pub(crate) force_asset_refresh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MermaidCacheContext {
    tmux_name: String,
    cwd: String,
}

impl MermaidCacheContext {
    fn from_session(session: &SessionSummary) -> Self {
        Self {
            tmux_name: session.tmux_name.clone(),
            cwd: normalize_path(&session.cwd),
        }
    }
}

#[derive(Clone, Debug)]
struct MermaidCacheEntry {
    context: MermaidCacheContext,
    artifact: Option<MermaidArtifactResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepoThemeCacheContext {
    cwd: String,
    repo_theme_id: Option<String>,
}

impl RepoThemeCacheContext {
    fn from_session(session: &SessionSummary) -> Self {
        Self {
            cwd: normalize_path(&session.cwd),
            repo_theme_id: session.repo_theme_id.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct RepoThemeCacheEntry {
    context: RepoThemeCacheContext,
    resolved: Option<(String, RepoTheme)>,
}

pub(crate) struct PendingSelectionPublicationResult {
    pub(crate) session_id: Option<String>,
    pub(crate) response: Result<(), String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ApiRefreshHealth {
    consecutive_errors: u8,
}

impl ApiRefreshHealth {
    fn record_success(&mut self) {
        self.consecutive_errors = 0;
    }

    fn record_failure(&mut self) {
        self.consecutive_errors = self.consecutive_errors.saturating_add(1);
    }

    fn banner_text(&self) -> Option<&'static str> {
        (self.consecutive_errors >= API_FAILURE_BANNER_THRESHOLD).then_some(API_STALE_BANNER_TEXT)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DaemonDefaultsStatus {
    #[default]
    Unknown,
    Available,
    Unavailable,
}

impl DaemonDefaultsStatus {
    fn from_defaults(defaults: Option<&DaemonDefaults>) -> Self {
        if defaults.is_some() {
            Self::Available
        } else {
            Self::Unavailable
        }
    }

    pub(crate) fn is_unavailable(self) -> bool {
        self == Self::Unavailable
    }
}

pub(crate) struct ThoughtConfigActionOutcome {
    pub(crate) message: String,
    pub(crate) updated_config: Option<ThoughtConfig>,
    pub(crate) openrouter_candidates: Option<Vec<String>>,
    pub(crate) close_editor: bool,
    pub(crate) refresh_sessions: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GroupInputTargets {
    pub(crate) session_ids: Vec<String>,
    pub(crate) label: String,
}

#[derive(Clone, Copy)]
enum PickerGroupUpdateMode {
    Add,
    Remove,
    Move,
}

pub(crate) enum PendingInteractionResult {
    OpenPicker {
        x: u16,
        y: u16,
        response: Result<DirListResponse, String>,
        repo_search: Result<DirRepoSearchResponse, String>,
    },
    ReloadPicker {
        managed_only: bool,
        group: Option<String>,
        preserve_selection: bool,
        response: Result<DirListResponse, String>,
    },
    StartRepoAction {
        repo_label: String,
        kind: RepoActionKind,
        reload_path: String,
        managed_only: bool,
        response: Result<DirRepoActionResponse, String>,
    },
    CreateSession {
        field: Rect,
        response: Result<CreateSessionResponse, String>,
    },
    CreateSessionsBatch {
        field: Rect,
        response: Result<CreateSessionsBatchResponse, String>,
    },
    SendGroupInput {
        response: Result<SessionGroupInputResponse, String>,
    },
    OpenSession {
        label: String,
        response: Result<NativeDesktopOpenResponse, String>,
    },
    OpenAttentionGroup {
        focus: bool,
        response: Result<NativeAttentionGroupOpenResponse, String>,
    },
    ToggleNativeApp {
        next_app: NativeDesktopApp,
        response: Result<NativeDesktopStatusResponse, String>,
    },
    ToggleGhosttyMode {
        next_mode: GhosttyOpenMode,
        response: Result<NativeDesktopStatusResponse, String>,
    },
    OpenThoughtConfig {
        response: Result<ThoughtConfigResponse, String>,
    },
    TestThoughtConfig {
        outcome: ThoughtConfigActionOutcome,
    },
    SaveThoughtConfig {
        outcome: ThoughtConfigActionOutcome,
    },
    VoiceTranscription {
        generation: u64,
        response: Result<String, String>,
    },
}

pub(crate) struct App<C: TuiApi> {
    pub(crate) runtime: Runtime,
    pub(crate) client: Arc<C>,
    pub(crate) artifact_opener: Arc<dyn ArtifactOpener>,
    pub(crate) commit_launcher: Arc<dyn CommitLauncher>,
    pub(crate) entities: Vec<SessionEntity>,
    pub(crate) thought_log: Vec<ThoughtLogEntry>,
    pub(crate) thought_filter: ThoughtFilter,
    pub(crate) thought_group_by: ThoughtGroupBy,
    pub(crate) thought_show_all: bool,
    pub(crate) last_logged_thoughts: HashMap<String, ThoughtFingerprint>,
    session_mermaid_cache: HashMap<String, MermaidCacheEntry>,
    pub(crate) session_skill_cache: HashMap<String, SkillCacheEntry>,
    session_repo_theme_cache: HashMap<String, RepoThemeCacheEntry>,
    pub(crate) mermaid_artifacts: HashMap<String, MermaidArtifactResponse>,
    pub(crate) repo_themes: HashMap<String, RepoTheme>,
    pub(crate) selected_id: Option<String>,
    pub(crate) published_selected_id: Option<String>,
    pub(crate) native_status: Option<NativeDesktopStatusResponse>,
    pub(crate) attention_group_session_ids: Vec<String>,
    pub(crate) last_attention_group_refresh: Option<Instant>,
    pub(crate) attention_group_refresh_armed: bool,
    pub(crate) daemon_defaults_status: DaemonDefaultsStatus,
    pub(crate) thought_config_editor: Option<ThoughtConfigEditorState>,
    pub(crate) picker: Option<PickerState>,
    pub(crate) spawn_tool: SpawnTool,
    pub(crate) launch_target: Option<String>,
    pub(crate) initial_request: Option<InitialRequestState>,
    pub(crate) group_input_targets: Option<GroupInputTargets>,
    pub(crate) initial_request_generation: u64,
    pub(crate) voice_state: VoiceUiState,
    pub(crate) voice_recording: Option<VoiceRecording>,
    pub(crate) fish_bowl_mode: FishBowlMode,
    pub(crate) sprite_theme_override: Option<SpriteTheme>,
    pub(crate) mermaid_drag: Option<MermaidDragState>,
    pub(crate) message: Option<(String, Instant)>,
    pub(crate) last_refresh: Option<Instant>,
    pub(crate) last_successful_refresh: Option<Instant>,
    api_refresh_health: ApiRefreshHealth,
    pub(crate) last_picker_refresh: Option<Instant>,
    pub(crate) cached_plans: Vec<PlanPanelEntry>,
    pub(crate) last_plans_refresh: Option<Instant>,
    pub(crate) thought_panel_ratio: f32,
    pub(crate) split_drag_active: bool,
    pub(crate) tick: u64,
    pub(crate) pending_refresh: Option<oneshot::Receiver<RefreshResult>>,
    pub(crate) pending_interaction: Option<oneshot::Receiver<PendingInteractionResult>>,
    pub(crate) pending_selection_publication:
        Option<oneshot::Receiver<PendingSelectionPublicationResult>>,
    pub(crate) queued_selection_publication: Option<(Option<String>, bool)>,
}

impl<C: TuiApi> App<C> {
    pub(crate) fn new(runtime: Runtime, client: C) -> Self {
        Self::with_helpers(
            runtime,
            client,
            Arc::new(SystemArtifactOpener),
            Arc::new(SystemCommitLauncher),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn with_artifact_opener(
        runtime: Runtime,
        client: C,
        artifact_opener: Arc<dyn ArtifactOpener>,
    ) -> Self {
        Self::with_helpers(
            runtime,
            client,
            artifact_opener,
            Arc::new(SystemCommitLauncher),
        )
    }

    pub(crate) fn with_helpers(
        runtime: Runtime,
        client: C,
        artifact_opener: Arc<dyn ArtifactOpener>,
        commit_launcher: Arc<dyn CommitLauncher>,
    ) -> Self {
        Self {
            runtime,
            client: Arc::new(client),
            artifact_opener,
            commit_launcher,
            entities: Vec::new(),
            thought_log: Vec::new(),
            thought_filter: ThoughtFilter::default(),
            thought_group_by: ThoughtGroupBy::default(),
            thought_show_all: false,
            last_logged_thoughts: HashMap::new(),
            session_mermaid_cache: HashMap::new(),
            session_skill_cache: HashMap::new(),
            session_repo_theme_cache: HashMap::new(),
            mermaid_artifacts: HashMap::new(),
            repo_themes: HashMap::new(),
            selected_id: None,
            published_selected_id: None,
            native_status: None,
            attention_group_session_ids: Vec::new(),
            last_attention_group_refresh: None,
            attention_group_refresh_armed: false,
            daemon_defaults_status: DaemonDefaultsStatus::Unknown,
            thought_config_editor: None,
            picker: None,
            spawn_tool: SpawnTool::Codex,
            launch_target: None,
            initial_request: None,
            group_input_targets: None,
            initial_request_generation: 0,
            voice_state: default_ui_state(),
            voice_recording: None,
            fish_bowl_mode: FishBowlMode::Aquarium,
            sprite_theme_override: None,
            mermaid_drag: None,
            message: None,
            last_refresh: None,
            last_successful_refresh: None,
            api_refresh_health: ApiRefreshHealth::default(),
            last_picker_refresh: None,
            cached_plans: Vec::new(),
            last_plans_refresh: None,
            thought_panel_ratio: THOUGHT_RAIL_DEFAULT_RATIO,
            split_drag_active: false,
            tick: 0,
            pending_refresh: None,
            pending_interaction: None,
            pending_selection_publication: None,
            queued_selection_publication: None,
        }
    }

    pub(crate) fn layout_for_terminal(&self, width: u16, height: u16) -> WorkspaceLayout {
        if thought_panel_needs_input(self) {
            WorkspaceLayout::for_terminal_with_ratio(width, height, self.thought_panel_ratio)
        } else {
            WorkspaceLayout::for_terminal_without_thought_panel(width, height)
        }
    }

    pub(crate) fn set_message(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self
            .message
            .as_ref()
            .map(|(existing, _)| existing == &message)
            .unwrap_or(false)
        {
            return;
        }
        self.message = Some((message, Instant::now()));
    }

    pub(crate) fn visible_message(&self) -> Option<&str> {
        self.message.as_ref().and_then(|(message, at)| {
            if at.elapsed() <= MESSAGE_TTL {
                Some(message.as_str())
            } else {
                None
            }
        })
    }

    /// Return a terse status-bar message for a refresh transport error.
    /// For the first `BACKEND_OFFLINE_ESCALATION` of downtime the user sees
    /// only "backend offline"; after that the full diagnostic is shown so
    /// they know how to recover.
    fn refresh_error_message(&self, verbose: String) -> String {
        let dominated = self
            .last_successful_refresh
            .map(|t| t.elapsed() < BACKEND_OFFLINE_ESCALATION)
            .unwrap_or(false);
        if dominated {
            "backend offline".to_string()
        } else {
            verbose
        }
    }

    pub(crate) fn should_refresh(&self) -> bool {
        self.last_refresh
            .map(|last| last.elapsed() >= REFRESH_INTERVAL)
            .unwrap_or(true)
    }

    pub(crate) fn should_refresh_picker(&self) -> bool {
        self.picker.is_some()
            && self.pending_interaction.is_none()
            && self.initial_request.is_none()
            && self
                .last_picker_refresh
                .map(|last| last.elapsed() >= REFRESH_INTERVAL)
                .unwrap_or(true)
    }

    pub(crate) fn maybe_refresh_picker(&mut self) {
        if !self.should_refresh_picker() {
            return;
        }

        let Some((path, managed_only, group)) = self.picker.as_ref().map(|picker| {
            (
                picker.current_path.clone(),
                picker.managed_only,
                picker.current_group.clone(),
            )
        }) else {
            return;
        };

        self.picker_reload_with_options(Some(path), managed_only, group, false, true);
    }

    pub(crate) fn maybe_refresh_plans(&mut self) {
        // Plan mtimes rarely change — 5s debounce is plenty.
        const PLANS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
        // Skip plans scanning on the very first frame: walking the skillbox
        // overlay tree can add ~20–40ms on a warm machine, which blows the
        // embedded-mode first-frame perf gate. Sessions/thoughts drive the
        // initial render anyway; plans appear on the next tick.
        if self.tick == 0 {
            return;
        }
        let due = self
            .last_plans_refresh
            .map(|last| last.elapsed() >= PLANS_REFRESH_INTERVAL)
            .unwrap_or(true);
        if !due {
            return;
        }
        self.last_plans_refresh = Some(Instant::now());
        match self.runtime.block_on(self.client.fetch_overlay_plans()) {
            Ok(plans) => {
                self.cached_plans = plans;
            }
            Err(err) => self.set_message(err),
        }
    }

    pub(crate) fn native_status_text(&self) -> String {
        match &self.native_status {
            Some(status) => {
                let app_label = status.app.as_deref().unwrap_or("available");
                let mode_suffix = status
                    .ghostty_mode
                    .filter(|_| self.current_native_app() == NativeDesktopApp::Ghostty)
                    .map(|mode| format!(" ({})", mode.display_label()))
                    .unwrap_or_default();
                if status.supported {
                    format!("terminal handoff: {app_label}{mode_suffix}")
                } else {
                    format!(
                        "terminal handoff: {app_label}{mode_suffix} unavailable: {}",
                        status.reason.as_deref().unwrap_or("unknown reason")
                    )
                }
            }
            None => "terminal handoff: checking".to_string(),
        }
    }

    pub(crate) fn header_right_text(&self) -> String {
        let native_text = self
            .thought_filter
            .tmux_name
            .as_deref()
            .map(|tmux_name| format!("num={tmux_name} | {}", self.native_status_text()))
            .unwrap_or_else(|| self.native_status_text());
        format!("{native_text} {ATTENTION_GROUP_LABEL}")
    }

    pub(crate) fn sprite_theme_rect(&self, _width: u16) -> Rect {
        let width = SpriteTheme::override_options()
            .iter()
            .fold(0u16, |acc, theme| {
                acc.saturating_add(sprite_theme_option_width(*theme))
            });
        let gaps = SpriteTheme::override_options().len().saturating_sub(1) as u16;
        Rect {
            x: SPRITE_THEME_TOGGLE_X,
            y: 1,
            width: width.saturating_add(gaps),
            height: 1,
        }
    }

    pub(crate) fn toggle_sprite_theme(&mut self) {
        let themes = SpriteTheme::override_options();
        let current = themes
            .iter()
            .position(|theme| *theme == self.sprite_theme_override)
            .unwrap_or(0);
        self.sprite_theme_override = themes[(current + 1) % themes.len()];
        self.set_message(format!(
            "sprite theme: {}",
            SpriteTheme::override_label(self.sprite_theme_override)
        ));
    }

    pub(crate) fn set_sprite_theme_from_click(&mut self, x: u16) {
        if let Some(theme) = self.sprite_theme_at_x(x) {
            self.set_sprite_theme_override(theme);
        }
    }

    fn sprite_theme_at_x(&self, x: u16) -> Option<Option<SpriteTheme>> {
        let rect = self.sprite_theme_rect(0);
        let mut cursor = rect.x;
        for theme in SpriteTheme::override_options() {
            let width = sprite_theme_option_width(theme);
            if x >= cursor && x < cursor.saturating_add(width) {
                return Some(theme);
            }
            cursor = cursor.saturating_add(width + 1);
        }
        None
    }

    fn set_sprite_theme_override(&mut self, theme: Option<SpriteTheme>) {
        if self.sprite_theme_override == theme {
            return;
        }
        self.sprite_theme_override = theme;
        self.set_message(format!(
            "sprite theme: {}",
            SpriteTheme::override_label(self.sprite_theme_override)
        ));
    }

    pub(crate) fn effective_sprite_theme_for_session(
        &self,
        session: &SessionSummary,
    ) -> SpriteTheme {
        self.sprite_theme_override
            .or_else(|| {
                let theme_id = session.repo_theme_id.as_ref()?;
                let repo_theme = self.repo_themes.get(theme_id)?;
                SpriteTheme::from_repo_theme(repo_theme)
            })
            .unwrap_or_default()
    }

    pub(crate) fn uses_balls_scene(&self, visible_entities: &[&SessionEntity]) -> bool {
        match self.sprite_theme_override {
            Some(SpriteTheme::Balls) => true,
            Some(_) => false,
            None => {
                !visible_entities.is_empty()
                    && visible_entities.iter().all(|entity| {
                        self.effective_sprite_theme_for_session(&entity.session)
                            == SpriteTheme::Balls
                    })
            }
        }
    }

    pub(crate) fn native_status_rect(&self, width: u16) -> Option<Rect> {
        let max_right_width = width.saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let text_width = right_text
            .find(ATTENTION_GROUP_LABEL)
            .map(|idx| display_width(right_text[..idx].trim_end()))
            .unwrap_or_else(|| display_width(&right_text));
        if text_width == 0 {
            return None;
        }

        let full_width = display_width(&right_text);
        Some(Rect {
            x: width.saturating_sub(full_width).saturating_sub(2),
            y: 1,
            width: text_width,
            height: 1,
        })
    }

    pub(crate) fn attention_group_rect(&self, width: u16) -> Option<Rect> {
        let max_right_width = width.saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let marker_idx = right_text.find(ATTENTION_GROUP_LABEL)?;
        let full_width = display_width(&right_text);
        let x = width
            .saturating_sub(full_width)
            .saturating_sub(2)
            .saturating_add(display_width(&right_text[..marker_idx]));
        Some(Rect {
            x,
            y: 1,
            width: display_width(ATTENTION_GROUP_LABEL),
            height: 1,
        })
    }

    pub(crate) fn ghostty_mode_rect(&self, width: u16) -> Option<Rect> {
        if self.current_native_app() != NativeDesktopApp::Ghostty {
            return None;
        }

        let max_right_width = width.saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let marker = format!("({})", self.current_ghostty_mode().display_label());
        let marker_idx = right_text.find(&marker)?;
        let prefix_width = display_width(&right_text[..marker_idx]);
        let marker_width = display_width(&marker);
        let full_width = display_width(&right_text);
        let x = width
            .saturating_sub(full_width)
            .saturating_sub(2)
            .saturating_add(prefix_width);

        Some(Rect {
            x,
            y: 1,
            width: marker_width,
            height: 1,
        })
    }

    fn current_native_app(&self) -> NativeDesktopApp {
        self.native_status
            .as_ref()
            .and_then(|status| {
                status
                    .app_id
                    .or_else(|| status.app.as_deref().map(NativeDesktopApp::from_env_value))
            })
            .unwrap_or(NativeDesktopApp::Iterm)
    }

    fn current_ghostty_mode(&self) -> GhosttyOpenMode {
        self.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode)
            .unwrap_or(GhosttyOpenMode::Swap)
    }

    pub(crate) fn toggle_native_app(&mut self) {
        let next_app = self.current_native_app().toggle();
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!(
            "switching terminal handoff target to {}...",
            next_app.display_name()
        ));
        self.runtime.spawn(async move {
            let response = client.set_native_app(next_app).await;
            let _ = tx.send(PendingInteractionResult::ToggleNativeApp { next_app, response });
        });
    }

    pub(crate) fn toggle_ghostty_mode(&mut self) {
        if self.current_native_app() != NativeDesktopApp::Ghostty {
            return;
        }

        let next_mode = self.current_ghostty_mode().toggle();
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!(
            "switching Ghostty placement to {}...",
            next_mode.display_label()
        ));
        self.runtime.spawn(async move {
            let response = client.set_native_mode(next_mode).await;
            let _ = tx.send(PendingInteractionResult::ToggleGhosttyMode {
                next_mode,
                response,
            });
        });
    }

    pub(crate) fn open_attention_group(&mut self) {
        self.request_attention_group(Vec::new(), true, true);
    }

    fn refresh_attention_group(&mut self) {
        let current_session_ids = self.attention_group_session_ids.clone();
        if current_session_ids.is_empty() {
            return;
        }
        self.request_attention_group(current_session_ids, false, false);
    }

    fn request_attention_group(
        &mut self,
        current_session_ids: Vec<String>,
        focus: bool,
        show_progress: bool,
    ) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        if show_progress {
            self.set_message("opening attention group...");
        }
        self.last_attention_group_refresh = Some(Instant::now());
        self.runtime.spawn(async move {
            let response = client
                .open_attention_group(ATTENTION_GROUP_MAX_SESSIONS, current_session_ids, focus)
                .await;
            let _ = tx.send(PendingInteractionResult::OpenAttentionGroup { focus, response });
        });
    }

    pub(crate) fn visible_entities(&self) -> Vec<&SessionEntity> {
        self.entities
            .iter()
            .filter(|entity| self.thought_filter.matches_session(&entity.session))
            .collect()
    }

    fn begin_pending_interaction(&mut self) -> Option<oneshot::Sender<PendingInteractionResult>> {
        if self.pending_interaction.is_some() {
            self.set_message("wait for the current action to finish");
            return None;
        }

        let (tx, rx) = oneshot::channel();
        self.pending_interaction = Some(rx);
        Some(tx)
    }

    pub(crate) fn publish_selection(&mut self, session_id: Option<String>, force: bool) {
        if !force && session_id == self.published_selected_id {
            return;
        }

        match self
            .runtime
            .block_on(self.client.publish_selection(session_id.as_deref()))
        {
            Ok(()) => {
                self.published_selected_id = session_id;
            }
            Err(err) => self.set_message(err),
        }
    }

    pub(crate) fn sync_selection_publication(&mut self) {
        self.queue_selection_publication(self.selected_id.clone(), false);
    }

    pub(crate) fn clear_published_selection(&mut self) {
        self.publish_selection(None, true);
    }

    fn queue_selection_publication(&mut self, session_id: Option<String>, force: bool) {
        self.queued_selection_publication = Some((session_id, force));
        self.maybe_spawn_selection_publication();
    }

    fn maybe_spawn_selection_publication(&mut self) {
        if self.pending_selection_publication.is_some() {
            return;
        }

        let Some((session_id, force)) = self.queued_selection_publication.take() else {
            return;
        };
        if !force && session_id == self.published_selected_id {
            return;
        }

        let (tx, rx) = oneshot::channel();
        self.pending_selection_publication = Some(rx);
        let client = Arc::clone(&self.client);
        self.runtime.spawn(async move {
            let response = client.publish_selection(session_id.as_deref()).await;
            let _ = tx.send(PendingSelectionPublicationResult {
                session_id,
                response,
            });
        });
    }

    pub(crate) fn poll_pending_selection_publication(&mut self) {
        let Some(rx) = &mut self.pending_selection_publication else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_selection_publication = None;
                self.maybe_spawn_selection_publication();
                return;
            }
        };

        self.pending_selection_publication = None;
        match result.response {
            Ok(()) => {
                self.published_selected_id = result.session_id;
            }
            Err(err) => self.set_message(err),
        }
        self.maybe_spawn_selection_publication();
    }

    pub(crate) fn reconcile_selection(&mut self) {
        let selected_visible = self
            .selected_id
            .as_ref()
            .map(|selected| {
                self.entities.iter().any(|entity| {
                    entity.session.session_id == *selected
                        && self.thought_filter.matches_session(&entity.session)
                })
            })
            .unwrap_or(false);

        if !selected_visible {
            self.selected_id = self
                .entities
                .iter()
                .find(|entity| self.thought_filter.matches_session(&entity.session))
                .map(|entity| entity.session.session_id.clone());
        }
    }

    pub(crate) fn trim_thought_log(&mut self, capacity: usize) {
        if capacity == 0 || self.thought_log.len() <= capacity {
            return;
        }

        let drop_count = self.thought_log.len() - capacity;
        self.thought_log.drain(0..drop_count);
    }

    pub(crate) fn upsert_thought_log_entries(
        &mut self,
        entries: impl IntoIterator<Item = ThoughtLogEntry>,
        capacity: usize,
    ) {
        for entry in entries {
            if let Some(index) = self
                .thought_log
                .iter()
                .position(|existing| existing.session_id == entry.session_id)
            {
                self.thought_log.remove(index);
            }
            self.thought_log.push(entry);
        }
        self.thought_log.sort_by(compare_thought_log_entries);
        self.trim_thought_log(capacity);
    }

    #[allow(dead_code)]
    pub(crate) fn visible_thought_entries(&self, capacity: usize) -> Vec<&ThoughtLogEntry> {
        if capacity == 0 {
            return Vec::new();
        }

        let filtered = self
            .thought_log
            .iter()
            .filter(|entry| self.thought_filter.matches(entry))
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(capacity);
        filtered[start..].to_vec()
    }

    pub(crate) fn thought_entry_display_color(&self, entry: &ThoughtLogEntry) -> Color {
        self.entities
            .iter()
            .find(|entity| entity.session.session_id == entry.session_id)
            .map(|entity| session_display_color(&entity.session, &self.repo_themes))
            .unwrap_or(entry.color)
    }

    pub(crate) fn header_repo_summaries(&self) -> Vec<ThoughtRepoSummary> {
        let mut grouped = HashMap::<String, ThoughtRepoSummary>::new();
        for (index, entity) in self.entities.iter().enumerate() {
            let session = &entity.session;
            let Some(label) = path_tail_label(&session.cwd) else {
                continue;
            };
            let cwd = normalize_path(&session.cwd);
            let color = session_display_color(session, &self.repo_themes);

            let summary = grouped
                .entry(cwd.clone())
                .or_insert_with(|| ThoughtRepoSummary {
                    cwd: cwd.clone(),
                    label,
                    count: 0,
                    color,
                    last_seen: index,
                });
            summary.count += 1;
            summary.color = color;
            summary.last_seen = index;
        }

        let mut summaries = grouped.into_values().collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_seen
                .cmp(&left.last_seen)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.cwd.cmp(&right.cwd))
        });
        summaries
    }

    #[allow(dead_code)]
    pub(crate) fn active_thought_filter_text(&self) -> String {
        if !self.thought_filter.is_active() {
            return "filter: none".to_string();
        }

        let mut parts = Vec::new();
        if let Some(cwd) = self.thought_filter.cwd.as_deref() {
            parts.push(format!(
                "pwd={}",
                path_tail_label(cwd).unwrap_or_else(|| cwd.to_string())
            ));
        }
        if !self.thought_filter.excluded_cwds.is_empty() {
            let mut hidden = self
                .thought_filter
                .excluded_cwds
                .iter()
                .map(|cwd| path_tail_label(cwd).unwrap_or_else(|| cwd.to_string()))
                .collect::<Vec<_>>();
            hidden.sort();
            parts.push(format!("hide={}", hidden.join(",")));
        }
        if let Some(tmux_name) = self.thought_filter.tmux_name.as_deref() {
            parts.push(format!("num={tmux_name}"));
        }
        format!("filter: {}", parts.join(", "))
    }

    pub(crate) fn set_thought_filter_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = Some(cwd);
        self.thought_filter.excluded_cwds.clear();
        self.thought_filter.filter_out_mode = false;
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_mode(&mut self) {
        self.thought_filter.filter_out_mode = !self.thought_filter.filter_out_mode;
        if self.thought_filter.filter_out_mode {
            self.thought_filter.cwd = None;
        } else {
            self.thought_filter.excluded_cwds.clear();
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = None;
        self.thought_filter.filter_out_mode = true;
        if !self.thought_filter.excluded_cwds.insert(cwd.clone()) {
            self.thought_filter.excluded_cwds.remove(&cwd);
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_group_by(&mut self) {
        self.thought_group_by = self.thought_group_by.toggled();
        self.set_message(format!(
            "thought rail grouped by {}",
            self.thought_group_by.label()
        ));
    }

    pub(crate) fn toggle_thought_show_all(&mut self) {
        self.thought_show_all = !self.thought_show_all;
        let mode = if self.thought_show_all {
            "showing all agents"
        } else {
            "showing asleep agents"
        };
        self.set_message(format!("clawgs rail {mode}"));
    }

    pub(crate) fn clear_thought_filters(&mut self) {
        self.thought_filter.clear();
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    fn active_session_ids(sessions: &[SessionSummary]) -> HashSet<&str> {
        sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect()
    }

    fn retain_cached_assets(&mut self, sessions: &[SessionSummary]) {
        let active_session_ids = Self::active_session_ids(sessions);
        self.session_mermaid_cache
            .retain(|session_id, _| active_session_ids.contains(session_id.as_str()));
        self.session_skill_cache
            .retain(|session_id, _| active_session_ids.contains(session_id.as_str()));
        self.session_repo_theme_cache
            .retain(|session_id, _| active_session_ids.contains(session_id.as_str()));
    }

    fn refresh_mermaid_viewer_from_cache(&mut self) {
        if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
            if let Some(artifact) = self.mermaid_artifacts.get(&viewer.session_id) {
                let path_changed = viewer.path != artifact.path;
                let source_changed = viewer.source != artifact.source;
                let error_changed = viewer.artifact_error != artifact.error;
                viewer.path = artifact.path.clone();
                viewer.source = artifact.source.clone();
                viewer.artifact_error = artifact.error.clone();
                viewer.render_error = None;
                if source_changed || error_changed {
                    viewer.invalidate_source_cache();
                } else if path_changed {
                    viewer.invalidate_viewport_cache();
                }
            }
        }
    }

    fn rebuild_mermaid_artifacts_from_cache(&mut self) {
        self.mermaid_artifacts = self
            .session_mermaid_cache
            .iter()
            .filter_map(|(session_id, entry)| {
                entry
                    .artifact
                    .clone()
                    .map(|artifact| (session_id.clone(), artifact))
            })
            .collect();
        self.refresh_mermaid_viewer_from_cache();
    }

    fn rebuild_repo_themes_from_cache(&mut self) {
        let mut next = HashMap::new();
        for entry in self.session_repo_theme_cache.values() {
            if let Some((theme_id, theme)) = &entry.resolved {
                next.insert(theme_id.clone(), theme.clone());
            }
        }
        self.repo_themes = next;
    }

    fn should_refresh_mermaid_with_contexts(
        cached_contexts: &HashMap<String, MermaidCacheContext>,
        session: &SessionSummary,
        force: bool,
    ) -> bool {
        if force {
            return true;
        }

        let context = MermaidCacheContext::from_session(session);
        cached_contexts
            .get(&session.session_id)
            .map(|cached| cached != &context)
            .unwrap_or(true)
    }

    fn should_refresh_skills_with_contexts(
        cached_contexts: &HashMap<String, SkillCacheContext>,
        session: &SessionSummary,
        force: bool,
    ) -> bool {
        if force {
            return true;
        }

        let context = SkillCacheContext::from_session(session);
        cached_contexts
            .get(&session.session_id)
            .map(|cached| cached != &context)
            .unwrap_or(true)
    }

    fn apply_mermaid_artifact_result(
        &mut self,
        session: &SessionSummary,
        result: Result<MermaidArtifactResponse, String>,
    ) {
        let context = MermaidCacheContext::from_session(session);
        let previous = self.session_mermaid_cache.get(&session.session_id).cloned();
        let preserve_cached = previous
            .as_ref()
            .map(|entry| entry.context == context)
            .unwrap_or(false);

        let artifact = match result {
            Ok(artifact) if artifact.available => Some(artifact),
            Ok(_) => None,
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                if preserve_cached {
                    previous.and_then(|entry| entry.artifact)
                } else {
                    None
                }
            }
        };

        self.session_mermaid_cache.insert(
            session.session_id.clone(),
            MermaidCacheEntry { context, artifact },
        );
    }

    fn apply_session_skill_result(
        &mut self,
        session: &SessionSummary,
        result: Result<SessionSkillListResponse, String>,
    ) {
        let context = SkillCacheContext::from_session(session);
        let previous = self.session_skill_cache.get(&session.session_id).cloned();
        let preserve_cached = previous
            .as_ref()
            .map(|entry| entry.context == context)
            .unwrap_or(false);

        let response = match result {
            Ok(response) => Some(response),
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                if preserve_cached {
                    previous.and_then(|entry| entry.response)
                } else {
                    None
                }
            }
        };

        self.session_skill_cache.insert(
            session.session_id.clone(),
            SkillCacheEntry { context, response },
        );
    }

    fn refresh_single_mermaid_artifact(&mut self, session: &SessionSummary, force: bool) {
        let cached_contexts = self
            .session_mermaid_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        if !Self::should_refresh_mermaid_with_contexts(&cached_contexts, session, force) {
            return;
        }

        let result = self
            .runtime
            .block_on(self.client.fetch_mermaid_artifact(&session.session_id));
        self.apply_mermaid_artifact_result(session, result);
        self.rebuild_mermaid_artifacts_from_cache();
    }

    pub(crate) fn refresh_mermaid_artifacts(&mut self, sessions: &[SessionSummary]) {
        self.retain_cached_assets(sessions);
        // Fan out the per-session artifact fetches concurrently. The previous
        // implementation `block_on`'d each session in sequence, so initial
        // frame paint scaled as `N * fetch_mermaid_artifact_timeout`. With ~16
        // sessions and a 5s per-call ceiling that pushed first paint past 30s,
        // long enough that the TUI looked hung on the `Launching TUI` line.
        // `spawn_background_refresh_with_policy` already uses this same
        // `join_all` shape; we mirror it here so the initial frame matches.
        let pending: Vec<&SessionSummary> = sessions
            .iter()
            .filter(|session| {
                let context = MermaidCacheContext::from_session(session);
                self.session_mermaid_cache
                    .get(&session.session_id)
                    .map(|entry| entry.context != context)
                    .unwrap_or(true)
            })
            .collect();

        if !pending.is_empty() {
            let client = &self.client;
            let results = self.runtime.block_on(async {
                futures::future::join_all(
                    pending
                        .iter()
                        .map(|session| client.fetch_mermaid_artifact(&session.session_id)),
                )
                .await
            });
            for (session, result) in pending.iter().zip(results) {
                self.apply_mermaid_artifact_result(session, result);
            }
        }
        self.rebuild_mermaid_artifacts_from_cache();
    }

    pub(crate) fn reconcile_thought_log_sessions(&mut self, sessions: &[SessionSummary]) {
        let session_by_id = sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();

        self.thought_log
            .retain(|entry| session_by_id.contains_key(entry.session_id.as_str()));
        self.last_logged_thoughts
            .retain(|session_id, _| session_by_id.contains_key(session_id.as_str()));

        for entry in &mut self.thought_log {
            let Some(session) = session_by_id.get(entry.session_id.as_str()) else {
                continue;
            };
            entry.tmux_name = session.tmux_name.clone();
            entry.cwd = normalize_path(&session.cwd);
            entry.pwd_label = path_tail_label(&session.cwd);
            entry.batch = session.batch.clone();
            entry.state = session.state;
            entry.current_command = session.current_command.clone();
            entry.tool = session.tool.clone();
            entry.rest_state = session.rest_state;
            entry.color = session_display_color(session, &self.repo_themes);
            entry.is_stale = session.is_stale;
            entry.transport_health = session.transport_health;
            entry.commit_candidate = session.commit_candidate;
        }

        self.thought_log.sort_by(compare_thought_log_entries);
    }

    pub(crate) fn capture_thought_updates(
        &mut self,
        sessions: &[SessionSummary],
        thought_capacity: usize,
    ) {
        let mut pending = Vec::new();
        for session in sessions {
            let Some(thought) = normalize_thought_text(session.thought.as_deref()) else {
                continue;
            };

            let incoming = ThoughtFingerprint {
                thought: thought.clone(),
                updated_at: session.thought_updated_at,
            };
            if !should_append_thought(
                self.last_logged_thoughts.get(&session.session_id),
                &incoming,
            ) {
                continue;
            }

            self.last_logged_thoughts
                .insert(session.session_id.clone(), incoming);
            pending.push(ThoughtLogEntry::from_session(
                session,
                thought,
                &self.repo_themes,
            ));
        }

        pending.sort_by(compare_thought_log_entries);

        if !pending.is_empty() {
            self.upsert_thought_log_entries(pending, thought_capacity);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn refresh(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback(layout, false);
    }

    pub(crate) fn refresh_initial_frame(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback_with_mode(layout, false, true);
    }

    pub(crate) fn manual_refresh(&mut self, _layout: WorkspaceLayout) {
        self.pending_refresh = None;
        self.spawn_background_refresh_with_policy(true, true);
    }

    #[allow(dead_code)]
    pub(crate) fn refresh_with_feedback(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) {
        self.refresh_with_feedback_with_mode(layout, show_success_message, false);
    }

    fn refresh_with_feedback_with_mode(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
        initial_frame: bool,
    ) {
        let sessions_result = if initial_frame {
            self.runtime
                .block_on(self.client.fetch_sessions_for_initial_frame())
        } else {
            self.runtime.block_on(self.client.fetch_sessions())
        };

        let sessions_ok = match sessions_result {
            Ok(sessions) => {
                self.sync_repo_themes(&sessions, false);
                self.refresh_mermaid_artifacts(&sessions);
                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);
                self.last_successful_refresh = Some(Instant::now());
                self.api_refresh_health.record_success();
                if show_success_message {
                    let count = self.entities.len();
                    self.set_message(format!("refreshed {count} session{}", pluralize(count)));
                }
                self.refresh_daemon_defaults_status_once();
                true
            }
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                self.api_refresh_health.record_failure();
                false
            }
        };

        if sessions_ok {
            match self.runtime.block_on(self.client.fetch_native_status()) {
                Ok(status) => {
                    self.native_status = Some(status);
                }
                Err(err) => {
                    self.set_message(self.refresh_error_message(err));
                }
            }
        }

        self.last_refresh = Some(Instant::now());
    }

    fn refresh_daemon_defaults_status_once(&mut self) {
        if self.daemon_defaults_status != DaemonDefaultsStatus::Unknown {
            return;
        }
        if let Ok(response) = self.runtime.block_on(self.client.fetch_thought_config()) {
            self.daemon_defaults_status =
                DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref());
        }
    }

    pub(crate) fn spawn_background_refresh(&mut self, show_success_message: bool) {
        self.spawn_background_refresh_with_policy(show_success_message, false);
    }

    fn spawn_background_refresh_with_policy(
        &mut self,
        show_success_message: bool,
        force_asset_refresh: bool,
    ) {
        let client = Arc::clone(&self.client);
        let check_daemon_defaults = self.daemon_defaults_status == DaemonDefaultsStatus::Unknown;
        let mermaid_contexts = self
            .session_mermaid_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        let skill_contexts = self
            .session_skill_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        let (tx, rx) = oneshot::channel();
        self.pending_refresh = Some(rx);
        self.runtime.spawn(async move {
            let sessions_result = client.fetch_sessions().await;

            let (mermaid_artifacts, session_skills, native_status) = match &sessions_result {
                Ok(sessions) => {
                    let mermaid_futs: Vec<_> = sessions
                        .iter()
                        .filter(|session| {
                            Self::should_refresh_mermaid_with_contexts(
                                &mermaid_contexts,
                                session,
                                force_asset_refresh,
                            )
                        })
                        .map(|s| {
                            let client = Arc::clone(&client);
                            let sid = s.session_id.clone();
                            async move {
                                let result = client.fetch_mermaid_artifact(&sid).await;
                                (sid, result)
                            }
                        })
                        .collect();

                    let mut skill_groups = BTreeMap::<String, (String, Vec<String>)>::new();
                    for session in sessions.iter().filter(|session| {
                        Self::should_refresh_skills_with_contexts(
                            &skill_contexts,
                            session,
                            force_asset_refresh,
                        )
                    }) {
                        let cwd = normalize_path(&session.cwd);
                        let entry = skill_groups
                            .entry(cwd)
                            .or_insert_with(|| (session.session_id.clone(), Vec::new()));
                        entry.1.push(session.session_id.clone());
                    }
                    let skill_futs: Vec<_> = skill_groups
                        .into_values()
                        .map(|(representative_id, session_ids)| {
                            let client = Arc::clone(&client);
                            async move {
                                let result = client.fetch_session_skills(&representative_id).await;
                                let mut out = Vec::new();
                                for session_id in session_ids {
                                    let adjusted = result.clone().map(|mut response| {
                                        response.session_id = session_id.clone();
                                        response
                                    });
                                    out.push((session_id, adjusted));
                                }
                                out
                            }
                        })
                        .collect();

                    let (mermaid_results, skill_results, native_result) = tokio::join!(
                        futures::future::join_all(mermaid_futs),
                        futures::future::join_all(skill_futs),
                        client.fetch_native_status(),
                    );

                    (
                        mermaid_results,
                        skill_results.into_iter().flatten().collect(),
                        Some(native_result),
                    )
                }
                Err(_) => (Vec::new(), Vec::new(), None),
            };

            let daemon_defaults_status = if check_daemon_defaults {
                client.fetch_thought_config().await.ok().map(|response| {
                    DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref())
                })
            } else {
                None
            };

            let _ = tx.send(RefreshResult {
                sessions: sessions_result,
                mermaid_artifacts,
                session_skills,
                native_status,
                daemon_defaults_status,
                show_success_message,
                force_asset_refresh,
            });
        });
    }

    pub(crate) fn poll_pending_interaction(&mut self) {
        let Some(rx) = &mut self.pending_interaction else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_interaction = None;
                return;
            }
        };

        self.pending_interaction = None;
        self.apply_pending_interaction_result(result);
    }

    fn apply_pending_interaction_result(&mut self, result: PendingInteractionResult) {
        match result {
            PendingInteractionResult::OpenPicker {
                x,
                y,
                response,
                repo_search,
            } => match response {
                Ok(response) => {
                    let mut picker = PickerState::new(
                        x,
                        y,
                        response,
                        true,
                        self.spawn_tool,
                        self.launch_target.clone(),
                    );
                    let repo_search_error = match repo_search {
                        Ok(response) => {
                            picker.set_repo_search_entries(response.entries);
                            None
                        }
                        Err(err) => Some(err),
                    };
                    self.launch_target = picker.launch_target.clone();
                    picker.sync_theme_colors(&mut self.repo_themes);
                    self.picker = Some(picker);
                    self.last_picker_refresh = Some(Instant::now());
                    if let Some(err) = repo_search_error {
                        self.set_message(format!("repository search unavailable: {err}"));
                    }
                }
                Err(err) => {
                    self.set_message(err);
                    self.picker = None;
                    self.last_picker_refresh = None;
                }
            },
            PendingInteractionResult::ReloadPicker {
                managed_only,
                group,
                preserve_selection,
                response,
            } => match response {
                Ok(response) => {
                    if let Some(picker) = &mut self.picker {
                        picker.managed_only = managed_only;
                        picker.current_group = group;
                        picker.apply_response(response, preserve_selection);
                        self.launch_target = picker.launch_target.clone();
                        picker.sync_theme_colors(&mut self.repo_themes);
                        self.last_picker_refresh = Some(Instant::now());
                    }
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::StartRepoAction {
                repo_label,
                kind,
                reload_path,
                managed_only,
                response,
            } => match response {
                Ok(_) => {
                    self.set_message(format!("{} started for {repo_label}", kind_label(kind)));
                    let group = self.picker.as_ref().and_then(|p| p.current_group.clone());
                    self.picker_reload_with_options(
                        Some(reload_path),
                        managed_only,
                        group,
                        false,
                        true,
                    );
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::CreateSession { field, response } => match response {
                Ok(response) => {
                    let repo_theme = response.repo_theme.clone();
                    let session = response.session;
                    let session_id = session.session_id.clone();
                    let tmux_name = session.tmux_name.clone();
                    self.remember_repo_theme(&session, repo_theme);
                    self.upsert_session(session, field);
                    self.selected_id = Some(session_id);
                    self.reconcile_selection();
                    self.sync_selection_publication();
                    self.close_picker();
                    self.set_message(format!("created {tmux_name}"));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::CreateSessionsBatch { field, response } => match response {
                Ok(response) => self.apply_batch_create_response(field, response),
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::SendGroupInput { response } => match response {
                Ok(response) => self.apply_group_input_response(response),
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::OpenSession { label, response } => match response {
                Ok(response) => {
                    self.set_message(format!("{} {}", response.status, label));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::OpenAttentionGroup { focus, response } => match response {
                Ok(response) => {
                    let previous = self.attention_group_session_ids.clone();
                    self.attention_group_session_ids = response.session_ids.clone();
                    self.last_attention_group_refresh = Some(Instant::now());
                    self.attention_group_refresh_armed = false;
                    if focus || previous != response.session_ids {
                        self.set_message(format!(
                            "{} attention group: {} sessions",
                            response.status, response.session_count
                        ));
                    }
                }
                Err(err) => {
                    self.attention_group_refresh_armed = false;
                    if !focus {
                        self.attention_group_session_ids.clear();
                    }
                    self.set_message(err);
                }
            },
            PendingInteractionResult::ToggleNativeApp { next_app, response } => match response {
                Ok(status) => {
                    self.native_status = Some(status.clone());
                    self.set_message(Self::native_status_message(&status, next_app));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::ToggleGhosttyMode {
                next_mode,
                response,
            } => match response {
                Ok(status) => {
                    self.native_status = Some(status);
                    self.set_message(format!("Ghostty placement: {}", next_mode.display_label()));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::OpenThoughtConfig { response } => match response {
                Ok(response) => {
                    self.daemon_defaults_status =
                        DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref());
                    self.thought_config_editor = Some(ThoughtConfigEditorState::new(
                        response.config,
                        response.daemon_defaults,
                    ));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::TestThoughtConfig { outcome }
            | PendingInteractionResult::SaveThoughtConfig { outcome } => {
                if let Some(candidates) = outcome.openrouter_candidates {
                    if let Some(editor) = &mut self.thought_config_editor {
                        editor.replace_openrouter_model_presets(candidates);
                    }
                }
                if let Some(config) = outcome.updated_config {
                    if let Some(editor) = &mut self.thought_config_editor {
                        editor.config = config;
                    }
                }
                if outcome.close_editor {
                    self.close_thought_config_editor();
                }
                if outcome.refresh_sessions {
                    self.pending_refresh = None;
                    self.spawn_background_refresh(false);
                }
                self.set_message(outcome.message);
            }
            PendingInteractionResult::VoiceTranscription {
                generation,
                response,
            } => match response {
                Ok(transcript) => self.insert_voice_transcript(generation, transcript),
                Err(err) => {
                    self.voice_state = VoiceUiState::Failed(err.clone());
                    self.set_message(err);
                }
            },
        }
    }

    fn native_status_message(
        status: &NativeDesktopStatusResponse,
        fallback_app: NativeDesktopApp,
    ) -> String {
        let app_label = status
            .app
            .clone()
            .unwrap_or_else(|| fallback_app.display_name().to_string());
        if status.supported {
            match status.ghostty_mode {
                Some(mode) => format!(
                    "terminal handoff target: {app_label} ({})",
                    mode.display_label()
                ),
                None => format!("terminal handoff target: {app_label}"),
            }
        } else {
            format!(
                "terminal handoff target: {app_label} | {}",
                status.reason.as_deref().unwrap_or("unavailable")
            )
        }
    }

    pub(crate) fn poll_refresh(&mut self, layout: WorkspaceLayout) {
        let Some(rx) = &mut self.pending_refresh else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_refresh = None;
                return;
            }
        };

        self.pending_refresh = None;
        self.apply_refresh_result(result, layout);
    }

    pub(crate) fn apply_refresh_result(&mut self, result: RefreshResult, layout: WorkspaceLayout) {
        match result.sessions {
            Ok(sessions) => {
                self.sync_repo_themes(&sessions, result.force_asset_refresh);

                self.retain_cached_assets(&sessions);
                let sessions_by_id = sessions
                    .iter()
                    .map(|session| (session.session_id.as_str(), session))
                    .collect::<HashMap<_, _>>();
                for (session_id, artifact_result) in result.mermaid_artifacts {
                    if let Some(session) = sessions_by_id.get(session_id.as_str()) {
                        self.apply_mermaid_artifact_result(session, artifact_result);
                    }
                }
                for (session_id, skills_result) in result.session_skills {
                    if let Some(session) = sessions_by_id.get(session_id.as_str()) {
                        self.apply_session_skill_result(session, skills_result);
                    }
                }
                self.rebuild_mermaid_artifacts_from_cache();

                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);
                self.last_successful_refresh = Some(Instant::now());
                self.api_refresh_health.record_success();

                if result.show_success_message {
                    let count = self.entities.len();
                    self.set_message(format!("refreshed {count} session{}", pluralize(count)));
                }
            }
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                self.api_refresh_health.record_failure();
            }
        }

        if let Some(native_result) = result.native_status {
            match native_result {
                Ok(status) => {
                    self.native_status = Some(status);
                }
                Err(err) => {
                    self.set_message(self.refresh_error_message(err));
                }
            }
        }
        if let Some(status) = result.daemon_defaults_status {
            self.daemon_defaults_status = status;
        }

        self.last_refresh = Some(Instant::now());
    }

    pub(crate) fn merge_sessions(&mut self, sessions: Vec<SessionSummary>, field: Rect) {
        let mut existing = HashMap::new();
        for entity in self.entities.drain(..) {
            existing.insert(entity.session.session_id.clone(), entity);
        }

        let mut next = Vec::with_capacity(sessions.len());
        for session in sessions {
            if let Some(mut entity) = existing.remove(&session.session_id) {
                entity.session = session;
                next.push(entity);
            } else {
                next.push(SessionEntity::new(session, field));
            }
        }

        next.sort_by(|a, b| compare_tmux_natural(&a.session, &b.session));
        self.entities = next;
        self.layout_resting_entities(field);
        self.reconcile_selection();
        self.sync_selection_publication();
        self.maybe_refresh_attention_group();
    }

    pub(crate) fn upsert_session(&mut self, session: SessionSummary, field: Rect) {
        let mut sessions: Vec<SessionSummary> = self
            .entities
            .iter()
            .map(|entity| entity.session.clone())
            .collect();
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.session_id == session.session_id)
        {
            *existing = session;
        } else {
            sessions.push(session);
        }
        self.merge_sessions(sessions, field);
    }

    fn maybe_refresh_attention_group(&mut self) {
        if self.initial_request.is_some()
            || self.pending_interaction.is_some()
            || self
                .last_attention_group_refresh
                .map(|last| last.elapsed() < ATTENTION_GROUP_REFRESH_INTERVAL)
                .unwrap_or(false)
        {
            return;
        }

        let ready_ids = self.ready_attention_group_session_ids();
        if self.attention_group_session_ids.is_empty() {
            if self.attention_group_auto_open_supported() && !ready_ids.is_empty() {
                self.request_attention_group(Vec::new(), true, false);
            }
            return;
        }

        let ready_id_set = ready_ids.iter().collect::<HashSet<_>>();
        let visible_has_unready = self
            .attention_group_session_ids
            .iter()
            .any(|session_id| !ready_id_set.contains(session_id));
        let visible_has_room =
            self.attention_group_session_ids.len() < ATTENTION_GROUP_MAX_SESSIONS;
        let ready_waiting_offscreen = ready_ids
            .iter()
            .any(|session_id| !self.attention_group_session_ids.contains(session_id));

        if visible_has_unready || (visible_has_room && ready_waiting_offscreen) {
            self.attention_group_refresh_armed = false;
            self.refresh_attention_group();
        }
    }

    fn attention_group_auto_open_supported(&self) -> bool {
        self.native_status
            .as_ref()
            .map(|status| {
                status.supported && self.current_native_app() == NativeDesktopApp::Ghostty
            })
            .unwrap_or(false)
    }

    fn ready_attention_group_session_ids(&self) -> Vec<String> {
        self.entities
            .iter()
            .filter(|entity| attention_group_session_is_eligible(&entity.session))
            .map(|entity| entity.session.session_id.clone())
            .collect()
    }

    pub(crate) fn sync_repo_themes(&mut self, sessions: &[SessionSummary], force: bool) {
        self.retain_cached_assets(sessions);
        let cached_themes = self.repo_themes.clone();
        for session in sessions {
            let context = RepoThemeCacheContext::from_session(session);
            let reuse_cached = !force
                && self
                    .session_repo_theme_cache
                    .get(&session.session_id)
                    .map(|entry| entry.context == context)
                    .unwrap_or(false);
            if reuse_cached {
                continue;
            }

            let resolved = discover_repo_theme(&session.cwd).or_else(|| {
                if force {
                    return None;
                }
                let theme_id = session.repo_theme_id.as_ref()?;
                let theme = cached_themes.get(theme_id)?.clone();
                Some((theme_id.clone(), theme))
            });
            self.session_repo_theme_cache.insert(
                session.session_id.clone(),
                RepoThemeCacheEntry { context, resolved },
            );
        }
        self.rebuild_repo_themes_from_cache();
    }

    pub(crate) fn remember_repo_theme(
        &mut self,
        session: &SessionSummary,
        theme: Option<RepoTheme>,
    ) {
        if let (Some(theme_id), Some(theme)) = (session.repo_theme_id.as_ref(), theme) {
            self.repo_themes.insert(theme_id.clone(), theme);
            return;
        }

        if let Some((theme_id, resolved)) = discover_repo_theme(&session.cwd) {
            self.repo_themes.insert(theme_id, resolved);
        }
    }

    pub(crate) fn tick(&mut self, field: Rect) {
        self.tick = self.tick.wrapping_add(1);
        self.layout_resting_entities(field);
        for entity in &mut self.entities {
            entity.tick(field, self.tick);
        }
        self.resolve_collisions(field);
    }

    pub(crate) fn layout_resting_entities(&mut self, field: Rect) {
        let mut bottom_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Bottom).then_some(index)
            })
            .collect::<Vec<_>>();
        let mut top_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Top).then_some(index)
            })
            .collect::<Vec<_>>();

        bottom_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });
        top_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });

        for (slot, entity_index) in bottom_resting.into_iter().enumerate() {
            let (x, y) = bottom_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
        }
        for (slot, entity_index) in top_resting.into_iter().enumerate() {
            let (x, y) = top_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
        }
    }

    pub(crate) fn resolve_collisions(&mut self, field: Rect) {
        let entity_count = self.entities.len();
        let throttle_collisions = entity_count > COLLISION_THROTTLE_ENTITY_THRESHOLD;

        for idx in 0..entity_count {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            if throttle_collisions && !right.is_empty() {
                let budget = COLLISION_THROTTLE_PAIR_BUDGET.min(right.len());
                let start = (self.tick as usize).wrapping_add(idx.saturating_mul(13)) % right.len();
                for step in 0..budget {
                    let b_index = (start + step) % right.len();
                    let b = &mut right[b_index];
                    Self::resolve_collision_pair(a, b, field);
                }
            } else {
                for b in right {
                    Self::resolve_collision_pair(a, b, field);
                }
            }
        }
    }

    fn resolve_collision_pair(a: &mut SessionEntity, b: &mut SessionEntity, field: Rect) {
        let a_rect = a.screen_rect(field);
        let b_rect = b.screen_rect(field);
        if !intersects(a_rect, b_rect) {
            return;
        }

        match (a.is_stationary(), b.is_stationary()) {
            (true, true) => {}
            (true, false) => separate_from_fixed_entity(b, a_rect, field),
            (false, true) => separate_from_fixed_entity(a, b_rect, field),
            (false, false) => {
                std::mem::swap(&mut a.vx, &mut b.vx);
                std::mem::swap(&mut a.vy, &mut b.vy);
                a.x = (a.x - 1.0).max(0.0);
                b.x = (b.x + 1.0).min(field.width.saturating_sub(ENTITY_WIDTH) as f32);
                a.swim_anchor_x = a.x;
                b.swim_anchor_x = b.x;
                a.swim_anchor_y = a.y;
                b.swim_anchor_y = b.y;
                a.swim_center_y = a.y;
                b.swim_center_y = b.y;
            }
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, field: Rect) {
        if let Some(picker) = &mut self.picker {
            let layout = picker_layout(picker, field);
            picker.move_selection(delta, layout.visible_entry_rows);
            return;
        }

        if self.entities.is_empty() {
            self.selected_id = None;
            self.sync_selection_publication();
            return;
        }

        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.selected_id = None;
            self.sync_selection_publication();
            return;
        }

        let current_index = self
            .selected_id
            .as_ref()
            .and_then(|selected| {
                visible_entities
                    .iter()
                    .position(|entity| entity.session.session_id == *selected)
            })
            .unwrap_or(0) as isize;

        let len = visible_entities.len() as isize;
        let next_index = (current_index + delta).rem_euclid(len) as usize;
        self.selected_id = Some(visible_entities[next_index].session.session_id.clone());
        self.sync_selection_publication();
    }

    pub(crate) fn selected(&self) -> Option<&SessionEntity> {
        let selected = self.selected_id.as_ref()?;
        self.entities.iter().find(|entity| {
            entity.session.session_id == *selected
                && self.thought_filter.matches_session(&entity.session)
        })
    }

    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
        self.close_initial_request();
        self.last_picker_refresh = None;
    }

    pub(crate) fn picker_search_push(&mut self, ch: char) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        picker.search.push(ch);
        picker.snap_selection_to_visible();
    }

    pub(crate) fn picker_search_pop(&mut self) -> bool {
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        if picker.search.is_empty() {
            return false;
        }
        picker.search.pop();
        picker.snap_selection_to_visible();
        true
    }

    pub(crate) fn picker_search_clear(&mut self) -> bool {
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        if picker.search.is_empty() {
            return false;
        }
        picker.search.clear();
        picker.snap_selection_to_visible();
        true
    }

    pub(crate) fn open_thought_config_editor(&mut self) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading thought config...");
        self.runtime.spawn(async move {
            let response = client.fetch_thought_config().await;
            let _ = tx.send(PendingInteractionResult::OpenThoughtConfig { response });
        });
    }

    pub(crate) fn close_thought_config_editor(&mut self) {
        self.thought_config_editor = None;
    }

    pub(crate) fn handle_thought_config_key(&mut self, key: KeyEvent, layout: WorkspaceLayout) {
        if self.pending_interaction.is_some() {
            self.handle_pending_thought_config_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc => self.close_thought_config_editor(),
            KeyCode::Up | KeyCode::BackTab => self.move_thought_config_focus(-1),
            KeyCode::Down | KeyCode::Tab => self.move_thought_config_focus(1),
            KeyCode::Left => self.adjust_thought_config_field(-1),
            KeyCode::Right | KeyCode::Char(' ') => self.adjust_thought_config_field(1),
            KeyCode::Backspace => self.pop_thought_config_model_char(),
            KeyCode::Enter => self.activate_thought_config_field(layout),
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_thought_config_model_char(ch);
            }
            _ => {}
        }
    }

    fn handle_pending_thought_config_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.close_thought_config_editor();
        } else {
            self.set_message("wait for the current action to finish");
        }
    }

    fn move_thought_config_focus(&mut self, delta: isize) {
        if let Some(editor) = &mut self.thought_config_editor {
            editor.move_focus(delta);
        }
    }

    fn pop_thought_config_model_char(&mut self) {
        if let Some(editor) = &mut self.thought_config_editor {
            if editor.focus == ThoughtConfigEditorField::Model {
                editor.config.model.pop();
            }
        }
    }

    fn push_thought_config_model_char(&mut self, ch: char) {
        if let Some(editor) = &mut self.thought_config_editor {
            if editor.focus == ThoughtConfigEditorField::Model {
                editor.config.model.push(ch);
            }
        }
    }

    pub(crate) fn handle_thought_config_paste(&mut self, text: &str) {
        if self.pending_interaction.is_some() {
            self.set_message("wait for the current action to finish");
            return;
        }
        if let Some(editor) = &mut self.thought_config_editor {
            if editor.focus == ThoughtConfigEditorField::Model {
                editor.config.model.push_str(text);
            }
        }
    }

    fn adjust_thought_config_field(&mut self, delta: isize) {
        let Some(editor) = &mut self.thought_config_editor else {
            return;
        };
        match editor.focus {
            ThoughtConfigEditorField::Enabled => editor.config.enabled = !editor.config.enabled,
            ThoughtConfigEditorField::Backend => editor.cycle_backend(delta),
            ThoughtConfigEditorField::Model => {
                let _ = editor.cycle_model_preset(delta);
            }
            ThoughtConfigEditorField::Test
            | ThoughtConfigEditorField::Save
            | ThoughtConfigEditorField::Cancel => {}
        }
    }

    fn activate_thought_config_field(&mut self, layout: WorkspaceLayout) {
        let Some(focus) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.focus)
        else {
            return;
        };
        match focus {
            ThoughtConfigEditorField::Enabled | ThoughtConfigEditorField::Backend => {
                self.adjust_thought_config_field(1)
            }
            ThoughtConfigEditorField::Model => {}
            ThoughtConfigEditorField::Test => self.test_thought_config(),
            ThoughtConfigEditorField::Save => self.submit_thought_config(layout),
            ThoughtConfigEditorField::Cancel => self.close_thought_config_editor(),
        }
    }

    fn test_thought_config(&mut self) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("testing thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_test_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::TestThoughtConfig { outcome });
        });
    }

    pub(crate) fn submit_thought_config(&mut self, _layout: WorkspaceLayout) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("saving thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_save_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::SaveThoughtConfig { outcome });
        });
    }

    async fn run_thought_config_test_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        let target = Self::thought_config_target_summary(&config);
        match client.test_thought_config(config.clone()).await {
            Ok(test) if test.ok => ThoughtConfigActionOutcome {
                message: format!("test ok: {target}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
            Ok(test) => Self::try_openrouter_rotation(
                client,
                &config,
                daemon_defaults,
                false,
                target.clone(),
                test.message.clone(),
            )
            .await
            .unwrap_or_else(|| ThoughtConfigActionOutcome {
                message: format!("test failed: {target} | {}", test.message),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            }),
            Err(err) => ThoughtConfigActionOutcome {
                message: format!("test error: {target} | {err}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
        }
    }

    async fn run_thought_config_save_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        match client.update_thought_config(config).await {
            Ok(saved) => {
                let save_summary = Self::thought_config_target_summary(&saved);
                let maybe_rotation = match client.test_thought_config(saved.clone()).await {
                    Ok(test) if test.ok => None,
                    Ok(test) => Self::try_openrouter_rotation(
                        Arc::clone(&client),
                        &saved,
                        daemon_defaults,
                        true,
                        save_summary.clone(),
                        test.message.clone(),
                    )
                    .await
                    .or_else(|| {
                        Some(ThoughtConfigActionOutcome {
                            message: format!("saved {save_summary} | {}", test.message),
                            updated_config: None,
                            openrouter_candidates: None,
                            close_editor: true,
                            refresh_sessions: true,
                        })
                    }),
                    Err(err) => Some(ThoughtConfigActionOutcome {
                        message: format!("saved {save_summary} | test error: {err}"),
                        updated_config: None,
                        openrouter_candidates: None,
                        close_editor: true,
                        refresh_sessions: true,
                    }),
                };
                maybe_rotation.unwrap_or(ThoughtConfigActionOutcome {
                    message: format!("saved {save_summary} | test ok"),
                    updated_config: None,
                    openrouter_candidates: None,
                    close_editor: true,
                    refresh_sessions: true,
                })
            }
            Err(err) => ThoughtConfigActionOutcome {
                message: err,
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
        }
    }

    async fn try_openrouter_rotation(
        client: Arc<C>,
        config: &ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
        persist: bool,
        target: String,
        failure_message: String,
    ) -> Option<ThoughtConfigActionOutcome> {
        if !Self::is_effective_openrouter_backend(config, daemon_defaults.as_ref())
            || !should_rotate_openrouter_model(&failure_message)
        {
            return None;
        }

        let candidates = client.refresh_openrouter_candidates().await.ok()?;
        for candidate in &candidates {
            if candidate.eq_ignore_ascii_case(config.model.trim()) {
                continue;
            }

            let mut rotated = config.clone();
            rotated.model = candidate.clone();
            let test = match client.test_thought_config(rotated.clone()).await {
                Ok(test) => test,
                Err(_) => continue,
            };
            if !test.ok {
                continue;
            }

            if persist {
                return Some(match client.update_thought_config(rotated).await {
                    Ok(_) => ThoughtConfigActionOutcome {
                        message: format!(
                            "saved {target} | rotated to {candidate} after OpenRouter catalog refresh | test ok"
                        ),
                        updated_config: None,
                        openrouter_candidates: Some(candidates),
                        close_editor: true,
                        refresh_sessions: true,
                    },
                    Err(err) => ThoughtConfigActionOutcome {
                        message: format!(
                            "saved {target} | rotated probe found {candidate}, but save failed: {err}"
                        ),
                        updated_config: None,
                        openrouter_candidates: Some(candidates),
                        close_editor: true,
                        refresh_sessions: true,
                    },
                });
            }

            return Some(ThoughtConfigActionOutcome {
                message: format!(
                    "test failed: {target} | rotated to {candidate} after OpenRouter catalog refresh | test ok"
                ),
                updated_config: Some(rotated),
                openrouter_candidates: Some(candidates),
                close_editor: false,
                refresh_sessions: false,
            });
        }

        None
    }

    fn is_effective_openrouter_backend(
        config: &ThoughtConfig,
        daemon_defaults: Option<&DaemonDefaults>,
    ) -> bool {
        if config.backend.eq_ignore_ascii_case("openrouter") {
            return true;
        }
        config.backend.trim().is_empty()
            && daemon_defaults
                .map(|defaults| defaults.backend.eq_ignore_ascii_case("openrouter"))
                .unwrap_or(false)
    }

    fn thought_config_target_summary(config: &ThoughtConfig) -> String {
        format!(
            "{} / {}",
            if config.backend.trim().is_empty() {
                "auto"
            } else {
                config.backend.as_str()
            },
            if config.model.trim().is_empty() {
                "daemon default"
            } else {
                config.model.as_str()
            }
        )
    }

    pub(crate) fn open_picker(&mut self, x: u16, y: u16) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading directories...");
        self.runtime.spawn(async move {
            let (response, repo_search) =
                tokio::join!(client.list_dirs(None, true, None), client.list_repo_dirs());
            let _ = tx.send(PendingInteractionResult::OpenPicker {
                x,
                y,
                response,
                repo_search,
            });
        });
    }

    pub(crate) fn picker_reload(
        &mut self,
        path: Option<String>,
        managed_only: bool,
        group: Option<String>,
    ) {
        self.picker_reload_with_options(path, managed_only, group, true, false);
    }

    pub(crate) fn picker_reload_with_options(
        &mut self,
        path: Option<String>,
        managed_only: bool,
        group: Option<String>,
        show_message: bool,
        preserve_selection: bool,
    ) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        if show_message {
            self.set_message("loading directories...");
        }
        self.runtime.spawn(async move {
            let response = client
                .list_dirs(path.as_deref(), managed_only, group.as_deref())
                .await;
            let _ = tx.send(PendingInteractionResult::ReloadPicker {
                managed_only,
                group,
                preserve_selection,
                response,
            });
        });
    }

    pub(crate) fn picker_up(&mut self) {
        let Some(picker) = &self.picker else {
            return;
        };

        // Inside a group → go back to root listing.
        if picker.current_group.is_some() {
            let managed_only = picker.managed_only;
            self.picker_reload(None, managed_only, None);
            return;
        }

        let Some(parent_path) = picker.parent_path() else {
            return;
        };
        let managed_only = picker.managed_only;
        self.picker_reload(Some(parent_path), managed_only, None);
    }

    pub(crate) fn picker_set_managed_only(&mut self, managed_only: bool) {
        let Some(picker) = &self.picker else {
            return;
        };
        if picker.managed_only == managed_only && picker.current_group.is_none() {
            return;
        }
        let path = if picker.current_group.is_some() {
            None
        } else {
            Some(picker.current_path.clone())
        };
        self.picker_reload(path, managed_only, None);
    }

    pub(crate) fn picker_set_group(&mut self, name: String) {
        let Some(picker) = &self.picker else {
            return;
        };
        if picker.current_group.as_ref() == Some(&name) {
            return;
        }
        self.picker_reload(None, picker.managed_only, Some(name));
    }

    pub(crate) fn picker_cycle_group_edit_target(&mut self) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        match picker.cycle_group_edit_target() {
            Some(target) => self.set_message(format!("directory group target: {target}")),
            None => self.set_message("no directory groups available"),
        }
    }

    pub(crate) fn picker_add_selected_to_group_target(&mut self) {
        self.update_selected_picker_entry_groups(PickerGroupUpdateMode::Add);
    }

    pub(crate) fn picker_remove_selected_from_group_target(&mut self) {
        self.update_selected_picker_entry_groups(PickerGroupUpdateMode::Remove);
    }

    pub(crate) fn picker_move_selected_to_group_target(&mut self) {
        self.update_selected_picker_entry_groups(PickerGroupUpdateMode::Move);
    }

    fn update_selected_picker_entry_groups(&mut self, mode: PickerGroupUpdateMode) {
        let Some((path, entry_label, add, remove, reload_path, managed_only, group)) =
            self.picker.as_ref().and_then(|picker| {
                let PickerSelection::Entry(index) = picker.selection else {
                    return None;
                };
                let target = picker.group_edit_target.clone()?;
                let entry = picker.entry_at(index)?;
                let path = picker.path_for_entry(index)?;
                let memberships = entry.groups.clone();
                let add = match mode {
                    PickerGroupUpdateMode::Add | PickerGroupUpdateMode::Move => {
                        vec![target.clone()]
                    }
                    PickerGroupUpdateMode::Remove => Vec::new(),
                };
                let remove = match mode {
                    PickerGroupUpdateMode::Add => Vec::new(),
                    PickerGroupUpdateMode::Remove => vec![target],
                    PickerGroupUpdateMode::Move => {
                        let mut remove = memberships
                            .into_iter()
                            .filter(|group| group != &target)
                            .collect::<Vec<_>>();
                        if let Some(current) = &picker.current_group {
                            if current != &target && !remove.iter().any(|group| group == current) {
                                remove.push(current.clone());
                            }
                        }
                        remove
                    }
                };
                Some((
                    path,
                    entry.name.clone(),
                    add,
                    remove,
                    picker.current_path.clone(),
                    picker.managed_only,
                    picker.current_group.clone(),
                ))
            })
        else {
            self.set_message("select a directory entry and group target first");
            return;
        };

        if add.is_empty() && remove.is_empty() {
            self.set_message("no directory group change selected");
            return;
        }

        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!("updating groups for {entry_label}..."));
        self.runtime.spawn(async move {
            let response = match client
                .update_dir_group_memberships(&path, add, remove)
                .await
            {
                Ok(_) => {
                    client
                        .list_dirs(Some(&reload_path), managed_only, group.as_deref())
                        .await
                }
                Err(err) => Err(err),
            };
            let _ = tx.send(PendingInteractionResult::ReloadPicker {
                managed_only,
                group,
                preserve_selection: true,
                response,
            });
        });
    }

    pub(crate) fn start_picker_repo_action(&mut self, index: usize, kind: RepoActionKind) {
        // Open is handled client-side — no server round-trip.
        if kind == RepoActionKind::Open {
            self.open_picker_url_at(index);
            return;
        }

        let Some((entry_path, repo_label, reload_path, managed_only)) =
            self.picker.as_ref().and_then(|picker| {
                let entry = picker.entry_at(index)?;
                let actions = picker_entry_actions(entry);
                let has_action = actions.iter().any(|a| a.kind == kind && a.clickable);
                if !has_action {
                    return None;
                }
                Some((
                    picker.path_for_entry(index)?,
                    entry.name.clone(),
                    picker.current_path.clone(),
                    picker.managed_only,
                ))
            })
        else {
            self.set_message(format!("no {} action for this entry", kind_label(kind)));
            return;
        };

        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!("starting {}...", kind_label(kind)));
        self.runtime.spawn(async move {
            let response = client.start_repo_action(&entry_path, kind).await;
            let _ = tx.send(PendingInteractionResult::StartRepoAction {
                repo_label,
                kind,
                reload_path,
                managed_only,
                response,
            });
        });
    }

    fn open_picker_url_at(&mut self, index: usize) {
        let Some(url) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.entry_at(index))
            .and_then(|entry| entry.open_url.clone())
        else {
            self.set_message("no open URL for this entry");
            return;
        };
        match open::that(&url) {
            Ok(_) => self.set_message(format!("opened {url}")),
            Err(err) => self.set_message(format!("failed to open {url}: {err}")),
        }
    }

    pub(crate) fn picker_open_url_for_selection(&mut self) {
        let Some(index) = self
            .picker
            .as_ref()
            .and_then(|picker| match picker.selection {
                PickerSelection::Entry(index) => Some(index),
                PickerSelection::SpawnHere => None,
            })
        else {
            self.set_message("select an entry first");
            return;
        };
        self.open_picker_url_at(index);
    }

    pub(crate) fn open_initial_request(&mut self, cwd: String, launch_target: Option<String>) {
        self.cancel_voice_recording();
        self.initial_request_generation = self.initial_request_generation.saturating_add(1);
        self.group_input_targets = None;
        self.initial_request = Some(InitialRequestState::new(cwd, launch_target));
        self.voice_state = default_ui_state();
    }

    pub(crate) fn open_batch_initial_request_for_visible_entries(&mut self) {
        let Some((visible_count, dirs)) = self.picker.as_ref().map(|picker| {
            (
                picker.visible_entries().len(),
                picker.batch_dirs_for_visible_entries(),
            )
        }) else {
            return;
        };

        if dirs.is_empty() {
            if visible_count == 0 {
                self.set_message("no visible directories to batch");
            } else {
                self.set_message("all visible directories are excluded");
            }
            return;
        };

        self.cancel_voice_recording();
        self.initial_request_generation = self.initial_request_generation.saturating_add(1);
        let launch_target = self
            .picker
            .as_ref()
            .and_then(|picker| picker.launch_target.clone());
        self.launch_target = launch_target.clone();
        self.group_input_targets = None;
        self.initial_request = Some(InitialRequestState::new_batch(dirs, launch_target));
        self.voice_state = default_ui_state();
    }

    pub(crate) fn open_group_input_request(&mut self, session_ids: Vec<String>, label: String) {
        if session_ids.is_empty() {
            self.set_message("no sessions in this school");
            return;
        }
        self.attention_group_refresh_armed = false;
        self.cancel_voice_recording();
        self.initial_request_generation = self.initial_request_generation.saturating_add(1);
        self.group_input_targets = Some(GroupInputTargets {
            session_ids,
            label: label.clone(),
        });
        self.initial_request = Some(InitialRequestState::new(label, None));
        self.voice_state = default_ui_state();
    }

    pub(crate) fn close_initial_request(&mut self) {
        self.cancel_voice_recording();
        self.initial_request_generation = self.initial_request_generation.saturating_add(1);
        self.initial_request = None;
        self.group_input_targets = None;
    }

    pub(crate) fn handle_initial_request_key(&mut self, key: KeyEvent, field: Rect) {
        match key.code {
            KeyCode::Esc => self.close_initial_request(),
            KeyCode::Enter => self.submit_initial_request(field),
            KeyCode::Char('v') if key.modifiers == KeyModifiers::CONTROL => {
                self.toggle_voice_recording();
            }
            KeyCode::Backspace => {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.pop();
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.push(ch);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        if self.thought_config_editor.is_some() {
            self.handle_thought_config_paste(text);
            return;
        }
        if let Some(initial_request) = &mut self.initial_request {
            initial_request.value.push_str(text);
        }
    }

    pub(crate) fn submit_initial_request(&mut self, field: Rect) {
        if self.voice_recording.is_some() {
            self.set_message("stop voice recording before creating a swimmer");
            return;
        }
        if matches!(self.voice_state, VoiceUiState::Transcribing) {
            self.set_message("wait for voice transcription to finish");
            return;
        }
        let Some(initial_request) = self
            .initial_request
            .as_ref()
            .and_then(InitialRequestState::trimmed_value)
        else {
            self.set_message("enter an initial request");
            return;
        };
        if let Some(targets) = self.group_input_targets.clone() {
            self.send_group_input(targets.session_ids, initial_request);
            return;
        }
        let Some(cwd) = self.initial_request.as_ref().map(|state| state.cwd.clone()) else {
            return;
        };
        if let Some(dirs) = self
            .initial_request
            .as_ref()
            .and_then(|state| state.batch_dirs.clone())
        {
            let launch_target = self
                .initial_request
                .as_ref()
                .and_then(|state| state.launch_target.clone());
            self.spawn_sessions_batch(dirs, launch_target, Some(initial_request), field);
        } else {
            let launch_target = self
                .initial_request
                .as_ref()
                .and_then(|state| state.launch_target.clone());
            self.spawn_session(&cwd, launch_target, Some(initial_request), field);
        }
    }

    pub(crate) fn toggle_voice_recording(&mut self) {
        if self.initial_request.is_none() {
            self.set_message("open an initial request first");
            return;
        }
        if matches!(self.voice_state, VoiceUiState::Transcribing) {
            self.set_message("wait for voice transcription to finish");
            return;
        }
        if let Some(recording) = self.voice_recording.take() {
            self.finish_voice_recording(recording);
        } else {
            self.start_voice_recording();
        }
    }

    fn start_voice_recording(&mut self) {
        match start_recording() {
            Ok(recording) => {
                self.voice_recording = Some(recording);
                self.voice_state = VoiceUiState::Recording;
                self.set_message("voice recording started");
            }
            Err(err) => {
                self.voice_state = VoiceUiState::Failed(err.clone());
                self.set_message(err);
            }
        }
    }

    fn finish_voice_recording(&mut self, recording: VoiceRecording) {
        if self.pending_interaction.is_some() {
            self.voice_recording = Some(recording);
            self.set_message("wait for the current action to finish");
            return;
        }

        let (tx, rx) = oneshot::channel();
        let generation = self.initial_request_generation;
        self.pending_interaction = Some(rx);
        self.voice_state = VoiceUiState::Transcribing;
        self.set_message("transcribing voice capture...");

        self.runtime.spawn(async move {
            let response = match tokio::task::spawn_blocking(move || recording.finish()).await {
                Ok(response) => response,
                Err(err) => Err(format!("voice task failed: {err}")),
            };
            let _ = tx.send(PendingInteractionResult::VoiceTranscription {
                generation,
                response,
            });
        });
    }

    fn cancel_voice_recording(&mut self) {
        if let Some(recording) = self.voice_recording.take() {
            recording.cancel();
        }
        self.voice_state = default_ui_state();
    }

    fn insert_voice_transcript(&mut self, generation: u64, transcript: String) {
        self.voice_state = default_ui_state();
        if generation != self.initial_request_generation {
            self.set_message("voice transcript finished after the composer changed");
            return;
        }
        let Some(initial_request) = &mut self.initial_request else {
            self.set_message("voice transcript finished after the composer closed");
            return;
        };

        if !initial_request.value.trim().is_empty() {
            initial_request.value.push('\n');
        }
        initial_request.value.push_str(transcript.trim());
        self.set_message("voice transcript inserted");
    }

    pub(crate) fn picker_activate_selection(&mut self, _field: Rect) {
        let Some((selection, current_path, entry_path, has_children)) =
            self.picker.as_ref().map(|picker| match picker.selection {
                PickerSelection::SpawnHere => (
                    PickerSelection::SpawnHere,
                    picker.current_path.clone(),
                    None,
                    false,
                ),
                PickerSelection::Entry(index) => (
                    PickerSelection::Entry(index),
                    picker.current_path.clone(),
                    picker.path_for_entry(index),
                    picker
                        .entries
                        .get(index)
                        .map(|entry| entry.has_children)
                        .unwrap_or(false),
                ),
            })
        else {
            return;
        };

        let launch_target = self
            .picker
            .as_ref()
            .and_then(|picker| picker.launch_target.clone());
        self.launch_target = launch_target.clone();

        match selection {
            PickerSelection::SpawnHere => self.open_initial_request(current_path, launch_target),
            PickerSelection::Entry(_) if has_children => {
                if let Some(path) = entry_path {
                    let managed_only = self
                        .picker
                        .as_ref()
                        .map(|picker| picker.managed_only)
                        .unwrap_or(true);
                    self.picker_reload(Some(path), managed_only, None);
                }
            }
            PickerSelection::Entry(_) => {
                if let Some(path) = entry_path {
                    self.open_initial_request(path, launch_target);
                }
            }
        }
    }

    pub(crate) fn picker_start_action_for_selection(&mut self, kind: RepoActionKind) {
        let Some(index) = self
            .picker
            .as_ref()
            .and_then(|picker| match picker.selection {
                PickerSelection::Entry(index) => Some(index),
                PickerSelection::SpawnHere => None,
            })
        else {
            self.set_message("select a repo first");
            return;
        };

        self.start_picker_repo_action(index, kind);
    }

    pub(crate) fn spawn_session(
        &mut self,
        cwd: &str,
        launch_target: Option<String>,
        initial_request: Option<String>,
        field: Rect,
    ) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let cwd = cwd.to_string();
        let spawn_tool = self.spawn_tool;
        self.launch_target = launch_target.clone();
        self.set_message("creating session...");
        self.runtime.spawn(async move {
            let response = client
                .create_session(&cwd, spawn_tool, launch_target, initial_request)
                .await;
            let _ = tx.send(PendingInteractionResult::CreateSession { field, response });
        });
    }

    pub(crate) fn spawn_sessions_batch(
        &mut self,
        dirs: Vec<String>,
        launch_target: Option<String>,
        initial_request: Option<String>,
        field: Rect,
    ) {
        let total = dirs.len();
        if total == 0 {
            self.set_message("no visible directories to batch");
            return;
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let spawn_tool = self.spawn_tool;
        self.launch_target = launch_target.clone();
        self.set_message(format!("creating {total} sessions..."));
        self.runtime.spawn(async move {
            let response = client
                .create_sessions_batch(dirs, spawn_tool, launch_target, initial_request)
                .await;
            let _ = tx.send(PendingInteractionResult::CreateSessionsBatch { field, response });
        });
    }

    pub(crate) fn send_group_input(&mut self, session_ids: Vec<String>, text: String) {
        let total = session_ids.len();
        if total == 0 {
            self.set_message("no sessions in this school");
            return;
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!("sending to {total} sessions..."));
        self.runtime.spawn(async move {
            let response = client.send_group_input(session_ids, text).await;
            let _ = tx.send(PendingInteractionResult::SendGroupInput { response });
        });
    }

    fn apply_batch_create_response(&mut self, field: Rect, response: CreateSessionsBatchResponse) {
        let total = response.results.len();
        let mut success_count = 0usize;
        let mut last_tmux_name = None;
        let mut last_session_id = None;
        let mut first_error = None;

        for result in response.results {
            if result.ok {
                if let Some(session) = result.session {
                    self.remember_repo_theme(&session, result.repo_theme);
                    last_tmux_name = Some(session.tmux_name.clone());
                    last_session_id = Some(session.session_id.clone());
                    self.upsert_session(session, field);
                    success_count += 1;
                }
            } else if first_error.is_none() {
                let detail = result
                    .error
                    .and_then(|error| error.message)
                    .unwrap_or_else(|| "unknown error".to_string());
                first_error = Some(format!("{}: {detail}", shorten_path(&result.cwd, 32)));
            }
        }

        if let Some(session_id) = last_session_id {
            self.selected_id = Some(session_id);
            self.reconcile_selection();
            self.sync_selection_publication();
        }

        if success_count > 0 {
            self.close_picker();
            if success_count > 1 {
                self.thought_group_by = ThoughtGroupBy::Batch;
                self.thought_show_all = true;
            }
            if success_count == total {
                match last_tmux_name {
                    Some(name) if success_count == 1 => self.set_message(format!("created {name}")),
                    _ => self.set_message(format!("created {success_count} sessions")),
                }
            } else {
                self.set_message(format!(
                    "created {success_count}/{total}; {}",
                    first_error.unwrap_or_else(|| "some sessions failed".to_string())
                ));
            }
        } else {
            self.set_message(first_error.unwrap_or_else(|| "batch create failed".to_string()));
        }
    }

    fn apply_group_input_response(&mut self, response: SessionGroupInputResponse) {
        let total = response.results.len();
        if response.skipped == 0 {
            self.attention_group_refresh_armed = response.delivered > 0;
            self.close_initial_request();
            self.set_message(format!("sent to {} sessions", response.delivered));
        } else if response.delivered > 0 {
            self.attention_group_refresh_armed = true;
            self.close_initial_request();
            self.set_message(format!(
                "sent to {}/{}; {} skipped",
                response.delivered, total, response.skipped
            ));
        } else {
            self.attention_group_refresh_armed = false;
            self.set_message(format!("sent to 0/{total}; all skipped"));
        }
    }

    pub(crate) fn open_session_for_label(&mut self, session_id: &str, label: &str) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let session_id = session_id.to_string();
        let label = label.to_string();
        self.set_message(format!("opening {label}..."));
        self.runtime.spawn(async move {
            let response = client.open_session(&session_id).await;
            let _ = tx.send(PendingInteractionResult::OpenSession { label, response });
        });
    }

    pub(crate) fn open_selected(&mut self) {
        let Some((selected_id, label)) = self.selected().map(|entity| {
            (
                entity.session.session_id.clone(),
                selected_label(Some(&entity.session.tmux_name)),
            )
        }) else {
            self.set_message("no session selected");
            return;
        };

        self.select_and_open_session(selected_id, label);
    }

    pub(crate) fn select_and_open_session(&mut self, session_id: String, label: String) {
        self.selected_id = Some(session_id.clone());
        self.sync_selection_publication();
        self.open_session_for_label(&session_id, &label);
    }

    pub(crate) fn handle_thought_click(
        &mut self,
        x: u16,
        y: u16,
        thought_content: Rect,
        entry_capacity: usize,
    ) {
        if let Some(action) = thought_panel_action_at(self, thought_content, entry_capacity, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    pub(crate) fn handle_header_filter_click(&mut self, renderer_width: u16, x: u16, y: u16) {
        if let Some(action) = header_filter_action_at(self, renderer_width, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    pub(crate) fn apply_thought_filter_action(&mut self, action: ThoughtPanelAction) {
        match action {
            ThoughtPanelAction::FilterByCwd(cwd) => self.set_thought_filter_cwd(cwd),
            ThoughtPanelAction::ToggleFilterOutMode => self.toggle_thought_filter_out_mode(),
            ThoughtPanelAction::ToggleFilterOutCwd(cwd) => self.toggle_thought_filter_out_cwd(cwd),
            ThoughtPanelAction::OpenSession { session_id, label } => {
                self.select_and_open_session(session_id, label);
            }
            ThoughtPanelAction::OpenInitialRequest { cwd } => {
                self.open_initial_request(cwd, self.launch_target.clone());
            }
            ThoughtPanelAction::SendGroup { session_ids, label } => {
                self.open_group_input_request(session_ids, label);
            }
            ThoughtPanelAction::LaunchCommitCodex(session_id) => {
                self.launch_commit_codex_for_session(&session_id);
            }
            ThoughtPanelAction::OpenMermaid(session_id) => self.open_mermaid_viewer(session_id),
            ThoughtPanelAction::OpenPlanFromDisk { schema_path, slug } => {
                self.open_plan_viewer(schema_path, slug);
            }
            ThoughtPanelAction::OpenRepoInEditor(cwd) => self.open_repo_in_editor(&cwd),
            ThoughtPanelAction::ClearFilters => self.clear_thought_filters(),
        }
    }

    pub(crate) fn launch_commit_codex_for_session(&mut self, session_id: &str) {
        let Some(session) = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .map(|entity| entity.session.clone())
        else {
            self.set_message("missing session for commit codex launch");
            return;
        };

        match self.commit_launcher.launch(&session) {
            Ok(launch) => self.set_message(format!("commit codex: {}", launch.watch_command)),
            Err(err) => self.set_message(format!("failed to launch commit codex: {err}")),
        }
    }

    pub(crate) fn open_repo_in_editor(&mut self, cwd: &str) {
        let repo_label = path_tail_label(cwd).unwrap_or_else(|| cwd.to_string());
        match ProcessCommand::new("code")
            .arg(".")
            .current_dir(cwd)
            .spawn()
        {
            Ok(_) => self.set_message(format!("code . -> {repo_label}")),
            Err(err) => self.set_message(format!("failed to run code .: {err}")),
        }
    }

    pub(crate) fn open_mermaid_artifact(&mut self) {
        let Some(path) = (match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                if viewer.active_tab != DomainPlanTab::Schema {
                    swimmers::session::artifacts::resolve_viewer_text_path(
                        &viewer.cwd,
                        viewer.path.as_deref(),
                        viewer.active_tab.filename(),
                    )
                    .map(|path| path.to_string_lossy().into_owned())
                } else {
                    viewer.openable_path().map(str::to_string)
                }
            }
            FishBowlMode::Aquarium => None,
        }) else {
            self.set_message("artifact path unavailable");
            return;
        };

        let path_label = path_tail_label(&path).unwrap_or_else(|| path.clone());
        match self.artifact_opener.open(&path) {
            Ok(_) => self.set_message(format!("open artifact -> {path_label}")),
            Err(err) => self.set_message(format!("failed to open artifact: {err}")),
        }
    }

    fn mermaid_viewer_state(
        session_id: String,
        tmux_name: String,
        cwd: String,
        path: Option<String>,
        source: Option<String>,
        artifact_error: Option<String>,
        plan_tabs: Option<Vec<DomainPlanTab>>,
        disk_only: bool,
        inline_plan_files: BTreeMap<DomainPlanTab, String>,
    ) -> MermaidViewerState {
        MermaidViewerState {
            session_id,
            tmux_name,
            cwd,
            path,
            source,
            artifact_error,
            render_error: None,
            unsupported_reason: detect_mermaid_backend_support(),
            zoom: 1.0,
            center_x: 0.0,
            center_y: 0.0,
            diagram_width: 0.0,
            diagram_height: 0.0,
            back_rect: None,
            content_rect: None,
            cached_rect: None,
            cached_zoom: 1.0,
            cached_center_x: 0.0,
            cached_center_y: 0.0,
            cached_lines: Vec::new(),
            cached_background_cells: Vec::new(),
            cached_semantic_lines: Vec::new(),
            focused_source_index: None,
            focus_status: None,
            prepared_render: None,
            source_prepare_count: 0,
            viewport_render_count: 0,
            plan_tabs,
            active_tab: DomainPlanTab::Schema,
            inline_plan_files,
            plan_text_content: None,
            plan_text_lines: Vec::new(),
            plan_text_scroll: 0,
            plan_text_cached_width: 0,
            tab_rects: Vec::new(),
            disk_only,
        }
    }

    pub(crate) fn open_mermaid_viewer(&mut self, session_id: String) {
        let Some(session) = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .map(|entity| entity.session.clone())
        else {
            self.set_message("missing session for Mermaid viewer");
            return;
        };

        let should_revalidate = self.session_mermaid_cache.contains_key(&session.session_id)
            || !self.mermaid_artifacts.contains_key(&session.session_id);
        if should_revalidate {
            self.refresh_single_mermaid_artifact(&session, true);
        }

        let Some(artifact) = self.mermaid_artifacts.get(&session.session_id).cloned() else {
            self.set_message("no Mermaid artifact found");
            return;
        };

        let plan_tabs = artifact.plan_files.and_then(|files| {
            let mut tabs = vec![DomainPlanTab::Schema];
            for name in &files {
                if let Some(tab) = DomainPlanTab::from_filename(name) {
                    if tab != DomainPlanTab::Schema {
                        tabs.push(tab);
                    }
                }
            }
            if tabs.len() > 1 {
                Some(tabs)
            } else {
                None
            }
        });
        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            session.session_id.clone(),
            session.tmux_name.clone(),
            session.cwd.clone(),
            artifact.path,
            artifact.source,
            artifact.error,
            plan_tabs,
            false,
            BTreeMap::new(),
        ));
    }

    /// Open the Mermaid/plan viewer directly from a `schema.mmd` path on disk.
    ///
    /// Unlike `open_mermaid_viewer`, this has no backing tmux session — the
    /// source is a skillbox-overlay plan directory. Plan tabs are populated by
    /// stat'ing sibling files, and tab content is read straight from disk.
    pub(crate) fn open_plan_viewer(&mut self, schema_path: String, slug: String) {
        let path = std::path::PathBuf::from(&schema_path);
        let Some(parent) = path.parent() else {
            self.set_message("plan path has no parent directory");
            return;
        };
        let cwd = parent.to_string_lossy().into_owned();
        let siblings = swimmers::session::artifacts::list_plan_siblings(&schema_path);
        let session_id = format!("plan::{schema_path}");
        let (source, artifact_error) = match std::fs::read_to_string(&path) {
            Ok(source) => (Some(source), None),
            Err(err) => (None, Some(format!("read {}: {err}", path.display()))),
        };

        let plan_tabs = {
            let mut tabs = vec![DomainPlanTab::Schema];
            for name in &siblings {
                if let Some(tab) = DomainPlanTab::from_filename(name) {
                    if tab != DomainPlanTab::Schema {
                        tabs.push(tab);
                    }
                }
            }
            if tabs.len() > 1 {
                Some(tabs)
            } else {
                None
            }
        };

        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            session_id,
            slug,
            cwd,
            Some(schema_path),
            source,
            artifact_error,
            plan_tabs,
            true,
            BTreeMap::new(),
        ));
    }

    pub(crate) fn open_skill_atlas_viewer(&mut self, action: SkillPanelAction) {
        let source = skill_atlas_mermaid_source(self, &action);
        let plan_text = skill_atlas_plan_text(self, &action);
        let cwd = self
            .selected()
            .map(|entity| entity.session.cwd.clone())
            .unwrap_or_default();
        let title = skill_atlas_focus_title(&action);
        let mut inline_plan_files = BTreeMap::new();
        inline_plan_files.insert(DomainPlanTab::Plan, plan_text);
        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            format!("skill-atlas::{title}"),
            format!("skill atlas: {title}"),
            cwd,
            skill_atlas_focus_path(&action),
            Some(source),
            None,
            Some(vec![DomainPlanTab::Schema, DomainPlanTab::Plan]),
            true,
            inline_plan_files,
        ));
    }

    pub(crate) fn close_mermaid_viewer(&mut self) {
        self.fish_bowl_mode = FishBowlMode::Aquarium;
        self.mermaid_drag = None;
    }

    pub(crate) fn mermaid_viewer_mut(&mut self) -> Option<&mut MermaidViewerState> {
        match &mut self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => Some(viewer),
            FishBowlMode::Aquarium => None,
        }
    }

    pub(crate) fn switch_plan_tab(&mut self, tab: DomainPlanTab) {
        let is_valid = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                viewer
                    .plan_tabs
                    .as_ref()
                    .is_some_and(|tabs| tabs.contains(&tab))
                    && viewer.active_tab != tab
            }
            FishBowlMode::Aquarium => false,
        };
        if !is_valid {
            return;
        }

        if tab != DomainPlanTab::Schema {
            let (session_id, schema_path, disk_only, inline_content) = match &self.fish_bowl_mode {
                FishBowlMode::Mermaid(v) => (
                    v.session_id.clone(),
                    v.path.clone(),
                    v.disk_only,
                    v.inline_plan_files.get(&tab).cloned(),
                ),
                _ => return,
            };
            let result = if let Some(content) = inline_content {
                Ok(PlanFileResponse {
                    session_id: session_id.clone(),
                    name: tab.filename().to_string(),
                    content: Some(content),
                    error: None,
                })
            } else if disk_only {
                read_plan_file_from_disk(schema_path.as_deref(), tab.filename())
            } else {
                self.runtime
                    .block_on(self.client.fetch_plan_file(&session_id, tab.filename()))
            };
            let viewer = self.mermaid_viewer_mut().unwrap();
            viewer.active_tab = tab;
            viewer.plan_text_scroll = 0;
            viewer.plan_text_lines.clear();
            viewer.plan_text_cached_width = 0;
            match result {
                Ok(response) => {
                    viewer.plan_text_content = response.content;
                    if let Some(err) = response.error {
                        self.set_message(format!("artifact file: {err}"));
                    }
                }
                Err(err) => {
                    viewer.plan_text_content = None;
                    self.set_message(format!("artifact file fetch failed: {err}"));
                }
            }
        } else {
            let viewer = self.mermaid_viewer_mut().unwrap();
            viewer.active_tab = DomainPlanTab::Schema;
            viewer.plan_text_content = None;
            viewer.plan_text_lines.clear();
            viewer.plan_text_scroll = 0;
            viewer.plan_text_cached_width = 0;
        }
    }

    pub(crate) fn cycle_plan_tab(&mut self, delta: isize) {
        let (tabs, current) = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => match &viewer.plan_tabs {
                Some(tabs) => (tabs.clone(), viewer.active_tab),
                None => return,
            },
            FishBowlMode::Aquarium => return,
        };
        let current_idx = tabs.iter().position(|t| *t == current).unwrap_or(0);
        let next_idx = (current_idx as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        let next_tab = tabs[next_idx];
        self.switch_plan_tab(next_tab);
    }

    pub(crate) fn scroll_plan_text(&mut self, delta: isize) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        let max = viewer.plan_text_lines.len().saturating_sub(1);
        viewer.plan_text_scroll =
            (viewer.plan_text_scroll as isize + delta).clamp(0, max as isize) as usize;
    }

    pub(crate) fn scroll_plan_text_page(&mut self, delta: isize) {
        let page_size = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                viewer.content_rect.map(|r| r.height as isize).unwrap_or(20)
            }
            _ => return,
        };
        self.scroll_plan_text(delta * page_size);
    }

    pub(crate) fn pan_mermaid_viewer(&mut self, dx: f32, dy: f32) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.center_x += dx;
        viewer.center_y += dy;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn zoom_mermaid_viewer(
        &mut self,
        delta_percent: i16,
        pointer: Option<(u16, u16)>,
        content_rect: Rect,
    ) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        let old_zoom = viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM);
        if mermaid_is_er_viewer(viewer) {
            let direction = delta_percent.signum() as i8;
            if direction == 0 {
                return;
            }
            let new_zoom = mermaid_er_zoom_step(old_zoom, direction);
            if (new_zoom - old_zoom).abs() < f32::EPSILON {
                return;
            }
            viewer.zoom = new_zoom;
            viewer.center_x = 0.0;
            viewer.center_y = 0.0;
            viewer.invalidate_viewport_cache();
            return;
        }
        let old_percent = mermaid_zoom_percent(old_zoom);
        let min_percent = mermaid_zoom_percent(MERMAID_MIN_ZOOM);
        let max_percent = mermaid_zoom_percent(MERMAID_MAX_ZOOM);
        let new_percent = (old_percent + delta_percent).clamp(min_percent, max_percent);
        let new_zoom = new_percent as f32 / 100.0;
        if (new_zoom - old_zoom).abs() < f32::EPSILON {
            return;
        }

        if let Some((column, row)) = pointer {
            let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
            let base_scale = mermaid_fit_scale(
                viewer.diagram_width,
                viewer.diagram_height,
                sample_width as f32,
                sample_height as f32,
            );
            let old_scale = base_scale * old_zoom;
            let new_scale = base_scale * new_zoom;
            if old_scale > 0.0 && new_scale > 0.0 {
                let anchor_x = (column.saturating_sub(content_rect.x) as f32) * 2.0;
                let anchor_y = (row.saturating_sub(content_rect.y) as f32) * 4.0;
                let dx = anchor_x - sample_width as f32 / 2.0;
                let dy = anchor_y - sample_height as f32 / 2.0;
                let diagram_x = viewer.center_x + dx / old_scale;
                let diagram_y = viewer.center_y + dy / old_scale;
                viewer.center_x = diagram_x - dx / new_scale;
                viewer.center_y = diagram_y - dy / new_scale;
            }
        }

        viewer.zoom = new_zoom;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn reset_mermaid_viewer_fit(&mut self) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.zoom = 1.0;
        viewer.center_x = 0.0;
        viewer.center_y = 0.0;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn cycle_mermaid_focus(&mut self, content_rect: Rect, direction: i8) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };

        let targets = match mermaid_visible_focus_targets(viewer, content_rect) {
            Ok(targets) => targets,
            Err(err) => {
                viewer.render_error = Some(err);
                viewer.focused_source_index = None;
                viewer.focus_status = Some("no semantic targets".to_string());
                return;
            }
        };

        if targets.is_empty() {
            viewer.focused_source_index = None;
            viewer.focus_status = Some("no semantic targets".to_string());
            return;
        }

        let current_index = viewer.focused_source_index.and_then(|source_index| {
            targets
                .iter()
                .position(|target| target.source_index == source_index)
        });
        let next_index = match (current_index, direction.is_negative()) {
            (Some(index), false) => (index + 1) % targets.len(),
            (Some(index), true) => index.checked_sub(1).unwrap_or(targets.len() - 1),
            (None, false) => 0,
            (None, true) => targets.len() - 1,
        };
        let target = &targets[next_index];
        Self::apply_mermaid_focus_target(viewer, target);
    }

    pub(crate) fn focus_next_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, 1);
    }

    pub(crate) fn focus_previous_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, -1);
    }

    fn apply_mermaid_focus_target(viewer: &mut MermaidViewerState, target: &MermaidFocusTarget) {
        viewer.focused_source_index = Some(target.source_index);
        viewer.focus_status = Some(format!("focus {}", target.text));

        let recenter_x = (viewer.center_x - target.diagram_x).abs() > f32::EPSILON;
        let recenter_y = (viewer.center_y - target.diagram_y).abs() > f32::EPSILON;
        viewer.center_x = target.diagram_x;
        viewer.center_y = target.diagram_y;
        if recenter_x || recenter_y {
            viewer.invalidate_viewport_cache();
        }
    }

    pub(crate) fn clear_mermaid_focus(&mut self) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        if viewer.focused_source_index.is_none() {
            return false;
        }
        viewer.focused_source_index = None;
        viewer.focus_status = None;
        true
    }

    pub(crate) fn handle_mermaid_mouse_down(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let back_rect = viewer.back_rect.unwrap_or(Rect {
            x: field.x,
            y: field.y,
            width: display_width(MERMAID_BACK_LABEL),
            height: 1,
        });
        if back_rect.contains(mouse.column, mouse.row) {
            self.close_mermaid_viewer();
            return true;
        }

        // Check tab clicks
        let clicked_tab = viewer
            .tab_rects
            .iter()
            .find(|(_, rect)| rect.contains(mouse.column, mouse.row))
            .map(|(tab, _)| *tab);
        if let Some(tab) = clicked_tab {
            self.switch_plan_tab(tab);
            return true;
        }

        let viewer = self.mermaid_viewer_mut().unwrap();
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if content_rect.contains(mouse.column, mouse.row) {
            match mermaid_visible_focus_targets(viewer, content_rect) {
                Ok(targets) => {
                    if let Some(target) = targets
                        .iter()
                        .find(|target| target.hitbox.contains(mouse.column, mouse.row))
                    {
                        Self::apply_mermaid_focus_target(viewer, target);
                        self.mermaid_drag = None;
                        return true;
                    }
                }
                Err(err) => {
                    viewer.render_error = Some(err);
                    return true;
                }
            }
            self.mermaid_drag = Some(MermaidDragState {
                start_column: mouse.column,
                start_row: mouse.row,
                start_center_x: viewer.center_x,
                start_center_y: viewer.center_y,
            });
            return true;
        }

        false
    }

    pub(crate) fn handle_mermaid_mouse_drag(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(drag) = self.mermaid_drag else {
            return false;
        };
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
        let scale = mermaid_fit_scale(
            viewer.diagram_width,
            viewer.diagram_height,
            sample_width as f32,
            sample_height as f32,
        ) * viewer.zoom.max(MERMAID_MIN_ZOOM);
        if scale <= 0.0 {
            return false;
        }
        let dx = (mouse.column as i32 - drag.start_column as i32) as f32 * 2.0;
        let dy = (mouse.row as i32 - drag.start_row as i32) as f32 * 4.0;
        viewer.center_x = drag.start_center_x - dx / scale;
        viewer.center_y = drag.start_center_y - dy / scale;
        viewer.invalidate_viewport_cache();
        true
    }

    pub(crate) fn handle_mermaid_mouse_up(&mut self) -> bool {
        let active = self.mermaid_drag.is_some();
        self.mermaid_drag = None;
        active
    }

    pub(crate) fn handle_mermaid_scroll(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
        direction: MermaidZoomDirection,
    ) -> bool {
        let (content_rect, is_text_tab) = {
            let Some(viewer) = self.mermaid_viewer_mut() else {
                return false;
            };
            let rect = viewer
                .content_rect
                .unwrap_or_else(|| mermaid_content_rect(field));
            (rect, viewer.active_tab != DomainPlanTab::Schema)
        };
        if !content_rect.contains(mouse.column, mouse.row) {
            return false;
        }

        // On text tabs, scroll text instead of zooming
        if is_text_tab {
            let delta: isize = match direction {
                MermaidZoomDirection::In => -3,
                MermaidZoomDirection::Out => 3,
            };
            self.scroll_plan_text(delta);
            return true;
        }

        let delta_percent = match direction {
            MermaidZoomDirection::In => MERMAID_SCROLL_ZOOM_STEP_PERCENT,
            MermaidZoomDirection::Out => -MERMAID_SCROLL_ZOOM_STEP_PERCENT,
        };
        self.zoom_mermaid_viewer(delta_percent, Some((mouse.column, mouse.row)), content_rect);
        true
    }

    pub(crate) fn start_split_drag(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let resized = self.resize_thought_panel(layout, x);
        self.split_drag_active = resized;
        resized
    }

    pub(crate) fn drag_split(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        if !self.split_drag_active {
            return false;
        }
        self.resize_thought_panel(layout, x)
    }

    pub(crate) fn stop_split_drag(&mut self) {
        self.split_drag_active = false;
    }

    pub(crate) fn resize_thought_panel(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let Some(ratio) = layout.thought_ratio_for_divider_x(x) else {
            return false;
        };
        self.thought_panel_ratio = ratio;
        true
    }

    pub(crate) fn handle_field_click(&mut self, x: u16, y: u16, field: Rect) {
        if self.initial_request.is_some() {
            return;
        }

        if let Some(picker) = &self.picker {
            let layout = picker_layout(picker, field);
            if layout.frame.contains(x, y) {
                if let Some(action) = picker_action_at(picker, &layout, x, y) {
                    self.handle_picker_action(action, field);
                }
                return;
            }
            self.close_picker();
            return;
        }

        if let Some(action) = skill_panel_action_at(self, field, x, y) {
            self.open_skill_atlas_viewer(action);
            return;
        }

        let tank_field = build_skill_panel(self, field).tank_field;
        if !tank_field.contains(x, y) {
            return;
        }

        let visible_entities = self.visible_entities();
        let hit = if self.uses_balls_scene(&visible_entities) {
            balls_theme_hit_test(&visible_entities, tank_field, x, y)
        } else {
            visible_entities
                .iter()
                .copied()
                .find(|entity| entity.screen_rect(tank_field).contains(x, y))
        }
        .map(|entity| {
            (
                entity.session.session_id.clone(),
                selected_label(Some(&entity.session.tmux_name)),
            )
        });

        if let Some((session_id, label)) = hit {
            self.select_and_open_session(session_id, label);
            return;
        }

        self.open_picker(x, y);
    }

    pub(crate) fn handle_picker_action(&mut self, action: PickerAction, field: Rect) {
        match action {
            PickerAction::Close => self.close_picker(),
            PickerAction::Up => self.picker_up(),
            PickerAction::ToggleManaged(managed_only) => {
                self.picker_set_managed_only(managed_only);
            }
            PickerAction::ActivateGroup(name) => {
                self.picker_set_group(name);
            }
            PickerAction::CycleGroupEditTarget => {
                if let Some(picker) = &mut self.picker {
                    if let Some(target) = picker.cycle_group_edit_target() {
                        self.set_message(format!("directory group target: {target}"));
                    }
                }
            }
            PickerAction::ToggleTool => {
                self.spawn_tool = self.spawn_tool.toggle();
                if let Some(picker) = &mut self.picker {
                    picker.spawn_tool = self.spawn_tool;
                }
            }
            PickerAction::ToggleLaunchTarget => {
                if let Some(picker) = &mut self.picker {
                    self.launch_target = picker.toggle_launch_target();
                }
            }
            PickerAction::ToggleBatchExcludeMode => {
                if let Some(picker) = &mut self.picker {
                    picker.batch_exclude_mode = !picker.batch_exclude_mode;
                }
            }
            PickerAction::BatchVisible => self.open_batch_initial_request_for_visible_entries(),
            PickerAction::ActivateCurrentPath => self.spawn_session_from_picker(field),
            PickerAction::ActivateEntry(index) => self.activate_picker_entry(index, field),
            PickerAction::ToggleBatchExclude(index) => {
                if let Some(picker) = &mut self.picker {
                    picker.toggle_batch_exclusion(index);
                }
            }
            PickerAction::StartRepoAction(index, kind) => {
                self.start_picker_repo_action(index, kind)
            }
        }
    }

    pub(crate) fn spawn_session_from_picker(&mut self, _field: Rect) {
        let Some(path) = self
            .picker
            .as_ref()
            .map(|picker| picker.current_path.clone())
        else {
            return;
        };
        let launch_target = self
            .picker
            .as_ref()
            .and_then(|picker| picker.launch_target.clone());
        self.launch_target = launch_target.clone();
        self.open_initial_request(path, launch_target);
    }

    pub(crate) fn activate_picker_entry(&mut self, index: usize, _field: Rect) {
        let Some((path, has_children, managed_only)) = self.picker.as_ref().and_then(|picker| {
            Some((
                picker.path_for_entry(index)?,
                picker.entry_at(index)?.has_children,
                picker.managed_only,
            ))
        }) else {
            return;
        };

        if has_children {
            self.picker_reload(Some(path), managed_only, None);
        } else {
            let launch_target = self
                .picker
                .as_ref()
                .and_then(|picker| picker.launch_target.clone());
            self.launch_target = launch_target.clone();
            self.open_initial_request(path, launch_target);
        }
    }

    pub(crate) fn render(&mut self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.clear();

        if renderer.width() < MIN_WIDTH || renderer.height() < MIN_HEIGHT {
            render_too_small(renderer);
            return;
        }

        self.render_shell(renderer, layout);

        if matches!(self.fish_bowl_mode, FishBowlMode::Aquarium) {
            self.render_aquarium(renderer, layout.overview_field);
        } else if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
            render_mermaid_viewer(renderer, layout.overview_field, viewer);
        }

        self.render_overlays(renderer, layout);
        render_footer(self, renderer, layout.footer_start_y);
    }

    fn render_shell(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.draw_box(
            frame_rect(renderer.width(), renderer.height()),
            Color::DarkGrey,
        );
        renderer.draw_text(2, 1, "swimmers tui", Color::Cyan);
        self.render_theme_toggle(renderer);
        self.render_header_right(renderer);
        render_header_filter_strip(self, renderer, renderer.width());
        renderer.draw_box(layout.workspace_box, Color::DarkGrey);
        self.render_thought_split(renderer, layout);
    }

    fn render_theme_toggle(&self, renderer: &mut Renderer) {
        let toggle_rect = self.sprite_theme_rect(renderer.width());
        let mut toggle_x = toggle_rect.x;
        for theme in SpriteTheme::override_options() {
            let label = format!("[{}]", SpriteTheme::override_label(theme));
            let color = if self.sprite_theme_override == theme {
                Color::Cyan
            } else {
                Color::DarkGrey
            };
            renderer.draw_text(toggle_x, toggle_rect.y, &label, color);
            toggle_x = toggle_x.saturating_add(display_width(&label) + 1);
        }
    }

    fn render_header_right(&self, renderer: &mut Renderer) {
        let max_right_width = renderer.width().saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let right_x = renderer
            .width()
            .saturating_sub(display_width(&right_text))
            .saturating_sub(2);
        renderer.draw_text(right_x, 1, &right_text, Color::DarkGrey);
    }

    fn render_thought_split(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        let (Some(thought_box), Some(thought_content)) =
            (layout.thought_box, layout.thought_content)
        else {
            return;
        };

        renderer.draw_box(thought_box, Color::DarkGrey);
        renderer.draw_box(layout.overview_box, Color::DarkGrey);
        if let Some(split_divider) = layout.split_divider {
            let divider_color = if self.split_drag_active {
                Color::Cyan
            } else {
                Color::DarkGrey
            };
            renderer.draw_vline(
                split_divider.x,
                split_divider.y,
                split_divider.height,
                ':',
                divider_color,
            );
        }
        render_thought_panel(
            self,
            renderer,
            thought_content,
            layout.thought_entry_capacity(),
        );
    }

    fn render_aquarium(&self, renderer: &mut Renderer, field: Rect) {
        let skill_panel = build_skill_panel(self, field);
        let tank_field = skill_panel.tank_field;
        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.render_empty_aquarium_message(renderer, tank_field);
        }

        if self.uses_balls_scene(&visible_entities) {
            render_balls_theme(
                renderer,
                tank_field,
                &visible_entities,
                self.selected_id.as_deref(),
                &self.repo_themes,
                self.tick,
            );
        } else {
            self.render_fish_scene(renderer, tank_field, visible_entities);
        }

        render_skill_panel(self, renderer, field);
        self.render_api_stale_banner(renderer, tank_field);
    }

    fn render_empty_aquarium_message(&self, renderer: &mut Renderer, field: Rect) {
        let empty = if self.entities.is_empty() {
            "no tmux sessions found - press r after starting one"
        } else if self.thought_filter.is_active() {
            "no swimmers match filters"
        } else {
            "no tmux sessions found - press r after starting one"
        };
        let x = field
            .x
            .saturating_add(field.width.saturating_sub(empty.len() as u16) / 2);
        let y = field.y + field.height / 2;
        renderer.draw_text(x, y, empty, Color::DarkGrey);
    }

    fn render_fish_scene(
        &self,
        renderer: &mut Renderer,
        field: Rect,
        visible_entities: Vec<&SessionEntity>,
    ) {
        render_aquarium_background(renderer, field, self.tick);

        for entity in visible_entities {
            let rect = entity.screen_rect(field);
            let selected = self
                .selected_id
                .as_ref()
                .map(|selected| *selected == entity.session.session_id)
                .unwrap_or(false);
            render_entity_with_theme(
                renderer,
                entity,
                rect,
                selected,
                self.tick,
                &self.repo_themes,
                self.effective_sprite_theme_for_session(&entity.session),
            );
        }
    }

    fn render_api_stale_banner(&self, renderer: &mut Renderer, field: Rect) {
        if let Some(banner) = self.api_refresh_health.banner_text() {
            renderer.draw_text(
                field.x,
                field.y,
                &truncate_label(banner, field.width as usize),
                Color::Red,
            );
        }
    }

    fn render_overlays(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        if let Some(picker) = &self.picker {
            render_picker(renderer, picker, layout.overview_field);
        }
        if let Some(initial_request) = &self.initial_request {
            let group_context = self
                .group_input_targets
                .as_ref()
                .map(|targets| (targets.label.as_str(), targets.session_ids.len()));
            render_initial_request(
                renderer,
                initial_request,
                &self.voice_state,
                layout.overview_field,
                group_context,
            );
        }
        if let Some(editor) = &self.thought_config_editor {
            render_thought_config_editor(renderer, editor, layout.overview_field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_banner_shows_after_three_failures_and_clears_on_success() {
        let mut health = ApiRefreshHealth::default();
        assert!(health.banner_text().is_none());

        health.record_failure();
        assert!(health.banner_text().is_none());
        health.record_failure();
        assert!(health.banner_text().is_none());
        health.record_failure();
        assert_eq!(health.banner_text(), Some(API_STALE_BANNER_TEXT));

        health.record_success();
        assert!(health.banner_text().is_none());
    }
}

/// Load the list of overlay-configured domain plans from disk, mapped into
/// the TUI's `PlanPanelEntry` shape.
pub(crate) fn load_overlay_plan_entries() -> Vec<PlanPanelEntry> {
    let Some(overlay) = swimmers::session::overlay::default_overlay() else {
        return Vec::new();
    };
    overlay
        .list_all_plans()
        .into_iter()
        .map(|entry| PlanPanelEntry {
            slug: entry.slug,
            client_label: entry.client_label,
            kind: entry.kind.to_string(),
            schema_path: entry.schema_path.to_string_lossy().into_owned(),
        })
        .collect()
}

/// Read a plan sibling file from disk, matching the shape of the server's
/// `fetch_plan_file` response so disk-backed and session-backed viewers can
/// share the tab-switching code path.
pub(crate) fn read_plan_file_from_disk(
    schema_path: Option<&str>,
    filename: &str,
) -> Result<PlanFileResponse, String> {
    let Some(schema_path) = schema_path else {
        return Err("plan viewer has no schema path".to_string());
    };
    let Some(dir) = std::path::Path::new(schema_path).parent() else {
        return Err("plan schema path has no parent".to_string());
    };
    let response = PlanFileResponse {
        session_id: format!("plan::{schema_path}"),
        name: filename.to_string(),
        content: None,
        error: None,
    };

    if !swimmers::session::artifacts::VIEWER_TEXT_FILENAMES.contains(&filename) {
        return Ok(PlanFileResponse {
            error: Some(format!("artifact file name not allowed: {filename}")),
            ..response
        });
    }

    let cwd = dir.to_string_lossy();
    let Some(target) =
        swimmers::session::artifacts::resolve_viewer_text_path(&cwd, Some(schema_path), filename)
    else {
        return Ok(PlanFileResponse {
            error: Some(format!("artifact file unavailable: {filename}")),
            ..response
        });
    };

    match std::fs::read_to_string(&target) {
        Ok(content) => Ok(PlanFileResponse {
            content: Some(content),
            ..response
        }),
        Err(err) => Ok(PlanFileResponse {
            error: Some(format!("read {}: {err}", target.display())),
            ..response
        }),
    }
}
