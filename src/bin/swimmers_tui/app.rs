use super::*;
use swimmers::openrouter_models::should_rotate_openrouter_model;

#[path = "mermaid_viewer.rs"]
mod mermaid_viewer;
#[cfg(test)]
pub(crate) use mermaid_viewer::read_plan_file_from_disk;
use mermaid_viewer::MermaidCacheEntry;

// Once the school is large, the O(n^2) pairwise collision pass gets expensive.
// 50 entities is where we start seeing frame-time spikes on laptops, so we cap
// checks per entity above that point and distribute checks across frames.
const COLLISION_THROTTLE_ENTITY_THRESHOLD: usize = 50;
const COLLISION_THROTTLE_PAIR_BUDGET: usize = 24;
const API_FAILURE_BANNER_THRESHOLD: u8 = 3;
const API_STALE_BANNER_TEXT: &str = "API disconnected - showing stale data";
const ATTENTION_GROUP_LABEL: &str = "[attention group]";
const ATTENTION_GROUP_MAX_SESSIONS: usize = 6;
const ATTENTION_GROUP_SIZE_ENV: &str = "SWIMMERS_ATTENTION_GROUP_SIZE";
const ATTENTION_GROUP_LAYOUT_ENV: &str = "SWIMMERS_ATTENTION_GROUP_LAYOUT";
const ATTENTION_GROUP_INCLUDE_UNNUMBERED_ENV: &str = "SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED";

fn sprite_theme_option_width(theme: Option<SpriteTheme>) -> u16 {
    let label = format!("[{}]", SpriteTheme::override_label(theme));
    display_width(&label)
}

pub(crate) struct RefreshResult {
    pub(crate) sessions: Result<Vec<SessionSummary>, String>,
    pub(crate) mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
    pub(crate) session_skills: Vec<(String, Result<SessionSkillListResponse, String>)>,
    pub(crate) backend_health: Result<BackendHealthResponse, String>,
    pub(crate) native_status: Option<Result<NativeDesktopStatusResponse, String>>,
    pub(crate) daemon_defaults_status: Option<DaemonDefaultsStatus>,
    pub(crate) show_success_message: bool,
    pub(crate) force_asset_refresh: bool,
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

enum PickerActivationPlan {
    OpenInitialRequest {
        path: String,
        launch_target: Option<String>,
    },
    ReloadDirectory {
        path: String,
        managed_only: bool,
    },
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

fn concise_health_detail(value: Option<&String>) -> Option<String> {
    value
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| truncate_label(text, 64))
}

fn backend_health_warning_text(health: &BackendHealthResponse) -> Option<String> {
    let persistence = &health.persistence;
    if !persistence.available {
        return Some("persistence unavailable".to_string());
    }
    if !persistence.ok {
        let operation = persistence
            .last_failed_operation
            .as_deref()
            .unwrap_or("write");
        let detail = concise_health_detail(persistence.last_error.as_ref())
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        return Some(format!("persistence degraded: {operation}{detail}"));
    }

    let thought = &health.thought_bridge;
    match thought.status.as_str() {
        "healthy" | "" => None,
        "degraded" => {
            let detail = concise_health_detail(thought.last_backend_error.as_ref())
                .or_else(|| concise_health_detail(thought.last_error.as_ref()))
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            Some(format!("thought bridge degraded{detail}"))
        }
        "unhealthy" => {
            let detail = concise_health_detail(
                thought
                    .shutdown_reason
                    .as_ref()
                    .or(thought.last_error.as_ref()),
            )
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
            Some(format!("thought bridge unhealthy{detail}"))
        }
        other => Some(format!("thought bridge {other}")),
    }
}

fn dependency_degradation_line(deps: &BackendDependencyLedger) -> Option<String> {
    let checks: &[(&str, &BackendDependencySnapshot)] = &[
        ("tmux capture", &deps.tmux_capture),
        ("native scripts", &deps.native_scripts),
        ("remote targets", &deps.remote_targets),
    ];
    let mut parts = Vec::new();
    for &(label, snap) in checks {
        let tag = match snap.status.as_str() {
            "unavailable" => "unavailable",
            "degraded" => "degraded",
            _ => continue,
        };
        let detail = concise_health_detail(snap.last_error.as_ref())
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        parts.push(format!("{label} {tag}{detail}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  "))
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

struct BatchCreateSuccess {
    session: SessionSummary,
    repo_theme: Option<RepoTheme>,
}

struct BatchCreatePartition {
    total: usize,
    successes: Vec<BatchCreateSuccess>,
    first_error: Option<String>,
}

struct AppliedBatchCreate {
    success_count: usize,
    last_tmux_name: Option<String>,
    last_session_id: Option<String>,
}

struct BatchCreateCompletion {
    total: usize,
    success_count: usize,
    last_tmux_name: Option<String>,
    last_session_id: Option<String>,
    first_error: Option<String>,
}

impl BatchCreateCompletion {
    fn should_close_picker(&self) -> bool {
        self.success_count > 0
    }

    fn should_show_batch_thoughts(&self) -> bool {
        self.success_count > 1
    }

    fn message(self) -> String {
        if self.success_count == 0 {
            return self
                .first_error
                .unwrap_or_else(|| "batch create failed".to_string());
        }

        if self.success_count != self.total {
            return format!(
                "created {}/{}; {}",
                self.success_count,
                self.total,
                self.first_error
                    .unwrap_or_else(|| "some sessions failed".to_string())
            );
        }

        match self.last_tmux_name {
            Some(name) if self.success_count == 1 => format!("created {name}"),
            _ => format!("created {} sessions", self.success_count),
        }
    }
}

fn partition_batch_create_results(response: CreateSessionsBatchResponse) -> BatchCreatePartition {
    let total = response.results.len();
    let mut successes = Vec::new();
    let mut first_error = None;

    for result in response.results {
        match (result.ok, result.session) {
            (true, Some(session)) => successes.push(BatchCreateSuccess {
                session,
                repo_theme: result.repo_theme,
            }),
            (false, _) if first_error.is_none() => {
                first_error = Some(batch_create_error_message(&result.cwd, result.error));
            }
            _ => {}
        }
    }

    BatchCreatePartition {
        total,
        successes,
        first_error,
    }
}

fn batch_create_error_message(cwd: &str, error: Option<ErrorResponse>) -> String {
    let detail = error
        .and_then(|error| error.message)
        .unwrap_or_else(|| "unknown error".to_string());
    format!("{}: {detail}", shorten_path(cwd, 32))
}

pub(crate) enum PendingInteractionResult {
    OpenPicker {
        x: u16,
        y: u16,
        response: Result<DirListResponse, String>,
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
    AdoptSession {
        field: Rect,
        response: Result<AdoptSessionResponse, String>,
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
    pub(crate) backend_health: Option<BackendHealthResponse>,
    pub(crate) attention_group_session_ids: Vec<String>,
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
    pub(crate) show_help: bool,
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
    pub(crate) pending_picker_repo_search:
        Option<oneshot::Receiver<Result<DirRepoSearchResponse, String>>>,
    pub(crate) pending_selection_publication:
        Option<oneshot::Receiver<PendingSelectionPublicationResult>>,
    pub(crate) queued_selection_publication: Option<(Option<String>, bool)>,
    embedded_shutdown: Option<swimmers::startup::EmbeddedTuiShutdown>,
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
            backend_health: None,
            attention_group_session_ids: Vec::new(),
            daemon_defaults_status: DaemonDefaultsStatus::Unknown,
            thought_config_editor: None,
            picker: None,
            spawn_tool: SpawnTool::Grok,
            launch_target: None,
            initial_request: None,
            group_input_targets: None,
            initial_request_generation: 0,
            voice_state: default_ui_state(),
            voice_recording: None,
            show_help: false,
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
            pending_picker_repo_search: None,
            pending_selection_publication: None,
            queued_selection_publication: None,
            embedded_shutdown: None,
        }
    }

    pub(crate) fn set_embedded_shutdown(
        &mut self,
        shutdown: swimmers::startup::EmbeddedTuiShutdown,
    ) {
        self.embedded_shutdown = Some(shutdown);
    }

    pub(crate) fn has_embedded_shutdown(&self) -> bool {
        self.embedded_shutdown.is_some()
    }

    pub(crate) fn shutdown_embedded(&mut self) -> anyhow::Result<()> {
        let Some(shutdown) = self.embedded_shutdown.take() else {
            return Ok(());
        };
        self.runtime
            .block_on(swimmers::startup::finalize_embedded_tui_shutdown(shutdown))
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
        let max_sessions = attention_group_max_sessions();
        let include_unnumbered_sessions = attention_group_include_unnumbered_sessions();
        let layout = attention_group_layout();
        if show_progress {
            self.set_message("opening attention group...");
        }
        self.runtime.spawn(async move {
            let response = client
                .open_attention_group(
                    max_sessions,
                    current_session_ids,
                    focus,
                    include_unnumbered_sessions,
                    layout,
                )
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

    fn rebuild_repo_themes_from_cache(&mut self) {
        let mut next = HashMap::new();
        for entry in self.session_repo_theme_cache.values() {
            if let Some((theme_id, theme)) = &entry.resolved {
                next.insert(theme_id.clone(), theme.clone());
            }
        }
        self.repo_themes = next;
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
            if let Ok(health) = self.runtime.block_on(self.client.fetch_backend_health()) {
                self.backend_health = Some(health);
            }
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
            let backend_health = client.fetch_backend_health().await;

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
                backend_health,
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

    fn start_picker_repo_search(&mut self) {
        if self.picker.is_none() || self.pending_picker_repo_search.is_some() {
            return;
        }

        let client = Arc::clone(&self.client);
        let (tx, rx) = oneshot::channel();
        self.pending_picker_repo_search = Some(rx);
        self.runtime.spawn(async move {
            let response = client.list_repo_dirs().await;
            let _ = tx.send(response);
        });
    }

    pub(crate) fn poll_pending_picker_repo_search(&mut self) {
        let Some(rx) = &mut self.pending_picker_repo_search else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_picker_repo_search = None;
                return;
            }
        };

        self.pending_picker_repo_search = None;
        let Some(picker) = &mut self.picker else {
            return;
        };

        match result {
            Ok(response) => {
                picker.set_repo_search_entries(response.entries);
                picker.sync_theme_colors(&mut self.repo_themes);
            }
            Err(err) => self.set_message(format!("repository search unavailable: {err}")),
        }
    }

    fn apply_pending_interaction_result(&mut self, result: PendingInteractionResult) {
        match result {
            PendingInteractionResult::OpenPicker { x, y, response } => {
                self.apply_open_picker_result(x, y, response);
            }
            PendingInteractionResult::ReloadPicker {
                managed_only,
                group,
                preserve_selection,
                response,
            } => self.apply_reload_picker_result(managed_only, group, preserve_selection, response),
            PendingInteractionResult::StartRepoAction {
                repo_label,
                kind,
                reload_path,
                managed_only,
                response,
            } => self.apply_start_repo_action_result(
                repo_label,
                kind,
                reload_path,
                managed_only,
                response,
            ),
            PendingInteractionResult::CreateSession { field, response } => {
                self.apply_create_session_result(field, response);
            }
            PendingInteractionResult::AdoptSession { field, response } => {
                self.apply_adopt_session_result(field, response);
            }
            PendingInteractionResult::CreateSessionsBatch { field, response } => {
                self.apply_create_sessions_batch_result(field, response);
            }
            PendingInteractionResult::SendGroupInput { response } => {
                self.apply_group_input_result(response);
            }
            PendingInteractionResult::OpenSession { label, response } => {
                self.apply_open_session_result(label, response);
            }
            PendingInteractionResult::OpenAttentionGroup { focus, response } => {
                self.apply_attention_group_result(focus, response);
            }
            PendingInteractionResult::ToggleNativeApp { next_app, response } => {
                self.apply_toggle_native_app_result(next_app, response);
            }
            PendingInteractionResult::ToggleGhosttyMode {
                next_mode,
                response,
            } => self.apply_toggle_ghostty_mode_result(next_mode, response),
            PendingInteractionResult::OpenThoughtConfig { response } => {
                self.apply_open_thought_config_result(response);
            }
            PendingInteractionResult::TestThoughtConfig { outcome }
            | PendingInteractionResult::SaveThoughtConfig { outcome } => {
                self.apply_thought_config_action_outcome(outcome);
            }
            PendingInteractionResult::VoiceTranscription {
                generation,
                response,
            } => self.apply_voice_transcription_result(generation, response),
        }
    }

    fn apply_open_picker_result(
        &mut self,
        x: u16,
        y: u16,
        response: Result<DirListResponse, String>,
    ) {
        match response {
            Ok(response) => {
                let mut picker = PickerState::new(
                    x,
                    y,
                    response,
                    true,
                    self.spawn_tool,
                    self.launch_target.clone(),
                );
                self.launch_target = picker.launch_target.clone();
                picker.sync_theme_colors(&mut self.repo_themes);
                self.picker = Some(picker);
                self.last_picker_refresh = Some(Instant::now());
                self.start_picker_repo_search();
            }
            Err(err) => {
                self.set_message(err);
                self.picker = None;
                self.last_picker_refresh = None;
            }
        }
    }

    fn apply_reload_picker_result(
        &mut self,
        managed_only: bool,
        group: Option<String>,
        preserve_selection: bool,
        response: Result<DirListResponse, String>,
    ) {
        match response {
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
        }
    }

    fn apply_start_repo_action_result(
        &mut self,
        repo_label: String,
        kind: RepoActionKind,
        reload_path: String,
        managed_only: bool,
        response: Result<DirRepoActionResponse, String>,
    ) {
        match response {
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
        }
    }

    fn apply_create_session_result(
        &mut self,
        field: Rect,
        response: Result<CreateSessionResponse, String>,
    ) {
        match response {
            Ok(response) => {
                let tmux_name = self.remember_and_upsert_pending_session(
                    field,
                    response.session,
                    response.repo_theme,
                );
                self.close_picker();
                self.set_message(format!("created {tmux_name}"));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn apply_adopt_session_result(
        &mut self,
        field: Rect,
        response: Result<AdoptSessionResponse, String>,
    ) {
        match response {
            Ok(response) => {
                let reused_session_id = response.reused_session_id;
                let tmux_name = self.remember_and_upsert_pending_session(
                    field,
                    response.session,
                    response.repo_theme,
                );
                if reused_session_id {
                    self.set_message(format!("reattached {tmux_name}"));
                } else {
                    self.set_message(format!("adopted {tmux_name}"));
                }
            }
            Err(err) => self.set_message(err),
        }
    }

    fn remember_and_upsert_pending_session(
        &mut self,
        field: Rect,
        session: SessionSummary,
        repo_theme: Option<RepoTheme>,
    ) -> String {
        let session_id = session.session_id.clone();
        let tmux_name = session.tmux_name.clone();
        self.remember_repo_theme(&session, repo_theme);
        self.upsert_session(session, field);
        self.selected_id = Some(session_id);
        self.reconcile_selection();
        self.sync_selection_publication();
        tmux_name
    }

    fn apply_create_sessions_batch_result(
        &mut self,
        field: Rect,
        response: Result<CreateSessionsBatchResponse, String>,
    ) {
        match response {
            Ok(response) => self.apply_batch_create_response(field, response),
            Err(err) => self.set_message(err),
        }
    }

    fn apply_group_input_result(&mut self, response: Result<SessionGroupInputResponse, String>) {
        match response {
            Ok(response) => self.apply_group_input_response(response),
            Err(err) => self.set_message(err),
        }
    }

    fn apply_open_session_result(
        &mut self,
        label: String,
        response: Result<NativeDesktopOpenResponse, String>,
    ) {
        match response {
            Ok(response) => {
                self.set_message(format!("{} {}", response.status, label));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn apply_attention_group_result(
        &mut self,
        focus: bool,
        response: Result<NativeAttentionGroupOpenResponse, String>,
    ) {
        match response {
            Ok(response) => {
                let previous = self.attention_group_session_ids.clone();
                self.attention_group_session_ids = response.session_ids.clone();
                if focus || previous != response.session_ids {
                    let mut message = format!(
                        "{} attention group: {} sessions",
                        response.status, response.session_count
                    );
                    if focus && !response.focused {
                        if let Some(command) = response.attach_command.as_deref() {
                            message.push_str(" | ");
                            message.push_str(command);
                        }
                    }
                    self.set_message(message);
                }
            }
            Err(err) => {
                if !focus {
                    self.attention_group_session_ids.clear();
                }
                self.set_message(err);
            }
        }
    }

    fn apply_toggle_native_app_result(
        &mut self,
        next_app: NativeDesktopApp,
        response: Result<NativeDesktopStatusResponse, String>,
    ) {
        match response {
            Ok(status) => {
                self.native_status = Some(status.clone());
                self.set_message(Self::native_status_message(&status, next_app));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn apply_toggle_ghostty_mode_result(
        &mut self,
        next_mode: GhosttyOpenMode,
        response: Result<NativeDesktopStatusResponse, String>,
    ) {
        match response {
            Ok(status) => {
                self.native_status = Some(status);
                self.set_message(format!("Ghostty placement: {}", next_mode.display_label()));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn apply_open_thought_config_result(
        &mut self,
        response: Result<ThoughtConfigResponse, String>,
    ) {
        match response {
            Ok(response) => {
                self.daemon_defaults_status =
                    DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref());
                self.thought_config_editor = Some(ThoughtConfigEditorState::new(
                    response.config,
                    response.daemon_defaults,
                ));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn apply_thought_config_action_outcome(&mut self, outcome: ThoughtConfigActionOutcome) {
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

    fn apply_voice_transcription_result(
        &mut self,
        generation: u64,
        response: Result<String, String>,
    ) {
        match response {
            Ok(transcript) => self.insert_voice_transcript(generation, transcript),
            Err(err) => {
                self.voice_state = VoiceUiState::Failed(err.clone());
                self.set_message(err);
            }
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
        if let Ok(health) = result.backend_health {
            self.backend_health = Some(health);
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
        self.pending_picker_repo_search = None;
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
            let response = client.list_dirs(None, true, None).await;
            let _ = tx.send(PendingInteractionResult::OpenPicker { x, y, response });
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
        if let Some(plan) = self
            .picker
            .as_ref()
            .and_then(|picker| picker_managed_only_reload_plan(picker, managed_only))
        {
            self.picker_reload(plan.path, plan.managed_only, plan.group);
        }
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
        let Some(plan) = self
            .picker
            .as_ref()
            .and_then(|picker| picker_group_update_plan(picker, mode))
        else {
            self.set_message("select a directory entry and group target first");
            return;
        };

        if !plan.delta.has_changes() {
            self.set_message("no directory group change selected");
            return;
        }

        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let PickerGroupUpdatePlan {
            path,
            entry_label,
            delta,
            reload_path,
            managed_only,
            group,
        } = plan;
        self.set_message(format!("updating groups for {entry_label}..."));
        self.runtime.spawn(async move {
            let response = match client
                .update_dir_group_memberships(&path, delta.add, delta.remove)
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
        self.dispatch_picker_activation(self.selected_picker_activation());
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

    pub(crate) fn adopt_tmux_session(
        &mut self,
        tmux_name: String,
        session_id: Option<String>,
        field: Rect,
    ) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let message_target = tmux_name.clone();
        let session_id_for_call = session_id.clone();
        if session_id.is_some() {
            self.set_message(format!("reattaching {message_target}..."));
        } else {
            self.set_message(format!("adopting {message_target}..."));
        }
        self.runtime.spawn(async move {
            let response = client
                .adopt_session(&tmux_name, session_id_for_call.as_deref())
                .await;
            let _ = tx.send(PendingInteractionResult::AdoptSession { field, response });
        });
    }

    pub(crate) fn reattach_selected_tmux_session(&mut self, field: Rect) {
        let Some(session) = self.selected().map(|entity| entity.session.clone()) else {
            self.set_message("no session selected");
            return;
        };
        if !session.is_stale {
            self.set_message("selected session is already live");
            return;
        }
        if session.tmux_name.is_empty() {
            self.set_message("selected session has no tmux name");
            return;
        }

        self.adopt_tmux_session(session.tmux_name, Some(session.session_id), field);
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
        let partition = partition_batch_create_results(response);
        let completion = self.apply_batch_create_successes(field, partition);

        self.apply_batch_create_selection(completion.last_session_id.clone());
        self.finish_batch_create(completion);
    }

    fn apply_batch_create_successes(
        &mut self,
        field: Rect,
        partition: BatchCreatePartition,
    ) -> BatchCreateCompletion {
        let mut applied = AppliedBatchCreate {
            success_count: 0,
            last_tmux_name: None,
            last_session_id: None,
        };

        for success in partition.successes {
            applied.last_tmux_name = Some(success.session.tmux_name.clone());
            applied.last_session_id = Some(success.session.session_id.clone());
            self.remember_repo_theme(&success.session, success.repo_theme);
            self.upsert_session(success.session, field);
            applied.success_count += 1;
        }

        BatchCreateCompletion {
            total: partition.total,
            success_count: applied.success_count,
            last_tmux_name: applied.last_tmux_name,
            last_session_id: applied.last_session_id,
            first_error: partition.first_error,
        }
    }

    fn apply_batch_create_selection(&mut self, selected_session_id: Option<String>) {
        let Some(session_id) = selected_session_id else {
            return;
        };

        self.selected_id = Some(session_id);
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    fn finish_batch_create(&mut self, completion: BatchCreateCompletion) {
        if completion.should_close_picker() {
            self.close_picker();
        }
        if completion.should_show_batch_thoughts() {
            self.thought_group_by = ThoughtGroupBy::Batch;
            self.thought_show_all = true;
        }
        self.set_message(completion.message());
    }

    fn apply_group_input_response(&mut self, response: SessionGroupInputResponse) {
        let total = response.results.len();
        if response.skipped == 0 {
            self.close_initial_request();
            self.set_message(format!("sent to {} sessions", response.delivered));
        } else if response.delivered > 0 {
            self.close_initial_request();
            self.set_message(format!(
                "sent to {}/{}; {} skipped",
                response.delivered, total, response.skipped
            ));
        } else {
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
            self.set_message("missing session for commit grok launch");
            return;
        };

        match self.commit_launcher.launch(&session) {
            Ok(launch) => self.set_message(format!("commit grok: {}", launch.watch_command)),
            Err(err) => self.set_message(format!("failed to launch commit grok: {err}")),
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
        self.dispatch_picker_activation(self.current_path_picker_activation());
    }

    pub(crate) fn activate_picker_entry(&mut self, index: usize, _field: Rect) {
        self.dispatch_picker_activation(self.entry_picker_activation(index));
    }

    fn selected_picker_activation(&self) -> Option<PickerActivationPlan> {
        let picker = self.picker.as_ref()?;
        match picker.selection {
            PickerSelection::SpawnHere => self.current_path_picker_activation(),
            PickerSelection::Entry(index) => self.entry_picker_activation(index),
        }
    }

    fn current_path_picker_activation(&self) -> Option<PickerActivationPlan> {
        let picker = self.picker.as_ref()?;
        Some(PickerActivationPlan::OpenInitialRequest {
            path: picker.current_path.clone(),
            launch_target: picker.launch_target.clone(),
        })
    }

    fn entry_picker_activation(&self, index: usize) -> Option<PickerActivationPlan> {
        let picker = self.picker.as_ref()?;
        let path = picker.path_for_entry(index)?;
        if picker.entry_at(index)?.has_children {
            Some(PickerActivationPlan::ReloadDirectory {
                path,
                managed_only: picker.managed_only,
            })
        } else {
            Some(PickerActivationPlan::OpenInitialRequest {
                path,
                launch_target: picker.launch_target.clone(),
            })
        }
    }

    fn dispatch_picker_activation(&mut self, plan: Option<PickerActivationPlan>) {
        match plan {
            Some(PickerActivationPlan::OpenInitialRequest {
                path,
                launch_target,
            }) => {
                self.launch_target = launch_target.clone();
                self.open_initial_request(path, launch_target);
            }
            Some(PickerActivationPlan::ReloadDirectory { path, managed_only }) => {
                self.picker_reload(Some(path), managed_only, None);
            }
            None => {}
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
        let (text, color) = if !self.entities.is_empty() && self.thought_filter.is_active() {
            ("no swimmers match filters", Color::DarkGrey)
        } else if self.tmux_dependency_unavailable() {
            (
                "tmux unavailable - run swimmers config doctor",
                Color::Yellow,
            )
        } else {
            (
                "no tmux sessions found - press r after starting one",
                Color::DarkGrey,
            )
        };
        let x = field
            .x
            .saturating_add(field.width.saturating_sub(text.len() as u16) / 2);
        let y = field.y + field.height / 2;
        renderer.draw_text(x, y, text, color);
    }

    fn tmux_dependency_unavailable(&self) -> bool {
        self.backend_health
            .as_ref()
            .and_then(|h| h.dependencies.as_ref())
            .map(|deps| deps.tmux_discovery.status == "unavailable")
            .unwrap_or(false)
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
        let max_width = field.width as usize;
        let mut y = field.y;
        if let Some(banner) = self.api_refresh_health.banner_text() {
            renderer.draw_text(field.x, y, &truncate_label(banner, max_width), Color::Red);
            return;
        }
        if let Some(banner) = self
            .backend_health
            .as_ref()
            .and_then(backend_health_warning_text)
        {
            renderer.draw_text(
                field.x,
                y,
                &truncate_label(&banner, max_width),
                Color::Yellow,
            );
            y = y.saturating_add(1);
        }
        if let Some(dep_line) = self
            .backend_health
            .as_ref()
            .and_then(|h| h.dependencies.as_ref())
            .and_then(dependency_degradation_line)
        {
            if y < field.bottom() {
                renderer.draw_text(
                    field.x,
                    y,
                    &truncate_label(&dep_line, max_width),
                    Color::Yellow,
                );
            }
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
        if self.show_help {
            render_help_overlay(renderer, layout.overview_field);
        }
    }
}

fn attention_group_max_sessions() -> usize {
    std::env::var(ATTENTION_GROUP_SIZE_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(1, ATTENTION_GROUP_MAX_SESSIONS))
        .unwrap_or(ATTENTION_GROUP_MAX_SESSIONS)
}

fn attention_group_layout() -> AttentionGroupLayout {
    std::env::var(ATTENTION_GROUP_LAYOUT_ENV)
        .ok()
        .as_deref()
        .map(AttentionGroupLayout::from_env_value)
        .unwrap_or_default()
}

fn attention_group_include_unnumbered_sessions() -> bool {
    env_bool(ATTENTION_GROUP_INCLUDE_UNNUMBERED_ENV)
}

fn env_bool(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
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

    fn healthy_backend_health() -> BackendHealthResponse {
        BackendHealthResponse {
            status: "healthy".to_string(),
            thought_bridge: BackendThoughtBridgeHealth {
                status: "healthy".to_string(),
                ..BackendThoughtBridgeHealth::default()
            },
            persistence: BackendPersistenceHealth {
                available: true,
                ok: true,
                last_successful_operation: Some("save_sessions".to_string()),
                ..BackendPersistenceHealth::default()
            },
            dependencies: Some(healthy_dependency_ledger()),
        }
    }

    fn healthy_dependency_ledger() -> BackendDependencyLedger {
        BackendDependencyLedger {
            tmux_discovery: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            tmux_capture: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            native_scripts: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            remote_targets: BackendDependencySnapshot {
                status: "not_configured".to_string(),
                last_error: None,
            },
        }
    }

    #[test]
    fn backend_health_banner_covers_persistence_degraded_and_recovered() {
        let mut health = healthy_backend_health();
        assert!(backend_health_warning_text(&health).is_none());

        health.persistence.ok = false;
        health.persistence.consecutive_failures = 2;
        health.persistence.last_failed_operation = Some("save_thought".to_string());
        health.persistence.last_error = Some("permission denied".to_string());
        assert_eq!(
            backend_health_warning_text(&health),
            Some("persistence degraded: save_thought: permission denied".to_string())
        );

        health.persistence.ok = true;
        health.persistence.consecutive_failures = 0;
        health.persistence.last_error = None;
        assert!(backend_health_warning_text(&health).is_none());
    }

    #[test]
    fn backend_health_banner_covers_thought_degraded_and_unhealthy() {
        let mut health = healthy_backend_health();
        health.thought_bridge.status = "degraded".to_string();
        health.thought_bridge.last_backend_error = Some("model timeout".to_string());
        assert_eq!(
            backend_health_warning_text(&health),
            Some("thought bridge degraded: model timeout".to_string())
        );

        health.thought_bridge.status = "unhealthy".to_string();
        health.thought_bridge.last_backend_error = None;
        health.thought_bridge.shutdown_reason = Some("self fence tripped".to_string());
        assert_eq!(
            backend_health_warning_text(&health),
            Some("thought bridge unhealthy: self fence tripped".to_string())
        );

        health.thought_bridge.status = "healthy".to_string();
        health.thought_bridge.shutdown_reason = None;
        assert!(backend_health_warning_text(&health).is_none());
    }

    #[test]
    fn dependency_degradation_line_reports_degraded_and_unavailable() {
        let mut ledger = healthy_dependency_ledger();
        assert!(dependency_degradation_line(&ledger).is_none());

        ledger.native_scripts.status = "unavailable".to_string();
        ledger.native_scripts.last_error = Some("script not found".to_string());
        assert_eq!(
            dependency_degradation_line(&ledger),
            Some("native scripts unavailable: script not found".to_string())
        );

        ledger.remote_targets.status = "degraded".to_string();
        ledger.remote_targets.last_error = Some("timeout".to_string());
        let line = dependency_degradation_line(&ledger).unwrap();
        assert!(line.contains("native scripts unavailable"));
        assert!(line.contains("remote targets degraded: timeout"));
    }

    #[test]
    fn dependency_degradation_line_ignores_healthy_and_not_configured() {
        let mut ledger = healthy_dependency_ledger();
        ledger.remote_targets.status = "not_configured".to_string();
        assert!(dependency_degradation_line(&ledger).is_none());
    }

    #[test]
    fn tmux_unavailable_detected_from_dependency_ledger() {
        let mut health = healthy_backend_health();
        assert!(!tmux_unavailable_from_health(health.dependencies.as_ref()));

        health.dependencies.as_mut().unwrap().tmux_discovery.status = "unavailable".to_string();
        assert!(tmux_unavailable_from_health(health.dependencies.as_ref()));
    }

    fn tmux_unavailable_from_health(deps: Option<&BackendDependencyLedger>) -> bool {
        deps.map(|d| d.tmux_discovery.status == "unavailable")
            .unwrap_or(false)
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
