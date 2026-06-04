use super::*;

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

pub(crate) fn mermaid_summary_subtokens(token: &str) -> Vec<String> {
    let trimmed = token.trim();
    if mermaid_is_numeric_prefix(trimmed) {
        return vec![trimmed.to_string()];
    }

    mermaid_clean_summary_token(token)
        .split(['_', '-', '/'])
        .filter_map(|part| {
            let part = part.trim();
            (!part.is_empty()).then(|| part.to_string())
        })
        .collect()
}

pub(crate) fn mermaid_compact_overview_text<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let raw_tokens = mermaid_overview_raw_tokens(lines);
    if raw_tokens.is_empty() {
        return None;
    }

    let (prefix, cleaned_tokens) = mermaid_overview_prefix_and_tokens(&raw_tokens);
    if cleaned_tokens.is_empty() {
        return prefix;
    }

    let source_tokens = mermaid_overview_source_tokens(cleaned_tokens);
    mermaid_overview_limited_text(prefix, source_tokens)
}

fn mermaid_overview_raw_tokens<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    lines
        .into_iter()
        .flat_map(|line| line.split_whitespace())
        .flat_map(mermaid_summary_subtokens)
        .collect()
}

fn mermaid_overview_prefix_and_tokens(raw_tokens: &[String]) -> (Option<String>, Vec<String>) {
    if mermaid_is_numeric_prefix(&raw_tokens[0]) {
        (
            Some(raw_tokens[0].trim().to_string()),
            raw_tokens[1..].to_vec(),
        )
    } else {
        (None, raw_tokens.to_vec())
    }
}

fn mermaid_overview_source_tokens(cleaned_tokens: Vec<String>) -> Vec<String> {
    let significant_tokens = cleaned_tokens
        .iter()
        .filter(|token| !mermaid_summary_stopword(&token.to_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>();
    if significant_tokens.is_empty() {
        cleaned_tokens
    } else {
        significant_tokens
    }
}

const MERMAID_OVERVIEW_MAX_CHARS: usize = 20;
const MERMAID_OVERVIEW_WORD_LIMIT: usize = 3;

fn mermaid_overview_limited_text(
    prefix: Option<String>,
    source_tokens: Vec<String>,
) -> Option<String> {
    let has_prefix = prefix.is_some();
    let mut out = prefix.unwrap_or_default();
    let _ = source_tokens
        .into_iter()
        .take(MERMAID_OVERVIEW_WORD_LIMIT - usize::from(has_prefix))
        .try_for_each(|token| mermaid_try_push_overview_token(&mut out, &token));
    (!out.is_empty()).then_some(out)
}

fn mermaid_try_push_overview_token(out: &mut String, token: &str) -> std::ops::ControlFlow<()> {
    let separator = usize::from(!out.is_empty());
    let next_len = out.chars().count() + separator + token.chars().count();
    if next_len > MERMAID_OVERVIEW_MAX_CHARS {
        return std::ops::ControlFlow::Break(());
    }
    if separator > 0 {
        out.push(' ');
    }
    out.push_str(token);
    std::ops::ControlFlow::Continue(())
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

    let title_lines = mermaid_label_title_lines(node, divider_idx);
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

    let attr_lines = mermaid_label_detail_lines(node, divider_idx);
    if attr_lines.is_empty() {
        return;
    }

    if let Some((columns, layout)) = mermaid_er_attribute_columns(&attr_lines).and_then(|columns| {
        mermaid_er_attribute_column_layout(&columns, node, node_padding_x, left_x)
            .map(|layout| (columns, layout))
    }) {
        push_mermaid_er_attribute_column_lines(
            target,
            columns,
            layout,
            start_y,
            class_line_height,
            owner_key,
            node,
        );
        return;
    }

    push_mermaid_er_attribute_name_lines(
        target,
        &attr_lines,
        left_x,
        start_y,
        class_line_height,
        owner_key,
        node,
    );
}

struct MermaidErAttributeColumn {
    source_idx: usize,
    data_type: String,
    name_text: String,
}

struct MermaidErAttributeColumns {
    attrs: Vec<MermaidErAttributeColumn>,
    max_type_chars: usize,
}

#[derive(Clone, Copy)]
struct MermaidErAttributeColumnLayout {
    left_x: f32,
    name_x: f32,
    max_type_chars: usize,
    column_char_width: f32,
}

fn mermaid_label_title_lines(
    node: &mermaid_rs_renderer::NodeLayout,
    divider_idx: usize,
) -> Vec<(usize, &str)> {
    node.label
        .lines
        .iter()
        .enumerate()
        .take(divider_idx)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty()).then_some((idx, line.as_str()))
        })
        .collect()
}

fn mermaid_label_detail_lines(
    node: &mermaid_rs_renderer::NodeLayout,
    divider_idx: usize,
) -> Vec<(usize, &str)> {
    node.label
        .lines
        .iter()
        .enumerate()
        .skip(divider_idx + 1)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty() && !mermaid_is_divider_line(text)).then_some((idx, line.as_str()))
        })
        .collect()
}

fn mermaid_er_attribute_columns(attr_lines: &[(usize, &str)]) -> Option<MermaidErAttributeColumns> {
    let mut attrs = Vec::new();
    let mut max_type_chars = 0usize;
    for (idx, line) in attr_lines {
        let mut parts = line.trim().split_whitespace();
        let data_type = parts.next()?;
        let name = parts.next()?;
        let suffix = parts.collect::<Vec<_>>().join(" ");
        let mut name_text = name.to_string();
        if !suffix.is_empty() {
            name_text.push(' ');
            name_text.push_str(&suffix);
        }
        max_type_chars = max_type_chars.max(data_type.chars().count());
        attrs.push(MermaidErAttributeColumn {
            source_idx: *idx,
            data_type: data_type.to_string(),
            name_text,
        });
    }
    Some(MermaidErAttributeColumns {
        attrs,
        max_type_chars,
    })
}

fn mermaid_er_attribute_column_layout(
    columns: &MermaidErAttributeColumns,
    node: &mermaid_rs_renderer::NodeLayout,
    node_padding_x: f32,
    left_x: f32,
) -> Option<MermaidErAttributeColumnLayout> {
    let gap_chars = 3usize;
    let column_char_width = 2.0f32;
    let name_x = left_x + ((columns.max_type_chars + gap_chars) as f32 * column_char_width);
    let content_width = (node.width - node_padding_x.max(10.0) * 2.0).max(0.0);
    (name_x < node.x + content_width).then_some(MermaidErAttributeColumnLayout {
        left_x,
        name_x,
        max_type_chars: columns.max_type_chars,
        column_char_width,
    })
}

fn push_mermaid_er_attribute_column_lines(
    target: &mut Vec<MermaidSemanticLine>,
    columns: MermaidErAttributeColumns,
    layout: MermaidErAttributeColumnLayout,
    start_y: f32,
    class_line_height: f32,
    owner_key: &str,
    node: &mermaid_rs_renderer::NodeLayout,
) {
    for attr in columns.attrs {
        let diagram_y = start_y + attr.source_idx as f32 * class_line_height;
        let type_x = layout.left_x
            + ((layout
                .max_type_chars
                .saturating_sub(attr.data_type.chars().count())) as f32
                * layout.column_char_width);
        target.push(MermaidSemanticLine {
            text: attr.data_type,
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
            text: attr.name_text,
            diagram_x: layout.name_x,
            diagram_y,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::ErAttributeName,
            owner_key: owner_key.to_string(),
            outline_eligible: false,
            owner_width: node.width,
            owner_height: node.height,
        });
    }
}

fn push_mermaid_er_attribute_name_lines(
    target: &mut Vec<MermaidSemanticLine>,
    attr_lines: &[(usize, &str)],
    left_x: f32,
    start_y: f32,
    class_line_height: f32,
    owner_key: &str,
    node: &mermaid_rs_renderer::NodeLayout,
) {
    push_mermaid_indexed_semantic_lines(
        target,
        attr_lines,
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

struct SemanticLineMetrics {
    theme_font_size: f32,
    base_line_height: f32,
    class_line_height: f32,
    state_font_size: f32,
    state_line_height: f32,
    node_padding_x: f32,
    is_state: bool,
}

impl SemanticLineMetrics {
    fn from(layout: &MermaidLayout, options: &RenderOptions) -> Self {
        let theme_font_size = options.theme.font_size;
        let is_state = layout.kind == DiagramKind::State;
        let state_font_size = if is_state {
            theme_font_size * 0.85
        } else {
            theme_font_size
        };
        Self {
            theme_font_size,
            base_line_height: theme_font_size * options.layout.label_line_height,
            class_line_height: theme_font_size * options.layout.class_label_line_height(),
            state_font_size,
            state_line_height: state_font_size * options.layout.label_line_height,
            node_padding_x: options.layout.node_padding_x,
            is_state,
        }
    }

    fn node_label_metrics(&self) -> (f32, f32) {
        if self.is_state {
            (self.state_font_size, self.state_line_height)
        } else {
            (self.theme_font_size, self.base_line_height)
        }
    }
}

pub(crate) fn build_mermaid_semantic_lines(
    layout: &MermaidLayout,
    options: &RenderOptions,
) -> Vec<MermaidSemanticLine> {
    if !mermaid_kind_supports_semantic_overlay(layout.kind) {
        return Vec::new();
    }

    let metrics = SemanticLineMetrics::from(layout, options);
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
        push_subgraph_semantic_lines(
            &mut semantic_lines,
            subgraph,
            subgraph_idx,
            top_level_subgraphs.contains(&subgraph_idx),
            &metrics,
        );
    }

    for (edge_idx, edge) in layout.edges.iter().enumerate() {
        push_edge_semantic_lines(&mut semantic_lines, edge, edge_idx, &metrics);
    }

    for node in layout.nodes.values() {
        push_node_semantic_lines(
            &mut semantic_lines,
            node,
            layout,
            &subgraph_node_ids,
            &metrics,
        );
    }

    semantic_lines
}

fn push_subgraph_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    subgraph: &mermaid_rs_renderer::layout::SubgraphLayout,
    subgraph_idx: usize,
    outline_eligible: bool,
    metrics: &SemanticLineMetrics,
) {
    let owner_key = mermaid_outline_subgraph_key(subgraph_idx);
    let (label_x, label_y, anchor, font_size, line_height) = if metrics.is_state {
        let header_height = (subgraph.label_block.height + metrics.theme_font_size * 0.75)
            .max(metrics.theme_font_size * 1.4);
        let label_x =
            subgraph.x + (metrics.theme_font_size * 0.6).max(subgraph.label_block.height * 0.35);
        let label_y = subgraph.y + header_height / 2.0;
        (
            label_x,
            label_y,
            MermaidTextAnchor::Start,
            metrics.state_font_size,
            metrics.state_line_height,
        )
    } else {
        let label_x = subgraph.x + subgraph.width / 2.0;
        let label_y = subgraph.y + 12.0 + subgraph.label_block.height / 2.0;
        (
            label_x,
            label_y,
            MermaidTextAnchor::Center,
            metrics.theme_font_size,
            metrics.base_line_height,
        )
    };

    push_mermaid_summary_line(
        semantic_lines,
        subgraph.label_block.lines.iter().map(String::as_str),
        label_x,
        label_y,
        anchor,
        MermaidSemanticKind::SubgraphSummary,
        &owner_key,
        outline_eligible,
        subgraph.width,
        subgraph.height,
    );
    push_mermaid_text_block_semantic_lines(
        semantic_lines,
        &subgraph.label_block,
        label_x,
        label_y,
        font_size,
        line_height,
        anchor,
        MermaidSemanticKind::SubgraphTitle,
        &owner_key,
        false,
        subgraph.width,
        subgraph.height,
    );
}

fn push_edge_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    edge: &mermaid_rs_renderer::layout::EdgeLayout,
    edge_idx: usize,
    metrics: &SemanticLineMetrics,
) {
    let owner_key = mermaid_outline_edge_key(edge_idx);
    for (label, anchor) in [
        (edge.label.as_ref(), edge.label_anchor),
        (edge.start_label.as_ref(), edge.start_label_anchor),
        (edge.end_label.as_ref(), edge.end_label_anchor),
    ] {
        push_edge_label_block(
            semantic_lines,
            label,
            anchor,
            metrics.is_state,
            metrics.state_font_size,
            metrics.state_line_height,
            metrics.theme_font_size,
            metrics.base_line_height,
            &owner_key,
        );
    }
}

fn push_node_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    layout: &MermaidLayout,
    subgraph_node_ids: &HashSet<String>,
    metrics: &SemanticLineMetrics,
) {
    let Some(context) = mermaid_node_semantic_context(node, layout, subgraph_node_ids) else {
        return;
    };
    push_visible_node_semantic_lines(semantic_lines, node, layout.kind, metrics, context);
}

struct MermaidNodeSemanticContext {
    owner_key: String,
    outline_eligible: bool,
    structured_label: bool,
}

fn mermaid_node_semantic_context(
    node: &mermaid_rs_renderer::NodeLayout,
    layout: &MermaidLayout,
    subgraph_node_ids: &HashSet<String>,
) -> Option<MermaidNodeSemanticContext> {
    if mermaid_node_semantic_lines_hidden(node) {
        return None;
    }
    Some(MermaidNodeSemanticContext {
        owner_key: mermaid_outline_node_key(&node.id),
        outline_eligible: mermaid_node_outline_eligible(node, layout, subgraph_node_ids),
        structured_label: mermaid_node_has_structured_label(node),
    })
}

fn push_visible_node_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    diagram_kind: DiagramKind,
    metrics: &SemanticLineMetrics,
    context: MermaidNodeSemanticContext,
) {
    if context.structured_label {
        push_structured_node_semantic_lines(
            semantic_lines,
            node,
            diagram_kind,
            metrics.theme_font_size,
            metrics.class_line_height,
            metrics.node_padding_x,
            &context.owner_key,
            context.outline_eligible,
        );
        return;
    }

    push_plain_node_semantic_lines(
        semantic_lines,
        node,
        metrics,
        &context.owner_key,
        context.outline_eligible,
    );
}

fn mermaid_node_label_hidden(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    mermaid_node_label_blank(node) || mermaid_node_is_terminal_marker(node)
}

fn mermaid_node_label_blank(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    node.label.lines.iter().all(|line| line.trim().is_empty())
}

fn mermaid_node_is_terminal_marker(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    node.id.starts_with("__start_") || node.id.starts_with("__end_")
}

fn mermaid_node_semantic_lines_hidden(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    mermaid_node_layout_hidden(node) || mermaid_node_label_hidden(node)
}

fn mermaid_node_layout_hidden(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    node.hidden || node.anchor_subgraph.is_some()
}

fn mermaid_node_outline_eligible(
    node: &mermaid_rs_renderer::NodeLayout,
    layout: &MermaidLayout,
    subgraph_node_ids: &HashSet<String>,
) -> bool {
    layout.subgraphs.is_empty() || !subgraph_node_ids.contains(&node.id)
}

fn mermaid_node_has_structured_label(node: &mermaid_rs_renderer::NodeLayout) -> bool {
    node.label
        .lines
        .iter()
        .any(|line| mermaid_is_divider_line(line))
}

fn push_structured_node_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    diagram_kind: DiagramKind,
    theme_font_size: f32,
    class_line_height: f32,
    node_padding_x: f32,
    owner_key: &str,
    outline_eligible: bool,
) {
    let extender = if diagram_kind == DiagramKind::Er {
        extend_mermaid_er_semantic_lines
    } else {
        extend_mermaid_class_semantic_lines
    };
    extender(
        semantic_lines,
        node,
        theme_font_size,
        class_line_height,
        node_padding_x,
        owner_key,
        outline_eligible,
    );
}

fn push_plain_node_semantic_lines(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    metrics: &SemanticLineMetrics,
    owner_key: &str,
    outline_eligible: bool,
) {
    let center_x = node.x + node.width / 2.0;
    let center_y = node.y + node.height / 2.0;
    let (font_size, line_height) = metrics.node_label_metrics();
    push_mermaid_summary_line(
        semantic_lines,
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
        semantic_lines,
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
            let Some(detail_level) = view_state.detail_level() else {
                tracing::warn!(
                    ?view_state,
                    "Mermaid detail state was missing a detail level; hiding line"
                );
                return false;
            };
            line.kind.min_detail_level() <= detail_level
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

#[derive(Clone, Copy)]
struct MermaidProjectionBounds {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

impl MermaidProjectionBounds {
    fn from_rect(rect: Rect) -> Self {
        Self {
            left: rect.x as i32,
            right: rect.right() as i32,
            top: rect.y as i32,
            bottom: rect.bottom() as i32,
        }
    }
}

/// Push a single optional edge label (main, start, or end) onto `semantic_lines`.
/// Handles the inner `if-let` guards and State vs. non-State font selection.
fn push_edge_label_block(
    semantic_lines: &mut Vec<MermaidSemanticLine>,
    label: Option<&mermaid_rs_renderer::layout::TextBlock>,
    anchor: Option<(f32, f32)>,
    is_state: bool,
    state_font_size: f32,
    state_line_height: f32,
    theme_font_size: f32,
    base_line_height: f32,
    owner_key: &str,
) {
    let Some(label) = label else { return };
    let Some((label_x, label_y)) = anchor else {
        return;
    };
    let (font_size, line_height) = if is_state {
        (state_font_size, state_line_height)
    } else {
        (theme_font_size, base_line_height)
    };
    push_mermaid_text_block_semantic_lines(
        semantic_lines,
        label,
        label_x,
        label_y,
        font_size,
        line_height,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::EdgeLabel,
        owner_key,
        false,
        0.0,
        0.0,
    );
}

/// Compact-row packing: group rows per owner so multi-line compact labels
/// occupy consecutive rows starting from the owner's first occupied row.
fn pack_compact_rows(candidates: &mut [MermaidProjectedCandidate], max_row: u16) {
    let mut compact_owner_rows = HashMap::<String, Vec<u16>>::new();
    for candidate in candidates.iter() {
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
    for candidate in candidates.iter_mut() {
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
}

/// In L2/L3 view, suppress owner-summary lines when the same owner has
/// detail lines visible (avoids redundant label stacking).
fn filter_owner_summaries_with_details(candidates: &mut Vec<MermaidProjectedCandidate>) {
    let owner_detail_keys = candidates
        .iter()
        .filter(|c| mermaid_kind_is_owner_detail(c.kind))
        .map(|c| c.owner_key.clone())
        .collect::<HashSet<_>>();
    candidates.retain(|c| {
        !mermaid_kind_is_owner_summary(c.kind) || !owner_detail_keys.contains(&c.owner_key)
    });
}

fn mermaid_candidate_allowed_for_state(
    line: &MermaidSemanticLine,
    view_state: MermaidViewState,
) -> bool {
    mermaid_line_visible_in_state(line, view_state)
        && !mermaid_compact_detail_hides_kind(view_state, line.kind)
}

fn mermaid_candidate_owner_grid(
    line: &MermaidSemanticLine,
    transform: MermaidViewportTransform,
) -> (f32, f32) {
    (
        (line.owner_width * transform.scale / 2.0).max(0.0),
        (line.owner_height * transform.scale / 4.0).max(0.0),
    )
}

fn mermaid_candidate_display_text(
    line: &MermaidSemanticLine,
    owner_cols: f32,
    view_state: MermaidViewState,
) -> Option<String> {
    let display_text = mermaid_display_text_for_view(line, owner_cols, view_state);
    (!display_text.trim().is_empty()).then_some(display_text)
}

fn mermaid_candidate_screen_y(
    line: &MermaidSemanticLine,
    transform: MermaidViewportTransform,
    bounds: MermaidProjectionBounds,
) -> Option<i32> {
    let projected_y = line.diagram_y * transform.scale + transform.ty;
    let screen_y = bounds.top + (projected_y / 4.0).floor() as i32;
    if screen_y >= bounds.top && screen_y < bounds.bottom {
        return Some(screen_y);
    }
    (line.kind.row_nudge_budget() > 0)
        .then(|| screen_y.clamp(bounds.top, bounds.bottom.saturating_sub(1)))
}

fn mermaid_candidate_anchor_x(
    line: &MermaidSemanticLine,
    transform: MermaidViewportTransform,
    bounds: MermaidProjectionBounds,
) -> i32 {
    let projected_x = line.diagram_x * transform.scale + transform.tx;
    bounds.left + (projected_x / 2.0).floor() as i32
}

fn mermaid_candidate_screen_x(
    line: &MermaidSemanticLine,
    display_text: &str,
    transform: MermaidViewportTransform,
    bounds: MermaidProjectionBounds,
) -> Option<(i32, usize, usize)> {
    let anchor_x = mermaid_candidate_anchor_x(line, transform, bounds);
    let text_width = mermaid_display_width_i32(display_text)?;
    let screen_x = mermaid_anchored_screen_x(line.anchor, anchor_x, text_width);
    mermaid_visible_screen_span(screen_x, text_width, bounds)
}

fn mermaid_display_width_i32(text: &str) -> Option<i32> {
    let width = display_width(text) as i32;
    (width > 0).then_some(width)
}

fn mermaid_anchored_screen_x(anchor: MermaidTextAnchor, anchor_x: i32, text_width: i32) -> i32 {
    match anchor {
        MermaidTextAnchor::Start => anchor_x,
        MermaidTextAnchor::Center => anchor_x - text_width / 2,
    }
}

fn mermaid_visible_screen_span(
    screen_x: i32,
    text_width: i32,
    bounds: MermaidProjectionBounds,
) -> Option<(i32, usize, usize)> {
    if mermaid_screen_span_outside_bounds(screen_x, text_width, bounds) {
        return None;
    }
    let visible_x = screen_x.max(bounds.left);
    let skipped_chars = (bounds.left - screen_x).max(0) as usize;
    let max_chars = bounds.right.saturating_sub(visible_x) as usize;
    Some((visible_x, skipped_chars, max_chars))
}

fn mermaid_screen_span_outside_bounds(
    screen_x: i32,
    text_width: i32,
    bounds: MermaidProjectionBounds,
) -> bool {
    screen_x >= bounds.right || screen_x + text_width <= bounds.left
}

fn mermaid_candidate_clipped_text(
    display_text: &str,
    skipped_chars: usize,
    max_chars: usize,
) -> Option<String> {
    let clipped = clip_mermaid_overlay_text(display_text, skipped_chars, max_chars);
    (!clipped.is_empty()).then_some(clipped)
}

fn mermaid_semantic_line_to_candidate(
    source_index: usize,
    line: &MermaidSemanticLine,
    transform: MermaidViewportTransform,
    bounds: MermaidProjectionBounds,
    view_state: MermaidViewState,
    owner_colors: &HashMap<String, Color>,
) -> Option<MermaidProjectedCandidate> {
    if !mermaid_candidate_allowed_for_state(line, view_state) {
        return None;
    }

    let (owner_cols, owner_rows) = mermaid_candidate_owner_grid(line, transform);
    if !line.kind.is_visible_for_owner(owner_cols, owner_rows) {
        return None;
    }

    let display_text = mermaid_candidate_display_text(line, owner_cols, view_state)?;
    let screen_y = mermaid_candidate_screen_y(line, transform, bounds)?;
    let (screen_x, skipped_chars, max_chars) =
        mermaid_candidate_screen_x(line, &display_text, transform, bounds)?;
    let clipped = mermaid_candidate_clipped_text(&display_text, skipped_chars, max_chars)?;

    Some(MermaidProjectedCandidate {
        priority: line.kind.priority(),
        area_rank: -((owner_cols * owner_rows * 10.0).round() as i32),
        kind: line.kind,
        owner_key: line.owner_key.clone(),
        compact_rows: mermaid_kind_uses_compact_rows(line.kind),
        source_index,
        x: screen_x as u16,
        y: screen_y as u16,
        text: clipped,
        color: mermaid_semantic_line_color(line.kind, &line.owner_key, owner_colors),
    })
}

fn mermaid_candidate_row_order(candidate: &MermaidProjectedCandidate) -> Vec<i32> {
    let budget = candidate.kind.row_nudge_budget();
    let mut row_candidates = vec![candidate.y as i32];
    for offset in 1..=budget {
        row_candidates.push(candidate.y as i32 + offset);
        row_candidates.push(candidate.y as i32 - offset);
    }
    row_candidates
}

fn mermaid_first_available_candidate_row(
    candidate: &MermaidProjectedCandidate,
    occupied_rows: &HashMap<u16, Vec<(u16, u16)>>,
    collision_padding: u16,
    bounds: MermaidProjectionBounds,
) -> Option<(u16, u16, u16)> {
    let start = candidate.x;
    let end = candidate
        .x
        .saturating_add(display_width(&candidate.text).max(1));
    let padded_start = start.saturating_sub(collision_padding);
    let padded_end = end.saturating_add(collision_padding);

    for row in mermaid_candidate_row_order(candidate) {
        if row < bounds.top || row >= bounds.bottom {
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
            return Some((row, padded_start, padded_end));
        }
    }

    None
}

pub(crate) fn project_mermaid_semantic_lines(
    lines: &[MermaidSemanticLine],
    transform: MermaidViewportTransform,
    content_rect: Rect,
    view_state: MermaidViewState,
) -> Vec<MermaidProjectedLine> {
    let mut candidates = Vec::new();
    let owner_colors = mermaid_owner_accent_map(lines);
    let bounds = MermaidProjectionBounds::from_rect(content_rect);

    for (source_index, line) in lines.iter().enumerate() {
        candidates.extend(mermaid_semantic_line_to_candidate(
            source_index,
            line,
            transform,
            bounds,
            view_state,
            &owner_colors,
        ));
    }

    let max_row = content_rect.bottom().saturating_sub(1);
    pack_compact_rows(&mut candidates, max_row);

    if matches!(view_state, MermaidViewState::L2 | MermaidViewState::L3) {
        filter_owner_summaries_with_details(&mut candidates);
    }

    candidates.sort_by_key(|line| (line.priority, line.area_rank, line.y, line.x));

    let mut occupied_rows: HashMap<u16, Vec<(u16, u16)>> = HashMap::new();
    let mut projected = Vec::new();
    let collision_padding = view_state.collision_padding();
    for candidate in candidates {
        let Some((target_y, padded_start, padded_end)) = mermaid_first_available_candidate_row(
            &candidate,
            &occupied_rows,
            collision_padding,
            bounds,
        ) else {
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
