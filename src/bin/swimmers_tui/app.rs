use super::*;
use swimmers::api::remote_sessions::is_remote_launch_target;

#[path = "mermaid_viewer.rs"]
mod mermaid_viewer;
#[cfg(test)]
pub(crate) use mermaid_viewer::read_plan_file_from_disk;
use mermaid_viewer::MermaidCacheEntry;
mod attention_config;
mod batch_create;
mod collisions;
#[path = "field_click.rs"]
mod field_click;
mod health;
mod initial_request;
mod native_actions;
mod overlay_plans;
mod picker_actions;
mod picker_refresh;
mod refresh;
mod rendering;
mod session_entities;
mod thought_config;
mod thought_log;
mod voice_interactions;
use attention_config::{
    attention_group_include_unnumbered_sessions, attention_group_layout,
    attention_group_max_sessions,
};
pub(crate) use health::DaemonDefaultsStatus;
#[cfg(test)]
use health::API_STALE_BANNER_TEXT;
use health::{backend_health_warning_text, dependency_degradation_line, ApiRefreshHealth};
pub(crate) use overlay_plans::load_overlay_plan_entries;
#[cfg(test)]
use rendering::{centered_empty_aquarium_position, empty_aquarium_message};
use session_entities::merge_session_entities;
pub(crate) use thought_config::ThoughtConfigActionOutcome;
#[cfg(test)]
use voice_interactions::{
    plan_finish_voice_recording, plan_toggle_voice_recording, FinishVoiceRecordingPlan,
    ToggleVoiceRecordingPlan,
};

// Once the school is large, the O(n^2) pairwise collision pass gets expensive.
// 50 entities is where we start seeing frame-time spikes on laptops, so we cap
// checks per entity above that point and distribute checks across frames.
const COLLISION_THROTTLE_ENTITY_THRESHOLD: usize = 50;
const COLLISION_THROTTLE_PAIR_BUDGET: usize = 24;
const ATTENTION_GROUP_LABEL: &str = "[attention group]";

fn sprite_theme_option_width(theme: Option<SpriteTheme>) -> u16 {
    let label = format!("[{}]", SpriteTheme::override_label(theme));
    display_width(&label)
}

pub(crate) struct RefreshResult {
    pub(crate) sessions: Result<SessionListResponse, String>,
    pub(crate) mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
    pub(crate) session_skills: Vec<(String, Result<SessionSkillListResponse, String>)>,
    pub(crate) backend_health: Result<BackendHealthResponse, String>,
    pub(crate) native_status: Option<Result<NativeDesktopStatusResponse, String>>,
    pub(crate) daemon_defaults_status: Option<DaemonDefaultsStatus>,
    pub(crate) show_success_message: bool,
    pub(crate) force_asset_refresh: bool,
    /// App.refresh_epoch captured when this refresh was dispatched; a later
    /// session upsert bumps the app epoch, marking this result stale.
    pub(crate) epoch: u64,
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

fn launch_target_blocker_message(preview: &LaunchTargetPreview) -> String {
    format!(
        "{}: {} for {}",
        preview.target_label,
        preview.blocked_reason.unwrap_or("blocked"),
        shorten_path(&preview.local_cwd, 32)
    )
}

fn launch_receipt_message(receipt: &LaunchReceipt) -> String {
    match receipt.outcome.as_str() {
        "handoff" => {
            let command = receipt
                .attach_hint
                .as_deref()
                .or(receipt.bootstrap_hint.as_deref())
                .unwrap_or("configure a safe ssh alias");
            format!("handoff {}: {}", receipt.target_label, command)
        }
        "created" => {
            let session = receipt.session_id.as_deref().unwrap_or("session");
            if let (Some(local), Some(remote)) =
                (receipt.local_cwd.as_deref(), receipt.remote_cwd.as_deref())
            {
                format!(
                    "created {session} on {}: {} -> {}",
                    receipt.target_label,
                    shorten_path(local, 24),
                    shorten_path(remote, 24)
                )
            } else if receipt.local_override {
                format!("created {session} on local machine (explicit local override)")
            } else {
                format!("created {session} on {}", receipt.target_label)
            }
        }
        "blocked" => receipt
            .message
            .clone()
            .unwrap_or_else(|| format!("launch blocked for {}", receipt.target_label)),
        _ => receipt
            .message
            .clone()
            .unwrap_or_else(|| format!("launch {} on {}", receipt.outcome, receipt.target_label)),
    }
}

fn remote_inventory_target(target: Option<&str>) -> Option<String> {
    target
        .map(str::trim)
        .filter(|target| is_remote_launch_target(Some(*target)))
        .map(str::to_string)
}

fn target_supports_remote_inventory(target: &LaunchTargetSummary) -> bool {
    target.kind.trim().eq_ignore_ascii_case("swimmers_api") && !target.path_mappings.is_empty()
}

fn environment_remote_inventory_target(
    environments: &[EnvironmentSummary],
    target_id: Option<&str>,
) -> Option<String> {
    let target_id = target_id
        .map(str::trim)
        .filter(|target| !target.is_empty())?;
    environments
        .iter()
        .find(|environment| {
            environment.id == target_id && environment.capabilities.remote_dir_inventory
        })
        .map(|environment| environment.id.clone())
}

fn selected_remote_inventory_target(
    picker: Option<&PickerState>,
    fallback_target: Option<&str>,
    launch_targets: &[LaunchTargetSummary],
    environments: &[EnvironmentSummary],
) -> Option<String> {
    if let Some(picker) = picker {
        let target = picker.selected_launch_target();
        return target_supports_remote_inventory(&target).then_some(target.id);
    }

    let target_id = fallback_target
        .map(str::trim)
        .filter(|target| !target.is_empty() && *target != "local")?;

    if let Some(target) = launch_targets.iter().find(|target| target.id == target_id) {
        return target_supports_remote_inventory(target).then(|| target.id.clone());
    }

    if environments
        .iter()
        .any(|environment| environment.id == target_id)
    {
        return environment_remote_inventory_target(environments, Some(target_id));
    }

    remote_inventory_target(Some(target_id))
}

fn remote_inventory_read_only_message(target: &str) -> String {
    format!("remote directory actions are read-only for {target}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GroupInputTargets {
    pub(crate) session_ids: Vec<String>,
    pub(crate) label: String,
}

pub(crate) enum PendingInteractionResult {
    OpenPicker {
        x: u16,
        y: u16,
        requested_target: Option<String>,
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

fn native_status_unavailable_text(
    app_label: &str,
    mode_suffix: &str,
    reason: Option<&str>,
) -> String {
    if native_status_should_show_tmux_fallback(reason) {
        return "terminal handoff: tmux attach only".to_string();
    }

    format!(
        "terminal handoff: {app_label}{mode_suffix} unavailable: {}",
        reason.unwrap_or("unknown reason")
    )
}

fn native_status_should_show_tmux_fallback(reason: Option<&str>) -> bool {
    let Some(reason) = reason else {
        return false;
    };
    reason.contains("only supported on macOS") || reason.contains("only available from localhost")
}

pub(crate) struct App<C: TuiApi> {
    pub(crate) runtime: Runtime,
    pub(crate) client: Arc<C>,
    pub(crate) artifact_opener: Arc<dyn ArtifactOpener>,
    pub(crate) commit_launcher: Arc<dyn CommitLauncher>,
    pub(crate) entities: Vec<SessionEntity>,
    pub(crate) environments: Vec<EnvironmentSummary>,
    pub(crate) fleet_presets: Vec<FleetLensPreset>,
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
    pub(crate) launch_targets: Vec<LaunchTargetSummary>,
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
    /// Bumped on every user-initiated session upsert (create/adopt/batch) so a
    /// background refresh dispatched before the upsert can be detected as stale
    /// and skip its wholesale session-list replace.
    pub(crate) refresh_epoch: u64,
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
            environments: Vec::new(),
            fleet_presets: swimmers::fleet_lens::build_fleet_lens_presets(Vec::new()),
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
            launch_targets: Vec::new(),
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
            refresh_epoch: 0,
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
                    native_status_unavailable_text(
                        app_label,
                        &mode_suffix,
                        status.reason.as_deref(),
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
        self.request_attention_group(self.attention_group_session_ids.clone(), true, true);
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
        // Drain any in-flight async publication first so it cannot land on the
        // server *after* this blocking clear and resurrect a stale selection on
        // the multi-threaded runtime (swimmers-9eqi).
        self.drain_pending_selection_publication();
        self.publish_selection(None, true);
    }

    /// Block until any spawned selection publication has completed, folding its
    /// result into local state. Used before the shutdown clear so the clear is
    /// guaranteed to be the last write to the published selection.
    fn drain_pending_selection_publication(&mut self) {
        let Some(rx) = self.pending_selection_publication.take() else {
            return;
        };
        if let Ok(result) = self.runtime.block_on(rx) {
            if result.response.is_ok() {
                self.published_selected_id = result.session_id;
            }
        }
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
            PendingInteractionResult::OpenPicker {
                x,
                y,
                requested_target,
                response,
            } => {
                self.apply_open_picker_result(x, y, requested_target, response);
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
        requested_target: Option<String>,
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
                self.launch_targets = picker.launch_targets.clone();
                picker.sync_theme_colors(&mut self.repo_themes);
                self.picker = Some(picker);
                self.last_picker_refresh = Some(Instant::now());
                self.start_picker_repo_search();
            }
            Err(err) => {
                if let Some(target) = requested_target {
                    self.launch_target = Some("local".to_string());
                    self.set_message(format!(
                        "{target} unavailable: {err}; switched picker to local"
                    ));
                } else {
                    self.set_message(err);
                }
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
                    self.launch_targets = picker.launch_targets.clone();
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
                let receipt_message = response.launch_receipt.as_ref().map(launch_receipt_message);
                let Some(session) = response.session else {
                    self.close_picker();
                    self.set_message(
                        receipt_message.unwrap_or_else(|| "launch handoff ready".to_string()),
                    );
                    return;
                };
                let tmux_name =
                    self.remember_and_upsert_pending_session(field, session, response.repo_theme);
                self.close_picker();
                self.set_message(receipt_message.unwrap_or_else(|| format!("created {tmux_name}")));
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
        // A refresh dispatched before this upsert would otherwise drop the new
        // session on a wholesale replace; bump the epoch so it's detected stale.
        self.refresh_epoch = self.refresh_epoch.wrapping_add(1);
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

    pub(crate) fn merge_sessions(&mut self, sessions: Vec<SessionSummary>, field: Rect) {
        self.entities = merge_session_entities(std::mem::take(&mut self.entities), sessions, field);
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
            let Some(entry) = self.next_repo_theme_cache_entry(session, force, &cached_themes)
            else {
                continue;
            };
            self.session_repo_theme_cache
                .insert(session.session_id.clone(), entry);
        }
        self.rebuild_repo_themes_from_cache();
    }

    fn next_repo_theme_cache_entry(
        &self,
        session: &SessionSummary,
        force: bool,
        cached_themes: &HashMap<String, RepoTheme>,
    ) -> Option<RepoThemeCacheEntry> {
        let context = RepoThemeCacheContext::from_session(session);
        if self.should_reuse_repo_theme_cache(session, &context, force) {
            return None;
        }

        Some(RepoThemeCacheEntry {
            context,
            resolved: Self::resolve_repo_theme_for_sync(session, force, cached_themes),
        })
    }

    fn should_reuse_repo_theme_cache(
        &self,
        session: &SessionSummary,
        context: &RepoThemeCacheContext,
        force: bool,
    ) -> bool {
        !force
            && self
                .session_repo_theme_cache
                .get(&session.session_id)
                .map(|entry| entry.context == *context)
                .unwrap_or(false)
    }

    fn resolve_repo_theme_for_sync(
        session: &SessionSummary,
        force: bool,
        cached_themes: &HashMap<String, RepoTheme>,
    ) -> Option<(String, RepoTheme)> {
        discover_repo_theme(&session.cwd).or_else(|| {
            if force {
                return None;
            }
            let theme_id = session.repo_theme_id.as_ref()?;
            let theme = cached_themes.get(theme_id)?.clone();
            Some((theme_id.clone(), theme))
        })
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

    pub(crate) fn open_picker(&mut self, x: u16, y: u16) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let target = selected_remote_inventory_target(
            self.picker.as_ref(),
            self.launch_target.as_deref(),
            &self.launch_targets,
            &self.environments,
        );
        let requested_target = target.clone();
        let client = Arc::clone(&self.client);
        self.set_message("loading directories...");
        self.runtime.spawn(async move {
            let response = client.list_dirs(None, true, None, target.as_deref()).await;
            let _ = tx.send(PendingInteractionResult::OpenPicker {
                x,
                y,
                requested_target,
                response,
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

        let target = selected_remote_inventory_target(
            self.picker.as_ref(),
            self.launch_target.as_deref(),
            &self.launch_targets,
            &self.environments,
        );
        let client = Arc::clone(&self.client);
        if show_message {
            self.set_message("loading directories...");
        }
        self.runtime.spawn(async move {
            let response = client
                .list_dirs(
                    path.as_deref(),
                    managed_only,
                    group.as_deref(),
                    target.as_deref(),
                )
                .await;
            let _ = tx.send(PendingInteractionResult::ReloadPicker {
                managed_only,
                group,
                preserve_selection,
                response,
            });
        });
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

        let target = remote_inventory_target(
            self.picker
                .as_ref()
                .and_then(|picker| picker.launch_target.as_deref())
                .or(self.launch_target.as_deref()),
        );
        if let Some(target) = target.as_deref() {
            self.set_message(remote_inventory_read_only_message(target));
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
                        .list_dirs(
                            Some(&reload_path),
                            managed_only,
                            group.as_deref(),
                            target.as_deref(),
                        )
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

        if let Some(target) = remote_inventory_target(
            self.picker
                .as_ref()
                .and_then(|picker| picker.launch_target.as_deref())
                .or(self.launch_target.as_deref()),
        ) {
            self.set_message(remote_inventory_read_only_message(&target));
            return;
        }

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

    pub(crate) fn open_initial_request(&mut self, cwd: String, launch_target: Option<String>) {
        self.cancel_voice_recording();
        self.initial_request_generation = self.initial_request_generation.saturating_add(1);
        self.group_input_targets = None;
        self.initial_request = Some(InitialRequestState::new(cwd, launch_target));
        self.voice_state = default_ui_state();
    }

    pub(crate) fn open_batch_initial_request_for_visible_entries(&mut self) {
        let Some((visible_count, dirs, blockers)) = self.picker.as_ref().map(|picker| {
            (
                picker.visible_entries().len(),
                picker.batch_dirs_for_visible_entries(),
                picker.batch_launch_blockers(),
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

        if let Some(blocker) = blockers.first() {
            self.set_message(format!(
                "remote batch has {} blocked dirs: {}",
                blockers.len(),
                launch_target_blocker_message(blocker)
            ));
            return;
        }

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
        let handoff_message = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .and_then(|entity| native_actions::remote_native_handoff_message(&entity.session));
        self.selected_id = Some(session_id.clone());
        self.sync_selection_publication();
        if let Some(message) = handoff_message {
            self.set_message(message);
            return;
        }
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
            ThoughtPanelAction::FilterByFleet(fleet) => self.set_thought_filter_fleet(fleet),
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
        field_click::handle_field_click(self, x, y, field);
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
                if let Some(blocker) = self
                    .picker
                    .as_ref()
                    .map(|picker| picker.launch_target_preview_for_path(&path))
                    .filter(LaunchTargetPreview::is_blocked)
                {
                    self.set_message(launch_target_blocker_message(&blocker));
                    return;
                }
                self.launch_target = launch_target.clone();
                self.open_initial_request(path, launch_target);
            }
            Some(PickerActivationPlan::ReloadDirectory { path, managed_only }) => {
                self.picker_reload(Some(path), managed_only, None);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests;
