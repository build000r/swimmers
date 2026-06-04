use super::*;

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
    let mut node_groups = mermaid_outline_top_level_node_groups(
        layout,
        mermaid_top_level_subgraph_indices(&layout.subgraphs),
    );
    mermaid_outline_add_node_groups(layout, &mut node_groups);

    let mut edges = HashMap::new();
    for edge in &layout.edges {
        let Some(from_key) = node_groups.get(&edge.from) else {
            continue;
        };
        let Some(to_key) = node_groups.get(&edge.to) else {
            continue;
        };
        let directed = edge.directed || edge.arrow_end || edge.arrow_start;
        if let Some((map_key, outline_edge)) =
            mermaid_outline_edge_from_keys(from_key, to_key, directed)
        {
            edges.entry(map_key).or_insert(outline_edge);
        }
    }

    edges
}

fn mermaid_outline_top_level_node_groups(
    layout: &MermaidLayout,
    top_level_subgraphs: HashSet<usize>,
) -> HashMap<String, String> {
    let mut node_groups = HashMap::new();
    for subgraph_idx in top_level_subgraphs {
        let key = mermaid_outline_subgraph_key(subgraph_idx);
        if let Some(subgraph) = layout.subgraphs.get(subgraph_idx) {
            for node_id in &subgraph.nodes {
                node_groups.insert(node_id.clone(), key.clone());
            }
        }
    }
    node_groups
}

fn mermaid_outline_add_node_groups(
    layout: &MermaidLayout,
    node_groups: &mut HashMap<String, String>,
) {
    for node in layout.nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        node_groups
            .entry(node.id.clone())
            .or_insert_with(|| mermaid_outline_node_key(&node.id));
    }
}

fn mermaid_outline_edge_from_keys(
    from_key: &str,
    to_key: &str,
    directed: bool,
) -> Option<(String, MermaidOutlineEdge)> {
    if from_key == to_key {
        return None;
    }

    Some((
        format!("{from_key}->{to_key}:{}", u8::from(directed)),
        MermaidOutlineEdge {
            from_key: from_key.to_string(),
            to_key: to_key.to_string(),
            directed,
        },
    ))
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
        ('|', '_') | ('_', '|') | ('|', '|') => '|',
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

fn mermaid_best_outline_lane(
    lane_cache: &mut HashMap<(i32, i32, i8), i32>,
    lane_key: (i32, i32, i8),
    preferred_lane: i32,
    candidates: Vec<i32>,
    reserved_segments: &[MermaidOutlineSegment],
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
    ignore_keys: [&str; 2],
    build_segments: impl Fn(i32) -> [MermaidOutlineSegment; 3],
) -> i32 {
    if let Some(existing) = lane_cache.get(&lane_key).copied() {
        return existing;
    }

    let best = candidates
        .into_iter()
        .min_by_key(|candidate| {
            let segment_score = build_segments(*candidate)
                .into_iter()
                .map(|segment| {
                    mermaid_outline_segment_score(
                        segment,
                        reserved_segments,
                        label_rects,
                        ignore_keys,
                    )
                })
                .sum::<i32>();
            segment_score + (candidate - preferred_lane).abs() * 2
        })
        .unwrap_or(preferred_lane);
    lane_cache.insert(lane_key, best);
    best
}

fn mermaid_plan_horizontal_outline_route(
    content_rect: Rect,
    edge: &MermaidOutlineEdge,
    start_x: i32,
    end_x: i32,
    from_y: i32,
    to_y: i32,
    direction: i8,
    reserved_segments: &mut Vec<MermaidOutlineSegment>,
    lane_cache_vertical: &mut HashMap<(i32, i32, i8), i32>,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> (Vec<MermaidOutlineSegment>, Option<MermaidOutlineArrow>) {
    if from_y == to_y {
        let segment =
            mermaid_outline_segment(MermaidOutlineAxis::Horizontal, from_y, start_x, end_x);
        reserved_segments.push(segment);
        let arrow = edge.directed.then_some(MermaidOutlineArrow {
            x: end_x,
            y: to_y,
            ch: if direction >= 0 { '>' } else { '<' },
        });
        return (vec![segment], arrow);
    }

    let lane_key = (from_y.min(to_y), from_y.max(to_y), direction);
    let preferred_lane = (start_x + end_x) / 2;
    let candidates = mermaid_outline_vertical_lane_candidates(content_rect, preferred_lane);
    let lane = mermaid_best_outline_lane(
        lane_cache_vertical,
        lane_key,
        preferred_lane,
        candidates,
        reserved_segments,
        label_rects,
        [&edge.from_key, &edge.to_key],
        |candidate| {
            [
                mermaid_outline_segment(MermaidOutlineAxis::Horizontal, from_y, start_x, candidate),
                mermaid_outline_segment(MermaidOutlineAxis::Vertical, candidate, from_y, to_y),
                mermaid_outline_segment(MermaidOutlineAxis::Horizontal, to_y, candidate, end_x),
            ]
        },
    );

    let segments = vec![
        mermaid_outline_segment(MermaidOutlineAxis::Horizontal, from_y, start_x, lane),
        mermaid_outline_segment(MermaidOutlineAxis::Vertical, lane, from_y, to_y),
        mermaid_outline_segment(MermaidOutlineAxis::Horizontal, to_y, lane, end_x),
    ];
    reserved_segments.extend(segments.iter().copied());
    let arrow = edge.directed.then_some(MermaidOutlineArrow {
        x: end_x,
        y: to_y,
        ch: if direction >= 0 { '>' } else { '<' },
    });
    (segments, arrow)
}

fn mermaid_plan_vertical_outline_route(
    content_rect: Rect,
    edge: &MermaidOutlineEdge,
    start_y: i32,
    end_y: i32,
    from_center_x: i32,
    to_center_x: i32,
    direction: i8,
    reserved_segments: &mut Vec<MermaidOutlineSegment>,
    lane_cache_horizontal: &mut HashMap<(i32, i32, i8), i32>,
    label_rects: &HashMap<String, MermaidOutlineLabelRect>,
) -> (Vec<MermaidOutlineSegment>, Option<MermaidOutlineArrow>) {
    if from_center_x == to_center_x {
        let segment =
            mermaid_outline_segment(MermaidOutlineAxis::Vertical, from_center_x, start_y, end_y);
        reserved_segments.push(segment);
        return (vec![segment], None);
    }

    let lane_key = (
        from_center_x.min(to_center_x),
        from_center_x.max(to_center_x),
        direction,
    );
    let preferred_lane = (start_y + end_y) / 2;
    let candidates = mermaid_outline_horizontal_lane_candidates(content_rect, preferred_lane);
    let lane = mermaid_best_outline_lane(
        lane_cache_horizontal,
        lane_key,
        preferred_lane,
        candidates,
        reserved_segments,
        label_rects,
        [&edge.from_key, &edge.to_key],
        |candidate| {
            [
                mermaid_outline_segment(
                    MermaidOutlineAxis::Vertical,
                    from_center_x,
                    start_y,
                    candidate,
                ),
                mermaid_outline_segment(
                    MermaidOutlineAxis::Horizontal,
                    candidate,
                    from_center_x,
                    to_center_x,
                ),
                mermaid_outline_segment(
                    MermaidOutlineAxis::Vertical,
                    to_center_x,
                    candidate,
                    end_y,
                ),
            ]
        },
    );

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
        mermaid_plan_horizontal_outline_route(
            content_rect,
            edge,
            if dx >= 0 {
                from.x as i32 + from.text_width as i32
            } else {
                from.x as i32 - 1
            },
            if dx >= 0 {
                to.x as i32 - 1
            } else {
                to.x as i32 + to.text_width as i32
            },
            from.y as i32,
            to.y as i32,
            dx.signum() as i8,
            reserved_segments,
            lane_cache_vertical,
            label_rects,
        )
    } else {
        mermaid_plan_vertical_outline_route(
            content_rect,
            edge,
            if dy >= 0 {
                from.y as i32 + 1
            } else {
                from.y as i32 - 1
            },
            if dy >= 0 {
                to.y as i32 - 1
            } else {
                to.y as i32 + 1
            },
            from_center_x,
            to_center_x,
            dy.signum() as i8,
            reserved_segments,
            lane_cache_horizontal,
            label_rects,
        )
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
        mermaid_plan_horizontal_outline_route(
            content_rect,
            edge,
            if dx >= 0 {
                from_rect.right + 1
            } else {
                from_rect.left - 1
            },
            if dx >= 0 {
                to_rect.left - 1
            } else {
                to_rect.right + 1
            },
            from_center_y,
            to_center_y,
            dx.signum() as i8,
            reserved_segments,
            lane_cache_vertical,
            label_rects,
        )
    } else {
        mermaid_plan_vertical_outline_route(
            content_rect,
            edge,
            if dy >= 0 {
                from_rect.bottom + 1
            } else {
                from_rect.top - 1
            },
            if dy >= 0 {
                to_rect.top - 1
            } else {
                to_rect.bottom + 1
            },
            from_center_x,
            to_center_x,
            dy.signum() as i8,
            reserved_segments,
            lane_cache_horizontal,
            label_rects,
        )
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
