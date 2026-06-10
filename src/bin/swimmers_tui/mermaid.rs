use super::mermaid_ascii::pixmap_to_ascii_lines;
use super::*;
pub(crate) use swimmers::host_actions::{ArtifactOpener, SystemArtifactOpener};

mod outline;
pub(crate) use outline::*;
mod semantic;
pub(crate) use semantic::*;
mod er;
pub(crate) use er::*;
mod viewer_render;
pub(crate) use viewer_render::render_mermaid_viewer;
#[cfg(test)]
use viewer_render::{
    mermaid_viewer_body_state, render_mermaid_cached_background, MermaidViewerBodyState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DomainPlanTab {
    Schema,
    Plan,
    Shared,
    Backend,
    Frontend,
    Flows,
    Workgraph,
    Readme,
    Vision,
}

impl DomainPlanTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Plan => "plan",
            Self::Shared => "shared",
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Flows => "flows",
            Self::Workgraph => "WORKGRAPH",
            Self::Readme => "README",
            Self::Vision => "VISION",
        }
    }

    pub(crate) fn filename(self) -> &'static str {
        match self {
            Self::Schema => "schema.mmd",
            Self::Plan => "plan.md",
            Self::Shared => "shared.md",
            Self::Backend => "backend.md",
            Self::Frontend => "frontend.md",
            Self::Flows => "flows.md",
            Self::Workgraph => "WORKGRAPH.md",
            Self::Readme => "README.md",
            Self::Vision => "VISION.md",
        }
    }

    pub(crate) fn from_filename(name: &str) -> Option<Self> {
        match name {
            "schema.mmd" => Some(Self::Schema),
            "plan.md" => Some(Self::Plan),
            "shared.md" => Some(Self::Shared),
            "backend.md" => Some(Self::Backend),
            "frontend.md" => Some(Self::Frontend),
            "flows.md" => Some(Self::Flows),
            "WORKGRAPH.md" => Some(Self::Workgraph),
            "README.md" => Some(Self::Readme),
            "VISION.md" => Some(Self::Vision),
            _ => None,
        }
    }
}

pub(crate) enum FishBowlMode {
    Aquarium,
    Mermaid(Box<MermaidViewerState>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MermaidSourceCacheKey {
    pub(crate) source_hash: u64,
    pub(crate) sample_width: u32,
    pub(crate) sample_height: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidPreparedRender {
    pub(crate) key: MermaidSourceCacheKey,
    pub(crate) tree: Tree,
    pub(crate) layout: MermaidLayout,
    pub(crate) semantic_lines: Vec<MermaidSemanticLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MermaidTextAnchor {
    Start,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MermaidDetailLevel {
    L1,
    L2,
    L3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MermaidViewState {
    Outline,
    L1,
    L2,
    L3,
    ErEntities,
    ErKeys,
    ErColumns,
    ErSchema,
}

impl MermaidViewState {
    pub(crate) fn status_label(self) -> &'static str {
        match self {
            MermaidViewState::Outline => "outline",
            MermaidViewState::L1 => "detail L1",
            MermaidViewState::L2 => "detail L2",
            MermaidViewState::L3 => "detail L3",
            MermaidViewState::ErEntities => "ER entities",
            MermaidViewState::ErKeys => "ER keys",
            MermaidViewState::ErColumns => "ER columns",
            MermaidViewState::ErSchema => "ER schema",
        }
    }

    pub(crate) fn detail_level(self) -> Option<MermaidDetailLevel> {
        match self {
            MermaidViewState::L1 => Some(MermaidDetailLevel::L1),
            MermaidViewState::L2 => Some(MermaidDetailLevel::L2),
            MermaidViewState::L3 => Some(MermaidDetailLevel::L3),
            MermaidViewState::Outline
            | MermaidViewState::ErEntities
            | MermaidViewState::ErKeys
            | MermaidViewState::ErColumns
            | MermaidViewState::ErSchema => None,
        }
    }

    pub(crate) fn collision_padding(self) -> u16 {
        match self {
            MermaidViewState::Outline | MermaidViewState::L1 => 2,
            MermaidViewState::L2 | MermaidViewState::ErEntities | MermaidViewState::ErKeys => 1,
            MermaidViewState::L3 | MermaidViewState::ErColumns | MermaidViewState::ErSchema => 0,
        }
    }

    pub(crate) fn is_er(self) -> bool {
        matches!(
            self,
            MermaidViewState::ErEntities
                | MermaidViewState::ErKeys
                | MermaidViewState::ErColumns
                | MermaidViewState::ErSchema
        )
    }
}

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MermaidSemanticKind {
    SubgraphSummary,
    SubgraphTitle,
    NodeSummary,
    NodeTitle,
    EdgeLabel,
    ClassMember,
    ErAttributeName,
    ErAttributeType,
}

#[derive(Clone, Copy, Debug)]
struct MermaidOwnerVisibilityThreshold {
    always_visible: bool,
    min_cols: f32,
    min_rows: f32,
}

impl MermaidOwnerVisibilityThreshold {
    const fn always() -> Self {
        Self {
            always_visible: true,
            min_cols: 0.0,
            min_rows: 0.0,
        }
    }

    const fn at_least(min_cols: f32, min_rows: f32) -> Self {
        Self {
            always_visible: false,
            min_cols,
            min_rows,
        }
    }

    fn is_visible_for(self, owner_cols: f32, owner_rows: f32) -> bool {
        self.always_visible || (owner_cols >= self.min_cols && owner_rows >= self.min_rows)
    }
}

const MERMAID_OWNER_VISIBILITY_THRESHOLDS: [MermaidOwnerVisibilityThreshold; 8] = [
    MermaidOwnerVisibilityThreshold::at_least(10.0, 1.0),
    MermaidOwnerVisibilityThreshold::always(),
    MermaidOwnerVisibilityThreshold::at_least(8.0, 1.0),
    MermaidOwnerVisibilityThreshold::always(),
    MermaidOwnerVisibilityThreshold::always(),
    MermaidOwnerVisibilityThreshold::at_least(10.0, 2.5),
    MermaidOwnerVisibilityThreshold::at_least(8.0, 2.5),
    MermaidOwnerVisibilityThreshold::at_least(12.0, 3.0),
];

impl MermaidSemanticKind {
    pub(crate) fn min_detail_level(self) -> MermaidDetailLevel {
        match self {
            MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => {
                MermaidDetailLevel::L1
            }
            MermaidSemanticKind::SubgraphTitle | MermaidSemanticKind::NodeTitle => {
                MermaidDetailLevel::L2
            }
            MermaidSemanticKind::EdgeLabel
            | MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName => MermaidDetailLevel::L2,
            MermaidSemanticKind::ErAttributeType => MermaidDetailLevel::L3,
        }
    }

    pub(crate) fn priority(self) -> u8 {
        match self {
            MermaidSemanticKind::SubgraphSummary => 0,
            MermaidSemanticKind::NodeSummary => 1,
            MermaidSemanticKind::SubgraphTitle => 2,
            MermaidSemanticKind::NodeTitle => 3,
            MermaidSemanticKind::ErAttributeName => 4,
            MermaidSemanticKind::ClassMember => 5,
            MermaidSemanticKind::EdgeLabel => 6,
            MermaidSemanticKind::ErAttributeType => 7,
        }
    }

    pub(crate) fn is_visible_for_owner(self, owner_cols: f32, owner_rows: f32) -> bool {
        self.owner_visibility_threshold()
            .is_visible_for(owner_cols, owner_rows)
    }

    fn owner_visibility_threshold(self) -> MermaidOwnerVisibilityThreshold {
        MERMAID_OWNER_VISIBILITY_THRESHOLDS[self as usize]
    }

    pub(crate) fn row_nudge_budget(self) -> i32 {
        match self {
            MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName
            | MermaidSemanticKind::ErAttributeType => 2,
            MermaidSemanticKind::SubgraphSummary
            | MermaidSemanticKind::NodeSummary
            | MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::EdgeLabel => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidSemanticLine {
    pub(crate) text: String,
    pub(crate) diagram_x: f32,
    pub(crate) diagram_y: f32,
    pub(crate) anchor: MermaidTextAnchor,
    pub(crate) kind: MermaidSemanticKind,
    pub(crate) owner_key: String,
    pub(crate) outline_eligible: bool,
    pub(crate) owner_width: f32,
    pub(crate) owner_height: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidProjectedLine {
    pub(crate) source_index: usize,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) text: String,
    pub(crate) color: Color,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidFocusTarget {
    pub(crate) source_index: usize,
    pub(crate) text: String,
    pub(crate) diagram_x: f32,
    pub(crate) diagram_y: f32,
    pub(crate) hitbox: Rect,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidOutlineNode {
    pub(crate) key: String,
    pub(crate) source_index: usize,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) text_width: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidOutlineEdge {
    pub(crate) from_key: String,
    pub(crate) to_key: String,
    pub(crate) directed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MermaidOutlineAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MermaidOutlineSegment {
    pub(crate) axis: MermaidOutlineAxis,
    pub(crate) fixed: i32,
    pub(crate) start: i32,
    pub(crate) end: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MermaidOutlineArrow {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) ch: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MermaidOutlineLabelRect {
    pub(crate) left: i32,
    pub(crate) right: i32,
    pub(crate) top: i32,
    pub(crate) bottom: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidPackedDetailLine {
    pub(crate) source_index: usize,
    pub(crate) text: String,
    pub(crate) color: Color,
    pub(crate) kind: MermaidSemanticKind,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidPackedDetailOwner {
    pub(crate) owner_key: String,
    pub(crate) sort_x: u16,
    pub(crate) sort_y: u16,
    pub(crate) lines: Vec<MermaidPackedDetailLine>,
}

#[derive(Clone, Copy)]
struct MermaidPackedDetailBoxSize {
    outer_width: u16,
    outer_height: u16,
}

#[derive(Clone, Copy)]
struct MermaidPackedDetailViewport {
    width: u16,
    height: u16,
    gap_x: u16,
    gap_y: u16,
}

struct MermaidPackedDetailLayout {
    column_count: usize,
    row_widths: Vec<u16>,
    row_heights: Vec<u16>,
    cluster_height: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidViewerState {
    pub(crate) session_id: String,
    pub(crate) tmux_name: String,
    pub(crate) cwd: String,
    pub(crate) path: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) artifact_error: Option<String>,
    pub(crate) render_error: Option<String>,
    pub(crate) unsupported_reason: Option<String>,
    pub(crate) zoom: f32,
    pub(crate) center_x: f32,
    pub(crate) center_y: f32,
    pub(crate) diagram_width: f32,
    pub(crate) diagram_height: f32,
    pub(crate) back_rect: Option<Rect>,
    pub(crate) content_rect: Option<Rect>,
    pub(crate) cached_rect: Option<Rect>,
    pub(crate) cached_zoom: f32,
    pub(crate) cached_center_x: f32,
    pub(crate) cached_center_y: f32,
    pub(crate) cached_lines: Vec<String>,
    pub(crate) cached_background_cells: Vec<Vec<Cell>>,
    pub(crate) cached_semantic_lines: Vec<MermaidProjectedLine>,
    pub(crate) focused_source_index: Option<usize>,
    pub(crate) focus_status: Option<String>,
    pub(crate) prepared_render: Option<MermaidPreparedRender>,
    pub(crate) source_prepare_count: u64,
    pub(crate) viewport_render_count: u64,
    // Domain plan tab state
    pub(crate) plan_tabs: Option<Vec<DomainPlanTab>>,
    pub(crate) active_tab: DomainPlanTab,
    pub(crate) inline_plan_files: BTreeMap<DomainPlanTab, String>,
    pub(crate) plan_text_content: Option<String>,
    pub(crate) plan_text_lines: Vec<String>,
    pub(crate) plan_text_scroll: usize,
    pub(crate) plan_text_cached_width: u16,
    pub(crate) tab_rects: Vec<(DomainPlanTab, Rect)>,
    /// True when the viewer was opened from a plan directory on disk (no
    /// backing tmux session). Tab-switching reads sibling files directly
    /// instead of calling the session-scoped plan-file API.
    pub(crate) disk_only: bool,
}

impl MermaidViewerState {
    pub(crate) fn display_path(&self) -> &str {
        self.path.as_deref().unwrap_or("unknown.mmd")
    }

    pub(crate) fn openable_path(&self) -> Option<&str> {
        self.path.as_deref().filter(|path| !path.trim().is_empty())
    }

    pub(crate) fn invalidate_viewport_cache(&mut self) {
        self.cached_rect = None;
    }

    pub(crate) fn invalidate_source_cache(&mut self) {
        self.prepared_render = None;
        self.cached_lines.clear();
        self.cached_background_cells.clear();
        self.cached_semantic_lines.clear();
        self.render_error = None;
        self.focused_source_index = None;
        self.focus_status = None;
        self.invalidate_viewport_cache();
    }
}

pub(crate) const MERMAID_CONNECTOR_COLOR: Color = Color::DarkGrey;
pub(crate) const MERMAID_BODY_COLOR: Color = Color::White;
pub(crate) const MERMAID_EDGE_LABEL_COLOR: Color = Color::Yellow;
pub(crate) const MERMAID_TYPE_COLOR: Color = Color::DarkCyan;
pub(crate) const MERMAID_FOCUS_COLOR: Color = Color::Magenta;
// Guardrail: skip parsing giant Mermaid inputs to keep the UI responsive and
// avoid unbounded render work on malformed or oversized artifacts.
const MERMAID_SOURCE_MAX_BYTES: usize = 64 * 1024;
// Guardrail: cap rendered rows to keep worst-case terminal work bounded.
const MERMAID_RENDER_MAX_ROWS: usize = 200;
const MERMAID_TRUNCATED_MARKER: &str = "(…truncated)";
pub(crate) const MERMAID_OWNER_ACCENTS: [Color; 4] =
    [Color::Cyan, Color::Green, Color::Yellow, Color::Blue];

pub(crate) fn mermaid_owner_accent_map(lines: &[MermaidSemanticLine]) -> HashMap<String, Color> {
    let mut owner_keys = lines
        .iter()
        .filter(|line| {
            mermaid_kind_is_owner_summary(line.kind) || mermaid_kind_is_owner_detail(line.kind)
        })
        .map(|line| line.owner_key.clone())
        .collect::<Vec<_>>();
    owner_keys.sort();
    owner_keys.dedup();
    owner_keys
        .into_iter()
        .enumerate()
        .map(|(idx, owner_key)| {
            (
                owner_key,
                MERMAID_OWNER_ACCENTS[idx % MERMAID_OWNER_ACCENTS.len()],
            )
        })
        .collect()
}

pub(crate) fn mermaid_owner_accent_color(
    owner_key: &str,
    owner_colors: &HashMap<String, Color>,
) -> Color {
    owner_colors
        .get(owner_key)
        .copied()
        .unwrap_or(MERMAID_OWNER_ACCENTS[0])
}

pub(crate) fn mermaid_semantic_line_color(
    kind: MermaidSemanticKind,
    owner_key: &str,
    owner_colors: &HashMap<String, Color>,
) -> Color {
    match kind {
        MermaidSemanticKind::SubgraphSummary
        | MermaidSemanticKind::SubgraphTitle
        | MermaidSemanticKind::NodeSummary
        | MermaidSemanticKind::NodeTitle => mermaid_owner_accent_color(owner_key, owner_colors),
        MermaidSemanticKind::EdgeLabel => MERMAID_EDGE_LABEL_COLOR,
        MermaidSemanticKind::ClassMember | MermaidSemanticKind::ErAttributeName => {
            MERMAID_BODY_COLOR
        }
        MermaidSemanticKind::ErAttributeType => MERMAID_TYPE_COLOR,
    }
}

pub(crate) fn mermaid_background_cells_from_lines(
    lines: &[String],
    default_color: Color,
) -> Vec<Vec<Cell>> {
    lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|ch| Cell {
                    ch,
                    fg: if ch == ' ' {
                        Color::Reset
                    } else {
                        default_color
                    },
                })
                .collect()
        })
        .collect()
}

pub(crate) fn mermaid_set_background_cell_color(
    cells: &mut [Vec<Cell>],
    content_rect: Rect,
    x: i32,
    y: i32,
    color: Color,
) {
    if let Some(cell) = mermaid_background_cell_mut(cells, content_rect, x, y) {
        mermaid_recolor_background_cell(cell, color);
    }
}

fn mermaid_background_cell_mut(
    cells: &mut [Vec<Cell>],
    content_rect: Rect,
    x: i32,
    y: i32,
) -> Option<&mut Cell> {
    let (grid_x, grid_y) = mermaid_background_cell_indices(content_rect, x, y)?;
    cells.get_mut(grid_y)?.get_mut(grid_x)
}

fn mermaid_background_cell_indices(content_rect: Rect, x: i32, y: i32) -> Option<(usize, usize)> {
    let left = content_rect.x as i32;
    let right = content_rect.right() as i32;
    let top = content_rect.y as i32;
    let bottom = content_rect.bottom() as i32;

    (x >= left && x < right && y >= top && y < bottom)
        .then(|| ((x - left) as usize, (y - top) as usize))
}

fn mermaid_recolor_background_cell(cell: &mut Cell, color: Color) {
    if mermaid_background_cell_can_take_color(cell) {
        cell.fg = color;
    }
}

fn mermaid_background_cell_can_take_color(cell: &Cell) -> bool {
    cell.ch != ' '
}

pub(crate) fn mermaid_apply_rect_border_colors(
    cells: &mut [Vec<Cell>],
    content_rect: Rect,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
    owner_colors: &HashMap<String, Color>,
) {
    for (owner_key, rect) in label_rects {
        let color = mermaid_owner_accent_color(owner_key, owner_colors);
        for x in rect.left..=rect.right {
            mermaid_set_background_cell_color(cells, content_rect, x, rect.top, color);
            mermaid_set_background_cell_color(cells, content_rect, x, rect.bottom, color);
        }
        for y in rect.top..=rect.bottom {
            mermaid_set_background_cell_color(cells, content_rect, rect.left, y, color);
            mermaid_set_background_cell_color(cells, content_rect, rect.right, y, color);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MermaidDragState {
    pub(crate) start_column: u16,
    pub(crate) start_row: u16,
    pub(crate) start_center_x: f32,
    pub(crate) start_center_y: f32,
}

pub(crate) fn detect_mermaid_backend_support() -> Option<String> {
    let term = env::var("TERM").unwrap_or_default();
    if term.is_empty() || term == "dumb" {
        return Some("inline Mermaid rendering is unsupported for TERM=dumb".to_string());
    }
    None
}

pub(crate) fn mermaid_content_rect(field: Rect) -> Rect {
    if field.height <= 1 {
        return Rect {
            x: field.x,
            y: field.y,
            width: field.width,
            height: 0,
        };
    }
    Rect {
        x: field.x,
        y: field.y + 1,
        width: field.width,
        height: field.height - 1,
    }
}

pub(crate) fn mermaid_sample_dimensions(content_rect: Rect) -> (u32, u32) {
    (
        u32::from(content_rect.width.max(1)) * 2,
        u32::from(content_rect.height.max(1)) * 4,
    )
}

pub(crate) fn mermaid_fit_scale(
    diagram_width: f32,
    diagram_height: f32,
    sample_width: f32,
    sample_height: f32,
) -> f32 {
    if diagram_width <= 0.0 || diagram_height <= 0.0 || sample_width <= 0.0 || sample_height <= 0.0
    {
        return 1.0;
    }
    (sample_width / diagram_width)
        .min(sample_height / diagram_height)
        .max(0.000_1)
}

pub(crate) fn mermaid_strip_svg_text(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut cursor = 0usize;

    while let Some(start_rel) = svg[cursor..].find("<text") {
        let start = cursor + start_rel;
        out.push_str(&svg[cursor..start]);
        let Some(end_rel) = svg[start..].find("</text>") else {
            cursor = svg.len();
            break;
        };
        cursor = start + end_rel + "</text>".len();
    }

    out.push_str(&svg[cursor..]);
    out
}

pub(crate) fn clamp_mermaid_center(center: f32, visible: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    if visible >= total {
        return total / 2.0;
    }
    center.clamp(visible / 2.0, total - visible / 2.0)
}

pub(crate) fn mermaid_pan_step(viewer: &MermaidViewerState, content_rect: Rect) -> (f32, f32) {
    if viewer.diagram_width <= 0.0 || viewer.diagram_height <= 0.0 {
        return (40.0, 24.0);
    }
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.max(MERMAID_MIN_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;
    (visible_width / 6.0, visible_height / 6.0)
}

pub(crate) fn mermaid_source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn mermaid_source_cache_key(source: &str, content_rect: Rect) -> MermaidSourceCacheKey {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    MermaidSourceCacheKey {
        source_hash: mermaid_source_hash(source),
        sample_width,
        sample_height,
    }
}

pub(crate) fn mermaid_render_options(content_rect: Rect) -> RenderOptions {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    RenderOptions::default()
        .with_preferred_aspect_ratio_parts(sample_width as f32, sample_height as f32)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MermaidViewportTransform {
    pub(crate) scale: f32,
    pub(crate) tx: f32,
    pub(crate) ty: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MermaidZoomDirection {
    In,
    Out,
}

pub(crate) fn mermaid_zoom_percent(zoom: f32) -> i16 {
    (zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM) * 100.0).round() as i16
}

pub(crate) fn mermaid_zoom_status_label(zoom: f32) -> String {
    let percent = mermaid_zoom_percent(zoom);
    if percent <= 100 {
        "fit 100%".to_string()
    } else {
        format!("zoom {percent}%")
    }
}

fn mermaid_truncate_lines_with_marker(lines: &mut Vec<String>, max_rows: usize) -> bool {
    if lines.len() <= max_rows {
        return false;
    }

    lines.truncate(max_rows);
    if max_rows != 0 {
        lines[max_rows - 1] = MERMAID_TRUNCATED_MARKER.to_string();
    }
    true
}

fn mermaid_apply_render_line_cap(viewer: &mut MermaidViewerState, content_rect: Rect) {
    let original_rows = viewer.cached_lines.len();
    if !mermaid_truncate_lines_with_marker(&mut viewer.cached_lines, MERMAID_RENDER_MAX_ROWS) {
        return;
    }

    tracing::warn!(
        session_id = %viewer.session_id,
        rendered_rows = original_rows,
        cap_rows = MERMAID_RENDER_MAX_ROWS,
        "Mermaid rendered output exceeded row cap; truncating"
    );

    viewer.cached_background_cells.clear();
    let cutoff_row = content_rect
        .y
        .saturating_add(MERMAID_RENDER_MAX_ROWS.saturating_sub(1) as u16);
    viewer
        .cached_semantic_lines
        .retain(|line| line.y < cutoff_row);
}

pub(crate) fn ensure_mermaid_viewport_cache(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let needs_rerender = viewer.cached_rect != Some(content_rect)
        || viewer.prepared_render.is_none()
        || (viewer.cached_zoom - viewer.zoom).abs() > f32::EPSILON
        || (viewer.cached_center_x - viewer.center_x).abs() > f32::EPSILON
        || (viewer.cached_center_y - viewer.center_y).abs() > f32::EPSILON;
    if needs_rerender {
        render_mermaid_lines(viewer, content_rect)?;
    }
    Ok(())
}

pub(crate) fn mermaid_view_state_for_view(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> MermaidViewState {
    let _ = content_rect;
    if mermaid_is_er_viewer(viewer) {
        return mermaid_er_view_state_for_zoom(viewer.zoom);
    }
    let zoom_percent = mermaid_zoom_percent(viewer.zoom);

    if zoom_percent <= 100 {
        MermaidViewState::Outline
    } else if zoom_percent >= 250 {
        MermaidViewState::L3
    } else if zoom_percent >= 150 {
        MermaidViewState::L2
    } else {
        MermaidViewState::L1
    }
}

pub(crate) fn mermaid_viewport_transform(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(u32, u32, MermaidViewportTransform), String> {
    ensure_mermaid_prepared_render(viewer, content_rect)?;
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    if viewer.center_x <= 0.0 && viewer.center_y <= 0.0 {
        viewer.center_x = viewer.diagram_width / 2.0;
        viewer.center_y = viewer.diagram_height / 2.0;
    }

    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;
    viewer.center_x = clamp_mermaid_center(viewer.center_x, visible_width, viewer.diagram_width);
    viewer.center_y = clamp_mermaid_center(viewer.center_y, visible_height, viewer.diagram_height);

    Ok((
        sample_width,
        sample_height,
        MermaidViewportTransform {
            scale,
            tx: sample_width as f32 / 2.0 - viewer.center_x * scale,
            ty: sample_height as f32 / 2.0 - viewer.center_y * scale,
        },
    ))
}

pub(crate) fn mermaid_is_compact_box_owner_key(owner_key: &str) -> bool {
    owner_key.starts_with("node:") || owner_key.starts_with("subgraph:")
}

pub(crate) fn mermaid_detail_box_rects(
    semantic_lines: &[MermaidSemanticLine],
    projected: &[MermaidProjectedLine],
    content_rect: Rect,
) -> HashMap<String, MermaidOutlineLabelRect> {
    let mut rects = collect_mermaid_detail_box_rects(semantic_lines, projected);
    let bounds = MermaidDetailBoxBounds::from_content_rect(content_rect);
    rects
        .values_mut()
        .for_each(|rect| expand_mermaid_detail_box_rect(rect, bounds));

    rects
}

#[derive(Clone, Copy)]
struct MermaidDetailBoxBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl MermaidDetailBoxBounds {
    fn from_content_rect(content_rect: Rect) -> Self {
        Self {
            min_x: content_rect.x as i32,
            max_x: content_rect.right() as i32 - 1,
            min_y: content_rect.y as i32,
            max_y: content_rect.bottom() as i32 - 1,
        }
    }
}

fn collect_mermaid_detail_box_rects(
    semantic_lines: &[MermaidSemanticLine],
    projected: &[MermaidProjectedLine],
) -> HashMap<String, MermaidOutlineLabelRect> {
    let mut rects = HashMap::<String, MermaidOutlineLabelRect>::new();
    for line in projected {
        if let Some((owner_key, line_rect)) = mermaid_detail_box_line_rect(semantic_lines, line) {
            merge_mermaid_detail_box_rect(&mut rects, owner_key, line_rect);
        }
    }
    rects
}

fn mermaid_detail_box_line_rect(
    semantic_lines: &[MermaidSemanticLine],
    line: &MermaidProjectedLine,
) -> Option<(String, MermaidOutlineLabelRect)> {
    let source = semantic_lines.get(line.source_index)?;
    mermaid_is_compact_box_owner_key(&source.owner_key).then(|| {
        let line_left = line.x as i32;
        let line_right = line_left + display_width(&line.text) as i32 - 1;
        (
            source.owner_key.clone(),
            MermaidOutlineLabelRect {
                left: line_left,
                right: line_right,
                top: line.y as i32,
                bottom: line.y as i32,
            },
        )
    })
}

fn merge_mermaid_detail_box_rect(
    rects: &mut HashMap<String, MermaidOutlineLabelRect>,
    owner_key: String,
    line_rect: MermaidOutlineLabelRect,
) {
    rects
        .entry(owner_key)
        .and_modify(|rect| {
            rect.left = rect.left.min(line_rect.left);
            rect.right = rect.right.max(line_rect.right);
            rect.top = rect.top.min(line_rect.top);
            rect.bottom = rect.bottom.max(line_rect.bottom);
        })
        .or_insert(line_rect);
}

fn expand_mermaid_detail_box_rect(
    rect: &mut MermaidOutlineLabelRect,
    bounds: MermaidDetailBoxBounds,
) {
    rect.left = (rect.left - 1).clamp(bounds.min_x, bounds.max_x);
    rect.right = (rect.right + 1).clamp(rect.left, bounds.max_x);
    rect.top = (rect.top - 1).clamp(bounds.min_y, bounds.max_y);
    rect.bottom = (rect.bottom + 1).clamp(rect.top, bounds.max_y);
    expand_mermaid_detail_box_min_extent(rect, bounds);
}

fn expand_mermaid_detail_box_min_extent(
    rect: &mut MermaidOutlineLabelRect,
    bounds: MermaidDetailBoxBounds,
) {
    if rect.right == rect.left && rect.right < bounds.max_x {
        rect.right += 1;
    }
    if rect.bottom == rect.top && rect.bottom < bounds.max_y {
        rect.bottom += 1;
    }
}

pub(crate) fn mermaid_build_packed_detail_owners(
    semantic_lines: &[MermaidSemanticLine],
    projected: &[MermaidProjectedLine],
) -> Vec<MermaidPackedDetailOwner> {
    let mut grouped = HashMap::<String, MermaidPackedDetailOwner>::new();

    for line in projected {
        let Some(source) = semantic_lines.get(line.source_index) else {
            continue;
        };
        if !mermaid_is_compact_box_owner_key(&source.owner_key) {
            continue;
        }

        grouped
            .entry(source.owner_key.clone())
            .and_modify(|owner| {
                owner.sort_x = owner.sort_x.min(line.x);
                owner.sort_y = owner.sort_y.min(line.y);
                owner.lines.push(MermaidPackedDetailLine {
                    source_index: line.source_index,
                    text: line.text.clone(),
                    color: line.color,
                    kind: source.kind,
                });
            })
            .or_insert_with(|| MermaidPackedDetailOwner {
                owner_key: source.owner_key.clone(),
                sort_x: line.x,
                sort_y: line.y,
                lines: vec![MermaidPackedDetailLine {
                    source_index: line.source_index,
                    text: line.text.clone(),
                    color: line.color,
                    kind: source.kind,
                }],
            });
    }

    let mut owners = grouped.into_values().collect::<Vec<_>>();
    owners.sort_by_key(|owner| (owner.sort_y, owner.sort_x, owner.owner_key.clone()));
    owners
}

fn mermaid_measure_packed_detail_box(
    owner: &MermaidPackedDetailOwner,
    viewport: MermaidPackedDetailViewport,
) -> MermaidPackedDetailBoxSize {
    let inner_width = owner
        .lines
        .iter()
        .map(|line| display_width(&line.text))
        .max()
        .unwrap_or(1)
        .min(viewport.width.saturating_sub(2).max(1));
    let inner_height = owner.lines.len().max(1) as u16;

    MermaidPackedDetailBoxSize {
        outer_width: inner_width.saturating_add(2).min(viewport.width.max(1)),
        outer_height: inner_height.saturating_add(2).min(viewport.height.max(1)),
    }
}

fn mermaid_measure_packed_detail_boxes(
    owners: &[MermaidPackedDetailOwner],
    viewport: MermaidPackedDetailViewport,
) -> Vec<MermaidPackedDetailBoxSize> {
    owners
        .iter()
        .map(|owner| mermaid_measure_packed_detail_box(owner, viewport))
        .collect()
}

fn mermaid_measure_packed_detail_row(
    row: &[MermaidPackedDetailBoxSize],
    gap_x: u16,
) -> Option<(u16, u16)> {
    let row_width = row
        .iter()
        .map(|spec| spec.outer_width)
        .sum::<u16>()
        .saturating_add(gap_x.saturating_mul(row.len().saturating_sub(1) as u16));
    let row_height = row.iter().map(|spec| spec.outer_height).max()?;

    Some((row_width, row_height))
}

fn mermaid_packed_detail_rows_fit(
    specs: &[MermaidPackedDetailBoxSize],
    viewport: MermaidPackedDetailViewport,
    column_count: usize,
) -> Option<(Vec<u16>, Vec<u16>)> {
    let mut row_widths = Vec::new();
    let mut row_heights = Vec::new();

    for row in specs.chunks(column_count) {
        let (row_width, row_height) = mermaid_measure_packed_detail_row(row, viewport.gap_x)?;
        if row_width > viewport.width || row_height > viewport.height {
            return None;
        }
        row_widths.push(row_width);
        row_heights.push(row_height);
    }

    (!row_widths.is_empty()).then_some((row_widths, row_heights))
}

fn mermaid_packed_detail_cluster_height(row_heights: &[u16], gap_y: u16) -> u16 {
    row_heights
        .iter()
        .copied()
        .sum::<u16>()
        .saturating_add(gap_y.saturating_mul(row_heights.len().saturating_sub(1) as u16))
}

fn mermaid_score_packed_detail_layout(
    viewport: MermaidPackedDetailViewport,
    cluster_width: u16,
    cluster_height: u16,
) -> f32 {
    let target_aspect = viewport.width as f32 / viewport.height as f32;
    let width_util = cluster_width as f32 / viewport.width as f32;
    let height_util = cluster_height as f32 / viewport.height as f32;
    let area_util = width_util * height_util;
    let aspect = cluster_width as f32 / cluster_height.max(1) as f32;
    let aspect_penalty = (aspect - target_aspect).abs();

    width_util.min(height_util) * 1000.0 + area_util * 400.0 - aspect_penalty * 40.0
}

fn mermaid_packed_detail_cluster_fits(
    viewport: MermaidPackedDetailViewport,
    cluster_width: u16,
    cluster_height: u16,
) -> bool {
    cluster_width <= viewport.width && cluster_height <= viewport.height
}

fn mermaid_build_packed_detail_layout(
    column_count: usize,
    row_widths: Vec<u16>,
    row_heights: Vec<u16>,
    cluster_height: u16,
) -> MermaidPackedDetailLayout {
    MermaidPackedDetailLayout {
        column_count,
        row_widths,
        row_heights,
        cluster_height,
    }
}

fn mermaid_packed_detail_layout_for_columns(
    specs: &[MermaidPackedDetailBoxSize],
    viewport: MermaidPackedDetailViewport,
    column_count: usize,
) -> Option<(MermaidPackedDetailLayout, f32)> {
    let (row_widths, row_heights) = mermaid_packed_detail_rows_fit(specs, viewport, column_count)?;
    let cluster_width = row_widths.iter().copied().max().unwrap_or(0);
    let cluster_height = mermaid_packed_detail_cluster_height(&row_heights, viewport.gap_y);

    mermaid_packed_detail_cluster_fits(viewport, cluster_width, cluster_height).then(|| {
        (
            mermaid_build_packed_detail_layout(
                column_count,
                row_widths,
                row_heights,
                cluster_height,
            ),
            mermaid_score_packed_detail_layout(viewport, cluster_width, cluster_height),
        )
    })
}

pub(crate) fn mermaid_pack_detail_box_rects(
    content_rect: Rect,
    owners: &[MermaidPackedDetailOwner],
) -> HashMap<String, MermaidOutlineLabelRect> {
    let viewport = MermaidPackedDetailViewport {
        width: content_rect.width.max(1),
        height: content_rect.height.max(1),
        gap_x: 2,
        gap_y: 1,
    };
    let specs = mermaid_measure_packed_detail_boxes(owners, viewport);
    if specs.is_empty() {
        return HashMap::new();
    }

    let layout = mermaid_best_packed_detail_layout(&specs, viewport)
        .unwrap_or_else(|| mermaid_fallback_packed_detail_layout(&specs, viewport));

    mermaid_place_packed_detail_box_rects(content_rect, owners, &specs, viewport, layout)
}

pub(crate) fn mermaid_project_packed_detail_lines(
    owners: &[MermaidPackedDetailOwner],
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> Vec<MermaidProjectedLine> {
    let mut projected = Vec::new();

    for owner in owners {
        let Some(rect) = label_rects.get(&owner.owner_key).copied() else {
            continue;
        };
        let inner_left = rect.left + 1;
        let inner_width = (rect.right - rect.left - 1).max(1) as u16;
        let mut line_y = rect.top + 1;
        for line in &owner.lines {
            let text = clip_mermaid_overlay_text(&line.text, 0, inner_width as usize);
            if text.is_empty() {
                continue;
            }
            let text_width = display_width(&text).min(inner_width);
            let x = match line.kind {
                MermaidSemanticKind::SubgraphSummary
                | MermaidSemanticKind::SubgraphTitle
                | MermaidSemanticKind::NodeSummary
                | MermaidSemanticKind::NodeTitle => {
                    inner_left + i32::from(inner_width.saturating_sub(text_width) / 2)
                }
                MermaidSemanticKind::ClassMember
                | MermaidSemanticKind::ErAttributeName
                | MermaidSemanticKind::ErAttributeType
                | MermaidSemanticKind::EdgeLabel => inner_left,
            };
            projected.push(MermaidProjectedLine {
                source_index: line.source_index,
                x: x as u16,
                y: line_y as u16,
                text,
                color: line.color,
            });
            line_y += 1;
        }
    }

    projected.sort_by_key(|line| (line.y, line.x));
    projected
}

pub(crate) fn mermaid_status_detail_label(view_state: MermaidViewState) -> String {
    view_state.status_label().to_string()
}

#[derive(Clone)]
struct MermaidFocusAccumulator {
    source_index: usize,
    text: String,
    diagram_x: f32,
    diagram_y: f32,
    priority: u8,
    sort_y: u16,
    sort_x: u16,
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
}

pub(crate) fn mermaid_visible_focus_targets(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<Vec<MermaidFocusTarget>, String> {
    if content_rect.width < MERMAID_VIEW_MIN_WIDTH || content_rect.height < MERMAID_VIEW_MIN_HEIGHT
    {
        return Ok(Vec::new());
    }
    if viewer.unsupported_reason.is_some() || viewer.artifact_error.is_some() {
        return Ok(Vec::new());
    }

    ensure_mermaid_viewport_cache(viewer, content_rect)?;
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Ok(Vec::new());
    };

    let grouped = mermaid_group_visible_focus_targets(
        &viewer.cached_semantic_lines,
        &prepared.semantic_lines,
    );

    let max_x = content_rect.right().saturating_sub(1);
    let max_y = content_rect.bottom().saturating_sub(1);
    let mut targets = grouped
        .into_values()
        .map(|target| {
            let left = target.left.saturating_sub(1).max(content_rect.x);
            let top = target.top.saturating_sub(1).max(content_rect.y);
            let right = target.right.saturating_add(1).min(max_x);
            let bottom = target.bottom.saturating_add(1).min(max_y);
            MermaidFocusTarget {
                source_index: target.source_index,
                text: target.text,
                diagram_x: target.diagram_x,
                diagram_y: target.diagram_y,
                hitbox: Rect {
                    x: left,
                    y: top,
                    width: right.saturating_sub(left).saturating_add(1),
                    height: bottom.saturating_sub(top).saturating_add(1),
                },
            }
        })
        .collect::<Vec<_>>();
    targets.sort_by_key(|target| (target.hitbox.y, target.hitbox.x, target.source_index));
    Ok(targets)
}

fn mermaid_group_visible_focus_targets(
    projected_lines: &[MermaidProjectedLine],
    source_lines: &[MermaidSemanticLine],
) -> HashMap<String, MermaidFocusAccumulator> {
    let mut grouped = HashMap::<String, MermaidFocusAccumulator>::new();
    for line in projected_lines {
        let Some(source) = source_lines.get(line.source_index) else {
            continue;
        };
        if mermaid_focus_source_is_visible_target(source.kind) {
            mermaid_accumulate_visible_focus_line(&mut grouped, line, source);
        }
    }
    grouped
}

fn mermaid_focus_source_is_visible_target(kind: MermaidSemanticKind) -> bool {
    mermaid_kind_is_owner_summary(kind) || mermaid_kind_is_owner_detail(kind)
}

fn mermaid_accumulate_visible_focus_line(
    grouped: &mut HashMap<String, MermaidFocusAccumulator>,
    line: &MermaidProjectedLine,
    source: &MermaidSemanticLine,
) {
    let line_width = display_width(&line.text).max(1);
    let line_right = line.x.saturating_add(line_width.saturating_sub(1));
    let priority = mermaid_focus_priority(source.kind);

    grouped
        .entry(source.owner_key.clone())
        .and_modify(|target| {
            target.left = target.left.min(line.x);
            target.right = target.right.max(line_right);
            target.top = target.top.min(line.y);
            target.bottom = target.bottom.max(line.y);

            if mermaid_focus_line_has_better_label(line, target, priority) {
                mermaid_replace_focus_label(target, line, source, priority);
            }
        })
        .or_insert_with(|| MermaidFocusAccumulator {
            source_index: line.source_index,
            text: source.text.clone(),
            diagram_x: source.diagram_x,
            diagram_y: source.diagram_y,
            priority,
            sort_y: line.y,
            sort_x: line.x,
            left: line.x,
            right: line_right,
            top: line.y,
            bottom: line.y,
        });
}

fn mermaid_focus_priority(kind: MermaidSemanticKind) -> u8 {
    match kind {
        MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => 0,
        MermaidSemanticKind::SubgraphTitle | MermaidSemanticKind::NodeTitle => 1,
        MermaidSemanticKind::ClassMember => 2,
        MermaidSemanticKind::ErAttributeName => 3,
        MermaidSemanticKind::ErAttributeType => 4,
        MermaidSemanticKind::EdgeLabel => 5,
    }
}

fn mermaid_focus_line_has_better_label(
    line: &MermaidProjectedLine,
    target: &MermaidFocusAccumulator,
    priority: u8,
) -> bool {
    let candidate = mermaid_focus_line_label_key(line, priority);
    let current = mermaid_focus_target_label_key(target);

    candidate < current
}

fn mermaid_replace_focus_label(
    target: &mut MermaidFocusAccumulator,
    line: &MermaidProjectedLine,
    source: &MermaidSemanticLine,
    priority: u8,
) {
    target.source_index = line.source_index;
    target.text = source.text.clone();
    target.diagram_x = source.diagram_x;
    target.diagram_y = source.diagram_y;
    target.priority = priority;
    target.sort_y = line.y;
    target.sort_x = line.x;
}

pub(crate) fn ensure_mermaid_prepared_render(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let source = viewer
        .source
        .as_deref()
        .ok_or_else(|| "Mermaid source unavailable".to_string())?;
    if source.len() > MERMAID_SOURCE_MAX_BYTES {
        tracing::warn!(
            session_id = %viewer.session_id,
            source_bytes = source.len(),
            cap_bytes = MERMAID_SOURCE_MAX_BYTES,
            "Mermaid source exceeded size cap; skipping render"
        );
        viewer.prepared_render = None;
        viewer.cached_lines.clear();
        viewer.cached_background_cells.clear();
        viewer.cached_semantic_lines.clear();
        return Err(format!(
            "Mermaid source exceeds {} KiB {MERMAID_TRUNCATED_MARKER}",
            MERMAID_SOURCE_MAX_BYTES / 1024
        ));
    }
    let key = mermaid_source_cache_key(source, content_rect);
    let prepared = if let Some(prepared) = viewer
        .prepared_render
        .as_ref()
        .filter(|prepared| prepared.key == key)
    {
        prepared.clone()
    } else {
        let options = mermaid_render_options(content_rect);
        let parsed = parse_mermaid(source).map_err(|err| err.to_string())?;
        let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
        let semantic_lines = build_mermaid_semantic_lines(&layout, &options);
        let svg = render_svg(&layout, &options.theme, &options.layout);
        let connector_svg = mermaid_strip_svg_text(&svg);
        // The terminal overlay renders Mermaid text separately, so the SVG fed
        // into usvg has all <text> nodes removed and does not need system font
        // discovery on the hot path.
        let tree = Tree::from_str(&connector_svg, &usvg::Options::default())
            .map_err(|err| format!("failed to parse rendered SVG: {err}"))?;
        let prepared = MermaidPreparedRender {
            key,
            tree,
            layout,
            semantic_lines,
        };
        viewer.prepared_render = Some(prepared.clone());
        viewer.source_prepare_count = viewer.source_prepare_count.saturating_add(1);
        prepared
    };
    viewer.diagram_width = prepared.layout.width.max(1.0);
    viewer.diagram_height = prepared.layout.height.max(1.0);
    Ok(())
}

pub(crate) fn render_mermaid_outline_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    transform: MermaidViewportTransform,
) -> Result<bool, String> {
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };

    let projected = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        MermaidViewState::Outline,
    );
    let outline_nodes = mermaid_outline_nodes_from_projected(prepared, &projected);
    if outline_nodes.is_empty() {
        return Ok(false);
    }

    let visible_keys = outline_nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<HashSet<_>>();
    let outline_edges = mermaid_outline_edge_map(&prepared.layout)
        .into_values()
        .filter(|edge| visible_keys.contains(&edge.from_key) && visible_keys.contains(&edge.to_key))
        .collect::<Vec<_>>();

    viewer.cached_lines =
        mermaid_render_outline_background(content_rect, &outline_nodes, outline_edges);
    viewer.cached_background_cells =
        mermaid_background_cells_from_lines(&viewer.cached_lines, MERMAID_CONNECTOR_COLOR);
    viewer.cached_semantic_lines = projected
        .into_iter()
        .filter(|line| {
            outline_nodes
                .iter()
                .any(|node| node.source_index == line.source_index)
        })
        .collect();
    Ok(true)
}

struct MermaidDetailRenderParts {
    projected: Vec<MermaidProjectedLine>,
    label_rects: HashMap<String, MermaidOutlineLabelRect>,
}

fn mermaid_detail_render_parts(
    semantic_lines: &[MermaidSemanticLine],
    content_rect: Rect,
    transform: MermaidViewportTransform,
    view_state: MermaidViewState,
) -> Option<MermaidDetailRenderParts> {
    let projected =
        project_mermaid_semantic_lines(semantic_lines, transform, content_rect, view_state);
    mermaid_detail_render_parts_from_projected(semantic_lines, projected, content_rect, view_state)
}

fn mermaid_detail_render_parts_from_projected(
    semantic_lines: &[MermaidSemanticLine],
    projected: Vec<MermaidProjectedLine>,
    content_rect: Rect,
    view_state: MermaidViewState,
) -> Option<MermaidDetailRenderParts> {
    if projected.is_empty() {
        return None;
    }

    if mermaid_view_uses_packed_detail(view_state) {
        return mermaid_packed_detail_render_parts(semantic_lines, &projected, content_rect);
    }

    let label_rects = mermaid_detail_box_rects(semantic_lines, &projected, content_rect);
    mermaid_detail_render_parts_from_label_rects(projected, label_rects)
}

fn mermaid_view_uses_packed_detail(view_state: MermaidViewState) -> bool {
    matches!(view_state, MermaidViewState::L2 | MermaidViewState::L3)
}

fn mermaid_packed_detail_render_parts(
    semantic_lines: &[MermaidSemanticLine],
    projected: &[MermaidProjectedLine],
    content_rect: Rect,
) -> Option<MermaidDetailRenderParts> {
    let owners = mermaid_build_packed_detail_owners(semantic_lines, projected);
    if owners.is_empty() {
        return None;
    }
    let label_rects = mermaid_pack_detail_box_rects(content_rect, &owners);
    if label_rects.is_empty() {
        return None;
    }
    let projected = mermaid_project_packed_detail_lines(&owners, &label_rects);
    Some(MermaidDetailRenderParts {
        projected,
        label_rects,
    })
}

fn mermaid_detail_render_parts_from_label_rects(
    projected: Vec<MermaidProjectedLine>,
    label_rects: HashMap<String, MermaidOutlineLabelRect>,
) -> Option<MermaidDetailRenderParts> {
    (!label_rects.is_empty()).then_some(MermaidDetailRenderParts {
        projected,
        label_rects,
    })
}

fn mermaid_filter_visible_outline_edges(
    outline_edges: impl IntoIterator<Item = MermaidOutlineEdge>,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> Vec<MermaidOutlineEdge> {
    let visible_keys = label_rects.keys().cloned().collect::<HashSet<_>>();
    outline_edges
        .into_iter()
        .filter(|edge| visible_keys.contains(&edge.from_key) && visible_keys.contains(&edge.to_key))
        .collect()
}

pub(crate) fn render_mermaid_detail_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    transform: MermaidViewportTransform,
    view_state: MermaidViewState,
) -> Result<bool, String> {
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };

    let Some(detail_parts) = mermaid_detail_render_parts(
        &prepared.semantic_lines,
        content_rect,
        transform,
        view_state,
    ) else {
        return Ok(false);
    };

    let owner_colors = mermaid_owner_accent_map(&prepared.semantic_lines);
    let outline_edges = mermaid_filter_visible_outline_edges(
        mermaid_outline_edge_map(&prepared.layout).into_values(),
        &detail_parts.label_rects,
    );

    viewer.cached_lines = mermaid_render_compact_detail_background(
        content_rect,
        &detail_parts.label_rects,
        outline_edges,
    );
    viewer.cached_background_cells =
        mermaid_background_cells_from_lines(&viewer.cached_lines, MERMAID_CONNECTOR_COLOR);
    mermaid_apply_rect_border_colors(
        &mut viewer.cached_background_cells,
        content_rect,
        &detail_parts.label_rects,
        &owner_colors,
    );
    viewer.cached_semantic_lines = detail_parts.projected;
    Ok(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MermaidSpecialRenderer {
    ErPacked,
    Outline,
    Detail,
}

fn mermaid_special_renderer_for_view(
    view_state: MermaidViewState,
) -> Option<MermaidSpecialRenderer> {
    if view_state.is_er() {
        return Some(MermaidSpecialRenderer::ErPacked);
    }
    if view_state == MermaidViewState::Outline {
        return Some(MermaidSpecialRenderer::Outline);
    }
    view_state
        .detail_level()
        .is_some()
        .then_some(MermaidSpecialRenderer::Detail)
}

fn render_mermaid_special_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    transform: MermaidViewportTransform,
    view_state: MermaidViewState,
) -> Result<bool, String> {
    let Some(renderer) = mermaid_special_renderer_for_view(view_state) else {
        return Ok(false);
    };

    match renderer {
        MermaidSpecialRenderer::ErPacked => {
            render_mermaid_er_packed_lines(viewer, content_rect, view_state)
        }
        MermaidSpecialRenderer::Outline => {
            render_mermaid_outline_lines(viewer, content_rect, transform)
        }
        MermaidSpecialRenderer::Detail => {
            render_mermaid_detail_lines(viewer, content_rect, transform, view_state)
        }
    }
}

fn render_mermaid_fallback_pixmap_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    sample_width: u32,
    sample_height: u32,
    transform: MermaidViewportTransform,
    view_state: MermaidViewState,
) -> Result<(), String> {
    let mut pixmap = Pixmap::new(sample_width, sample_height)
        .ok_or_else(|| "failed to allocate Mermaid viewport".to_string())?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(255, 255, 255, 255));

    let mut pixmap_mut = pixmap.as_mut();
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };
    resvg::render(
        &prepared.tree,
        Transform::from_row(
            transform.scale,
            0.0,
            0.0,
            transform.scale,
            transform.tx,
            transform.ty,
        ),
        &mut pixmap_mut,
    );

    let cached_lines = pixmap_to_ascii_lines(&pixmap, content_rect);
    let cached_background_cells =
        mermaid_background_cells_from_lines(&cached_lines, MERMAID_CONNECTOR_COLOR);
    let cached_semantic_lines = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        view_state,
    );

    viewer.cached_lines = cached_lines;
    viewer.cached_background_cells = cached_background_cells;
    viewer.cached_semantic_lines = cached_semantic_lines;
    Ok(())
}

fn mermaid_mark_viewport_cache_rendered(viewer: &mut MermaidViewerState, content_rect: Rect) {
    viewer.cached_rect = Some(content_rect);
    viewer.cached_zoom = viewer.zoom;
    viewer.cached_center_x = viewer.center_x;
    viewer.cached_center_y = viewer.center_y;
    viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
}

pub(crate) fn render_mermaid_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let (sample_width, sample_height, transform) =
        mermaid_viewport_transform(viewer, content_rect)?;
    let view_state = mermaid_view_state_for_view(viewer, content_rect);
    if render_mermaid_special_lines(viewer, content_rect, transform, view_state)? {
        mermaid_mark_viewport_cache_rendered(viewer, content_rect);
        return Ok(());
    }

    render_mermaid_fallback_pixmap_lines(
        viewer,
        content_rect,
        sample_width,
        sample_height,
        transform,
        view_state,
    )?;
    mermaid_mark_viewport_cache_rendered(viewer, content_rect);
    Ok(())
}

fn mermaid_best_packed_detail_layout(
    specs: &[MermaidPackedDetailBoxSize],
    viewport: MermaidPackedDetailViewport,
) -> Option<MermaidPackedDetailLayout> {
    (1..=specs.len())
        .filter_map(|column_count| {
            mermaid_packed_detail_layout_for_columns(specs, viewport, column_count)
        })
        .max_by(|(_, left_score), (_, right_score)| left_score.total_cmp(right_score))
        .map(|(layout, _)| layout)
}

fn mermaid_fallback_packed_detail_layout(
    specs: &[MermaidPackedDetailBoxSize],
    viewport: MermaidPackedDetailViewport,
) -> MermaidPackedDetailLayout {
    let row_widths = specs
        .iter()
        .map(|spec| spec.outer_width)
        .collect::<Vec<_>>();
    let row_heights = specs
        .iter()
        .map(|spec| spec.outer_height)
        .collect::<Vec<_>>();
    let cluster_height =
        mermaid_packed_detail_cluster_height(&row_heights, viewport.gap_y).min(viewport.height);

    mermaid_build_packed_detail_layout(1, row_widths, row_heights, cluster_height)
}

fn mermaid_pack_detail_box_row_rects(
    rects: &mut HashMap<String, MermaidOutlineLabelRect>,
    row: &[MermaidPackedDetailOwner],
    specs: &[MermaidPackedDetailBoxSize],
    owner_index: &mut usize,
    row_top: u16,
    row_left: u16,
    gap_x: u16,
) {
    let mut column_left = row_left;

    for owner in row {
        let spec = specs[*owner_index];
        rects.insert(
            owner.owner_key.clone(),
            MermaidOutlineLabelRect {
                left: column_left as i32,
                right: column_left
                    .saturating_add(spec.outer_width)
                    .saturating_sub(1) as i32,
                top: row_top as i32,
                bottom: row_top.saturating_add(spec.outer_height).saturating_sub(1) as i32,
            },
        );
        column_left = column_left
            .saturating_add(spec.outer_width)
            .saturating_add(gap_x);
        *owner_index += 1;
    }
}

fn mermaid_place_packed_detail_box_rects(
    content_rect: Rect,
    owners: &[MermaidPackedDetailOwner],
    specs: &[MermaidPackedDetailBoxSize],
    viewport: MermaidPackedDetailViewport,
    layout: MermaidPackedDetailLayout,
) -> HashMap<String, MermaidOutlineLabelRect> {
    let start_y = content_rect
        .y
        .saturating_add(viewport.height.saturating_sub(layout.cluster_height) / 2);

    let mut rects = HashMap::new();
    let mut row_top = start_y;
    let mut owner_index = 0usize;

    for (row_index, row_width) in layout.row_widths.iter().enumerate() {
        let row = &owners[owner_index..(owner_index + layout.column_count).min(owners.len())];
        let row_left = content_rect
            .x
            .saturating_add(viewport.width.saturating_sub(*row_width) / 2);

        mermaid_pack_detail_box_row_rects(
            &mut rects,
            row,
            specs,
            &mut owner_index,
            row_top,
            row_left,
            viewport.gap_x,
        );

        row_top = row_top
            .saturating_add(layout.row_heights[row_index])
            .saturating_add(viewport.gap_y);
    }

    rects
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MermaidFocusLabelKey {
    priority: u8,
    y: u16,
    x: u16,
    source_index: usize,
}

fn mermaid_focus_line_label_key(line: &MermaidProjectedLine, priority: u8) -> MermaidFocusLabelKey {
    MermaidFocusLabelKey {
        priority,
        y: line.y,
        x: line.x,
        source_index: line.source_index,
    }
}

fn mermaid_focus_target_label_key(target: &MermaidFocusAccumulator) -> MermaidFocusLabelKey {
    MermaidFocusLabelKey {
        priority: target.priority,
        y: target.sort_y,
        x: target.sort_x,
        source_index: target.source_index,
    }
}

#[cfg(test)]
mod tests;
