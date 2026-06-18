use super::*;
use swimmers::color::hsl_to_rgb;
use swimmers::fleet_lens::build_fleet_lens_summary;
use swimmers::session_labels::{session_canonical_cwd_key, session_cwd_label};
use swimmers::types::{
    ActionCueKind, AdvisoryMetadataSummary, FleetLensBucket, FleetLensBucketKind,
    SessionEnvironmentScope,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const THOUGHT_COMMIT_LABEL: &str = "[commit]";
const THOUGHT_LAUNCH_LABEL: &str = "[launch]";
const THOUGHT_SEND_LABEL: &str = "[send]";
const THOUGHT_PLANS_LABEL: &str = "[plans]";
const NO_RECENT_THOUGHT: &str = "no recent thought";
const HEADER_FILTER_OUT_LABEL: &str = "[filter out]";
const HEADER_CLEAR_FILTERS_LABEL: &str = "[clear filters]";
const HEADER_FILTER_LEFT_X: u16 = 2;
const HEADER_FILTER_RIGHT_PADDING: u16 = 2;
const HEADER_FILTER_GAP: u16 = 2;

pub(crate) struct ThoughtFingerprint {
    pub(crate) thought: String,
    pub(crate) updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtLogEntry {
    pub(crate) session_id: String,
    pub(crate) tmux_name: String,
    pub(crate) cwd: String,
    pub(crate) pwd_label: Option<String>,
    pub(crate) target_key: String,
    pub(crate) target_label: String,
    pub(crate) state_key: String,
    pub(crate) readiness_key: String,
    pub(crate) transport_key: String,
    pub(crate) batch: Option<SessionBatchMembership>,
    pub(crate) state: SessionState,
    pub(crate) current_command: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) thought: String,
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) rest_state: RestState,
    pub(crate) color: Color,
    pub(crate) is_stale: bool,
    pub(crate) transport_health: TransportHealth,
    pub(crate) commit_candidate: bool,
    pub(crate) advisory_label: Option<String>,
}

impl ThoughtLogEntry {
    pub(crate) fn from_session(
        session: &SessionSummary,
        thought: String,
        repo_themes: &HashMap<String, RepoTheme>,
    ) -> Self {
        Self {
            session_id: session.session_id.clone(),
            tmux_name: session.tmux_name.clone(),
            cwd: session_canonical_cwd_key(session),
            pwd_label: session_cwd_label(session),
            target_key: thought_filter_target_key(session),
            target_label: session_target_label(session),
            state_key: thought_filter_state_key(session.state).to_string(),
            readiness_key: thought_filter_readiness_key(session).to_string(),
            transport_key: thought_filter_transport_key(session.transport_health).to_string(),
            batch: session.batch.clone(),
            state: session.state,
            current_command: session.current_command.clone(),
            tool: session.tool.clone(),
            thought,
            updated_at: session.thought_updated_at,
            rest_state: session.rest_state,
            color: session_display_color(session, repo_themes),
            is_stale: session.is_stale,
            transport_health: session.transport_health,
            commit_candidate: session.commit_candidate,
            advisory_label: advisory_metadata_label(&session.environment.advisory),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtFleetFilter {
    pub(crate) kind: FleetLensBucketKind,
    pub(crate) key: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThoughtFilter {
    pub(crate) cwd: Option<String>,
    pub(crate) tmux_name: Option<String>,
    pub(crate) fleet: Option<ThoughtFleetFilter>,
    pub(crate) excluded_cwds: HashSet<String>,
    pub(crate) filter_out_mode: bool,
}

impl ThoughtFilter {
    pub(crate) fn is_active(&self) -> bool {
        self.cwd.is_some()
            || self.tmux_name.is_some()
            || self.fleet.is_some()
            || !self.excluded_cwds.is_empty()
    }

    pub(crate) fn matches(&self, entry: &ThoughtLogEntry) -> bool {
        let cwd_matches = self
            .cwd
            .as_ref()
            .map(|cwd| entry.cwd == *cwd)
            .or_else(|| {
                (!self.excluded_cwds.is_empty()).then_some(!self.excluded_cwds.contains(&entry.cwd))
            })
            .unwrap_or(true);
        let tmux_matches = self
            .tmux_name
            .as_ref()
            .map(|tmux_name| entry.tmux_name == *tmux_name)
            .unwrap_or(true);
        let fleet_matches = self
            .fleet
            .as_ref()
            .map(|fleet| thought_filter_entry_fleet_matches(entry, fleet))
            .unwrap_or(true);
        cwd_matches && tmux_matches && fleet_matches
    }

    pub(crate) fn matches_session(&self, session: &SessionSummary) -> bool {
        let canonical_cwd = session_canonical_cwd_key(session);
        let cwd_matches = self
            .cwd
            .as_ref()
            .map(|cwd| canonical_cwd == *cwd)
            .or_else(|| {
                (!self.excluded_cwds.is_empty())
                    .then_some(!self.excluded_cwds.contains(&canonical_cwd))
            })
            .unwrap_or(true);
        let tmux_matches = self
            .tmux_name
            .as_ref()
            .map(|tmux_name| session.tmux_name == *tmux_name)
            .unwrap_or(true);
        let fleet_matches = self
            .fleet
            .as_ref()
            .map(|fleet| thought_filter_session_fleet_matches(session, fleet))
            .unwrap_or(true);
        cwd_matches && tmux_matches && fleet_matches
    }

    pub(crate) fn excludes_cwd(&self, cwd: &str) -> bool {
        self.excluded_cwds.contains(cwd)
    }

    pub(crate) fn clear(&mut self) {
        self.cwd = None;
        self.tmux_name = None;
        self.fleet = None;
        self.excluded_cwds.clear();
        self.filter_out_mode = false;
    }
}

#[derive(Clone, Debug)]

pub(crate) struct ThoughtRepoSummary {
    pub(crate) cwd: String,
    pub(crate) label: String,
    pub(crate) count: usize,
    pub(crate) color: Color,
    pub(crate) last_seen: usize,
}

pub(crate) fn normalize_thought_text(thought: Option<&str>) -> Option<String> {
    let thought = thought?.trim();
    if thought.is_empty() {
        return None;
    }
    Some(thought.to_string())
}

pub(crate) fn should_append_thought(
    previous: Option<&ThoughtFingerprint>,
    incoming: &ThoughtFingerprint,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    let freshness = incoming.updated_at.cmp(&previous.updated_at);
    if freshness == Ordering::Less {
        return false;
    }

    !(freshness == Ordering::Equal && incoming.thought == previous.thought)
}

pub(crate) fn compare_thought_log_entries(
    left: &ThoughtLogEntry,
    right: &ThoughtLogEntry,
) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.tmux_name.cmp(&right.tmux_name))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ThoughtPanelAction {
    FilterByCwd(String),
    FilterByFleet(ThoughtFleetFilter),
    ToggleFilterOutMode,
    ToggleFilterOutCwd(String),
    OpenSession {
        session_id: String,
        label: String,
    },
    OpenInitialRequest {
        cwd: String,
    },
    SendGroup {
        session_ids: Vec<String>,
        label: String,
    },
    LaunchCommitCodex(String),
    OpenMermaid(String),
    OpenPlanFromDisk {
        schema_path: String,
        slug: String,
    },
    OpenRepoInEditor(String),
    ClearFilters,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtChipLayout {
    pub(crate) rect: Rect,
    pub(crate) cwd: String,
    pub(crate) fleet: Option<ThoughtFleetFilter>,
    pub(crate) label: String,
    pub(crate) color: Color,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtRowLayout {
    pub(crate) session_rect: Option<Rect>,
    pub(crate) text_rect: Option<Rect>,
    pub(crate) mermaid_rect: Option<Rect>,
    pub(crate) mermaid_label: Option<String>,
    pub(crate) launch_rect: Option<Rect>,
    pub(crate) commit_rect: Option<Rect>,
    pub(crate) send_rect: Option<Rect>,
    pub(crate) plan_rect: Option<Rect>,
    pub(crate) plan_schema_path: Option<String>,
    pub(crate) plan_slug: Option<String>,
    pub(crate) group_session_ids: Option<Vec<String>>,
    pub(crate) session_id: String,
    pub(crate) cwd: String,
    pub(crate) label: String,
    pub(crate) tmux_name: String,
    pub(crate) line: String,
    pub(crate) color: Color,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThoughtPanelLayout {
    pub(crate) rows: Vec<ThoughtRowLayout>,
    pub(crate) empty_message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeaderFilterLayout {
    pub(crate) chips: Vec<ThoughtChipLayout>,
    pub(crate) filter_out_rect: Option<Rect>,
    pub(crate) clear_filters_rect: Option<Rect>,
}

pub(crate) fn header_filter_row() -> u16 {
    2
}

pub(crate) fn build_header_filter_layout<C: TuiApi>(
    app: &App<C>,
    width: u16,
) -> HeaderFilterLayout {
    let Some(bounds) = header_filter_bounds(width) else {
        return HeaderFilterLayout::default();
    };
    let Some(controls) =
        header_filter_controls(app.thought_filter.is_active(), bounds.available_width)
    else {
        return HeaderFilterLayout::default();
    };

    let (included, chips_width) = gather_filter_chips(app, controls.chip_budget);
    let total_width = header_filter_total_width(controls, chips_width);
    let mut cursor_x = bounds.right_edge.saturating_sub(total_width);

    let clear_filters_rect = if controls.show_clear {
        let rect = header_filter_rect(cursor_x, controls.clear_width);
        cursor_x = cursor_x
            .saturating_add(controls.clear_width)
            .saturating_add(HEADER_FILTER_GAP);
        Some(rect)
    } else {
        None
    };

    let filter_out_rect = Some(header_filter_rect(cursor_x, controls.filter_out_width));
    cursor_x = cursor_x.saturating_add(controls.filter_out_width);
    if chips_width > 0 {
        cursor_x = cursor_x.saturating_add(HEADER_FILTER_GAP);
    }

    let chips = included
        .into_iter()
        .map(|chip| {
            let rect = Rect {
                x: cursor_x,
                y: header_filter_row(),
                width: chip.width,
                height: 1,
            };
            cursor_x = cursor_x.saturating_add(chip.width).saturating_add(2);
            ThoughtChipLayout {
                rect,
                cwd: chip.cwd.unwrap_or_default(),
                fleet: chip.fleet,
                label: chip.label,
                color: chip.color,
            }
        })
        .collect::<Vec<_>>();

    HeaderFilterLayout {
        chips,
        filter_out_rect,
        clear_filters_rect,
    }
}

#[derive(Clone, Copy)]
struct HeaderFilterBounds {
    right_edge: u16,
    available_width: u16,
}

#[derive(Clone, Copy)]
struct HeaderFilterControls {
    filter_out_width: u16,
    clear_width: u16,
    show_clear: bool,
    chip_budget: u16,
}

fn header_filter_bounds(width: u16) -> Option<HeaderFilterBounds> {
    let right_edge = width.saturating_sub(HEADER_FILTER_RIGHT_PADDING);
    (right_edge > HEADER_FILTER_LEFT_X).then_some(HeaderFilterBounds {
        right_edge,
        available_width: right_edge.saturating_sub(HEADER_FILTER_LEFT_X),
    })
}

fn header_filter_controls(
    filter_is_active: bool,
    available_width: u16,
) -> Option<HeaderFilterControls> {
    let filter_out_width = display_width(HEADER_FILTER_OUT_LABEL);
    let clear_width = display_width(HEADER_CLEAR_FILTERS_LABEL);
    let mut remaining_width = available_width.checked_sub(filter_out_width)?;
    let show_clear =
        filter_is_active && remaining_width >= HEADER_FILTER_GAP.saturating_add(clear_width);

    if show_clear {
        remaining_width =
            remaining_width.saturating_sub(HEADER_FILTER_GAP.saturating_add(clear_width));
    }

    Some(HeaderFilterControls {
        filter_out_width,
        clear_width,
        show_clear,
        chip_budget: remaining_width.saturating_sub(HEADER_FILTER_GAP),
    })
}

fn header_filter_total_width(controls: HeaderFilterControls, chips_width: u16) -> u16 {
    let mut total_width = controls.filter_out_width;
    if controls.show_clear {
        total_width = total_width
            .saturating_add(HEADER_FILTER_GAP)
            .saturating_add(controls.clear_width);
    }
    if chips_width > 0 {
        total_width = total_width
            .saturating_add(HEADER_FILTER_GAP)
            .saturating_add(chips_width);
    }
    total_width
}

fn header_filter_rect(x: u16, width: u16) -> Rect {
    Rect {
        x,
        y: header_filter_row(),
        width,
        height: 1,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FilterChipData {
    cwd: Option<String>,
    fleet: Option<ThoughtFleetFilter>,
    label: String,
    color: Color,
    width: u16,
}

/// Build the per-repo filter chips and their cumulative width, stopping as
/// soon as the next chip would overflow the budget. Returns the chip data
/// (cwd, label, color, width) so the caller can place rects later.
fn gather_filter_chips<C: TuiApi>(app: &App<C>, chip_budget: u16) -> (Vec<FilterChipData>, u16) {
    let summaries = app.header_repo_summaries();
    let mut included = Vec::new();
    let mut chips_width: u16 = 0;

    if !app.thought_filter.filter_out_mode {
        for chip in fleet_filter_chips(app, &app.thought_filter) {
            if !append_filter_chip(&mut included, &mut chips_width, chip, chip_budget) {
                return (included, chips_width);
            }
        }
    }

    for chip in repo_filter_chips_for_summaries(&summaries, &app.thought_filter) {
        if !append_filter_chip(&mut included, &mut chips_width, chip, chip_budget) {
            break;
        }
    }

    (included, chips_width)
}

#[cfg(test)]
fn filter_chips_for_summaries(
    summaries: &[ThoughtRepoSummary],
    filter: &ThoughtFilter,
    chip_budget: u16,
) -> (Vec<FilterChipData>, u16) {
    let mut included = Vec::new();
    let mut chips_width: u16 = 0;
    for chip in repo_filter_chips_for_summaries(summaries, filter) {
        if !append_filter_chip(&mut included, &mut chips_width, chip, chip_budget) {
            break;
        }
    }
    (included, chips_width)
}

fn repo_filter_chips_for_summaries(
    summaries: &[ThoughtRepoSummary],
    filter: &ThoughtFilter,
) -> Vec<FilterChipData> {
    let mut included = Vec::new();
    for summary in summaries {
        let Some(chip) = filter_chip_data(summary, filter) else {
            continue;
        };
        included.push(chip);
    }
    included
}

fn fleet_filter_chips<C: TuiApi>(app: &App<C>, filter: &ThoughtFilter) -> Vec<FilterChipData> {
    let sessions = app
        .entities
        .iter()
        .map(|entity| entity.session.clone())
        .collect::<Vec<_>>();
    let lens = build_fleet_lens_summary(&sessions);
    lens.buckets
        .iter()
        .filter(|bucket| fleet_bucket_is_useful_chip(bucket, &lens.buckets))
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|bucket| fleet_filter_chip_data(bucket, filter))
        .collect()
}

fn fleet_bucket_is_useful_chip(bucket: &FleetLensBucket, buckets: &[FleetLensBucket]) -> bool {
    let bucket_count = buckets
        .iter()
        .filter(|candidate| candidate.kind == bucket.kind)
        .count();
    match bucket.kind {
        FleetLensBucketKind::Target | FleetLensBucketKind::State => bucket_count > 1,
        FleetLensBucketKind::Readiness => bucket_count > 1 || bucket.key == "needs_attention",
        FleetLensBucketKind::Transport => bucket_count > 1 || bucket.key != "healthy",
        FleetLensBucketKind::Repo => false,
    }
}

fn fleet_filter_chip_data(
    bucket: FleetLensBucket,
    filter: &ThoughtFilter,
) -> Option<FilterChipData> {
    if bucket.kind == FleetLensBucketKind::Repo || bucket.count == 0 {
        return None;
    }
    let fleet = ThoughtFleetFilter {
        kind: bucket.kind,
        key: bucket.key,
        label: bucket.label,
    };
    let label = fleet_filter_chip_label(&fleet, filter.fleet.as_ref() == Some(&fleet));
    let width = display_width(&label);
    (width > 0).then(|| FilterChipData {
        cwd: None,
        fleet: Some(fleet.clone()),
        label,
        color: fleet_filter_chip_color(&fleet, filter),
        width,
    })
}

fn fleet_filter_chip_label(fleet: &ThoughtFleetFilter, is_active: bool) -> String {
    let kind = fleet_filter_kind_label(fleet.kind);
    if is_active {
        format!("{kind} .")
    } else {
        format!("{kind}:{}", fleet.label)
    }
}

fn fleet_filter_chip_color(fleet: &ThoughtFleetFilter, filter: &ThoughtFilter) -> Color {
    if filter.fleet.as_ref() == Some(fleet) {
        return Color::Cyan;
    }
    match fleet.kind {
        FleetLensBucketKind::Target => Color::Yellow,
        FleetLensBucketKind::State => Color::Green,
        FleetLensBucketKind::Readiness => Color::Magenta,
        FleetLensBucketKind::Transport if fleet.key != "healthy" => Color::Red,
        FleetLensBucketKind::Transport => Color::DarkGrey,
        FleetLensBucketKind::Repo => Color::Cyan,
    }
}

fn chip_color(
    summary_color: Color,
    filter_out_mode: bool,
    is_excluded: bool,
    has_active_cwd: bool,
    is_include_active: bool,
) -> Color {
    if chip_is_muted(
        filter_out_mode,
        is_excluded,
        has_active_cwd,
        is_include_active,
    ) {
        Color::DarkGrey
    } else {
        summary_color
    }
}

fn chip_is_muted(
    filter_out_mode: bool,
    is_excluded: bool,
    has_active_cwd: bool,
    is_include_active: bool,
) -> bool {
    if filter_out_mode {
        return is_excluded;
    }
    has_active_cwd && !is_include_active
}

pub(crate) fn render_header_filter_strip<C: TuiApi>(
    app: &App<C>,
    renderer: &mut Renderer,
    width: u16,
) {
    let layout = build_header_filter_layout(app, width);
    if let Some(rect) = layout.filter_out_rect {
        let color = if app.thought_filter.filter_out_mode {
            Color::Cyan
        } else {
            Color::DarkGrey
        };
        renderer.draw_text(rect.x, rect.y, HEADER_FILTER_OUT_LABEL, color);
    }

    for chip in &layout.chips {
        renderer.draw_text(chip.rect.x, chip.rect.y, &chip.label, chip.color);
    }

    if let Some(rect) = layout.clear_filters_rect {
        renderer.draw_text(rect.x, rect.y, HEADER_CLEAR_FILTERS_LABEL, Color::Cyan);
    }
}

pub(crate) fn header_filter_action_at<C: TuiApi>(
    app: &App<C>,
    width: u16,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    let layout = build_header_filter_layout(app, width);
    header_filter_action_for_layout(&layout, &app.thought_filter, x, y)
}

fn header_filter_action_for_layout(
    layout: &HeaderFilterLayout,
    filter: &ThoughtFilter,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    if header_filter_control_contains(layout.filter_out_rect, x, y) {
        return Some(ThoughtPanelAction::ToggleFilterOutMode);
    }
    if header_filter_control_contains(layout.clear_filters_rect, x, y) {
        return Some(ThoughtPanelAction::ClearFilters);
    }

    header_filter_chip_at(layout, x, y).map(|chip| header_filter_chip_action(filter, chip))
}

fn header_filter_control_contains(rect: Option<Rect>, x: u16, y: u16) -> bool {
    rect.is_some_and(|rect| rect.contains(x, y))
}

fn header_filter_chip_at(
    layout: &HeaderFilterLayout,
    x: u16,
    y: u16,
) -> Option<&ThoughtChipLayout> {
    layout.chips.iter().find(|chip| chip.rect.contains(x, y))
}

fn header_filter_chip_action(
    filter: &ThoughtFilter,
    chip: &ThoughtChipLayout,
) -> ThoughtPanelAction {
    if let Some(fleet) = chip.fleet.clone() {
        return ThoughtPanelAction::FilterByFleet(fleet);
    }
    if filter.filter_out_mode {
        return ThoughtPanelAction::ToggleFilterOutCwd(chip.cwd.clone());
    }
    if filter.cwd.as_deref() == Some(chip.cwd.as_str()) {
        return ThoughtPanelAction::OpenRepoInEditor(chip.cwd.clone());
    }
    ThoughtPanelAction::FilterByCwd(chip.cwd.clone())
}

pub(crate) fn display_width(text: &str) -> u16 {
    UnicodeWidthStr::width(text).min(u16::MAX as usize) as u16
}

pub(crate) fn path_tail_label(path: &str) -> Option<String> {
    let normalized = normalize_path(path.trim());
    if normalized == "/" {
        return None;
    }

    normalized
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn active_thought_filter_text(filter: &ThoughtFilter) -> String {
    let parts = active_thought_filter_parts(filter);
    if parts.is_empty() {
        return "filter: none".to_string();
    }
    format!("filter: {}", parts.join(", "))
}

fn active_thought_filter_parts(filter: &ThoughtFilter) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(cwd) = filter.cwd.as_deref() {
        parts.push(format!("pwd={}", thought_filter_cwd_label(cwd)));
    }
    if !filter.excluded_cwds.is_empty() {
        parts.push(format!("hide={}", excluded_thought_filter_labels(filter)));
    }
    if let Some(tmux_name) = filter.tmux_name.as_deref() {
        parts.push(format!("num={tmux_name}"));
    }
    if let Some(fleet) = filter.fleet.as_ref() {
        parts.push(format!(
            "{}={}",
            fleet_filter_kind_label(fleet.kind),
            fleet.label
        ));
    }
    parts
}

fn excluded_thought_filter_labels(filter: &ThoughtFilter) -> String {
    let mut hidden = filter
        .excluded_cwds
        .iter()
        .map(|cwd| thought_filter_cwd_label(cwd))
        .collect::<Vec<_>>();
    hidden.sort();
    hidden.join(",")
}

fn thought_filter_cwd_label(cwd: &str) -> String {
    path_tail_label(cwd).unwrap_or_else(|| cwd.to_string())
}

pub(crate) fn thought_filter_target_key(session: &SessionSummary) -> String {
    if session.environment.scope == SessionEnvironmentScope::Remote {
        return [
            session.environment.target_id.as_str(),
            session.environment.display_host.as_str(),
            session.environment.target_label.as_str(),
        ]
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("remote")
        .to_string();
    }
    "local".to_string()
}

pub(crate) fn thought_filter_state_key(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Busy => "busy",
        SessionState::Error => "error",
        SessionState::Attention => "attention",
        SessionState::Exited => "exited",
    }
}

pub(crate) fn thought_filter_readiness_key(session: &SessionSummary) -> &'static str {
    if session.state == SessionState::Attention
        || session.commit_candidate
        || session.action_cues.iter().any(|cue| {
            matches!(
                cue.kind,
                ActionCueKind::AwaitingUser
                    | ActionCueKind::CommitReady
                    | ActionCueKind::ValidationMissingAfterEdit
                    | ActionCueKind::DirtyCheckMissing
            )
        })
    {
        "needs_attention"
    } else if session.state == SessionState::Busy {
        "working"
    } else if matches!(
        session.rest_state,
        RestState::Sleeping | RestState::DeepSleep
    ) {
        "sleeping"
    } else {
        "quiet"
    }
}

pub(crate) fn thought_filter_transport_key(health: TransportHealth) -> &'static str {
    match health {
        TransportHealth::Healthy => "healthy",
        TransportHealth::Degraded => "degraded",
        TransportHealth::Overloaded => "overloaded",
        TransportHealth::Disconnected => "disconnected",
    }
}

fn thought_filter_entry_fleet_matches(entry: &ThoughtLogEntry, fleet: &ThoughtFleetFilter) -> bool {
    match fleet.kind {
        FleetLensBucketKind::Target => entry.target_key == fleet.key,
        FleetLensBucketKind::Repo => entry.cwd == fleet.key,
        FleetLensBucketKind::State => entry.state_key == fleet.key,
        FleetLensBucketKind::Readiness => entry.readiness_key == fleet.key,
        FleetLensBucketKind::Transport => entry.transport_key == fleet.key,
    }
}

fn thought_filter_session_fleet_matches(
    session: &SessionSummary,
    fleet: &ThoughtFleetFilter,
) -> bool {
    match fleet.kind {
        FleetLensBucketKind::Target => thought_filter_target_key(session) == fleet.key,
        FleetLensBucketKind::Repo => session_canonical_cwd_key(session) == fleet.key,
        FleetLensBucketKind::State => thought_filter_state_key(session.state) == fleet.key,
        FleetLensBucketKind::Readiness => thought_filter_readiness_key(session) == fleet.key,
        FleetLensBucketKind::Transport => {
            thought_filter_transport_key(session.transport_health) == fleet.key
        }
    }
}

fn fleet_filter_kind_label(kind: FleetLensBucketKind) -> &'static str {
    match kind {
        FleetLensBucketKind::Target => "host",
        FleetLensBucketKind::Repo => "pwd",
        FleetLensBucketKind::State => "state",
        FleetLensBucketKind::Readiness => "ready",
        FleetLensBucketKind::Transport => "health",
    }
}

pub(crate) fn thought_session_label(pwd_label: Option<&str>, tmux_name: &str) -> String {
    match pwd_label.map(str::trim).filter(|label| !label.is_empty()) {
        Some(pwd_label) if !tmux_name.trim().is_empty() => format!("{pwd_label}/{tmux_name}"),
        Some(pwd_label) => pwd_label.to_string(),
        None if !tmux_name.trim().is_empty() => tmux_name.to_string(),
        None => "session".to_string(),
    }
}

pub(crate) fn thought_session_click_label(label: &str) -> String {
    format!("[{label}]")
}

pub(crate) fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return Vec::new();
    }

    let mut remaining = text.trim();
    if remaining.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    while !remaining.is_empty() {
        let (line, rest) = next_wrapped_line(remaining, max_chars);
        lines.push(line);
        remaining = rest;
    }

    lines
}

fn next_wrapped_line(remaining: &str, max_chars: usize) -> (String, &str) {
    if UnicodeWidthStr::width(remaining) <= max_chars {
        return (remaining.to_string(), "");
    }

    let break_idx = wrap_break_index(remaining, max_chars);
    let (line, rest) = remaining.split_at(break_idx);
    (line.trim_end().to_string(), rest.trim_start())
}

fn wrap_break_index(text: &str, max_chars: usize) -> usize {
    let scan = scan_wrappable_prefix(text, max_chars);
    scan.last_space
        .unwrap_or_else(|| visible_prefix_end(text, scan.split_at))
        .max(1)
}

#[derive(Clone, Copy)]
struct WrappablePrefix {
    split_at: usize,
    last_space: Option<usize>,
}

fn scan_wrappable_prefix(text: &str, max_chars: usize) -> WrappablePrefix {
    let mut used_cols = 0usize;
    let mut split_at = 0usize;
    let mut last_space = None;
    for (idx, ch) in text.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if ch_width > 0 && used_cols.saturating_add(ch_width) > max_chars {
            break;
        }
        used_cols = used_cols.saturating_add(ch_width);
        split_at = idx + ch.len_utf8();
        if ch.is_whitespace() {
            last_space = Some(idx);
        }
    }

    WrappablePrefix {
        split_at,
        last_space,
    }
}

fn visible_prefix_end(text: &str, split_at: usize) -> usize {
    if split_at > 0 {
        return split_at;
    }

    // Ensure forward progress when the first visible scalar is wider than the
    // available space for this wrapped row.
    text.char_indices()
        .next()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(text.len())
        .max(1)
}

#[derive(Clone, Debug)]
pub(crate) struct ThoughtGroup {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) color: Color,
    pub(crate) entries: Vec<ThoughtPanelEntryView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ThoughtGroupBy {
    #[default]
    Pwd,
    Batch,
}

impl ThoughtGroupBy {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pwd => "pwd",
            Self::Batch => "batch",
        }
    }

    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Pwd => Self::Batch,
            Self::Batch => Self::Pwd,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ThoughtPanelEntryView {
    pub(crate) session_id: String,
    pub(crate) label: String,
    pub(crate) tmux_name: String,
    pub(crate) cwd: String,
    pub(crate) target_key: String,
    pub(crate) target_label: String,
    pub(crate) batch: Option<SessionBatchMembership>,
    pub(crate) state: SessionState,
    pub(crate) current_command: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) rest_state: RestState,
    pub(crate) color: Color,
    pub(crate) is_stale: bool,
    pub(crate) transport_health: TransportHealth,
    pub(crate) thought: String,
    pub(crate) mermaid_label: Option<String>,
    pub(crate) has_commit_candidate: bool,
    pub(crate) advisory_label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThoughtWorkStatus {
    Working,
    Asleep,
    Stopped,
}

impl ThoughtWorkStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Working => "[work]",
            Self::Asleep => "[asleep]",
            Self::Stopped => "[done]",
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Self::Working => 0,
            Self::Stopped => 1,
            Self::Asleep => 2,
        }
    }
}

fn thought_work_status(entry: &ThoughtPanelEntryView) -> ThoughtWorkStatus {
    if thought_entry_needs_input(entry) {
        ThoughtWorkStatus::Asleep
    } else if entry.state == SessionState::Exited
        || entry.is_stale
        || entry.transport_health == TransportHealth::Disconnected
        || entry.rest_state == RestState::DeepSleep
    {
        ThoughtWorkStatus::Stopped
    } else {
        ThoughtWorkStatus::Working
    }
}

pub(crate) fn thought_entry_needs_input(entry: &ThoughtPanelEntryView) -> bool {
    entry.rest_state == RestState::Sleeping
}

pub(crate) fn thought_panel_needs_input<C: TuiApi>(app: &App<C>) -> bool {
    if app.daemon_defaults_status.is_unavailable() {
        return true;
    }
    if app.group_input_targets.is_some() {
        return true;
    }
    let entries = build_thought_panel_entries(app);
    entries.iter().any(thought_entry_needs_input)
        || thought_panel_entries_have_sendable_batch(&entries)
}

fn thought_panel_entries_have_sendable_batch(entries: &[ThoughtPanelEntryView]) -> bool {
    build_thought_groups(entries, ThoughtGroupBy::Batch)
        .iter()
        .any(|group| group_send_session_ids(ThoughtGroupBy::Batch, group).is_some())
}

pub(crate) const DARK_TERMINAL_BG_RGB: (u8, u8, u8) = (0x11, 0x11, 0x11);
pub(crate) const MIN_DARK_TERMINAL_CONTRAST: f64 = 4.5;
pub(crate) const DARK_TERMINAL_COLOR_SEARCH_STEPS: usize = 12;

pub(crate) fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let trimmed = value.trim();
    // Require ASCII so the byte-index slices below always land on char
    // boundaries: a 7-*byte* multibyte string (e.g. "#€abc") would otherwise
    // panic when a slice splits a multi-byte char. Every valid #RRGGBB is
    // ASCII, so this only rejects input that could never parse anyway.
    if trimmed.len() != 7 || !trimmed.starts_with('#') || !trimmed.is_ascii() {
        return None;
    }

    let r = u8::from_str_radix(&trimmed[1..3], 16).ok()?;
    let g = u8::from_str_radix(&trimmed[3..5], 16).ok()?;
    let b = u8::from_str_radix(&trimmed[5..7], 16).ok()?;
    Some((r, g, b))
}

pub(crate) fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb { r, g, b }
}

pub(crate) fn linearize_srgb_channel(channel: u8) -> f64 {
    let value = channel as f64 / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    0.2126 * linearize_srgb_channel(r)
        + 0.7152 * linearize_srgb_channel(g)
        + 0.0722 * linearize_srgb_channel(b)
}

pub(crate) fn contrast_ratio(foreground: (u8, u8, u8), background: (u8, u8, u8)) -> f64 {
    let fg = relative_luminance(foreground);
    let bg = relative_luminance(background);
    let (lighter, darker) = if fg >= bg { (fg, bg) } else { (bg, fg) };
    (lighter + 0.05) / (darker + 0.05)
}

pub(crate) fn mix_towards_white((r, g, b): (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |channel: u8| {
        (channel as f64 + (255.0 - channel as f64) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (mix(r), mix(g), mix(b))
}

// Assume a representative dark terminal background because the terminal theme
// itself is not observable from crossterm.
pub(crate) fn adjust_for_dark_terminal(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    if contrast_ratio(rgb, DARK_TERMINAL_BG_RGB) >= MIN_DARK_TERMINAL_CONTRAST {
        return rgb;
    }

    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..DARK_TERMINAL_COLOR_SEARCH_STEPS {
        let mid = (low + high) / 2.0;
        let candidate = mix_towards_white(rgb, mid);
        if contrast_ratio(candidate, DARK_TERMINAL_BG_RGB) >= MIN_DARK_TERMINAL_CONTRAST {
            high = mid;
        } else {
            low = mid;
        }
    }

    mix_towards_white(rgb, high)
}

pub(crate) fn repo_theme_display_color(value: &str) -> Option<Color> {
    let rgb = parse_hex_rgb(value)?;
    Some(rgb_color(adjust_for_dark_terminal(rgb)))
}

pub(crate) fn session_theme_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Option<Color> {
    let theme_id = session.repo_theme_id.as_ref()?;
    let theme = repo_themes.get(theme_id)?;
    repo_theme_display_color(&theme.body)
}

pub(crate) fn session_display_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Color {
    if session_state_evidence_unverified(session) {
        return Color::DarkGrey;
    }
    session_theme_color(session, repo_themes)
        .unwrap_or_else(|| name_based_color(&session.tmux_name))
}

pub(crate) fn session_target_label(session: &SessionSummary) -> String {
    if session.environment.scope != SessionEnvironmentScope::Remote {
        return "local".to_string();
    }
    [
        session.environment.display_host.as_str(),
        session.environment.target_label.as_str(),
        session.environment.target_id.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .find(|value| !value.is_empty())
    .unwrap_or("remote")
    .to_string()
}

pub(crate) fn session_state_evidence_unverified(session: &SessionSummary) -> bool {
    session.state_evidence.observed_at.is_none()
        || matches!(session.state_evidence.confidence, StateConfidence::Low)
        || !matches!(session.transport_health, TransportHealth::Healthy)
}

/// Deterministic color derived from the session name so that sessions without a
/// repo theme directory still show a stable, recognisable hue.
pub(crate) fn name_based_color(name: &str) -> Color {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let seed = hasher.finish();

    let hue = (seed % 3600) as f64 / 10.0; // 0..360
    let saturation = 0.50 + ((seed >> 16) % 200) as f64 / 1000.0; // 0.50..0.70
    let lightness = 0.45 + ((seed >> 32) % 150) as f64 / 1000.0; // 0.45..0.60

    let rgb = hsl_to_rgb(hue, saturation, lightness);
    rgb_color(adjust_for_dark_terminal(rgb))
}

pub(crate) fn compare_thought_panel_entries(
    left: &ThoughtPanelEntryView,
    right: &ThoughtPanelEntryView,
) -> Ordering {
    thought_work_status(left)
        .sort_rank()
        .cmp(&thought_work_status(right).sort_rank())
        .then_with(|| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.tmux_name.cmp(&right.tmux_name))
                .then_with(|| left.session_id.cmp(&right.session_id))
        })
}

pub(crate) fn build_thought_panel_entries<C: TuiApi>(app: &App<C>) -> Vec<ThoughtPanelEntryView> {
    let mut entries = Vec::new();
    let mut thought_sessions = HashSet::new();

    for entry in app
        .thought_log
        .iter()
        .filter(|entry| app.thought_filter.matches(entry))
    {
        thought_sessions.insert(entry.session_id.clone());
        let label = thought_session_label(entry.pwd_label.as_deref(), &entry.tmux_name);
        entries.push(ThoughtPanelEntryView {
            session_id: entry.session_id.clone(),
            label: label.clone(),
            tmux_name: entry.tmux_name.clone(),
            cwd: entry.cwd.clone(),
            target_key: entry.target_key.clone(),
            target_label: entry.target_label.clone(),
            batch: entry.batch.clone(),
            state: entry.state,
            current_command: entry.current_command.clone(),
            tool: entry.tool.clone(),
            updated_at: entry.updated_at,
            rest_state: entry.rest_state,
            color: app.thought_entry_display_color(entry),
            is_stale: entry.is_stale,
            transport_health: entry.transport_health,
            thought: entry.thought.replace('\n', " "),
            mermaid_label: app
                .mermaid_artifacts
                .get(&entry.session_id)
                .filter(|artifact| artifact.available)
                .map(|artifact| mermaid_badge_label(artifact.slice_name.as_deref())),
            has_commit_candidate: entry.commit_candidate,
            advisory_label: entry.advisory_label.clone(),
        });
    }

    for entity in app.visible_entities() {
        if thought_sessions.contains(&entity.session.session_id) {
            continue;
        }
        let artifact = app.mermaid_artifacts.get(&entity.session.session_id);
        let cwd_label = session_cwd_label(&entity.session);
        let label = thought_session_label(cwd_label.as_deref(), &entity.session.tmux_name);
        entries.push(ThoughtPanelEntryView {
            session_id: entity.session.session_id.clone(),
            label: label.clone(),
            tmux_name: entity.session.tmux_name.clone(),
            cwd: session_canonical_cwd_key(&entity.session),
            target_key: thought_filter_target_key(&entity.session),
            target_label: session_target_label(&entity.session),
            batch: entity.session.batch.clone(),
            state: entity.session.state,
            current_command: entity.session.current_command.clone(),
            tool: entity.session.tool.clone(),
            updated_at: artifact.and_then(|artifact| artifact.updated_at),
            rest_state: entity.session.rest_state,
            color: session_display_color(&entity.session, &app.repo_themes),
            is_stale: entity.session.is_stale,
            transport_health: entity.session.transport_health,
            thought: normalize_thought_text(entity.session.thought.as_deref()).unwrap_or_else(
                || {
                    if artifact.is_some_and(|artifact| artifact.available) {
                        "artifacts ready".to_string()
                    } else {
                        "no recent thought".to_string()
                    }
                },
            ),
            mermaid_label: artifact
                .filter(|artifact| artifact.available)
                .map(|artifact| mermaid_badge_label(artifact.slice_name.as_deref())),
            has_commit_candidate: entity.session.commit_candidate,
            advisory_label: advisory_metadata_label(&entity.session.environment.advisory),
        });
    }

    entries.sort_by(compare_thought_panel_entries);
    entries
}

fn scoped_thought_panel_entries<C: TuiApi>(app: &App<C>) -> Vec<ThoughtPanelEntryView> {
    let entries = build_thought_panel_entries(app);
    let Some(targets) = &app.group_input_targets else {
        return entries;
    };
    let target_ids = targets.session_ids.iter().collect::<HashSet<_>>();
    entries
        .into_iter()
        .filter(|entry| target_ids.contains(&entry.session_id))
        .collect()
}

pub(crate) fn thought_group_label(
    group_by: ThoughtGroupBy,
    entry: &ThoughtPanelEntryView,
) -> String {
    match group_by {
        ThoughtGroupBy::Pwd => path_tail_label(&entry.cwd).unwrap_or_else(|| entry.cwd.clone()),
        ThoughtGroupBy::Batch => entry
            .batch
            .as_ref()
            .map(|batch| batch.label.clone())
            .unwrap_or_else(|| "unbatched".to_string()),
    }
}

fn thought_group_key(group_by: ThoughtGroupBy, entry: &ThoughtPanelEntryView) -> String {
    match group_by {
        ThoughtGroupBy::Pwd => format!("pwd:{}", entry.cwd),
        ThoughtGroupBy::Batch => entry
            .batch
            .as_ref()
            .map(|batch| format!("batch:{}", batch.id))
            .unwrap_or_else(|| "batch:unbatched".to_string()),
    }
}

pub(crate) fn build_thought_groups(
    entries: &[ThoughtPanelEntryView],
    group_by: ThoughtGroupBy,
) -> Vec<ThoughtGroup> {
    let mut groups: Vec<ThoughtGroup> = Vec::new();
    for entry in entries {
        let key = thought_group_key(group_by, entry);
        if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
            group.entries.push(entry.clone());
            group.color = entry.color;
            continue;
        }
        groups.push(ThoughtGroup {
            key,
            label: thought_group_label(group_by, entry),
            color: entry.color,
            entries: vec![entry.clone()],
        });
    }
    groups.sort_by(|left, right| {
        match (left.entries.last(), right.entries.last()) {
            (Some(left_entry), Some(right_entry)) => {
                compare_thought_panel_entries(left_entry, right_entry)
            }
            _ => Ordering::Equal,
        }
        .then_with(|| left.label.cmp(&right.label))
        .then_with(|| left.key.cmp(&right.key))
    });
    groups
}

fn plan_key(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

fn cached_plan_for_group(
    group_by: ThoughtGroupBy,
    group: &ThoughtGroup,
    plans: &[PlanPanelEntry],
) -> Option<PlanPanelEntry> {
    if group_by != ThoughtGroupBy::Pwd {
        return None;
    }
    let group_key = plan_key(&group.label);
    plans
        .iter()
        .find(|plan| plan_key(&plan.client_label) == group_key || plan_key(&plan.slug) == group_key)
        .cloned()
}

fn artifact_plan_for_group<C: TuiApi>(
    app: &App<C>,
    group_by: ThoughtGroupBy,
    group: &ThoughtGroup,
) -> Option<PlanPanelEntry> {
    if group_by != ThoughtGroupBy::Pwd {
        return None;
    }
    group.entries.iter().rev().find_map(|entry| {
        let artifact = app.mermaid_artifacts.get(&entry.session_id)?;
        if !artifact.available {
            return None;
        }
        let schema_path = artifact.path.as_deref()?;
        let slug = artifact
            .slice_name
            .as_deref()
            .or_else(|| swimmers::session::artifacts::extract_mmd_slice_name(schema_path))?;
        Some(PlanPanelEntry {
            slug: slug.to_string(),
            client_label: group.label.clone(),
            kind: "session".to_string(),
            schema_path: schema_path.to_string(),
        })
    })
}

fn plan_for_group<C: TuiApi>(
    app: &App<C>,
    group_by: ThoughtGroupBy,
    group: &ThoughtGroup,
) -> Option<PlanPanelEntry> {
    artifact_plan_for_group(app, group_by, group)
        .or_else(|| cached_plan_for_group(group_by, group, &app.cached_plans))
}

fn thought_entry_is_group_input_ready(entry: &ThoughtPanelEntryView) -> bool {
    !entry.is_stale
        && entry.transport_health == TransportHealth::Healthy
        && entry.state != SessionState::Exited
        && entry.rest_state != RestState::DeepSleep
        && (entry.rest_state == RestState::Sleeping || entry.state == SessionState::Attention)
}

fn group_send_session_ids(group_by: ThoughtGroupBy, group: &ThoughtGroup) -> Option<Vec<String>> {
    if group_by != ThoughtGroupBy::Batch || group.entries.iter().any(|entry| entry.batch.is_none())
    {
        return None;
    }
    let ready_entries = group
        .entries
        .iter()
        .filter(|entry| thought_entry_is_group_input_ready(entry))
        .collect::<Vec<_>>();
    let session_ids = ready_entries
        .iter()
        .map(|entry| entry.session_id.clone())
        .collect::<Vec<_>>();
    (session_ids.len() > 1 && group_send_ids_have_single_scope(&ready_entries))
        .then_some(session_ids)
}

fn group_send_ids_have_single_scope(entries: &[&ThoughtPanelEntryView]) -> bool {
    let mut scopes = entries
        .iter()
        .map(|entry| group_send_scope_key(entry))
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();
    scopes.len() <= 1
}

fn group_send_scope_key(entry: &ThoughtPanelEntryView) -> &str {
    entry.target_key.as_str()
}

fn compact_target_label(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.eq_ignore_ascii_case("local") {
        return "L".to_string();
    }
    trimmed
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("remote")
        .to_string()
}

fn thought_group_target_summary(group_by: ThoughtGroupBy, group: &ThoughtGroup) -> Option<String> {
    if group_by != ThoughtGroupBy::Pwd {
        return None;
    }
    let mut labels = group
        .entries
        .iter()
        .map(|entry| compact_target_label(&entry.target_label))
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    (labels.len() > 1).then(|| labels.join("+"))
}

fn thought_group_header_label(group_by: ThoughtGroupBy, group: &ThoughtGroup) -> String {
    thought_group_target_summary(group_by, group)
        .map(|summary| format!("{} {}", group.label, summary))
        .unwrap_or_else(|| group.label.clone())
}

fn group_header_row<C: TuiApi>(
    app: &App<C>,
    group: &ThoughtGroup,
    group_by: ThoughtGroupBy,
    thought_content: Rect,
) -> ThoughtRowLayout {
    let plan = plan_for_group(app, group_by, group);
    let base_label = format!(
        "v {} ({})",
        thought_group_header_label(group_by, group),
        group.entries.len()
    );
    let send_session_ids = group_send_session_ids(group_by, group);
    let send_start = send_session_ids
        .as_ref()
        .map(|_| display_width(&base_label).saturating_add(1));
    let label_before_plan = if send_session_ids.is_some() {
        format!("{base_label} {THOUGHT_SEND_LABEL}")
    } else {
        base_label
    };
    let plan_start = plan
        .as_ref()
        .map(|_| display_width(&label_before_plan).saturating_add(1));
    let label = if plan.is_some() {
        format!("{label_before_plan} {THOUGHT_PLANS_LABEL}")
    } else {
        label_before_plan
    };
    let line = truncate_label(&label, thought_content.width as usize);
    let width = display_width(&line);
    let send_rect = send_start.and_then(|start| {
        visible_segment_rect(
            thought_content.x,
            start,
            display_width(THOUGHT_SEND_LABEL),
            width,
        )
    });
    let plan_rect = plan_start.and_then(|start| {
        visible_segment_rect(
            thought_content.x,
            start,
            display_width(THOUGHT_PLANS_LABEL),
            width,
        )
    });
    ThoughtRowLayout {
        session_rect: None,
        text_rect: (width > 0).then_some(Rect {
            x: thought_content.x,
            y: 0,
            width,
            height: 1,
        }),
        mermaid_rect: None,
        mermaid_label: None,
        launch_rect: None,
        commit_rect: None,
        send_rect,
        plan_rect,
        plan_schema_path: plan.as_ref().map(|plan| plan.schema_path.clone()),
        plan_slug: plan.as_ref().map(|plan| plan.slug.clone()),
        group_session_ids: send_session_ids,
        session_id: String::new(),
        cwd: String::new(),
        label: group.label.clone(),
        tmux_name: String::new(),
        line,
        color: Color::DarkGrey,
    }
}

fn clean_inline_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn thought_agent_label(entry: &ThoughtPanelEntryView) -> Option<String> {
    entry
        .tool
        .as_deref()
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .map(|tool| tool.to_ascii_lowercase())
}

fn thought_detail_line(entry: &ThoughtPanelEntryView) -> String {
    let detail = thought_text_detail(entry)
        .or_else(|| command_detail(entry))
        .unwrap_or_else(|| thought_status_detail(entry).to_string());
    entry
        .advisory_label
        .as_deref()
        .map(|label| format!("{detail} · {label}"))
        .unwrap_or(detail)
}

fn advisory_metadata_label(advisory: &[AdvisoryMetadataSummary]) -> Option<String> {
    let labels = advisory
        .iter()
        .filter_map(|item| {
            let label = item.label.trim();
            let value = item.value.trim();
            if label.is_empty() || value.is_empty() {
                return None;
            }
            let status = item.status.trim();
            let status = if status.is_empty() {
                "external"
            } else {
                status
            };
            let stale = if item.stale { " stale" } else { "" };
            Some(format!("{status} {label}: {value}{stale}"))
        })
        .collect::<Vec<_>>();
    (!labels.is_empty()).then(|| labels.join(" · "))
}

fn thought_text_detail(entry: &ThoughtPanelEntryView) -> Option<String> {
    let thought = clean_inline_text(&entry.thought);
    (!thought.is_empty() && thought != NO_RECENT_THOUGHT).then_some(thought)
}

fn command_detail(entry: &ThoughtPanelEntryView) -> Option<String> {
    let command = clean_inline_text(entry.current_command.as_deref()?);
    (!command.is_empty()).then(|| format!("cmd: {command}"))
}

fn thought_status_detail(entry: &ThoughtPanelEntryView) -> &'static str {
    if entry.is_stale {
        return "stale session";
    }
    if thought_has_no_daemon(entry) {
        return "no daemon";
    }
    if thought_is_sleeping(entry) {
        return "sleeping";
    }

    NO_RECENT_THOUGHT
}

fn thought_has_no_daemon(entry: &ThoughtPanelEntryView) -> bool {
    entry.transport_health == TransportHealth::Disconnected || entry.state == SessionState::Exited
}

fn thought_is_sleeping(entry: &ThoughtPanelEntryView) -> bool {
    entry.rest_state == RestState::Sleeping || entry.rest_state == RestState::DeepSleep
}

fn visible_segment_rect(
    base_x: u16,
    start_col: u16,
    segment_width: u16,
    visible_width: u16,
) -> Option<Rect> {
    if segment_width == 0 || start_col >= visible_width {
        return None;
    }
    Some(Rect {
        x: base_x.saturating_add(start_col),
        y: 0,
        width: segment_width.min(visible_width.saturating_sub(start_col)),
        height: 1,
    })
}

pub(crate) fn build_rows_for_panel_entry(
    entry: &ThoughtPanelEntryView,
    thought_content: Rect,
) -> Vec<ThoughtRowLayout> {
    let session_label = thought_session_click_label(&entry.label);
    let session_width = display_width(&session_label);
    let status = thought_work_status(entry);
    let status_label = status.label();
    let status_width = display_width(status_label);
    let launch_width = display_width(THOUGHT_LAUNCH_LABEL);
    let commit_width = display_width(THOUGHT_COMMIT_LABEL);
    let mermaid_width = entry
        .mermaid_label
        .as_deref()
        .map(display_width)
        .unwrap_or(0);

    let mut line = String::new();
    let mut cursor = 0u16;

    let mermaid_start = entry.mermaid_label.as_ref().map(|label| {
        let start = cursor;
        line.push_str(label);
        cursor = cursor.saturating_add(display_width(label));
        line.push(' ');
        cursor = cursor.saturating_add(1);
        start
    });

    let status_start = cursor;
    line.push_str(status_label);
    cursor = cursor.saturating_add(status_width);
    line.push(' ');
    cursor = cursor.saturating_add(1);

    let launch_start = thought_entry_needs_input(entry).then(|| {
        let start = cursor;
        line.push_str(THOUGHT_LAUNCH_LABEL);
        cursor = cursor.saturating_add(launch_width);
        line.push(' ');
        cursor = cursor.saturating_add(1);
        start
    });

    let commit_start = entry.has_commit_candidate.then(|| {
        let start = cursor;
        line.push_str(THOUGHT_COMMIT_LABEL);
        cursor = cursor.saturating_add(commit_width);
        line.push(' ');
        cursor = cursor.saturating_add(1);
        start
    });

    let session_start = cursor;
    line.push_str(&session_label);

    if let Some(agent) = thought_agent_label(entry) {
        line.push(' ');
        line.push_str(&agent);
    }

    let line = truncate_label(&line, thought_content.width as usize);
    let visible_width = display_width(&line);
    let mermaid_rect = mermaid_start.and_then(|start| {
        visible_segment_rect(thought_content.x, start, mermaid_width, visible_width)
    });
    let _status_rect =
        visible_segment_rect(thought_content.x, status_start, status_width, visible_width);
    let launch_rect = launch_start.and_then(|start| {
        visible_segment_rect(thought_content.x, start, launch_width, visible_width)
    });
    let commit_rect = commit_start.and_then(|start| {
        visible_segment_rect(thought_content.x, start, commit_width, visible_width)
    });
    let session_rect = visible_segment_rect(
        thought_content.x,
        session_start,
        session_width,
        visible_width,
    );

    let mut rows = vec![ThoughtRowLayout {
        session_rect,
        text_rect: (visible_width > 0).then_some(Rect {
            x: thought_content.x,
            y: 0,
            width: visible_width,
            height: 1,
        }),
        mermaid_rect,
        mermaid_label: entry.mermaid_label.clone(),
        launch_rect,
        commit_rect,
        send_rect: None,
        plan_rect: None,
        plan_schema_path: None,
        plan_slug: None,
        group_session_ids: None,
        session_id: entry.session_id.clone(),
        cwd: entry.cwd.clone(),
        label: entry.label.clone(),
        tmux_name: entry.tmux_name.clone(),
        line,
        color: entry.color,
    }];

    let detail_line = truncate_label(
        &format!("  {}", thought_detail_line(entry)),
        thought_content.width as usize,
    );
    let detail_width = display_width(&detail_line);
    if detail_width > 0 {
        rows.push(ThoughtRowLayout {
            session_rect: None,
            text_rect: Some(Rect {
                x: thought_content.x,
                y: 0,
                width: detail_width,
                height: 1,
            }),
            mermaid_rect: None,
            mermaid_label: None,
            launch_rect: None,
            commit_rect: None,
            send_rect: None,
            plan_rect: None,
            plan_schema_path: None,
            plan_slug: None,
            group_session_ids: None,
            session_id: entry.session_id.clone(),
            cwd: entry.cwd.clone(),
            label: entry.label.clone(),
            tmux_name: entry.tmux_name.clone(),
            line: detail_line,
            color: Color::DarkGrey,
        });
    }

    rows
}

pub(crate) fn thought_panel_header<C: TuiApi>(app: &App<C>) -> String {
    if let Some(targets) = &app.group_input_targets {
        return format!(
            "clawgs / group draft · {} · {} target{}",
            targets.label,
            targets.session_ids.len(),
            pluralize(targets.session_ids.len())
        );
    }
    let entries = scoped_thought_panel_entries(app);
    let stopped = entries
        .iter()
        .filter(|entry| thought_entry_needs_input(entry))
        .count();
    let total = entries.len();
    let mode = thought_panel_display_mode(app, &entries);
    let scope = if mode.show_all { "all" } else { "asleep" };
    let toggle = if mode.show_all { "> asleep" } else { "> all" };
    let fleet = thought_panel_fleet_lens_header(app)
        .map(|summary| format!(" · {summary}"))
        .unwrap_or_default();
    format!(
        "clawgs / {} / {} · {}/{} asleep · {}{}",
        mode.group_by.label(),
        scope,
        stopped,
        total,
        toggle,
        fleet
    )
}

fn thought_panel_fleet_lens_header<C: TuiApi>(app: &App<C>) -> Option<String> {
    let sessions = app
        .entities
        .iter()
        .map(|entity| entity.session.clone())
        .collect::<Vec<_>>();
    let lens = build_fleet_lens_summary(&sessions);
    let target_count = lens
        .buckets
        .iter()
        .filter(|bucket| bucket.kind == FleetLensBucketKind::Target)
        .count();
    let advisory_count = app
        .entities
        .iter()
        .map(|entity| entity.session.environment.advisory.len())
        .sum::<usize>();
    if target_count <= 1 && advisory_count == 0 {
        return None;
    }
    let repo_count = lens
        .buckets
        .iter()
        .filter(|bucket| bucket.kind == FleetLensBucketKind::Repo)
        .count();
    let degraded = lens
        .buckets
        .iter()
        .filter(|bucket| bucket.kind == FleetLensBucketKind::Transport && bucket.key != "healthy")
        .map(|bucket| bucket.count)
        .sum::<usize>();
    let inbox = lens
        .buckets
        .iter()
        .find(|bucket| {
            bucket.kind == FleetLensBucketKind::Readiness && bucket.key == "needs_attention"
        })
        .map(|bucket| bucket.count)
        .unwrap_or(0);
    let inbox_suffix = if inbox > 0 {
        format!(" · inbox {inbox}")
    } else {
        String::new()
    };
    let degraded_suffix = if degraded > 0 {
        format!(" · {degraded} degraded")
    } else {
        String::new()
    };
    let advisory_suffix = if advisory_count > 0 {
        format!(" · ext {advisory_count}")
    } else {
        String::new()
    };
    Some(format!(
        "fleet {target_count} host{} / {repo_count} project{}{inbox_suffix}{degraded_suffix}{advisory_suffix}",
        pluralize(target_count),
        pluralize(repo_count)
    ))
}

pub(crate) fn render_thought_panel<C: TuiApi>(
    app: &App<C>,
    renderer: &mut Renderer,
    thought_content: Rect,
    entry_capacity: usize,
) {
    if thought_content.width == 0 || thought_content.height == 0 {
        return;
    }

    let panel = build_thought_panel(app, thought_content, entry_capacity);
    render_thought_panel_header(app, renderer, thought_content);

    if entry_capacity == 0 || render_thought_panel_empty_message(renderer, thought_content, &panel)
    {
        return;
    }

    render_thought_panel_rows(renderer, thought_content, &panel.rows);
}

fn render_thought_panel_header<C: TuiApi>(
    app: &App<C>,
    renderer: &mut Renderer,
    thought_content: Rect,
) {
    renderer.draw_text(
        thought_content.x,
        thought_content.y,
        &truncate_label(&thought_panel_header(app), thought_content.width as usize),
        Color::Cyan,
    );
}

fn render_thought_panel_empty_message(
    renderer: &mut Renderer,
    thought_content: Rect,
    panel: &ThoughtPanelLayout,
) -> bool {
    let Some(message) = panel.empty_message.as_deref() else {
        return false;
    };

    renderer.draw_text(
        thought_content.x,
        thought_content.y + THOUGHT_RAIL_HEADER_ROWS,
        &truncate_label(message, thought_content.width as usize),
        Color::DarkGrey,
    );
    true
}

fn render_thought_panel_rows(
    renderer: &mut Renderer,
    thought_content: Rect,
    rows: &[ThoughtRowLayout],
) {
    let start_y = thought_content.bottom().saturating_sub(rows.len() as u16);
    for (offset, row) in rows.iter().enumerate() {
        let y = start_y + offset as u16;
        render_thought_panel_row(renderer, row, y);
    }
}

fn render_thought_panel_row(renderer: &mut Renderer, row: &ThoughtRowLayout, y: u16) {
    render_mermaid_label(renderer, row, y);
    render_optional_text(renderer, row.text_rect, y, &row.line, row.color);
    render_truncated_label(
        renderer,
        row.launch_rect,
        y,
        THOUGHT_LAUNCH_LABEL,
        Color::Cyan,
    );
    render_optional_text(
        renderer,
        row.commit_rect,
        y,
        THOUGHT_COMMIT_LABEL,
        row.color,
    );
    render_truncated_label(renderer, row.send_rect, y, THOUGHT_SEND_LABEL, Color::Cyan);
    render_truncated_label(renderer, row.plan_rect, y, THOUGHT_PLANS_LABEL, Color::Cyan);
}

fn render_mermaid_label(renderer: &mut Renderer, row: &ThoughtRowLayout, y: u16) {
    row.mermaid_rect
        .zip(row.mermaid_label.as_deref())
        .into_iter()
        .for_each(|(rect, label)| renderer.draw_text(rect.x, y, label, row.color));
}

fn render_optional_text(
    renderer: &mut Renderer,
    rect: Option<Rect>,
    y: u16,
    text: &str,
    color: Color,
) {
    rect.into_iter()
        .for_each(|rect| renderer.draw_text(rect.x, y, text, color));
}

fn render_truncated_label(
    renderer: &mut Renderer,
    rect: Option<Rect>,
    y: u16,
    label: &str,
    color: Color,
) {
    rect.into_iter().for_each(|rect| {
        renderer.draw_text(
            rect.x,
            y,
            &truncate_label(label, rect.width as usize),
            color,
        );
    });
}

pub(crate) fn build_thought_panel<C: TuiApi>(
    app: &App<C>,
    thought_content: Rect,
    entry_capacity: usize,
) -> ThoughtPanelLayout {
    if thought_content.width == 0 || thought_content.height == 0 {
        return ThoughtPanelLayout::default();
    }

    let all_entries = scoped_thought_panel_entries(app);
    let total_count = all_entries.len();
    let mode = thought_panel_display_mode(app, &all_entries);
    let entries = thought_panel_visible_entries(all_entries, mode);
    let empty_message =
        thought_panel_empty_message(app, entry_capacity, &entries, total_count, mode);
    let rows = build_thought_panel_rows(app, &entries, mode, thought_content, entry_capacity);

    ThoughtPanelLayout {
        rows,
        empty_message,
    }
}

fn thought_panel_visible_entries(
    all_entries: Vec<ThoughtPanelEntryView>,
    mode: ThoughtPanelDisplayMode,
) -> Vec<ThoughtPanelEntryView> {
    if mode.show_all {
        return all_entries;
    }

    all_entries
        .iter()
        .filter(|entry| thought_entry_needs_input(entry))
        .cloned()
        .collect()
}

fn thought_panel_empty_message<C: TuiApi>(
    app: &App<C>,
    entry_capacity: usize,
    entries: &[ThoughtPanelEntryView],
    total_count: usize,
    mode: ThoughtPanelDisplayMode,
) -> Option<String> {
    if entry_capacity == 0 {
        return None;
    }
    if let Some(targets) = &app.group_input_targets {
        return Some(group_input_empty_message(targets));
    }
    if entries.is_empty() {
        return Some(empty_thought_panel_message(app, total_count, mode));
    }

    None
}

fn group_input_empty_message(targets: &GroupInputTargets) -> String {
    format!(
        "drafting group input for {} ({} session{})",
        targets.label,
        targets.session_ids.len(),
        pluralize(targets.session_ids.len())
    )
}

fn empty_thought_panel_message<C: TuiApi>(
    app: &App<C>,
    total_count: usize,
    mode: ThoughtPanelDisplayMode,
) -> String {
    if !mode.show_all && total_count > 0 {
        return asleep_filter_empty_message(app, total_count);
    }
    if app.thought_filter.is_active() {
        return "no thoughts match filters".to_string();
    }
    if app.daemon_defaults_status.is_unavailable() {
        return "clawgs unavailable - press t to configure".to_string();
    }

    "waiting for clawgs - press t to configure".to_string()
}

fn asleep_filter_empty_message<C: TuiApi>(app: &App<C>, total_count: usize) -> String {
    if app.daemon_defaults_status.is_unavailable() {
        "clawgs unavailable - press t to configure".to_string()
    } else {
        format!("0 asleep / {total_count} working")
    }
}

fn build_thought_panel_rows<C: TuiApi>(
    app: &App<C>,
    entries: &[ThoughtPanelEntryView],
    mode: ThoughtPanelDisplayMode,
    thought_content: Rect,
    entry_capacity: usize,
) -> Vec<ThoughtRowLayout> {
    if should_group_thought_entries(app, mode.group_by, entries) {
        build_grouped_rows(entries, app, mode.group_by, thought_content, entry_capacity)
    } else {
        build_flat_rows(entries, thought_content, entry_capacity)
    }
}

#[derive(Clone, Copy)]
struct ThoughtPanelDisplayMode {
    group_by: ThoughtGroupBy,
    show_all: bool,
}

fn thought_panel_display_mode<C: TuiApi>(
    app: &App<C>,
    entries: &[ThoughtPanelEntryView],
) -> ThoughtPanelDisplayMode {
    if app.group_input_targets.is_some() {
        return ThoughtPanelDisplayMode {
            group_by: ThoughtGroupBy::Batch,
            show_all: true,
        };
    }
    if thought_panel_entries_have_sendable_batch(entries) {
        ThoughtPanelDisplayMode {
            group_by: ThoughtGroupBy::Batch,
            show_all: true,
        }
    } else {
        ThoughtPanelDisplayMode {
            group_by: app.thought_group_by,
            show_all: app.thought_show_all,
        }
    }
}

fn should_group_thought_entries<C: TuiApi>(
    app: &App<C>,
    group_by: ThoughtGroupBy,
    entries: &[ThoughtPanelEntryView],
) -> bool {
    let groups = build_thought_groups(entries, group_by);
    groups.len() > 1
        || (group_by == ThoughtGroupBy::Batch && entries.iter().any(|entry| entry.batch.is_some()))
        || groups
            .iter()
            .any(|group| plan_for_group(app, group_by, group).is_some())
}

fn build_flat_rows(
    entries: &[ThoughtPanelEntryView],
    thought_content: Rect,
    entry_capacity: usize,
) -> Vec<ThoughtRowLayout> {
    if entry_capacity == 0 {
        return Vec::new();
    }

    let mut rows_rev = Vec::new();
    let mut remaining = entry_capacity;
    for entry in entries.iter().rev() {
        let entry_rows = build_rows_for_panel_entry(entry, thought_content);
        let take = match flat_row_selection(entry_rows.len(), remaining, !rows_rev.is_empty()) {
            FlatRowSelection::Skip => continue,
            FlatRowSelection::Stop => break,
            FlatRowSelection::Take(take) => take,
        };
        rows_rev.extend(entry_rows.into_iter().rev().take(take));
        remaining -= take;
        if remaining == 0 {
            break;
        }
    }
    rows_rev.reverse();
    rows_rev
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlatRowSelection {
    Skip,
    Stop,
    Take(usize),
}

fn flat_row_selection(
    entry_row_count: usize,
    remaining_capacity: usize,
    has_collected_rows: bool,
) -> FlatRowSelection {
    if entry_row_count == 0 {
        return FlatRowSelection::Skip;
    }
    if entry_row_count > remaining_capacity && has_collected_rows {
        return FlatRowSelection::Stop;
    }
    FlatRowSelection::Take(entry_row_count.min(remaining_capacity))
}

fn build_grouped_rows<C: TuiApi>(
    entries: &[ThoughtPanelEntryView],
    app: &App<C>,
    group_by: ThoughtGroupBy,
    thought_content: Rect,
    entry_capacity: usize,
) -> Vec<ThoughtRowLayout> {
    if entry_capacity == 0 {
        return Vec::new();
    }

    let groups = build_thought_groups(entries, group_by);
    let mut rows = Vec::new();
    for group in &groups {
        rows.push(group_header_row(app, group, group_by, thought_content));
        for entry in &group.entries {
            rows.extend(build_rows_for_panel_entry(entry, thought_content));
        }
    }

    let start = rows.len().saturating_sub(entry_capacity);
    rows[start..].to_vec()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanPanelEntry {
    pub(crate) slug: String,
    pub(crate) client_label: String,
    pub(crate) kind: String, // "released" | "draft"
    pub(crate) schema_path: String,
}

#[derive(Clone, Copy)]
enum ThoughtRowHitKind {
    Plan,
    Send,
    Launch,
    Commit,
    Mermaid,
    Session,
}

const THOUGHT_ROW_HIT_ORDER: [ThoughtRowHitKind; 6] = [
    ThoughtRowHitKind::Plan,
    ThoughtRowHitKind::Send,
    ThoughtRowHitKind::Launch,
    ThoughtRowHitKind::Commit,
    ThoughtRowHitKind::Mermaid,
    ThoughtRowHitKind::Session,
];

pub(crate) fn thought_panel_action_at<C: TuiApi>(
    app: &App<C>,
    thought_content: Rect,
    entry_capacity: usize,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    let panel = build_thought_panel(app, thought_content, entry_capacity);

    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    for (offset, row) in panel.rows.iter().enumerate() {
        let row_y = row_start_y + offset as u16;
        if let Some(action) = thought_row_action_at(row, row_y, x, y) {
            return Some(action);
        }
    }

    None
}

fn thought_row_action_at(
    row: &ThoughtRowLayout,
    row_y: u16,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    for hit_kind in THOUGHT_ROW_HIT_ORDER {
        let Some(rect) = thought_row_hit_rect(row, hit_kind).map(|rect| row_rect_at(rect, row_y))
        else {
            continue;
        };
        if !rect.contains(x, y) {
            continue;
        }
        if let Some(action) = thought_row_action_for_hit(row, hit_kind) {
            return Some(action);
        }
    }

    None
}

fn row_rect_at(rect: Rect, y: u16) -> Rect {
    Rect { y, ..rect }
}

fn thought_row_hit_rect(row: &ThoughtRowLayout, hit_kind: ThoughtRowHitKind) -> Option<Rect> {
    match hit_kind {
        ThoughtRowHitKind::Plan => row.plan_rect,
        ThoughtRowHitKind::Send => row.send_rect,
        ThoughtRowHitKind::Launch => row.launch_rect,
        ThoughtRowHitKind::Commit => row.commit_rect,
        ThoughtRowHitKind::Mermaid => row.mermaid_rect,
        ThoughtRowHitKind::Session => row.session_rect,
    }
}

fn thought_row_action_for_hit(
    row: &ThoughtRowLayout,
    hit_kind: ThoughtRowHitKind,
) -> Option<ThoughtPanelAction> {
    match hit_kind {
        ThoughtRowHitKind::Plan => {
            let (Some(schema_path), Some(slug)) = (&row.plan_schema_path, &row.plan_slug) else {
                return None;
            };
            Some(ThoughtPanelAction::OpenPlanFromDisk {
                schema_path: schema_path.clone(),
                slug: slug.clone(),
            })
        }
        ThoughtRowHitKind::Send => {
            row.group_session_ids
                .as_ref()
                .map(|session_ids| ThoughtPanelAction::SendGroup {
                    session_ids: session_ids.clone(),
                    label: row.label.clone(),
                })
        }
        ThoughtRowHitKind::Launch => Some(ThoughtPanelAction::OpenInitialRequest {
            cwd: row.cwd.clone(),
        }),
        ThoughtRowHitKind::Commit => Some(ThoughtPanelAction::LaunchCommitCodex(
            row.session_id.clone(),
        )),
        ThoughtRowHitKind::Mermaid => Some(ThoughtPanelAction::OpenMermaid(row.session_id.clone())),
        ThoughtRowHitKind::Session => Some(ThoughtPanelAction::OpenSession {
            session_id: row.session_id.clone(),
            label: row.label.clone(),
        }),
    }
}

fn filter_chip_data(
    summary: &ThoughtRepoSummary,
    filter: &ThoughtFilter,
) -> Option<FilterChipData> {
    let is_include_active = filter.cwd.as_deref() == Some(summary.cwd.as_str());
    let label = filter_chip_label(summary, is_include_active);
    let width = display_width(&label);
    (width > 0).then(|| FilterChipData {
        cwd: Some(summary.cwd.clone()),
        fleet: None,
        label,
        color: filter_chip_color(summary, filter, is_include_active),
        width,
    })
}

fn filter_chip_label(summary: &ThoughtRepoSummary, is_include_active: bool) -> String {
    if is_include_active {
        "code .".to_string()
    } else {
        format!("{}x{}", summary.count, summary.label)
    }
}

fn filter_chip_color(
    summary: &ThoughtRepoSummary,
    filter: &ThoughtFilter,
    is_include_active: bool,
) -> Color {
    chip_color(
        summary.color,
        filter.filter_out_mode,
        filter.excludes_cwd(&summary.cwd),
        filter.cwd.is_some(),
        is_include_active,
    )
}

fn append_filter_chip(
    included: &mut Vec<FilterChipData>,
    chips_width: &mut u16,
    chip: FilterChipData,
    chip_budget: u16,
) -> bool {
    let next_width = next_filter_chips_width(*chips_width, !included.is_empty(), chip.width);
    if next_width > chip_budget {
        return false;
    }

    *chips_width = next_width;
    included.push(chip);
    true
}

fn next_filter_chips_width(chips_width: u16, has_preceding_chip: bool, chip_width: u16) -> u16 {
    if has_preceding_chip {
        chips_width.saturating_add(2).saturating_add(chip_width)
    } else {
        chip_width
    }
}

#[cfg(test)]
mod tests;
