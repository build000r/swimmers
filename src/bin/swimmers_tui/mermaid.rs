use super::*;
pub(crate) use swimmers::host_actions::{ArtifactOpener, SystemArtifactOpener};

pub(crate) enum FishBowlMode {
    Aquarium,
    Mermaid(MermaidViewerState),
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
            MermaidViewState::Outline => None,
            MermaidViewState::L1 => Some(MermaidDetailLevel::L1),
            MermaidViewState::L2 => Some(MermaidDetailLevel::L2),
            MermaidViewState::L3 => Some(MermaidDetailLevel::L3),
            MermaidViewState::ErEntities
            | MermaidViewState::ErKeys
            | MermaidViewState::ErColumns
            | MermaidViewState::ErSchema => None,
        }
    }

    pub(crate) fn collision_padding(self) -> u16 {
        match self {
            MermaidViewState::Outline | MermaidViewState::L1 => 2,
            MermaidViewState::L2 => 1,
            MermaidViewState::L3 => 0,
            MermaidViewState::ErEntities | MermaidViewState::ErKeys => 1,
            MermaidViewState::ErColumns | MermaidViewState::ErSchema => 0,
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
        match self {
            MermaidSemanticKind::SubgraphSummary => owner_cols >= 10.0 && owner_rows >= 1.0,
            MermaidSemanticKind::NodeSummary => owner_cols >= 8.0 && owner_rows >= 1.0,
            MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::EdgeLabel => true,
            MermaidSemanticKind::ClassMember => owner_cols >= 10.0 && owner_rows >= 2.5,
            MermaidSemanticKind::ErAttributeName => owner_cols >= 8.0 && owner_rows >= 2.5,
            MermaidSemanticKind::ErAttributeType => owner_cols >= 12.0 && owner_rows >= 3.0,
        }
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
pub(crate) struct MermaidErPackedAttrRow {
    pub(crate) name_source_index: usize,
    pub(crate) name_text: String,
    pub(crate) type_source_index: Option<usize>,
    pub(crate) type_text: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidErPackedBox {
    pub(crate) owner_key: String,
    pub(crate) sort_x: f32,
    pub(crate) sort_y: f32,
    pub(crate) title_lines: Vec<(usize, String)>,
    pub(crate) attr_rows: Vec<MermaidErPackedAttrRow>,
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

#[derive(Clone, Debug)]
pub(crate) struct MermaidErOrderNode {
    pub(crate) owner_key: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) neighbors: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidViewerState {
    pub(crate) session_id: String,
    pub(crate) tmux_name: String,
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
    if x < content_rect.x as i32
        || x >= content_rect.right() as i32
        || y < content_rect.y as i32
        || y >= content_rect.bottom() as i32
    {
        return;
    }
    let grid_x = (x - content_rect.x as i32) as usize;
    let grid_y = (y - content_rect.y as i32) as usize;
    let Some(row) = cells.get_mut(grid_y) else {
        return;
    };
    let Some(cell) = row.get_mut(grid_x) else {
        return;
    };
    if cell.ch != ' ' {
        cell.fg = color;
    }
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

pub(crate) fn mermaid_kind_supports_semantic_overlay(kind: DiagramKind) -> bool {
    matches!(
        kind,
        DiagramKind::Flowchart
            | DiagramKind::Class
            | DiagramKind::State
            | DiagramKind::Er
            | DiagramKind::Requirement
            | DiagramKind::Packet
    )
}

pub(crate) fn mermaid_is_divider_line(line: &str) -> bool {
    line.trim() == "---"
}

pub(crate) fn mermaid_is_numeric_prefix(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut chars = trimmed.chars().peekable();
    let mut saw_digit = false;
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            chars.next();
            continue;
        }
        break;
    }
    if !saw_digit {
        return false;
    }
    match chars.next() {
        None => true,
        Some('.') | Some(')') | Some(':') => chars.next().is_none(),
        _ => false,
    }
}

pub(crate) fn mermaid_summary_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "for"
            | "from"
            | "in"
            | "is"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "with"
    )
}

pub(crate) fn mermaid_clean_summary_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        })
        .to_string()
}

pub(crate) fn mermaid_outline_subgraph_key(index: usize) -> String {
    format!("subgraph:{index}")
}

pub(crate) fn mermaid_outline_node_key(id: &str) -> String {
    format!("node:{id}")
}

pub(crate) fn mermaid_outline_edge_key(index: usize) -> String {
    format!("edge:{index}")
}

pub(crate) fn mermaid_subgraph_contains(
    outer: &mermaid_rs_renderer::SubgraphLayout,
    inner: &mermaid_rs_renderer::SubgraphLayout,
) -> bool {
    let epsilon = 0.5;
    outer.x - epsilon <= inner.x
        && outer.y - epsilon <= inner.y
        && outer.x + outer.width + epsilon >= inner.x + inner.width
        && outer.y + outer.height + epsilon >= inner.y + inner.height
}

pub(crate) fn mermaid_top_level_subgraph_indices(
    subgraphs: &[mermaid_rs_renderer::SubgraphLayout],
) -> HashSet<usize> {
    let mut top_level = HashSet::new();
    for (idx, subgraph) in subgraphs.iter().enumerate() {
        let parent = subgraphs
            .iter()
            .enumerate()
            .filter(|(other_idx, other)| {
                *other_idx != idx
                    && mermaid_subgraph_contains(other, subgraph)
                    && (other.width * other.height) > (subgraph.width * subgraph.height)
            })
            .min_by(|(_, left), (_, right)| {
                (left.width * left.height)
                    .partial_cmp(&(right.width * right.height))
                    .unwrap_or(Ordering::Equal)
            });
        if parent.is_none() {
            top_level.insert(idx);
        }
    }
    top_level
}

pub(crate) fn mermaid_summary_subtokens(token: &str) -> Vec<String> {
    let trimmed = token.trim();
    if mermaid_is_numeric_prefix(trimmed) {
        return vec![trimmed.to_string()];
    }

    mermaid_clean_summary_token(token)
        .split(|ch: char| matches!(ch, '_' | '-' | '/'))
        .filter_map(|part| {
            let part = part.trim();
            (!part.is_empty()).then(|| part.to_string())
        })
        .collect()
}

pub(crate) fn mermaid_compact_overview_text<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let raw_tokens = lines
        .into_iter()
        .flat_map(|line| line.split_whitespace())
        .flat_map(mermaid_summary_subtokens)
        .collect::<Vec<_>>();
    if raw_tokens.is_empty() {
        return None;
    }

    let mut prefix = None;
    let mut start_idx = 0usize;
    if mermaid_is_numeric_prefix(&raw_tokens[0]) {
        prefix = Some(raw_tokens[0].trim().to_string());
        start_idx = 1;
    }

    let cleaned_tokens = raw_tokens[start_idx..].to_vec();
    if cleaned_tokens.is_empty() {
        return prefix;
    }

    let significant_tokens = cleaned_tokens
        .iter()
        .filter(|token| !mermaid_summary_stopword(&token.to_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>();
    let source_tokens = if significant_tokens.is_empty() {
        cleaned_tokens
    } else {
        significant_tokens
    };

    let word_limit = if prefix.is_some() { 2 } else { 3 };
    let max_chars = 20usize;
    let mut out = prefix.unwrap_or_default();
    let mut added = 0usize;
    for token in source_tokens {
        if added >= word_limit {
            break;
        }
        let separator = usize::from(!out.is_empty());
        let next_len = out.chars().count() + separator + token.chars().count();
        if next_len > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&token);
        added += 1;
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn push_mermaid_summary_line<'a>(
    target: &mut Vec<MermaidSemanticLine>,
    lines: impl IntoIterator<Item = &'a str>,
    diagram_x: f32,
    diagram_y: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_key: &str,
    outline_eligible: bool,
    owner_width: f32,
    owner_height: f32,
) {
    let Some(text) = mermaid_compact_overview_text(lines) else {
        return;
    };
    target.push(MermaidSemanticLine {
        text,
        diagram_x,
        diagram_y,
        anchor,
        kind,
        owner_key: owner_key.to_string(),
        outline_eligible,
        owner_width,
        owner_height,
    });
}

pub(crate) fn push_mermaid_indexed_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    lines: &[(usize, &str)],
    diagram_x: f32,
    start_y: f32,
    line_height: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_key: &str,
    outline_eligible: bool,
    owner_width: f32,
    owner_height: f32,
) {
    for (idx, line) in lines {
        let text = line.trim();
        if text.is_empty() || mermaid_is_divider_line(text) {
            continue;
        }
        target.push(MermaidSemanticLine {
            text: text.to_string(),
            diagram_x,
            diagram_y: start_y + *idx as f32 * line_height,
            anchor,
            kind,
            owner_key: owner_key.to_string(),
            outline_eligible,
            owner_width,
            owner_height,
        });
    }
}

pub(crate) fn push_mermaid_text_block_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    label: &mermaid_rs_renderer::layout::TextBlock,
    diagram_x: f32,
    center_y: f32,
    font_size: f32,
    line_height: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_key: &str,
    outline_eligible: bool,
    owner_width: f32,
    owner_height: f32,
) {
    if label.lines.is_empty() {
        return;
    }
    let total_height = label.lines.len() as f32 * line_height;
    let start_y = center_y - total_height / 2.0 + font_size;
    let indexed_lines: Vec<(usize, &str)> = label
        .lines
        .iter()
        .enumerate()
        .map(|(idx, line)| (idx, line.as_str()))
        .collect();
    push_mermaid_indexed_semantic_lines(
        target,
        &indexed_lines,
        diagram_x,
        start_y,
        line_height,
        anchor,
        kind,
        owner_key,
        outline_eligible,
        owner_width,
        owner_height,
    );
}

pub(crate) fn extend_mermaid_class_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    theme_font_size: f32,
    class_line_height: f32,
    node_padding_x: f32,
    owner_key: &str,
    outline_eligible: bool,
) {
    let total_height = node.label.lines.len() as f32 * class_line_height;
    let start_y = node.y + node.height / 2.0 - total_height / 2.0 + theme_font_size;
    let center_x = node.x + node.width / 2.0;
    let left_x = node.x + node_padding_x.max(10.0);
    let Some(divider_idx) = node
        .label
        .lines
        .iter()
        .position(|line| mermaid_is_divider_line(line))
    else {
        let indexed_lines: Vec<(usize, &str)> = node
            .label
            .lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (idx, line.as_str()))
            .collect();
        push_mermaid_summary_line(
            target,
            node.label.lines.iter().map(String::as_str),
            center_x,
            node.y + node.height / 2.0,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeSummary,
            owner_key,
            outline_eligible,
            node.width,
            node.height,
        );
        push_mermaid_indexed_semantic_lines(
            target,
            &indexed_lines,
            center_x,
            start_y,
            class_line_height,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeTitle,
            owner_key,
            false,
            node.width,
            node.height,
        );
        return;
    };

    let title_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .take(divider_idx)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty()).then_some((idx, line.as_str()))
        })
        .collect();
    let member_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .skip(divider_idx + 1)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty() && !mermaid_is_divider_line(text)).then_some((idx, line.as_str()))
        })
        .collect();

    push_mermaid_summary_line(
        target,
        title_lines.iter().map(|(_, line)| *line),
        center_x,
        node.y + node.height / 2.0,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeSummary,
        owner_key,
        outline_eligible,
        node.width,
        node.height,
    );
    push_mermaid_indexed_semantic_lines(
        target,
        &title_lines,
        center_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeTitle,
        owner_key,
        false,
        node.width,
        node.height,
    );
    push_mermaid_indexed_semantic_lines(
        target,
        &member_lines,
        left_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Start,
        MermaidSemanticKind::ClassMember,
        owner_key,
        false,
        node.width,
        node.height,
    );
}

pub(crate) fn extend_mermaid_er_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    theme_font_size: f32,
    class_line_height: f32,
    node_padding_x: f32,
    owner_key: &str,
    outline_eligible: bool,
) {
    let Some(divider_idx) = node
        .label
        .lines
        .iter()
        .position(|line| mermaid_is_divider_line(line))
    else {
        return;
    };

    let total_height = node.label.lines.len() as f32 * class_line_height;
    let start_y = node.y + node.height / 2.0 - total_height / 2.0 + theme_font_size;
    let center_x = node.x + node.width / 2.0;
    let left_x = node.x + node_padding_x.max(10.0);

    let title_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .take(divider_idx)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty()).then_some((idx, line.as_str()))
        })
        .collect();
    push_mermaid_summary_line(
        target,
        title_lines.iter().map(|(_, line)| *line),
        center_x,
        node.y + node.height / 2.0,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeSummary,
        owner_key,
        outline_eligible,
        node.width,
        node.height,
    );
    push_mermaid_indexed_semantic_lines(
        target,
        &title_lines,
        center_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeTitle,
        owner_key,
        false,
        node.width,
        node.height,
    );

    let attr_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .skip(divider_idx + 1)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty() && !mermaid_is_divider_line(text)).then_some((idx, line.as_str()))
        })
        .collect();
    if attr_lines.is_empty() {
        return;
    }

    let mut parsed_attrs = Vec::new();
    let mut max_type_chars = 0usize;
    let mut use_columns = true;
    for (idx, line) in &attr_lines {
        let trimmed = line.trim();
        let mut parts = trimmed.split_whitespace();
        let Some(data_type) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            use_columns = false;
            break;
        };
        let suffix = parts.collect::<Vec<_>>().join(" ");
        let mut name_text = name.to_string();
        if !suffix.is_empty() {
            name_text.push(' ');
            name_text.push_str(&suffix);
        }
        max_type_chars = max_type_chars.max(data_type.chars().count());
        parsed_attrs.push((*idx, data_type.to_string(), name_text));
    }

    let gap_chars = 3usize;
    let column_char_width = 2.0f32;
    let name_x = left_x + ((max_type_chars + gap_chars) as f32 * column_char_width);
    let content_width = (node.width - node_padding_x.max(10.0) * 2.0).max(0.0);
    if use_columns && parsed_attrs.len() == attr_lines.len() && name_x < node.x + content_width {
        for (idx, data_type, name) in parsed_attrs {
            let diagram_y = start_y + idx as f32 * class_line_height;
            let type_x = left_x
                + ((max_type_chars.saturating_sub(data_type.chars().count())) as f32
                    * column_char_width);
            target.push(MermaidSemanticLine {
                text: data_type,
                diagram_x: type_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeType,
                owner_key: owner_key.to_string(),
                outline_eligible: false,
                owner_width: node.width,
                owner_height: node.height,
            });
            target.push(MermaidSemanticLine {
                text: name,
                diagram_x: name_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeName,
                owner_key: owner_key.to_string(),
                outline_eligible: false,
                owner_width: node.width,
                owner_height: node.height,
            });
        }
        return;
    }

    push_mermaid_indexed_semantic_lines(
        target,
        &attr_lines,
        left_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Start,
        MermaidSemanticKind::ErAttributeName,
        owner_key,
        false,
        node.width,
        node.height,
    );
}

pub(crate) fn build_mermaid_semantic_lines(
    layout: &MermaidLayout,
    options: &RenderOptions,
) -> Vec<MermaidSemanticLine> {
    if !mermaid_kind_supports_semantic_overlay(layout.kind) {
        return Vec::new();
    }

    let theme_font_size = options.theme.font_size;
    let base_line_height = theme_font_size * options.layout.label_line_height;
    let class_line_height = theme_font_size * options.layout.class_label_line_height();
    let state_font_size = if layout.kind == DiagramKind::State {
        theme_font_size * 0.85
    } else {
        theme_font_size
    };
    let state_line_height = state_font_size * options.layout.label_line_height;
    let top_level_subgraphs = mermaid_top_level_subgraph_indices(&layout.subgraphs);
    let subgraph_node_ids = layout
        .subgraphs
        .iter()
        .flat_map(|subgraph| subgraph.nodes.iter().cloned())
        .collect::<HashSet<_>>();
    let mut semantic_lines = Vec::new();

    for (subgraph_idx, subgraph) in layout.subgraphs.iter().enumerate() {
        if subgraph.label.trim().is_empty() {
            continue;
        }
        let owner_key = mermaid_outline_subgraph_key(subgraph_idx);
        let outline_eligible = top_level_subgraphs.contains(&subgraph_idx);
        if layout.kind == DiagramKind::State {
            let header_height =
                (subgraph.label_block.height + theme_font_size * 0.75).max(theme_font_size * 1.4);
            let label_x =
                subgraph.x + (theme_font_size * 0.6).max(subgraph.label_block.height * 0.35);
            let label_y = subgraph.y + header_height / 2.0;
            push_mermaid_summary_line(
                &mut semantic_lines,
                subgraph.label_block.lines.iter().map(String::as_str),
                label_x,
                label_y,
                MermaidTextAnchor::Start,
                MermaidSemanticKind::SubgraphSummary,
                &owner_key,
                outline_eligible,
                subgraph.width,
                subgraph.height,
            );
            push_mermaid_text_block_semantic_lines(
                &mut semantic_lines,
                &subgraph.label_block,
                label_x,
                label_y,
                state_font_size,
                state_line_height,
                MermaidTextAnchor::Start,
                MermaidSemanticKind::SubgraphTitle,
                &owner_key,
                false,
                subgraph.width,
                subgraph.height,
            );
        } else {
            let label_x = subgraph.x + subgraph.width / 2.0;
            let label_y = subgraph.y + 12.0 + subgraph.label_block.height / 2.0;
            push_mermaid_summary_line(
                &mut semantic_lines,
                subgraph.label_block.lines.iter().map(String::as_str),
                label_x,
                label_y,
                MermaidTextAnchor::Center,
                MermaidSemanticKind::SubgraphSummary,
                &owner_key,
                outline_eligible,
                subgraph.width,
                subgraph.height,
            );
            push_mermaid_text_block_semantic_lines(
                &mut semantic_lines,
                &subgraph.label_block,
                label_x,
                label_y,
                theme_font_size,
                base_line_height,
                MermaidTextAnchor::Center,
                MermaidSemanticKind::SubgraphTitle,
                &owner_key,
                false,
                subgraph.width,
                subgraph.height,
            );
        }
    }

    for (edge_idx, edge) in layout.edges.iter().enumerate() {
        let owner_key = mermaid_outline_edge_key(edge_idx);
        if let Some(label) = edge.label.as_ref() {
            if let Some((label_x, label_y)) = edge.label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    &owner_key,
                    false,
                    0.0,
                    0.0,
                );
            }
        }
        if let Some(label) = edge.start_label.as_ref() {
            if let Some((label_x, label_y)) = edge.start_label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    &owner_key,
                    false,
                    0.0,
                    0.0,
                );
            }
        }
        if let Some(label) = edge.end_label.as_ref() {
            if let Some((label_x, label_y)) = edge.end_label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    &owner_key,
                    false,
                    0.0,
                    0.0,
                );
            }
        }
    }

    for node in layout.nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        let hide_label = node.label.lines.iter().all(|line| line.trim().is_empty())
            || node.id.starts_with("__start_")
            || node.id.starts_with("__end_");
        if hide_label {
            continue;
        }

        if layout.kind == DiagramKind::Er
            && node
                .label
                .lines
                .iter()
                .any(|line| mermaid_is_divider_line(line))
        {
            let outline_eligible =
                layout.subgraphs.is_empty() || !subgraph_node_ids.contains(&node.id);
            let owner_key = mermaid_outline_node_key(&node.id);
            extend_mermaid_er_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
                &owner_key,
                outline_eligible,
            );
            continue;
        }

        if node
            .label
            .lines
            .iter()
            .any(|line| mermaid_is_divider_line(line))
        {
            let outline_eligible =
                layout.subgraphs.is_empty() || !subgraph_node_ids.contains(&node.id);
            let owner_key = mermaid_outline_node_key(&node.id);
            extend_mermaid_class_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
                &owner_key,
                outline_eligible,
            );
            continue;
        }

        let center_x = node.x + node.width / 2.0;
        let center_y = node.y + node.height / 2.0;
        let outline_eligible = layout.subgraphs.is_empty() || !subgraph_node_ids.contains(&node.id);
        let owner_key = mermaid_outline_node_key(&node.id);
        let (font_size, line_height) = if layout.kind == DiagramKind::State {
            (state_font_size, state_line_height)
        } else {
            (theme_font_size, base_line_height)
        };
        push_mermaid_summary_line(
            &mut semantic_lines,
            node.label.lines.iter().map(String::as_str),
            center_x,
            center_y,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeSummary,
            &owner_key,
            outline_eligible,
            node.width,
            node.height,
        );
        push_mermaid_text_block_semantic_lines(
            &mut semantic_lines,
            &node.label,
            center_x,
            center_y,
            font_size,
            line_height,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeTitle,
            &owner_key,
            false,
            node.width,
            node.height,
        );
    }

    semantic_lines
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

pub(crate) fn mermaid_is_er_source(source: &str) -> bool {
    source
        .lines()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with("%%"))
        .map(|line| line.trim_start().starts_with("erDiagram"))
        .unwrap_or(false)
}

pub(crate) fn mermaid_is_er_viewer(viewer: &MermaidViewerState) -> bool {
    viewer
        .prepared_render
        .as_ref()
        .map(|prepared| prepared.layout.kind == DiagramKind::Er)
        .unwrap_or_else(|| {
            viewer
                .source
                .as_deref()
                .map(mermaid_is_er_source)
                .unwrap_or(false)
        })
}

pub(crate) fn mermaid_er_view_states() -> [MermaidViewState; 4] {
    [
        MermaidViewState::ErEntities,
        MermaidViewState::ErKeys,
        MermaidViewState::ErColumns,
        MermaidViewState::ErSchema,
    ]
}

pub(crate) fn mermaid_er_state_zoom(view_state: MermaidViewState) -> f32 {
    match view_state {
        MermaidViewState::ErEntities => 1.0,
        MermaidViewState::ErKeys => 1.25,
        MermaidViewState::ErColumns => 1.5,
        MermaidViewState::ErSchema => 1.75,
        MermaidViewState::Outline
        | MermaidViewState::L1
        | MermaidViewState::L2
        | MermaidViewState::L3 => 1.0,
    }
}

pub(crate) fn mermaid_er_view_state_for_zoom(zoom: f32) -> MermaidViewState {
    let zoom_percent = mermaid_zoom_percent(zoom);
    if zoom_percent <= 112 {
        MermaidViewState::ErEntities
    } else if zoom_percent <= 137 {
        MermaidViewState::ErKeys
    } else if zoom_percent <= 162 {
        MermaidViewState::ErColumns
    } else {
        MermaidViewState::ErSchema
    }
}

pub(crate) fn mermaid_er_zoom_step(current_zoom: f32, direction: i8) -> f32 {
    let states = mermaid_er_view_states();
    let current = mermaid_er_view_state_for_zoom(current_zoom);
    let current_index = states
        .iter()
        .position(|state| *state == current)
        .unwrap_or(0) as i32;
    let next_index = (current_index + i32::from(direction))
        .clamp(0, states.len().saturating_sub(1) as i32) as usize;
    mermaid_er_state_zoom(states[next_index])
}

pub(crate) fn mermaid_fit_whole_words(text: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    if text.chars().count() <= budget {
        return text.to_string();
    }

    let mut out = String::new();
    for word in text.split_whitespace() {
        let separator = usize::from(!out.is_empty());
        let next_len = out.chars().count() + separator + word.chars().count();
        if next_len > budget {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

pub(crate) fn mermaid_zoom_status_label(zoom: f32) -> String {
    let percent = mermaid_zoom_percent(zoom);
    if percent <= 100 {
        "fit 100%".to_string()
    } else {
        format!("zoom {percent}%")
    }
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

pub(crate) fn clip_mermaid_overlay_text(text: &str, _skip: usize, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    text.chars().take(max_chars).collect()
}

pub(crate) fn mermaid_display_text_for_view(
    line: &MermaidSemanticLine,
    owner_cols: f32,
    view_state: MermaidViewState,
) -> String {
    match view_state {
        MermaidViewState::Outline => match line.kind {
            MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => {
                let budget = owner_cols.floor().max(8.0).min(18.0) as usize;
                mermaid_fit_whole_words(&line.text, budget)
            }
            _ => line.text.clone(),
        },
        MermaidViewState::L1 => match line.kind {
            MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => {
                let budget = owner_cols.floor().max(8.0).min(18.0) as usize;
                mermaid_fit_whole_words(&line.text, budget)
            }
            _ => line.text.clone(),
        },
        MermaidViewState::L2 | MermaidViewState::L3 => line.text.clone(),
        MermaidViewState::ErEntities => match line.kind {
            MermaidSemanticKind::NodeSummary => {
                let budget = owner_cols.floor().max(8.0).min(18.0) as usize;
                mermaid_fit_whole_words(&line.text, budget)
            }
            _ => line.text.clone(),
        },
        MermaidViewState::ErKeys | MermaidViewState::ErColumns | MermaidViewState::ErSchema => {
            line.text.clone()
        }
    }
}

pub(crate) fn mermaid_kind_uses_compact_rows(kind: MermaidSemanticKind) -> bool {
    matches!(
        kind,
        MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName
            | MermaidSemanticKind::ErAttributeType
    )
}

pub(crate) fn mermaid_kind_is_owner_summary(kind: MermaidSemanticKind) -> bool {
    matches!(
        kind,
        MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary
    )
}

pub(crate) fn mermaid_kind_is_owner_detail(kind: MermaidSemanticKind) -> bool {
    matches!(
        kind,
        MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName
            | MermaidSemanticKind::ErAttributeType
    )
}

pub(crate) fn mermaid_line_visible_in_state(
    line: &MermaidSemanticLine,
    view_state: MermaidViewState,
) -> bool {
    match view_state {
        MermaidViewState::Outline => line.outline_eligible,
        MermaidViewState::L1 | MermaidViewState::L2 | MermaidViewState::L3 => {
            line.kind.min_detail_level()
                <= view_state
                    .detail_level()
                    .expect("non-outline states always have a detail level")
        }
        MermaidViewState::ErEntities => line.kind == MermaidSemanticKind::NodeSummary,
        MermaidViewState::ErKeys => {
            line.kind == MermaidSemanticKind::NodeTitle
                || (line.kind == MermaidSemanticKind::ErAttributeName
                    && (line.text.contains(" PK") || line.text.contains(" FK")))
        }
        MermaidViewState::ErColumns => {
            matches!(
                line.kind,
                MermaidSemanticKind::NodeTitle | MermaidSemanticKind::ErAttributeName
            )
        }
        MermaidViewState::ErSchema => {
            matches!(
                line.kind,
                MermaidSemanticKind::NodeTitle
                    | MermaidSemanticKind::ErAttributeName
                    | MermaidSemanticKind::ErAttributeType
            )
        }
    }
}

pub(crate) fn mermaid_outline_nodes_from_projected(
    prepared: &MermaidPreparedRender,
    projected: &[MermaidProjectedLine],
) -> Vec<MermaidOutlineNode> {
    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    for line in projected {
        let Some(source) = prepared.semantic_lines.get(line.source_index) else {
            continue;
        };
        if !matches!(
            source.kind,
            MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary
        ) {
            continue;
        }
        if !seen.insert(source.owner_key.clone()) {
            continue;
        }
        nodes.push(MermaidOutlineNode {
            key: source.owner_key.clone(),
            source_index: line.source_index,
            x: line.x,
            y: line.y,
            text_width: display_width(&line.text).max(1),
        });
    }
    nodes
}

pub(crate) fn mermaid_outline_edge_map(
    layout: &MermaidLayout,
) -> HashMap<String, MermaidOutlineEdge> {
    let top_level_subgraphs = mermaid_top_level_subgraph_indices(&layout.subgraphs);
    let mut node_groups = HashMap::new();

    for subgraph_idx in top_level_subgraphs {
        let key = mermaid_outline_subgraph_key(subgraph_idx);
        if let Some(subgraph) = layout.subgraphs.get(subgraph_idx) {
            for node_id in &subgraph.nodes {
                node_groups.insert(node_id.clone(), key.clone());
            }
        }
    }

    for node in layout.nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        node_groups
            .entry(node.id.clone())
            .or_insert_with(|| mermaid_outline_node_key(&node.id));
    }

    let mut edges = HashMap::new();
    for edge in &layout.edges {
        let Some(from_key) = node_groups.get(&edge.from) else {
            continue;
        };
        let Some(to_key) = node_groups.get(&edge.to) else {
            continue;
        };
        if from_key == to_key {
            continue;
        }
        let directed = edge.directed || edge.arrow_end || edge.arrow_start;
        let map_key = format!("{from_key}->{to_key}:{}", u8::from(directed));
        edges.entry(map_key).or_insert_with(|| MermaidOutlineEdge {
            from_key: from_key.clone(),
            to_key: to_key.clone(),
            directed,
        });
    }

    edges
}

pub(crate) fn mermaid_outline_segment(
    axis: MermaidOutlineAxis,
    fixed: i32,
    start: i32,
    end: i32,
) -> MermaidOutlineSegment {
    if start <= end {
        MermaidOutlineSegment {
            axis,
            fixed,
            start,
            end,
        }
    } else {
        MermaidOutlineSegment {
            axis,
            fixed,
            start: end,
            end: start,
        }
    }
}

pub(crate) fn mermaid_outline_intervals_overlap(
    left_start: i32,
    left_end: i32,
    right_start: i32,
    right_end: i32,
) -> bool {
    left_start <= right_end && right_start <= left_end
}

pub(crate) fn mermaid_outline_overlap_len(
    left_start: i32,
    left_end: i32,
    right_start: i32,
    right_end: i32,
) -> i32 {
    if !mermaid_outline_intervals_overlap(left_start, left_end, right_start, right_end) {
        return 0;
    }
    left_end.min(right_end) - left_start.max(right_start) + 1
}

pub(crate) fn mermaid_outline_label_rects(
    nodes: &[MermaidOutlineNode],
) -> HashMap<String, MermaidOutlineLabelRect> {
    nodes
        .iter()
        .map(|node| {
            (
                node.key.clone(),
                MermaidOutlineLabelRect {
                    left: node.x as i32,
                    right: node.x as i32 + node.text_width as i32 - 1,
                    top: node.y as i32,
                    bottom: node.y as i32,
                },
            )
        })
        .collect()
}

pub(crate) fn mermaid_outline_segment_crosses_label(
    segment: MermaidOutlineSegment,
    rect: MermaidOutlineLabelRect,
) -> bool {
    match segment.axis {
        MermaidOutlineAxis::Horizontal => {
            segment.fixed >= rect.top
                && segment.fixed <= rect.bottom
                && mermaid_outline_intervals_overlap(
                    segment.start,
                    segment.end,
                    rect.left,
                    rect.right,
                )
        }
        MermaidOutlineAxis::Vertical => {
            segment.fixed >= rect.left
                && segment.fixed <= rect.right
                && mermaid_outline_intervals_overlap(
                    segment.start,
                    segment.end,
                    rect.top,
                    rect.bottom,
                )
        }
    }
}

pub(crate) fn mermaid_outline_segment_score(
    segment: MermaidOutlineSegment,
    reserved_segments: &[MermaidOutlineSegment],
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
    ignore_keys: [&str; 2],
) -> i32 {
    let mut score = 0;

    for (key, rect) in label_rects {
        if ignore_keys.contains(&key.as_str()) {
            continue;
        }
        if mermaid_outline_segment_crosses_label(segment, *rect) {
            score += 1_000;
        }
    }

    for reserved in reserved_segments {
        match (segment.axis, reserved.axis) {
            (MermaidOutlineAxis::Horizontal, MermaidOutlineAxis::Horizontal)
            | (MermaidOutlineAxis::Vertical, MermaidOutlineAxis::Vertical) => {
                let overlap = mermaid_outline_overlap_len(
                    segment.start,
                    segment.end,
                    reserved.start,
                    reserved.end,
                );
                if overlap == 0 {
                    continue;
                }
                if segment.fixed == reserved.fixed {
                    score -= overlap * 6;
                } else if (segment.fixed - reserved.fixed).abs() <= 1 {
                    score += overlap * 14;
                } else if (segment.fixed - reserved.fixed).abs() == 2 {
                    score += overlap * 4;
                }
            }
            _ => {
                let crosses = mermaid_outline_intervals_overlap(
                    segment.start,
                    segment.end,
                    reserved.fixed,
                    reserved.fixed,
                ) && mermaid_outline_intervals_overlap(
                    reserved.start,
                    reserved.end,
                    segment.fixed,
                    segment.fixed,
                );
                if crosses {
                    score += 2;
                }
            }
        }
    }

    score
}

pub(crate) fn mermaid_outline_vertical_lane_candidates(
    content_rect: Rect,
    preferred: i32,
) -> Vec<i32> {
    let mut out = Vec::new();
    for offset in [0, -2, 2, -4, 4, -6, 6, -8, 8, -10, 10] {
        let candidate = preferred + offset;
        let clamped = candidate.clamp(content_rect.x as i32 + 1, content_rect.right() as i32 - 2);
        if !out.contains(&clamped) {
            out.push(clamped);
        }
    }
    out
}

pub(crate) fn mermaid_outline_horizontal_lane_candidates(
    content_rect: Rect,
    preferred: i32,
) -> Vec<i32> {
    let mut out = Vec::new();
    for offset in [0, -1, 1, -2, 2, -3, 3, -4, 4, -5, 5] {
        let candidate = preferred + offset;
        let clamped = candidate.clamp(content_rect.y as i32 + 1, content_rect.bottom() as i32 - 2);
        if !out.contains(&clamped) {
            out.push(clamped);
        }
    }
    out
}

pub(crate) fn mermaid_merge_outline_segments(
    segments: &[MermaidOutlineSegment],
) -> Vec<MermaidOutlineSegment> {
    let mut merged = segments.to_vec();
    merged.sort_by_key(|segment| {
        (
            match segment.axis {
                MermaidOutlineAxis::Horizontal => 0,
                MermaidOutlineAxis::Vertical => 1,
            },
            segment.fixed,
            segment.start,
            segment.end,
        )
    });

    let mut out: Vec<MermaidOutlineSegment> = Vec::new();
    for segment in merged {
        if let Some(last) = out.last_mut() {
            if last.axis == segment.axis
                && last.fixed == segment.fixed
                && segment.start <= last.end + 1
            {
                last.end = last.end.max(segment.end);
                continue;
            }
        }
        out.push(segment);
    }
    out
}

pub(crate) fn mermaid_outline_merge_char(existing: char, next: char) -> char {
    match (existing, next) {
        (' ', ch) => ch,
        (current, ch) if current == ch => current,
        ('>', _) | ('<', _) => existing,
        (_, '>') | (_, '<') => next,
        ('|', '_') | ('_', '|') => '|',
        ('|', '|') => '|',
        ('_', '_') => '_',
        (_, ch) => ch,
    }
}

pub(crate) fn mermaid_set_outline_cell(
    grid: &mut [Vec<char>],
    content_rect: Rect,
    x: i32,
    y: i32,
    ch: char,
) {
    if x < content_rect.x as i32
        || x >= content_rect.right() as i32
        || y < content_rect.y as i32
        || y >= content_rect.bottom() as i32
    {
        return;
    }
    let grid_x = (x - content_rect.x as i32) as usize;
    let grid_y = (y - content_rect.y as i32) as usize;
    let existing = grid[grid_y][grid_x];
    grid[grid_y][grid_x] = mermaid_outline_merge_char(existing, ch);
}

pub(crate) fn mermaid_draw_outline_horizontal(
    grid: &mut [Vec<char>],
    content_rect: Rect,
    left: i32,
    right: i32,
    y: i32,
) {
    let (start, end) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    for x in start..=end {
        mermaid_set_outline_cell(grid, content_rect, x, y, '_');
    }
}

pub(crate) fn mermaid_draw_outline_vertical(
    grid: &mut [Vec<char>],
    content_rect: Rect,
    x: i32,
    top: i32,
    bottom: i32,
) {
    let (start, end) = if top <= bottom {
        (top, bottom)
    } else {
        (bottom, top)
    };
    for y in start..=end {
        mermaid_set_outline_cell(grid, content_rect, x, y, '|');
    }
}

pub(crate) fn mermaid_plan_outline_route(
    content_rect: Rect,
    edge: &MermaidOutlineEdge,
    from: &MermaidOutlineNode,
    to: &MermaidOutlineNode,
    reserved_segments: &mut Vec<MermaidOutlineSegment>,
    lane_cache_vertical: &mut HashMap<(i32, i32, i8), i32>,
    lane_cache_horizontal: &mut HashMap<(i32, i32, i8), i32>,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> (Vec<MermaidOutlineSegment>, Option<MermaidOutlineArrow>) {
    let from_center_x = from.x as i32 + from.text_width as i32 / 2;
    let to_center_x = to.x as i32 + to.text_width as i32 / 2;
    let dx = to_center_x - from_center_x;
    let dy = to.y as i32 - from.y as i32;
    let prefer_horizontal = dx.abs() >= dy.abs();

    if prefer_horizontal {
        let start_x = if dx >= 0 {
            from.x as i32 + from.text_width as i32
        } else {
            from.x as i32 - 1
        };
        let end_x = if dx >= 0 {
            to.x as i32 - 1
        } else {
            to.x as i32 + to.text_width as i32
        };

        if from.y == to.y {
            let segment = mermaid_outline_segment(
                MermaidOutlineAxis::Horizontal,
                from.y as i32,
                start_x,
                end_x,
            );
            reserved_segments.push(segment);
            let arrow = edge.directed.then_some(MermaidOutlineArrow {
                x: end_x,
                y: to.y as i32,
                ch: if dx >= 0 { '>' } else { '<' },
            });
            return (vec![segment], arrow);
        }

        let lane_key = (
            (from.y as i32).min(to.y as i32),
            (from.y as i32).max(to.y as i32),
            dx.signum() as i8,
        );
        let preferred_lane = (start_x + end_x) / 2;
        let lane = if let Some(existing) = lane_cache_vertical.get(&lane_key).copied() {
            existing
        } else {
            let candidates = mermaid_outline_vertical_lane_candidates(content_rect, preferred_lane);
            let best = candidates
                .into_iter()
                .min_by_key(|candidate| {
                    let segments = [
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            from.y as i32,
                            start_x,
                            *candidate,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            *candidate,
                            from.y as i32,
                            to.y as i32,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            to.y as i32,
                            *candidate,
                            end_x,
                        ),
                    ];
                    segments
                        .into_iter()
                        .map(|segment| {
                            mermaid_outline_segment_score(
                                segment,
                                reserved_segments,
                                label_rects,
                                [&edge.from_key, &edge.to_key],
                            )
                        })
                        .sum::<i32>()
                        + (candidate - preferred_lane).abs() * 2
                })
                .unwrap_or(preferred_lane);
            lane_cache_vertical.insert(lane_key, best);
            best
        };

        let segments = vec![
            mermaid_outline_segment(MermaidOutlineAxis::Horizontal, from.y as i32, start_x, lane),
            mermaid_outline_segment(
                MermaidOutlineAxis::Vertical,
                lane,
                from.y as i32,
                to.y as i32,
            ),
            mermaid_outline_segment(MermaidOutlineAxis::Horizontal, to.y as i32, lane, end_x),
        ];
        reserved_segments.extend(segments.iter().copied());
        let arrow = edge.directed.then_some(MermaidOutlineArrow {
            x: end_x,
            y: to.y as i32,
            ch: if dx >= 0 { '>' } else { '<' },
        });
        (segments, arrow)
    } else {
        let start_y = if dy >= 0 {
            from.y as i32 + 1
        } else {
            from.y as i32 - 1
        };
        let end_y = if dy >= 0 {
            to.y as i32 - 1
        } else {
            to.y as i32 + 1
        };

        if from_center_x == to_center_x {
            let segment = mermaid_outline_segment(
                MermaidOutlineAxis::Vertical,
                from_center_x,
                start_y,
                end_y,
            );
            reserved_segments.push(segment);
            return (vec![segment], None);
        }

        let lane_key = (
            from_center_x.min(to_center_x),
            from_center_x.max(to_center_x),
            dy.signum() as i8,
        );
        let preferred_lane = (start_y + end_y) / 2;
        let lane = if let Some(existing) = lane_cache_horizontal.get(&lane_key).copied() {
            existing
        } else {
            let candidates =
                mermaid_outline_horizontal_lane_candidates(content_rect, preferred_lane);
            let best = candidates
                .into_iter()
                .min_by_key(|candidate| {
                    let segments = [
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            from_center_x,
                            start_y,
                            *candidate,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            *candidate,
                            from_center_x,
                            to_center_x,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            to_center_x,
                            *candidate,
                            end_y,
                        ),
                    ];
                    segments
                        .into_iter()
                        .map(|segment| {
                            mermaid_outline_segment_score(
                                segment,
                                reserved_segments,
                                label_rects,
                                [&edge.from_key, &edge.to_key],
                            )
                        })
                        .sum::<i32>()
                        + (candidate - preferred_lane).abs() * 2
                })
                .unwrap_or(preferred_lane);
            lane_cache_horizontal.insert(lane_key, best);
            best
        };

        let segments = vec![
            mermaid_outline_segment(MermaidOutlineAxis::Vertical, from_center_x, start_y, lane),
            mermaid_outline_segment(
                MermaidOutlineAxis::Horizontal,
                lane,
                from_center_x,
                to_center_x,
            ),
            mermaid_outline_segment(MermaidOutlineAxis::Vertical, to_center_x, lane, end_y),
        ];
        reserved_segments.extend(segments.iter().copied());
        (segments, None)
    }
}

pub(crate) fn mermaid_build_outline_paths(
    content_rect: Rect,
    nodes: &[MermaidOutlineNode],
    edges: impl IntoIterator<Item = MermaidOutlineEdge>,
) -> (Vec<MermaidOutlineSegment>, Vec<MermaidOutlineArrow>) {
    let node_map = nodes
        .iter()
        .map(|node| (node.key.clone(), node))
        .collect::<HashMap<_, _>>();
    let label_rects = mermaid_outline_label_rects(nodes);
    let mut reserved_segments = Vec::new();
    let mut raw_segments = Vec::new();
    let mut arrows = Vec::new();
    let mut lane_cache_vertical = HashMap::new();
    let mut lane_cache_horizontal = HashMap::new();

    let mut edge_list = edges.into_iter().collect::<Vec<_>>();
    edge_list.sort_by_key(|edge| {
        let from = node_map.get(&edge.from_key);
        let to = node_map.get(&edge.to_key);
        match (from, to) {
            (Some(from), Some(to)) => {
                let from_center_x = from.x as i32 + from.text_width as i32 / 2;
                let to_center_x = to.x as i32 + to.text_width as i32 / 2;
                (
                    -((to_center_x - from_center_x).abs() + (to.y as i32 - from.y as i32).abs()),
                    from.y,
                    to.y,
                )
            }
            _ => (0, 0, 0),
        }
    });

    for edge in edge_list {
        let Some(from) = node_map.get(&edge.from_key) else {
            continue;
        };
        let Some(to) = node_map.get(&edge.to_key) else {
            continue;
        };
        let (segments, arrow) = mermaid_plan_outline_route(
            content_rect,
            &edge,
            from,
            to,
            &mut reserved_segments,
            &mut lane_cache_vertical,
            &mut lane_cache_horizontal,
            &label_rects,
        );
        raw_segments.extend(segments);
        if let Some(arrow) = arrow {
            arrows.push(arrow);
        }
    }

    (mermaid_merge_outline_segments(&raw_segments), arrows)
}

pub(crate) fn mermaid_render_outline_background(
    content_rect: Rect,
    nodes: &[MermaidOutlineNode],
    edges: impl IntoIterator<Item = MermaidOutlineEdge>,
) -> Vec<String> {
    let mut grid = vec![vec![' '; content_rect.width as usize]; content_rect.height as usize];

    let (segments, arrows) = mermaid_build_outline_paths(content_rect, nodes, edges);
    for segment in segments {
        match segment.axis {
            MermaidOutlineAxis::Horizontal => {
                mermaid_draw_outline_horizontal(
                    &mut grid,
                    content_rect,
                    segment.start,
                    segment.end,
                    segment.fixed,
                );
            }
            MermaidOutlineAxis::Vertical => {
                mermaid_draw_outline_vertical(
                    &mut grid,
                    content_rect,
                    segment.fixed,
                    segment.start,
                    segment.end,
                );
            }
        }
    }
    for arrow in arrows {
        mermaid_set_outline_cell(&mut grid, content_rect, arrow.x, arrow.y, arrow.ch);
    }

    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect()
}

pub(crate) fn mermaid_outline_rect_center_x(rect: MermaidOutlineLabelRect) -> i32 {
    (rect.left + rect.right) / 2
}

pub(crate) fn mermaid_outline_rect_center_y(rect: MermaidOutlineLabelRect) -> i32 {
    (rect.top + rect.bottom) / 2
}

pub(crate) fn mermaid_is_compact_box_owner_key(owner_key: &str) -> bool {
    owner_key.starts_with("node:") || owner_key.starts_with("subgraph:")
}

pub(crate) fn mermaid_detail_box_rects(
    semantic_lines: &[MermaidSemanticLine],
    projected: &[MermaidProjectedLine],
    content_rect: Rect,
) -> HashMap<String, MermaidOutlineLabelRect> {
    let mut rects = HashMap::<String, MermaidOutlineLabelRect>::new();

    for line in projected {
        let Some(source) = semantic_lines.get(line.source_index) else {
            continue;
        };
        if !mermaid_is_compact_box_owner_key(&source.owner_key) {
            continue;
        }

        let line_left = line.x as i32;
        let line_right = line_left + display_width(&line.text) as i32 - 1;
        let line_y = line.y as i32;

        rects
            .entry(source.owner_key.clone())
            .and_modify(|rect| {
                rect.left = rect.left.min(line_left);
                rect.right = rect.right.max(line_right);
                rect.top = rect.top.min(line_y);
                rect.bottom = rect.bottom.max(line_y);
            })
            .or_insert(MermaidOutlineLabelRect {
                left: line_left,
                right: line_right,
                top: line_y,
                bottom: line_y,
            });
    }

    let min_x = content_rect.x as i32;
    let max_x = content_rect.right() as i32 - 1;
    let min_y = content_rect.y as i32;
    let max_y = content_rect.bottom() as i32 - 1;

    for rect in rects.values_mut() {
        rect.left = (rect.left - 1).clamp(min_x, max_x);
        rect.right = (rect.right + 1).clamp(rect.left, max_x);
        rect.top = (rect.top - 1).clamp(min_y, max_y);
        rect.bottom = (rect.bottom + 1).clamp(rect.top, max_y);

        if rect.right == rect.left && rect.right < max_x {
            rect.right += 1;
        }
        if rect.bottom == rect.top && rect.bottom < max_y {
            rect.bottom += 1;
        }
    }

    rects
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

pub(crate) fn mermaid_pack_detail_box_rects(
    content_rect: Rect,
    owners: &[MermaidPackedDetailOwner],
) -> HashMap<String, MermaidOutlineLabelRect> {
    #[derive(Clone, Copy)]
    struct BoxSize {
        outer_width: u16,
        outer_height: u16,
    }

    let specs = owners
        .iter()
        .map(|owner| {
            let inner_width = owner
                .lines
                .iter()
                .map(|line| display_width(&line.text))
                .max()
                .unwrap_or(1)
                .min(content_rect.width.saturating_sub(2).max(1));
            let inner_height = owner.lines.len().max(1) as u16;
            BoxSize {
                outer_width: inner_width.saturating_add(2).min(content_rect.width.max(1)),
                outer_height: inner_height
                    .saturating_add(2)
                    .min(content_rect.height.max(1)),
            }
        })
        .collect::<Vec<_>>();
    if specs.is_empty() {
        return HashMap::new();
    }

    let gap_x = 2u16;
    let gap_y = 1u16;
    let viewport_width = content_rect.width.max(1);
    let viewport_height = content_rect.height.max(1);
    let target_aspect = viewport_width as f32 / viewport_height as f32;

    let mut best_layout = None::<(usize, Vec<u16>, Vec<u16>, u16, u16, f32)>;
    for column_count in 1..=owners.len() {
        let mut row_widths = Vec::new();
        let mut row_heights = Vec::new();
        let mut row_start = 0usize;
        let mut fits = true;
        while row_start < specs.len() {
            let row = &specs[row_start..(row_start + column_count).min(specs.len())];
            let row_width = row
                .iter()
                .map(|spec| spec.outer_width)
                .sum::<u16>()
                .saturating_add(gap_x.saturating_mul(row.len().saturating_sub(1) as u16));
            let row_height = row.iter().map(|spec| spec.outer_height).max().unwrap_or(0);
            if row_width > viewport_width || row_height > viewport_height {
                fits = false;
                break;
            }
            row_widths.push(row_width);
            row_heights.push(row_height);
            row_start += column_count;
        }
        if !fits || row_widths.is_empty() {
            continue;
        }

        let cluster_width = row_widths.iter().copied().max().unwrap_or(0);
        let cluster_height = row_heights
            .iter()
            .copied()
            .sum::<u16>()
            .saturating_add(gap_y.saturating_mul(row_heights.len().saturating_sub(1) as u16));
        if cluster_width > viewport_width || cluster_height > viewport_height {
            continue;
        }

        let width_util = cluster_width as f32 / viewport_width as f32;
        let height_util = cluster_height as f32 / viewport_height as f32;
        let area_util = width_util * height_util;
        let aspect = cluster_width as f32 / cluster_height.max(1) as f32;
        let aspect_penalty = (aspect - target_aspect).abs();
        let score =
            width_util.min(height_util) * 1000.0 + area_util * 400.0 - aspect_penalty * 40.0;

        match best_layout {
            Some((_, _, _, _, _, best_score)) if best_score >= score => {}
            _ => {
                best_layout = Some((
                    column_count,
                    row_widths,
                    row_heights,
                    cluster_width,
                    cluster_height,
                    score,
                ));
            }
        }
    }

    let (column_count, row_widths, row_heights, _cluster_width, cluster_height, _) = best_layout
        .unwrap_or_else(|| {
            let row_widths = specs
                .iter()
                .map(|spec| spec.outer_width)
                .collect::<Vec<_>>();
            let row_heights = specs
                .iter()
                .map(|spec| spec.outer_height)
                .collect::<Vec<_>>();
            let cluster_width = row_widths
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .min(viewport_width);
            let cluster_height = row_heights
                .iter()
                .copied()
                .sum::<u16>()
                .saturating_add(gap_y.saturating_mul(row_heights.len().saturating_sub(1) as u16))
                .min(viewport_height);
            (
                1,
                row_widths,
                row_heights,
                cluster_width,
                cluster_height,
                0.0,
            )
        });

    let start_y = content_rect
        .y
        .saturating_add(viewport_height.saturating_sub(cluster_height) / 2);

    let mut rects = HashMap::new();
    let mut row_top = start_y;
    let mut owner_index = 0usize;
    for (row_index, row_width) in row_widths.iter().enumerate() {
        let row = &owners[owner_index..(owner_index + column_count).min(owners.len())];
        let row_left = content_rect
            .x
            .saturating_add(viewport_width.saturating_sub(*row_width) / 2);
        let mut column_left = row_left;
        for owner in row {
            let spec = specs[owner_index];
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
            owner_index += 1;
        }
        row_top = row_top
            .saturating_add(row_heights[row_index])
            .saturating_add(gap_y);
    }

    rects
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

pub(crate) fn mermaid_draw_outline_rect(
    grid: &mut [Vec<char>],
    content_rect: Rect,
    rect: MermaidOutlineLabelRect,
) {
    mermaid_draw_outline_horizontal(grid, content_rect, rect.left, rect.right, rect.top);
    mermaid_draw_outline_horizontal(grid, content_rect, rect.left, rect.right, rect.bottom);
    mermaid_draw_outline_vertical(grid, content_rect, rect.left, rect.top, rect.bottom);
    mermaid_draw_outline_vertical(grid, content_rect, rect.right, rect.top, rect.bottom);
}

pub(crate) fn mermaid_plan_outline_rect_route(
    content_rect: Rect,
    edge: &MermaidOutlineEdge,
    from_rect: MermaidOutlineLabelRect,
    to_rect: MermaidOutlineLabelRect,
    reserved_segments: &mut Vec<MermaidOutlineSegment>,
    lane_cache_vertical: &mut HashMap<(i32, i32, i8), i32>,
    lane_cache_horizontal: &mut HashMap<(i32, i32, i8), i32>,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> (Vec<MermaidOutlineSegment>, Option<MermaidOutlineArrow>) {
    let from_center_x = mermaid_outline_rect_center_x(from_rect);
    let to_center_x = mermaid_outline_rect_center_x(to_rect);
    let from_center_y = mermaid_outline_rect_center_y(from_rect);
    let to_center_y = mermaid_outline_rect_center_y(to_rect);
    let dx = to_center_x - from_center_x;
    let dy = to_center_y - from_center_y;
    let prefer_horizontal = dx.abs() >= dy.abs();

    if prefer_horizontal {
        let start_x = if dx >= 0 {
            from_rect.right + 1
        } else {
            from_rect.left - 1
        };
        let end_x = if dx >= 0 {
            to_rect.left - 1
        } else {
            to_rect.right + 1
        };

        if from_center_y == to_center_y {
            let segment = mermaid_outline_segment(
                MermaidOutlineAxis::Horizontal,
                from_center_y,
                start_x,
                end_x,
            );
            reserved_segments.push(segment);
            let arrow = edge.directed.then_some(MermaidOutlineArrow {
                x: end_x,
                y: to_center_y,
                ch: if dx >= 0 { '>' } else { '<' },
            });
            return (vec![segment], arrow);
        }

        let lane_key = (
            from_center_y.min(to_center_y),
            from_center_y.max(to_center_y),
            dx.signum() as i8,
        );
        let preferred_lane = (start_x + end_x) / 2;
        let lane = if let Some(existing) = lane_cache_vertical.get(&lane_key).copied() {
            existing
        } else {
            let candidates = mermaid_outline_vertical_lane_candidates(content_rect, preferred_lane);
            let best = candidates
                .into_iter()
                .min_by_key(|candidate| {
                    let segments = [
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            from_center_y,
                            start_x,
                            *candidate,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            *candidate,
                            from_center_y,
                            to_center_y,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            to_center_y,
                            *candidate,
                            end_x,
                        ),
                    ];
                    segments
                        .into_iter()
                        .map(|segment| {
                            mermaid_outline_segment_score(
                                segment,
                                reserved_segments,
                                label_rects,
                                [&edge.from_key, &edge.to_key],
                            )
                        })
                        .sum::<i32>()
                        + (candidate - preferred_lane).abs() * 2
                })
                .unwrap_or(preferred_lane);
            lane_cache_vertical.insert(lane_key, best);
            best
        };

        let segments = vec![
            mermaid_outline_segment(MermaidOutlineAxis::Horizontal, from_center_y, start_x, lane),
            mermaid_outline_segment(
                MermaidOutlineAxis::Vertical,
                lane,
                from_center_y,
                to_center_y,
            ),
            mermaid_outline_segment(MermaidOutlineAxis::Horizontal, to_center_y, lane, end_x),
        ];
        reserved_segments.extend(segments.iter().copied());
        let arrow = edge.directed.then_some(MermaidOutlineArrow {
            x: end_x,
            y: to_center_y,
            ch: if dx >= 0 { '>' } else { '<' },
        });
        (segments, arrow)
    } else {
        let start_y = if dy >= 0 {
            from_rect.bottom + 1
        } else {
            from_rect.top - 1
        };
        let end_y = if dy >= 0 {
            to_rect.top - 1
        } else {
            to_rect.bottom + 1
        };

        if from_center_x == to_center_x {
            let segment = mermaid_outline_segment(
                MermaidOutlineAxis::Vertical,
                from_center_x,
                start_y,
                end_y,
            );
            reserved_segments.push(segment);
            return (vec![segment], None);
        }

        let lane_key = (
            from_center_x.min(to_center_x),
            from_center_x.max(to_center_x),
            dy.signum() as i8,
        );
        let preferred_lane = (start_y + end_y) / 2;
        let lane = if let Some(existing) = lane_cache_horizontal.get(&lane_key).copied() {
            existing
        } else {
            let candidates =
                mermaid_outline_horizontal_lane_candidates(content_rect, preferred_lane);
            let best = candidates
                .into_iter()
                .min_by_key(|candidate| {
                    let segments = [
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            from_center_x,
                            start_y,
                            *candidate,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Horizontal,
                            *candidate,
                            from_center_x,
                            to_center_x,
                        ),
                        mermaid_outline_segment(
                            MermaidOutlineAxis::Vertical,
                            to_center_x,
                            *candidate,
                            end_y,
                        ),
                    ];
                    segments
                        .into_iter()
                        .map(|segment| {
                            mermaid_outline_segment_score(
                                segment,
                                reserved_segments,
                                label_rects,
                                [&edge.from_key, &edge.to_key],
                            )
                        })
                        .sum::<i32>()
                        + (candidate - preferred_lane).abs() * 2
                })
                .unwrap_or(preferred_lane);
            lane_cache_horizontal.insert(lane_key, best);
            best
        };

        let segments = vec![
            mermaid_outline_segment(MermaidOutlineAxis::Vertical, from_center_x, start_y, lane),
            mermaid_outline_segment(
                MermaidOutlineAxis::Horizontal,
                lane,
                from_center_x,
                to_center_x,
            ),
            mermaid_outline_segment(MermaidOutlineAxis::Vertical, to_center_x, lane, end_y),
        ];
        reserved_segments.extend(segments.iter().copied());
        (segments, None)
    }
}

pub(crate) fn mermaid_build_rect_outline_paths(
    content_rect: Rect,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
    edges: impl IntoIterator<Item = MermaidOutlineEdge>,
) -> (Vec<MermaidOutlineSegment>, Vec<MermaidOutlineArrow>) {
    let mut reserved_segments = Vec::new();
    let mut raw_segments = Vec::new();
    let mut arrows = Vec::new();
    let mut lane_cache_vertical = HashMap::new();
    let mut lane_cache_horizontal = HashMap::new();

    let mut edge_list = edges.into_iter().collect::<Vec<_>>();
    edge_list.sort_by_key(|edge| {
        match (
            label_rects.get(&edge.from_key).copied(),
            label_rects.get(&edge.to_key).copied(),
        ) {
            (Some(from_rect), Some(to_rect)) => (
                -((mermaid_outline_rect_center_x(to_rect)
                    - mermaid_outline_rect_center_x(from_rect))
                .abs()
                    + (mermaid_outline_rect_center_y(to_rect)
                        - mermaid_outline_rect_center_y(from_rect))
                    .abs()),
                from_rect.top,
                to_rect.top,
            ),
            _ => (0, 0, 0),
        }
    });

    for edge in edge_list {
        let Some(from_rect) = label_rects.get(&edge.from_key).copied() else {
            continue;
        };
        let Some(to_rect) = label_rects.get(&edge.to_key).copied() else {
            continue;
        };
        let (segments, arrow) = mermaid_plan_outline_rect_route(
            content_rect,
            &edge,
            from_rect,
            to_rect,
            &mut reserved_segments,
            &mut lane_cache_vertical,
            &mut lane_cache_horizontal,
            label_rects,
        );
        raw_segments.extend(segments);
        if let Some(arrow) = arrow {
            arrows.push(arrow);
        }
    }

    (mermaid_merge_outline_segments(&raw_segments), arrows)
}

pub(crate) fn mermaid_render_compact_detail_background(
    content_rect: Rect,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
    edges: impl IntoIterator<Item = MermaidOutlineEdge>,
) -> Vec<String> {
    let mut grid = vec![vec![' '; content_rect.width as usize]; content_rect.height as usize];

    let (segments, arrows) = mermaid_build_rect_outline_paths(content_rect, label_rects, edges);
    for segment in segments {
        match segment.axis {
            MermaidOutlineAxis::Horizontal => {
                mermaid_draw_outline_horizontal(
                    &mut grid,
                    content_rect,
                    segment.start,
                    segment.end,
                    segment.fixed,
                );
            }
            MermaidOutlineAxis::Vertical => {
                mermaid_draw_outline_vertical(
                    &mut grid,
                    content_rect,
                    segment.fixed,
                    segment.start,
                    segment.end,
                );
            }
        }
    }
    for rect in label_rects.values().copied() {
        mermaid_draw_outline_rect(&mut grid, content_rect, rect);
    }
    for arrow in arrows {
        mermaid_set_outline_cell(&mut grid, content_rect, arrow.x, arrow.y, arrow.ch);
    }

    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect()
}

pub(crate) fn mermaid_build_er_packed_boxes(
    prepared: &MermaidPreparedRender,
    view_state: MermaidViewState,
) -> Vec<MermaidErPackedBox> {
    if prepared.layout.kind != DiagramKind::Er || !view_state.is_er() {
        return Vec::new();
    }

    #[derive(Default)]
    struct AttrParts {
        name_source_index: Option<usize>,
        name_text: Option<String>,
        type_source_index: Option<usize>,
        type_text: Option<String>,
    }

    let mut source_indices_by_owner = HashMap::<String, Vec<usize>>::new();
    for (source_index, line) in prepared.semantic_lines.iter().enumerate() {
        source_indices_by_owner
            .entry(line.owner_key.clone())
            .or_default()
            .push(source_index);
    }

    let mut ordered_nodes = prepared
        .layout
        .nodes
        .values()
        .filter(|node| {
            !node.hidden
                && node.anchor_subgraph.is_none()
                && node
                    .label
                    .lines
                    .iter()
                    .any(|line| mermaid_is_divider_line(line))
        })
        .collect::<Vec<_>>();
    ordered_nodes.sort_by(|left, right| {
        left.y
            .partial_cmp(&right.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.x
                    .partial_cmp(&right.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut out = Vec::new();
    for node in ordered_nodes {
        let owner_key = mermaid_outline_node_key(&node.id);
        let Some(source_indices) = source_indices_by_owner.get(&owner_key) else {
            continue;
        };

        let mut summary_line = None;
        let mut title_lines = Vec::<(usize, String)>::new();
        let mut attr_parts = std::collections::BTreeMap::<i32, AttrParts>::new();
        for source_index in source_indices {
            let Some(line) = prepared.semantic_lines.get(*source_index) else {
                continue;
            };
            match line.kind {
                MermaidSemanticKind::NodeSummary => {
                    summary_line = Some((*source_index, mermaid_fit_whole_words(&line.text, 18)));
                }
                MermaidSemanticKind::NodeTitle => {
                    title_lines.push((*source_index, line.text.clone()));
                }
                MermaidSemanticKind::ErAttributeName => {
                    let key = (line.diagram_y * 10.0).round() as i32;
                    let entry = attr_parts.entry(key).or_default();
                    entry.name_source_index = Some(*source_index);
                    entry.name_text = Some(line.text.clone());
                }
                MermaidSemanticKind::ErAttributeType => {
                    let key = (line.diagram_y * 10.0).round() as i32;
                    let entry = attr_parts.entry(key).or_default();
                    entry.type_source_index = Some(*source_index);
                    entry.type_text = Some(line.text.clone());
                }
                MermaidSemanticKind::SubgraphSummary
                | MermaidSemanticKind::SubgraphTitle
                | MermaidSemanticKind::EdgeLabel
                | MermaidSemanticKind::ClassMember => {}
            }
        }

        title_lines.sort_by_key(|(source_index, _)| *source_index);
        let attr_rows = attr_parts
            .into_values()
            .filter_map(|parts| {
                let name_source_index = parts.name_source_index?;
                let name_text = parts.name_text?;
                Some(MermaidErPackedAttrRow {
                    name_source_index,
                    name_text,
                    type_source_index: parts.type_source_index,
                    type_text: parts.type_text,
                })
            })
            .filter(|row| match view_state {
                MermaidViewState::ErEntities => false,
                MermaidViewState::ErKeys => {
                    row.name_text.contains(" PK") || row.name_text.contains(" FK")
                }
                MermaidViewState::ErColumns | MermaidViewState::ErSchema => true,
                MermaidViewState::Outline
                | MermaidViewState::L1
                | MermaidViewState::L2
                | MermaidViewState::L3 => false,
            })
            .collect::<Vec<_>>();

        let title_lines = match view_state {
            MermaidViewState::ErEntities => summary_line
                .into_iter()
                .collect::<Vec<_>>()
                .into_iter()
                .take(1)
                .collect(),
            MermaidViewState::ErKeys | MermaidViewState::ErColumns | MermaidViewState::ErSchema => {
                title_lines
            }
            MermaidViewState::Outline
            | MermaidViewState::L1
            | MermaidViewState::L2
            | MermaidViewState::L3 => Vec::new(),
        };

        if title_lines.is_empty() && attr_rows.is_empty() {
            continue;
        }

        out.push(MermaidErPackedBox {
            owner_key,
            sort_x: node.x,
            sort_y: node.y,
            title_lines,
            attr_rows,
        });
    }

    out
}

pub(crate) fn mermaid_er_box_inner_size(
    er_box: &MermaidErPackedBox,
    view_state: MermaidViewState,
) -> (u16, u16, u16) {
    let title_width = er_box
        .title_lines
        .iter()
        .map(|(_, text)| display_width(text))
        .max()
        .unwrap_or(0);
    let type_col_width = if view_state == MermaidViewState::ErSchema {
        er_box
            .attr_rows
            .iter()
            .filter_map(|row| row.type_text.as_ref().map(|text| display_width(text)))
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    let attr_width = er_box
        .attr_rows
        .iter()
        .map(|row| {
            let name_width = display_width(&row.name_text);
            if view_state == MermaidViewState::ErSchema && type_col_width > 0 {
                type_col_width.saturating_add(2).saturating_add(name_width)
            } else {
                name_width
            }
        })
        .max()
        .unwrap_or(0);
    let inner_width = title_width.max(attr_width).max(1);
    let inner_height = (er_box.title_lines.len() + er_box.attr_rows.len()).max(1) as u16;
    (inner_width, inner_height, type_col_width)
}

fn mermaid_cmp_f32(left: f32, right: f32) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

fn mermaid_distance_sq(left: (f32, f32), right: (f32, f32)) -> f32 {
    let dx = left.0 - right.0;
    let dy = left.1 - right.1;
    dx * dx + dy * dy
}

fn mermaid_er_node_position<'a>(
    positions: &'a HashMap<String, (f32, f32)>,
    owner_key: &str,
) -> &'a (f32, f32) {
    positions
        .get(owner_key)
        .unwrap_or_else(|| panic!("missing ER order node position for {owner_key}"))
}

fn mermaid_er_compare_seed_nodes(
    left: &str,
    right: &str,
    positions: &HashMap<String, (f32, f32)>,
    adjacency: &HashMap<String, BTreeSet<String>>,
    centroid: (f32, f32),
) -> Ordering {
    let left_degree = adjacency.get(left).map_or(0, BTreeSet::len);
    let right_degree = adjacency.get(right).map_or(0, BTreeSet::len);
    let left_position = *mermaid_er_node_position(positions, left);
    let right_position = *mermaid_er_node_position(positions, right);

    right_degree
        .cmp(&left_degree)
        .then_with(|| {
            mermaid_cmp_f32(
                mermaid_distance_sq(left_position, centroid),
                mermaid_distance_sq(right_position, centroid),
            )
        })
        .then_with(|| mermaid_cmp_f32(left_position.1, right_position.1))
        .then_with(|| mermaid_cmp_f32(left_position.0, right_position.0))
        .then_with(|| left.cmp(right))
}

fn mermaid_er_candidate_metrics(
    owner_key: &str,
    placed: &HashSet<String>,
    positions: &HashMap<String, (f32, f32)>,
    adjacency: &HashMap<String, BTreeSet<String>>,
) -> (usize, f32) {
    let neighbors = adjacency.get(owner_key);
    let adjacent_neighbors = neighbors
        .into_iter()
        .flatten()
        .filter(|neighbor| placed.contains(*neighbor))
        .collect::<Vec<_>>();
    let adjacent_count = adjacent_neighbors.len();
    let position = *mermaid_er_node_position(positions, owner_key);
    let min_neighbor_distance = adjacent_neighbors
        .into_iter()
        .map(|neighbor| {
            mermaid_distance_sq(position, *mermaid_er_node_position(positions, neighbor))
        })
        .min_by(|left, right| mermaid_cmp_f32(*left, *right))
        .unwrap_or(f32::INFINITY);
    (adjacent_count, min_neighbor_distance)
}

fn mermaid_er_compare_component_candidates(
    left: &str,
    right: &str,
    placed: &HashSet<String>,
    positions: &HashMap<String, (f32, f32)>,
    adjacency: &HashMap<String, BTreeSet<String>>,
    component_centroid: (f32, f32),
) -> Ordering {
    let (left_adjacent_count, left_neighbor_distance) =
        mermaid_er_candidate_metrics(left, placed, positions, adjacency);
    let (right_adjacent_count, right_neighbor_distance) =
        mermaid_er_candidate_metrics(right, placed, positions, adjacency);
    let left_degree = adjacency.get(left).map_or(0, BTreeSet::len);
    let right_degree = adjacency.get(right).map_or(0, BTreeSet::len);
    let left_position = *mermaid_er_node_position(positions, left);
    let right_position = *mermaid_er_node_position(positions, right);

    right_adjacent_count
        .cmp(&left_adjacent_count)
        .then_with(|| mermaid_cmp_f32(left_neighbor_distance, right_neighbor_distance))
        .then_with(|| right_degree.cmp(&left_degree))
        .then_with(|| {
            mermaid_cmp_f32(
                mermaid_distance_sq(left_position, component_centroid),
                mermaid_distance_sq(right_position, component_centroid),
            )
        })
        .then_with(|| mermaid_cmp_f32(left_position.1, right_position.1))
        .then_with(|| mermaid_cmp_f32(left_position.0, right_position.0))
        .then_with(|| left.cmp(right))
}

pub(crate) fn mermaid_order_er_nodes(nodes: &[MermaidErOrderNode]) -> Vec<String> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let known_keys = nodes
        .iter()
        .map(|node| node.owner_key.clone())
        .collect::<HashSet<_>>();
    let mut positions = HashMap::<String, (f32, f32)>::new();
    let mut adjacency = HashMap::<String, BTreeSet<String>>::new();

    for node in nodes {
        positions.insert(node.owner_key.clone(), (node.x, node.y));
        let entry = adjacency.entry(node.owner_key.clone()).or_default();
        for neighbor in &node.neighbors {
            if neighbor != &node.owner_key && known_keys.contains(neighbor) {
                entry.insert(neighbor.clone());
            }
        }
    }

    for node in nodes {
        let owner_key = node.owner_key.clone();
        let neighbors = adjacency.get(&owner_key).cloned().unwrap_or_default();
        adjacency.entry(owner_key.clone()).or_default();
        for neighbor in neighbors {
            adjacency
                .entry(neighbor)
                .or_default()
                .insert(owner_key.clone());
        }
    }

    let centroid = {
        let mut total_x = 0.0f32;
        let mut total_y = 0.0f32;
        for node in nodes {
            total_x += node.x;
            total_y += node.y;
        }
        (total_x / nodes.len() as f32, total_y / nodes.len() as f32)
    };

    let mut unseen = positions.keys().cloned().collect::<BTreeSet<_>>();
    let mut components = Vec::<Vec<String>>::new();
    while let Some(start) = unseen.iter().next().cloned() {
        let mut stack = vec![start.clone()];
        let mut component = Vec::new();
        unseen.remove(&start);
        while let Some(owner_key) = stack.pop() {
            component.push(owner_key.clone());
            let neighbors = adjacency.get(&owner_key).cloned().unwrap_or_default();
            for neighbor in neighbors {
                if unseen.remove(&neighbor) {
                    stack.push(neighbor);
                }
            }
        }
        component.sort_by(|left, right| {
            mermaid_er_compare_seed_nodes(left, right, &positions, &adjacency, centroid)
        });
        components.push(component);
    }

    components.sort_by(|left, right| {
        let left_seed = left.first().expect("component seed");
        let right_seed = right.first().expect("component seed");
        right.len().cmp(&left.len()).then_with(|| {
            mermaid_er_compare_seed_nodes(left_seed, right_seed, &positions, &adjacency, centroid)
        })
    });

    let mut ordered = Vec::with_capacity(nodes.len());
    for component in components {
        let component_centroid = {
            let mut total_x = 0.0f32;
            let mut total_y = 0.0f32;
            for owner_key in &component {
                let position = mermaid_er_node_position(&positions, owner_key);
                total_x += position.0;
                total_y += position.1;
            }
            (
                total_x / component.len() as f32,
                total_y / component.len() as f32,
            )
        };

        let mut remaining = component.into_iter().collect::<BTreeSet<_>>();
        let seed = remaining
            .iter()
            .min_by(|left, right| {
                mermaid_er_compare_seed_nodes(
                    left,
                    right,
                    &positions,
                    &adjacency,
                    component_centroid,
                )
            })
            .cloned()
            .expect("component seed");
        remaining.remove(&seed);
        ordered.push(seed.clone());

        let mut placed = HashSet::from([seed]);
        while !remaining.is_empty() {
            let next = remaining
                .iter()
                .min_by(|left, right| {
                    mermaid_er_compare_component_candidates(
                        left,
                        right,
                        &placed,
                        &positions,
                        &adjacency,
                        component_centroid,
                    )
                })
                .cloned()
                .expect("remaining ER order node");
            remaining.remove(&next);
            placed.insert(next.clone());
            ordered.push(next);
        }
    }

    ordered
}

pub(crate) fn mermaid_er_order_from_layout(
    layout: &MermaidLayout,
    boxes: &[MermaidErPackedBox],
) -> Vec<String> {
    let owner_keys = boxes
        .iter()
        .map(|er_box| er_box.owner_key.clone())
        .collect::<HashSet<_>>();
    if owner_keys.is_empty() {
        return Vec::new();
    }

    let mut positions = HashMap::<String, (f32, f32)>::new();
    let mut owners_by_node_id = HashMap::<String, String>::new();
    for node in layout.nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        let owner_key = mermaid_outline_node_key(&node.id);
        if !owner_keys.contains(&owner_key) {
            continue;
        }
        positions.insert(owner_key.clone(), (node.x, node.y));
        owners_by_node_id.insert(node.id.clone(), owner_key);
    }

    let mut neighbors = HashMap::<String, BTreeSet<String>>::new();
    for edge in &layout.edges {
        let Some(from_key) = owners_by_node_id.get(&edge.from) else {
            continue;
        };
        let Some(to_key) = owners_by_node_id.get(&edge.to) else {
            continue;
        };
        if from_key == to_key {
            continue;
        }
        neighbors
            .entry(from_key.clone())
            .or_default()
            .insert(to_key.clone());
        neighbors
            .entry(to_key.clone())
            .or_default()
            .insert(from_key.clone());
    }

    let mut nodes = positions
        .into_iter()
        .map(|(owner_key, (x, y))| MermaidErOrderNode {
            neighbors: neighbors
                .remove(&owner_key)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            owner_key,
            x,
            y,
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        mermaid_cmp_f32(left.y, right.y)
            .then_with(|| mermaid_cmp_f32(left.x, right.x))
            .then_with(|| left.owner_key.cmp(&right.owner_key))
    });
    mermaid_order_er_nodes(&nodes)
}

pub(crate) fn mermaid_pack_er_box_rects(
    content_rect: Rect,
    boxes: &[MermaidErPackedBox],
    view_state: MermaidViewState,
) -> HashMap<String, (MermaidOutlineLabelRect, u16)> {
    #[derive(Clone, Copy)]
    struct BoxSize {
        outer_width: u16,
        outer_height: u16,
        type_col_width: u16,
    }

    let specs = boxes
        .iter()
        .map(|er_box| {
            let (inner_width, inner_height, type_col_width) =
                mermaid_er_box_inner_size(er_box, view_state);
            BoxSize {
                outer_width: inner_width.saturating_add(2),
                outer_height: inner_height.saturating_add(2),
                type_col_width,
            }
        })
        .collect::<Vec<_>>();
    if specs.is_empty() {
        return HashMap::new();
    }

    let gap_x = 2u16;
    let gap_y = 1u16;
    let viewport_width = content_rect.width.max(1);
    let viewport_height = content_rect.height.max(1);
    let target_aspect = viewport_width as f32 / viewport_height as f32;

    let mut best_layout = None::<(usize, Vec<u16>, Vec<u16>, u16, u16, f32)>;
    for column_count in 1..=boxes.len() {
        let mut row_widths = Vec::new();
        let mut row_heights = Vec::new();
        let mut row_start = 0usize;
        let mut fits = true;
        while row_start < specs.len() {
            let row = &specs[row_start..(row_start + column_count).min(specs.len())];
            let row_width = row
                .iter()
                .map(|spec| spec.outer_width)
                .sum::<u16>()
                .saturating_add(gap_x.saturating_mul(row.len().saturating_sub(1) as u16));
            let row_height = row.iter().map(|spec| spec.outer_height).max().unwrap_or(0);
            if row_width > viewport_width || row_height > viewport_height {
                fits = false;
                break;
            }
            row_widths.push(row_width);
            row_heights.push(row_height);
            row_start += column_count;
        }
        if !fits || row_widths.is_empty() {
            continue;
        }

        let cluster_width = row_widths.iter().copied().max().unwrap_or(0);
        let cluster_height = row_heights
            .iter()
            .copied()
            .sum::<u16>()
            .saturating_add(gap_y.saturating_mul(row_heights.len().saturating_sub(1) as u16));
        if cluster_width > viewport_width || cluster_height > viewport_height {
            continue;
        }

        let width_util = cluster_width as f32 / viewport_width as f32;
        let height_util = cluster_height as f32 / viewport_height as f32;
        let area_util = width_util * height_util;
        let aspect = cluster_width as f32 / cluster_height.max(1) as f32;
        let aspect_penalty = (aspect - target_aspect).abs();
        let score =
            width_util.min(height_util) * 1000.0 + area_util * 400.0 - aspect_penalty * 40.0;

        match best_layout {
            Some((_, _, _, _, _, best_score)) if best_score >= score => {}
            _ => {
                best_layout = Some((
                    column_count,
                    row_widths,
                    row_heights,
                    cluster_width,
                    cluster_height,
                    score,
                ));
            }
        }
    }

    let (column_count, row_widths, row_heights, _cluster_width, cluster_height, _) = best_layout
        .unwrap_or_else(|| {
            let row_widths = specs
                .iter()
                .map(|spec| spec.outer_width)
                .collect::<Vec<_>>();
            let row_heights = specs
                .iter()
                .map(|spec| spec.outer_height)
                .collect::<Vec<_>>();
            let cluster_width = row_widths
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .min(viewport_width);
            let cluster_height = row_heights
                .iter()
                .copied()
                .sum::<u16>()
                .saturating_add(gap_y.saturating_mul(row_heights.len().saturating_sub(1) as u16))
                .min(viewport_height);
            (
                1,
                row_widths,
                row_heights,
                cluster_width,
                cluster_height,
                0.0,
            )
        });

    let start_y = content_rect
        .y
        .saturating_add(viewport_height.saturating_sub(cluster_height) / 2);

    let mut rects = HashMap::new();
    let mut row_top = start_y;
    let mut box_index = 0usize;
    for (row_index, row_width) in row_widths.iter().enumerate() {
        let row = &boxes[box_index..(box_index + column_count).min(boxes.len())];
        let row_left = content_rect
            .x
            .saturating_add(viewport_width.saturating_sub(*row_width) / 2);
        let mut column_left = row_left;
        for er_box in row {
            let spec = specs[box_index];
            let rect = MermaidOutlineLabelRect {
                left: column_left as i32,
                right: column_left
                    .saturating_add(spec.outer_width)
                    .saturating_sub(1) as i32,
                top: row_top as i32,
                bottom: row_top.saturating_add(spec.outer_height).saturating_sub(1) as i32,
            };
            rects.insert(er_box.owner_key.clone(), (rect, spec.type_col_width));
            column_left = column_left
                .saturating_add(spec.outer_width)
                .saturating_add(gap_x);
            box_index += 1;
        }
        row_top = row_top
            .saturating_add(row_heights[row_index])
            .saturating_add(gap_y);
    }

    rects
}

pub(crate) fn mermaid_project_er_packed_lines(
    content_rect: Rect,
    boxes: &[MermaidErPackedBox],
    view_state: MermaidViewState,
    owner_colors: &HashMap<String, Color>,
) -> (
    Vec<MermaidProjectedLine>,
    HashMap<String, MermaidOutlineLabelRect>,
) {
    let rects = mermaid_pack_er_box_rects(content_rect, boxes, view_state);
    let mut projected = Vec::new();
    let mut label_rects = HashMap::new();

    for er_box in boxes {
        let Some((rect, type_col_width)) = rects.get(&er_box.owner_key).copied() else {
            continue;
        };
        label_rects.insert(er_box.owner_key.clone(), rect);

        let inner_left = rect.left + 1;
        let mut line_y = rect.top + 1;
        let inner_width = (rect.right - rect.left - 1).max(1) as u16;
        for (source_index, text) in &er_box.title_lines {
            let text_width = display_width(text).min(inner_width);
            let title_x = inner_left + ((inner_width.saturating_sub(text_width)) / 2) as i32;
            projected.push(MermaidProjectedLine {
                source_index: *source_index,
                x: title_x as u16,
                y: line_y as u16,
                text: text.clone(),
                color: mermaid_owner_accent_color(&er_box.owner_key, owner_colors),
            });
            line_y += 1;
        }
        for row in &er_box.attr_rows {
            if view_state == MermaidViewState::ErSchema {
                if let (Some(source_index), Some(type_text)) =
                    (row.type_source_index, row.type_text.as_ref())
                {
                    projected.push(MermaidProjectedLine {
                        source_index,
                        x: inner_left as u16,
                        y: line_y as u16,
                        text: type_text.clone(),
                        color: MERMAID_TYPE_COLOR,
                    });
                }
                projected.push(MermaidProjectedLine {
                    source_index: row.name_source_index,
                    x: (inner_left + i32::from(type_col_width) + 2) as u16,
                    y: line_y as u16,
                    text: row.name_text.clone(),
                    color: MERMAID_BODY_COLOR,
                });
            } else {
                projected.push(MermaidProjectedLine {
                    source_index: row.name_source_index,
                    x: inner_left as u16,
                    y: line_y as u16,
                    text: row.name_text.clone(),
                    color: MERMAID_BODY_COLOR,
                });
            }
            line_y += 1;
        }
    }

    projected.sort_by_key(|line| (line.y, line.x));
    (projected, label_rects)
}

pub(crate) fn render_mermaid_er_packed_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    view_state: MermaidViewState,
) -> Result<bool, String> {
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };
    if prepared.layout.kind != DiagramKind::Er || !view_state.is_er() {
        return Ok(false);
    }

    let mut boxes = mermaid_build_er_packed_boxes(prepared, view_state);
    let order = mermaid_er_order_from_layout(&prepared.layout, &boxes);
    let order_index = order
        .into_iter()
        .enumerate()
        .map(|(idx, owner_key)| (owner_key, idx))
        .collect::<HashMap<_, _>>();
    boxes.sort_by(|left, right| {
        let left_index = order_index
            .get(&left.owner_key)
            .copied()
            .unwrap_or(usize::MAX);
        let right_index = order_index
            .get(&right.owner_key)
            .copied()
            .unwrap_or(usize::MAX);
        left_index.cmp(&right_index).then_with(|| {
            mermaid_cmp_f32(left.sort_y, right.sort_y)
                .then_with(|| mermaid_cmp_f32(left.sort_x, right.sort_x))
                .then_with(|| left.owner_key.cmp(&right.owner_key))
        })
    });
    if boxes.is_empty() {
        return Ok(false);
    }

    let owner_colors = mermaid_owner_accent_map(&prepared.semantic_lines);
    let (projected, label_rects) =
        mermaid_project_er_packed_lines(content_rect, &boxes, view_state, &owner_colors);
    if projected.is_empty() || label_rects.is_empty() {
        return Ok(false);
    }

    let visible_keys = label_rects.keys().cloned().collect::<HashSet<_>>();
    let outline_edges = mermaid_outline_edge_map(&prepared.layout)
        .into_values()
        .filter(|edge| visible_keys.contains(&edge.from_key) && visible_keys.contains(&edge.to_key))
        .collect::<Vec<_>>();
    viewer.cached_lines =
        mermaid_render_compact_detail_background(content_rect, &label_rects, outline_edges);
    viewer.cached_background_cells =
        mermaid_background_cells_from_lines(&viewer.cached_lines, MERMAID_CONNECTOR_COLOR);
    mermaid_apply_rect_border_colors(
        &mut viewer.cached_background_cells,
        content_rect,
        &label_rects,
        &owner_colors,
    );
    viewer.cached_semantic_lines = projected;
    Ok(true)
}

pub(crate) fn mermaid_status_detail_label(view_state: MermaidViewState) -> String {
    view_state.status_label().to_string()
}

pub(crate) fn mermaid_compact_detail_hides_kind(
    view_state: MermaidViewState,
    kind: MermaidSemanticKind,
) -> bool {
    matches!(
        (view_state, kind),
        (
            MermaidViewState::L1 | MermaidViewState::L2 | MermaidViewState::L3,
            MermaidSemanticKind::EdgeLabel
        )
    )
}

pub(crate) fn project_mermaid_semantic_lines(
    lines: &[MermaidSemanticLine],
    transform: MermaidViewportTransform,
    content_rect: Rect,
    view_state: MermaidViewState,
) -> Vec<MermaidProjectedLine> {
    #[derive(Clone)]
    struct MermaidProjectedCandidate {
        priority: u8,
        area_rank: i32,
        kind: MermaidSemanticKind,
        owner_key: String,
        compact_rows: bool,
        source_index: usize,
        x: u16,
        y: u16,
        text: String,
        color: Color,
    }

    let mut candidates = Vec::new();
    let owner_colors = mermaid_owner_accent_map(lines);
    let left = content_rect.x as i32;
    let right = content_rect.right() as i32;
    let top = content_rect.y as i32;
    let bottom = content_rect.bottom() as i32;

    for (source_index, line) in lines.iter().enumerate() {
        if !mermaid_line_visible_in_state(line, view_state) {
            continue;
        }
        if mermaid_compact_detail_hides_kind(view_state, line.kind) {
            continue;
        }

        let owner_cols = (line.owner_width * transform.scale / 2.0).max(0.0);
        let owner_rows = (line.owner_height * transform.scale / 4.0).max(0.0);
        if !line.kind.is_visible_for_owner(owner_cols, owner_rows) {
            continue;
        }
        let display_text = mermaid_display_text_for_view(line, owner_cols, view_state);
        if display_text.trim().is_empty() {
            continue;
        }

        let projected_x = line.diagram_x * transform.scale + transform.tx;
        let projected_y = line.diagram_y * transform.scale + transform.ty;
        let mut screen_y = top + (projected_y / 4.0).floor() as i32;
        if screen_y < top || screen_y >= bottom {
            if line.kind.row_nudge_budget() > 0 {
                screen_y = screen_y.clamp(top, bottom.saturating_sub(1));
            } else {
                continue;
            }
        }

        let anchor_x = left + (projected_x / 2.0).floor() as i32;
        let text_width = display_width(&display_text) as i32;
        if text_width <= 0 {
            continue;
        }
        let mut screen_x = match line.anchor {
            MermaidTextAnchor::Start => anchor_x,
            MermaidTextAnchor::Center => anchor_x - text_width / 2,
        };
        if screen_x >= right || screen_x + text_width <= left {
            continue;
        }

        let skipped_chars = if screen_x < left {
            (left - screen_x) as usize
        } else {
            0
        };
        if screen_x < left {
            screen_x = left;
        }
        let max_chars = right.saturating_sub(screen_x) as usize;
        let clipped = clip_mermaid_overlay_text(&display_text, skipped_chars, max_chars);
        if clipped.is_empty() {
            continue;
        }

        candidates.push(MermaidProjectedCandidate {
            priority: line.kind.priority(),
            area_rank: -((owner_cols * owner_rows * 10.0).round() as i32),
            kind: line.kind,
            owner_key: line.owner_key.clone(),
            compact_rows: mermaid_kind_uses_compact_rows(line.kind),
            source_index,
            x: screen_x as u16,
            y: screen_y as u16,
            text: clipped,
            color: mermaid_semantic_line_color(line.kind, &line.owner_key, &owner_colors),
        });
    }

    let mut compact_owner_rows = HashMap::<String, Vec<u16>>::new();
    for candidate in &candidates {
        if !candidate.compact_rows {
            continue;
        }
        compact_owner_rows
            .entry(candidate.owner_key.clone())
            .or_default()
            .push(candidate.y);
    }
    for rows in compact_owner_rows.values_mut() {
        rows.sort_unstable();
        rows.dedup();
    }
    let max_row = content_rect.bottom().saturating_sub(1);
    for candidate in &mut candidates {
        if !candidate.compact_rows {
            continue;
        }
        let Some(rows) = compact_owner_rows.get(&candidate.owner_key) else {
            continue;
        };
        let Some(base_row) = rows.first().copied() else {
            continue;
        };
        let Some(compact_offset) = rows.iter().position(|row| *row == candidate.y) else {
            continue;
        };
        candidate.y = base_row.saturating_add(compact_offset as u16).min(max_row);
    }

    if matches!(view_state, MermaidViewState::L2 | MermaidViewState::L3) {
        let owner_detail_keys = candidates
            .iter()
            .filter(|candidate| mermaid_kind_is_owner_detail(candidate.kind))
            .map(|candidate| candidate.owner_key.clone())
            .collect::<HashSet<_>>();
        candidates.retain(|candidate| {
            !mermaid_kind_is_owner_summary(candidate.kind)
                || !owner_detail_keys.contains(&candidate.owner_key)
        });
    }

    candidates.sort_by_key(|line| (line.priority, line.area_rank, line.y, line.x));

    let mut occupied_rows: HashMap<u16, Vec<(u16, u16)>> = HashMap::new();
    let mut projected = Vec::new();
    let collision_padding = view_state.collision_padding();
    for candidate in candidates {
        let start = candidate.x;
        let end = candidate
            .x
            .saturating_add(display_width(&candidate.text).max(1));
        let padded_start = start.saturating_sub(collision_padding);
        let padded_end = end.saturating_add(collision_padding);
        let budget = candidate.kind.row_nudge_budget();
        let mut target_y = None;
        let mut row_candidates = vec![candidate.y as i32];
        for offset in 1..=budget {
            row_candidates.push(candidate.y as i32 + offset);
            row_candidates.push(candidate.y as i32 - offset);
        }

        for row in row_candidates {
            if row < top || row >= bottom {
                continue;
            }
            let row = row as u16;
            let overlaps = occupied_rows
                .get(&row)
                .map(|ranges| {
                    ranges
                        .iter()
                        .any(|(left, right)| padded_start < *right && padded_end > *left)
                })
                .unwrap_or(false);
            if !overlaps {
                target_y = Some(row);
                break;
            }
        }

        let Some(target_y) = target_y else {
            continue;
        };

        occupied_rows
            .entry(target_y)
            .or_default()
            .push((padded_start, padded_end));
        projected.push(MermaidProjectedLine {
            source_index: candidate.source_index,
            x: candidate.x,
            y: target_y,
            text: candidate.text,
            color: candidate.color,
        });
    }

    projected.sort_by_key(|line| (line.y, line.x));
    projected
}

pub(crate) fn mermaid_visible_focus_targets(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<Vec<MermaidFocusTarget>, String> {
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

    let mut grouped = HashMap::<String, MermaidFocusAccumulator>::new();
    for line in &viewer.cached_semantic_lines {
        let Some(source) = prepared.semantic_lines.get(line.source_index) else {
            continue;
        };
        if !mermaid_kind_is_owner_summary(source.kind) && !mermaid_kind_is_owner_detail(source.kind)
        {
            continue;
        }

        let line_width = display_width(&line.text).max(1);
        let line_right = line.x.saturating_add(line_width.saturating_sub(1));
        let priority = match source.kind {
            MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => 0,
            MermaidSemanticKind::SubgraphTitle | MermaidSemanticKind::NodeTitle => 1,
            MermaidSemanticKind::ClassMember => 2,
            MermaidSemanticKind::ErAttributeName => 3,
            MermaidSemanticKind::ErAttributeType => 4,
            MermaidSemanticKind::EdgeLabel => 5,
        };
        grouped
            .entry(source.owner_key.clone())
            .and_modify(|target| {
                target.left = target.left.min(line.x);
                target.right = target.right.max(line_right);
                target.top = target.top.min(line.y);
                target.bottom = target.bottom.max(line.y);

                if priority < target.priority
                    || (priority == target.priority
                        && (line.y, line.x, line.source_index)
                            < (target.sort_y, target.sort_x, target.source_index))
                {
                    target.source_index = line.source_index;
                    target.text = source.text.clone();
                    target.diagram_x = source.diagram_x;
                    target.diagram_y = source.diagram_y;
                    target.priority = priority;
                    target.sort_y = line.y;
                    target.sort_x = line.x;
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

pub(crate) fn pixel_is_dark(pixmap: &Pixmap, x: u32, y: u32) -> bool {
    let width = pixmap.width();
    let height = pixmap.height();
    if x >= width || y >= height {
        return false;
    }
    let idx = ((y * width + x) * 4) as usize;
    let data = pixmap.data();
    if idx + 3 >= data.len() {
        return false;
    }
    let b = data[idx] as f32;
    let g = data[idx + 1] as f32;
    let r = data[idx + 2] as f32;
    let a = data[idx + 3] as f32 / 255.0;
    if a <= 0.1 {
        return false;
    }
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    luminance < 230.0
}

pub(crate) fn mermaid_ascii_cell(pixmap: &Pixmap, cell_x: u16, cell_y: u16) -> char {
    let base_x = u32::from(cell_x) * 2;
    let base_y = u32::from(cell_y) * 4;
    let mut grid = [[false; 2]; 4];
    let mut row_counts = [0u8; 4];
    let mut col_counts = [0u8; 2];
    let mut total = 0u8;

    for sub_y in 0..4 {
        for sub_x in 0..2 {
            let dark = pixel_is_dark(pixmap, base_x + sub_x as u32, base_y + sub_y as u32);
            grid[sub_y][sub_x] = dark;
            if dark {
                row_counts[sub_y] += 1;
                col_counts[sub_x] += 1;
                total += 1;
            }
        }
    }

    if total == 0 {
        return ' ';
    }

    let horizontal = row_counts.into_iter().any(|count| count == 2);
    let vertical = col_counts[0] >= 3 && col_counts[1] >= 3;
    let right_arrow =
        col_counts[1] >= 3 && col_counts[0] <= 1 && horizontal && (grid[1][1] || grid[2][1]);
    let left_arrow =
        col_counts[0] >= 3 && col_counts[1] <= 1 && horizontal && (grid[1][0] || grid[2][0]);
    let diagonal = ((grid[0][0] || grid[1][0]) && (grid[2][1] || grid[3][1]))
        || ((grid[0][1] || grid[1][1]) && (grid[2][0] || grid[3][0]));

    if right_arrow {
        '>'
    } else if left_arrow {
        '<'
    } else if vertical {
        '|'
    } else if horizontal {
        '_'
    } else if diagonal {
        '\\'
    } else if col_counts[0] >= 2 || col_counts[1] >= 2 {
        '|'
    } else if total >= 2 {
        '_'
    } else {
        ' '
    }
}

pub(crate) fn pixmap_to_ascii_lines(pixmap: &Pixmap, content_rect: Rect) -> Vec<String> {
    let mut lines = Vec::new();
    for cell_y in 0..content_rect.height {
        let mut line = String::with_capacity(content_rect.width as usize);
        for cell_x in 0..content_rect.width {
            line.push(mermaid_ascii_cell(pixmap, cell_x, cell_y));
        }
        lines.push(line);
    }
    lines
}

pub(crate) fn ensure_mermaid_prepared_render(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let source = viewer
        .source
        .as_deref()
        .ok_or_else(|| "Mermaid source unavailable".to_string())?;
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

pub(crate) fn render_mermaid_detail_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
    transform: MermaidViewportTransform,
    view_state: MermaidViewState,
) -> Result<bool, String> {
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };

    let projected = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        view_state,
    );
    if projected.is_empty() {
        return Ok(false);
    }

    let owner_colors = mermaid_owner_accent_map(&prepared.semantic_lines);
    let (projected, label_rects) =
        if matches!(view_state, MermaidViewState::L2 | MermaidViewState::L3) {
            let owners = mermaid_build_packed_detail_owners(&prepared.semantic_lines, &projected);
            if owners.is_empty() {
                return Ok(false);
            }
            let label_rects = mermaid_pack_detail_box_rects(content_rect, &owners);
            if label_rects.is_empty() {
                return Ok(false);
            }
            (
                mermaid_project_packed_detail_lines(&owners, &label_rects),
                label_rects,
            )
        } else {
            let label_rects =
                mermaid_detail_box_rects(&prepared.semantic_lines, &projected, content_rect);
            (projected, label_rects)
        };
    if label_rects.is_empty() {
        return Ok(false);
    }

    let visible_keys = label_rects.keys().cloned().collect::<HashSet<_>>();
    let outline_edges = mermaid_outline_edge_map(&prepared.layout)
        .into_values()
        .filter(|edge| visible_keys.contains(&edge.from_key) && visible_keys.contains(&edge.to_key))
        .collect::<Vec<_>>();

    viewer.cached_lines =
        mermaid_render_compact_detail_background(content_rect, &label_rects, outline_edges);
    viewer.cached_background_cells =
        mermaid_background_cells_from_lines(&viewer.cached_lines, MERMAID_CONNECTOR_COLOR);
    mermaid_apply_rect_border_colors(
        &mut viewer.cached_background_cells,
        content_rect,
        &label_rects,
        &owner_colors,
    );
    viewer.cached_semantic_lines = projected;
    Ok(true)
}

pub(crate) fn render_mermaid_lines(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let (sample_width, sample_height, transform) =
        mermaid_viewport_transform(viewer, content_rect)?;
    let view_state = mermaid_view_state_for_view(viewer, content_rect);
    if view_state.is_er() && render_mermaid_er_packed_lines(viewer, content_rect, view_state)? {
        viewer.cached_rect = Some(content_rect);
        viewer.cached_zoom = viewer.zoom;
        viewer.cached_center_x = viewer.center_x;
        viewer.cached_center_y = viewer.center_y;
        viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
        return Ok(());
    }

    if view_state == MermaidViewState::Outline
        && render_mermaid_outline_lines(viewer, content_rect, transform)?
    {
        viewer.cached_rect = Some(content_rect);
        viewer.cached_zoom = viewer.zoom;
        viewer.cached_center_x = viewer.center_x;
        viewer.cached_center_y = viewer.center_y;
        viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
        return Ok(());
    }

    if matches!(
        view_state,
        MermaidViewState::L1 | MermaidViewState::L2 | MermaidViewState::L3
    ) && render_mermaid_detail_lines(viewer, content_rect, transform, view_state)?
    {
        viewer.cached_rect = Some(content_rect);
        viewer.cached_zoom = viewer.zoom;
        viewer.cached_center_x = viewer.center_x;
        viewer.cached_center_y = viewer.center_y;
        viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
        return Ok(());
    }

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

    viewer.cached_lines = pixmap_to_ascii_lines(&pixmap, content_rect);
    viewer.cached_background_cells =
        mermaid_background_cells_from_lines(&viewer.cached_lines, MERMAID_CONNECTOR_COLOR);
    viewer.cached_semantic_lines = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        view_state,
    );
    viewer.cached_rect = Some(content_rect);
    viewer.cached_zoom = viewer.zoom;
    viewer.cached_center_x = viewer.center_x;
    viewer.cached_center_y = viewer.center_y;
    viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
    Ok(())
}

pub(crate) fn render_wrapped_lines(renderer: &mut Renderer, rect: Rect, text: &str, color: Color) {
    let mut y = rect.y;
    for line in wrap_text(text, rect.width as usize) {
        if y >= rect.bottom() {
            break;
        }
        renderer.draw_text(
            rect.x,
            y,
            &truncate_label(&line, rect.width as usize),
            color,
        );
        y += 1;
    }
}

pub(crate) fn render_mermaid_viewer(
    renderer: &mut Renderer,
    field: Rect,
    viewer: &mut MermaidViewerState,
) {
    renderer.fill_rect(field, ' ', Color::Reset);
    viewer.back_rect = Some(Rect {
        x: field.x,
        y: field.y,
        width: display_width(MERMAID_BACK_LABEL),
        height: 1,
    });
    renderer.draw_text(field.x, field.y, MERMAID_BACK_LABEL, Color::Cyan);

    let content_rect = mermaid_content_rect(field);
    viewer.content_rect = Some(content_rect);
    let view_state = mermaid_view_state_for_view(viewer, content_rect);
    let status_x = field
        .x
        .saturating_add(display_width(MERMAID_BACK_LABEL) + 1);
    let status_width = field.right().saturating_sub(status_x) as usize;
    let detail_label = mermaid_status_detail_label(view_state);
    let zoom_label = mermaid_zoom_status_label(viewer.zoom);
    let focus_width = viewer
        .focus_status
        .as_deref()
        .map(|status| usize::from(display_width(" | ")) + usize::from(display_width(status)))
        .unwrap_or(0);
    let fixed_width = usize::from(display_width(&viewer.tmux_name))
        + usize::from(display_width(" | "))
        + usize::from(display_width(&detail_label))
        + usize::from(display_width(" | "))
        + usize::from(display_width(" | "))
        + usize::from(display_width(&zoom_label))
        + usize::from(display_width(" | o open"))
        + focus_width;
    let mut status = format!(
        "{} | {} | {} | {} | o open",
        viewer.tmux_name,
        detail_label,
        shorten_path(
            viewer.display_path(),
            status_width.saturating_sub(fixed_width)
        ),
        zoom_label,
    );
    if let Some(focus_status) = viewer.focus_status.as_deref() {
        status.push_str(" | ");
        status.push_str(focus_status);
    }
    renderer.draw_text(
        status_x,
        field.y,
        &truncate_label(&status, status_width),
        Color::DarkGrey,
    );

    if content_rect.width < MERMAID_VIEW_MIN_WIDTH || content_rect.height < MERMAID_VIEW_MIN_HEIGHT
    {
        render_wrapped_lines(
            renderer,
            content_rect,
            "Mermaid view too small",
            Color::DarkGrey,
        );
        return;
    }

    if let Some(reason) = viewer.unsupported_reason.as_deref() {
        render_wrapped_lines(renderer, content_rect, reason, Color::DarkGrey);
        return;
    }
    if let Some(error) = viewer.artifact_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
        return;
    }

    viewer.render_error = ensure_mermaid_viewport_cache(viewer, content_rect).err();

    if let Some(error) = viewer.render_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
        return;
    }

    if viewer.cached_background_cells.len() == viewer.cached_lines.len() {
        for (row_offset, row) in viewer.cached_background_cells.iter().enumerate() {
            let y = content_rect.y + row_offset as u16;
            if y >= content_rect.bottom() {
                break;
            }
            for (column_offset, cell) in row.iter().enumerate() {
                if cell.ch == ' ' {
                    continue;
                }
                renderer.draw_char(content_rect.x + column_offset as u16, y, cell.ch, cell.fg);
            }
        }
    } else {
        for (offset, line) in viewer.cached_lines.iter().enumerate() {
            let y = content_rect.y + offset as u16;
            if y >= content_rect.bottom() {
                break;
            }
            renderer.draw_text(content_rect.x, y, line, MERMAID_CONNECTOR_COLOR);
        }
    }
    for line in &viewer.cached_semantic_lines {
        let color = if Some(line.source_index) == viewer.focused_source_index {
            MERMAID_FOCUS_COLOR
        } else {
            line.color
        };
        renderer.draw_text(line.x, line.y, &line.text, color);
    }
}
