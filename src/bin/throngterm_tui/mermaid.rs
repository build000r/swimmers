use super::*;

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

impl MermaidDetailLevel {
    pub(crate) fn label(self) -> &'static str {
        match self {
            MermaidDetailLevel::L1 => "L1",
            MermaidDetailLevel::L2 => "L2",
            MermaidDetailLevel::L3 => "L3",
        }
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
    pub(crate) owner_width: f32,
    pub(crate) owner_height: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct MermaidProjectedLine {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) text: String,
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
    pub(crate) cached_semantic_lines: Vec<MermaidProjectedLine>,
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
        self.cached_semantic_lines.clear();
        self.render_error = None;
        self.invalidate_viewport_cache();
    }
}

pub(crate) trait ArtifactOpener: Send + Sync {
    fn open(&self, path: &str) -> io::Result<()>;
}

#[derive(Default)]
pub(crate) struct SystemArtifactOpener;

impl ArtifactOpener for SystemArtifactOpener {
    fn open(&self, path: &str) -> io::Result<()> {
        if cfg!(target_os = "macos") {
            ProcessCommand::new("open").arg(path).spawn().map(|_| ())
        } else if cfg!(target_os = "windows") {
            ProcessCommand::new("cmd")
                .args(["/C", "start", "", path])
                .spawn()
                .map(|_| ())
        } else {
            ProcessCommand::new("xdg-open")
                .arg(path)
                .spawn()
                .map(|_| ())
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

pub(crate) fn mermaid_fontdb() -> Arc<usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTDB
        .get_or_init(|| {
            let mut fontdb = usvg::fontdb::Database::new();
            fontdb.load_system_fonts();
            Arc::new(fontdb)
        })
        .clone()
}

pub(crate) fn mermaid_usvg_options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: mermaid_fontdb(),
        ..usvg::Options::default()
    }
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
        "a"
            | "an"
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
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'))
        .to_string()
}

pub(crate) fn mermaid_compact_overview_text<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let raw_tokens = lines
        .into_iter()
        .flat_map(|line| line.split_whitespace())
        .filter(|token| !token.trim().is_empty())
        .collect::<Vec<_>>();
    if raw_tokens.is_empty() {
        return None;
    }

    let mut prefix = None;
    let mut start_idx = 0usize;
    if mermaid_is_numeric_prefix(raw_tokens[0]) {
        prefix = Some(raw_tokens[0].trim().to_string());
        start_idx = 1;
    }

    let cleaned_tokens = raw_tokens[start_idx..]
        .iter()
        .map(|token| mermaid_clean_summary_token(token))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
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
            if added == 0 {
                let budget = max_chars.saturating_sub(out.chars().count() + separator);
                if budget > 0 {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(&truncate_label(&token, budget));
                }
            }
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
        max_type_chars = max_type_chars.max(data_type.chars().count());
        parsed_attrs.push((*idx, data_type.to_string(), name.to_string()));
    }

    let gap_chars = 2usize;
    let name_x = left_x + ((max_type_chars + gap_chars) as f32 * theme_font_size * 0.6);
    let content_width = (node.width - node_padding_x.max(10.0) * 2.0).max(0.0);
    if use_columns && parsed_attrs.len() == attr_lines.len() && name_x < node.x + content_width {
        for (idx, data_type, name) in parsed_attrs {
            let diagram_y = start_y + idx as f32 * class_line_height;
            target.push(MermaidSemanticLine {
                text: data_type,
                diagram_x: left_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeType,
                owner_width: node.width,
                owner_height: node.height,
            });
            target.push(MermaidSemanticLine {
                text: name,
                diagram_x: name_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeName,
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
    let mut semantic_lines = Vec::new();

    for subgraph in &layout.subgraphs {
        if subgraph.label.trim().is_empty() {
            continue;
        }
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
                subgraph.width,
                subgraph.height,
            );
        }
    }

    for edge in &layout.edges {
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
            extend_mermaid_er_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
            );
            continue;
        }

        if node
            .label
            .lines
            .iter()
            .any(|line| mermaid_is_divider_line(line))
        {
            extend_mermaid_class_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
            );
            continue;
        }

        let center_x = node.x + node.width / 2.0;
        let center_y = node.y + node.height / 2.0;
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

pub(crate) fn mermaid_detail_level_for_view(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> MermaidDetailLevel {
    let _ = content_rect;
    let effective_zoom = viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM);

    if effective_zoom >= 2.4 {
        MermaidDetailLevel::L3
    } else if effective_zoom >= 1.4 {
        MermaidDetailLevel::L2
    } else {
        MermaidDetailLevel::L1
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
    detail_level: MermaidDetailLevel,
) -> String {
    if detail_level != MermaidDetailLevel::L1 {
        return line.text.clone();
    }

    match line.kind {
        MermaidSemanticKind::SubgraphSummary | MermaidSemanticKind::NodeSummary => {
            let budget = owner_cols.floor().max(8.0).min(18.0) as usize;
            truncate_label(&line.text, budget)
        }
        _ => line.text.clone(),
    }
}

pub(crate) fn project_mermaid_semantic_lines(
    lines: &[MermaidSemanticLine],
    transform: MermaidViewportTransform,
    content_rect: Rect,
    detail_level: MermaidDetailLevel,
) -> Vec<MermaidProjectedLine> {
    #[derive(Clone)]
    struct MermaidProjectedCandidate {
        priority: u8,
        area_rank: i32,
        kind: MermaidSemanticKind,
        x: u16,
        y: u16,
        text: String,
    }

    let mut candidates = Vec::new();
    let left = content_rect.x as i32;
    let right = content_rect.right() as i32;
    let top = content_rect.y as i32;
    let bottom = content_rect.bottom() as i32;

    for line in lines {
        if line.kind.min_detail_level() > detail_level {
            continue;
        }

        let owner_cols = (line.owner_width * transform.scale / 2.0).max(0.0);
        let owner_rows = (line.owner_height * transform.scale / 4.0).max(0.0);
        if !line.kind.is_visible_for_owner(owner_cols, owner_rows) {
            continue;
        }
        let display_text = mermaid_display_text_for_view(line, owner_cols, detail_level);
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
            x: screen_x as u16,
            y: screen_y as u16,
            text: clipped,
        });
    }

    candidates.sort_by_key(|line| (line.priority, line.area_rank, line.y, line.x));

    let mut occupied_rows: HashMap<u16, Vec<(u16, u16)>> = HashMap::new();
    let mut projected = Vec::new();
    let collision_padding = match detail_level {
        MermaidDetailLevel::L1 => 2,
        MermaidDetailLevel::L2 => 1,
        MermaidDetailLevel::L3 => 0,
    };
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
            x: candidate.x,
            y: target_y,
            text: candidate.text,
        });
    }

    projected.sort_by_key(|line| (line.y, line.x));
    projected
}

pub(crate) fn braille_bit(sub_x: u32, sub_y: u32) -> u32 {
    match (sub_x, sub_y) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0,
    }
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

pub(crate) fn pixmap_to_braille_lines(pixmap: &Pixmap, content_rect: Rect) -> Vec<String> {
    let mut lines = Vec::new();
    for cell_y in 0..content_rect.height {
        let mut line = String::with_capacity(content_rect.width as usize);
        for cell_x in 0..content_rect.width {
            let mut bits = 0u32;
            let base_x = u32::from(cell_x) * 2;
            let base_y = u32::from(cell_y) * 4;
            for sub_y in 0..4 {
                for sub_x in 0..2 {
                    if pixel_is_dark(pixmap, base_x + sub_x, base_y + sub_y) {
                        bits |= braille_bit(sub_x, sub_y);
                    }
                }
            }
            if bits == 0 {
                line.push(' ');
            } else {
                line.push(char::from_u32(0x2800 + bits).unwrap_or(' '));
            }
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
        let tree = Tree::from_str(&svg, &mermaid_usvg_options())
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

pub(crate) fn render_mermaid_lines(viewer: &mut MermaidViewerState, content_rect: Rect) -> Result<(), String> {
    let (sample_width, sample_height, transform) =
        mermaid_viewport_transform(viewer, content_rect)?;
    let detail_level = mermaid_detail_level_for_view(viewer, content_rect);

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

    viewer.cached_lines = pixmap_to_braille_lines(&pixmap, content_rect);
    viewer.cached_semantic_lines = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        detail_level,
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

pub(crate) fn render_mermaid_viewer(renderer: &mut Renderer, field: Rect, viewer: &mut MermaidViewerState) {
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
    let detail_level = mermaid_detail_level_for_view(viewer, content_rect);
    let status_x = field
        .x
        .saturating_add(display_width(MERMAID_BACK_LABEL) + 1);
    let status_width = field.right().saturating_sub(status_x) as usize;
    let detail_label = format!("detail {}", detail_level.label());
    let fixed_width = usize::from(display_width(&viewer.tmux_name))
        + usize::from(display_width(" | "))
        + usize::from(display_width(&detail_label))
        + usize::from(display_width(" | "))
        + usize::from(display_width(" | zoom 100% | "))
        + usize::from(display_width(" | o open"));
    let status = format!(
        "{} | {} | {} | zoom {:>3.0}% | o open",
        viewer.tmux_name,
        detail_label,
        shorten_path(
            viewer.display_path(),
            status_width.saturating_sub(fixed_width)
        ),
        viewer.zoom * 100.0,
    );
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

    let needs_rerender = viewer.cached_rect != Some(content_rect)
        || viewer.prepared_render.is_none()
        || (viewer.cached_zoom - viewer.zoom).abs() > f32::EPSILON
        || (viewer.cached_center_x - viewer.center_x).abs() > f32::EPSILON
        || (viewer.cached_center_y - viewer.center_y).abs() > f32::EPSILON;
    if needs_rerender {
        viewer.render_error = render_mermaid_lines(viewer, content_rect).err();
    }

    if let Some(error) = viewer.render_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
        return;
    }

    for (offset, line) in viewer.cached_lines.iter().enumerate() {
        let y = content_rect.y + offset as u16;
        if y >= content_rect.bottom() {
            break;
        }
        renderer.draw_text(content_rect.x, y, line, Color::White);
    }
    for line in &viewer.cached_semantic_lines {
        renderer.draw_text(line.x, line.y, &line.text, Color::White);
    }
}
