use super::*;

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
pub(crate) struct MermaidErOrderNode {
    pub(crate) owner_key: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) neighbors: Vec<String>,
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
        MermaidViewState::ErKeys => 1.25,
        MermaidViewState::ErColumns => 1.5,
        MermaidViewState::ErSchema => 1.75,
        MermaidViewState::ErEntities
        | MermaidViewState::Outline
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
                MermaidViewState::ErKeys => {
                    row.name_text.contains(" PK") || row.name_text.contains(" FK")
                }
                MermaidViewState::ErColumns | MermaidViewState::ErSchema => true,
                MermaidViewState::ErEntities
                | MermaidViewState::Outline
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

fn mermaid_er_node_position(
    positions: &HashMap<String, (f32, f32)>,
    owner_key: &str,
) -> (f32, f32) {
    match positions.get(owner_key) {
        Some(pos) => *pos,
        None => {
            tracing::warn!(
                owner_key = %owner_key,
                "missing ER order node position, falling back to (0.0, 0.0)"
            );
            (0.0, 0.0)
        }
    }
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
    let left_position = mermaid_er_node_position(positions, left);
    let right_position = mermaid_er_node_position(positions, right);

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
    let position = mermaid_er_node_position(positions, owner_key);
    let min_neighbor_distance = adjacent_neighbors
        .into_iter()
        .map(|neighbor| {
            mermaid_distance_sq(position, mermaid_er_node_position(positions, neighbor))
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
    let left_position = mermaid_er_node_position(positions, left);
    let right_position = mermaid_er_node_position(positions, right);

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
        let size_cmp = right.len().cmp(&left.len());
        match (left.first(), right.first()) {
            (Some(left_seed), Some(right_seed)) => size_cmp.then_with(|| {
                mermaid_er_compare_seed_nodes(
                    left_seed, right_seed, &positions, &adjacency, centroid,
                )
            }),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => size_cmp,
        }
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
        let Some(seed) = remaining
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
        else {
            tracing::warn!("Mermaid ER ordering skipped an empty component");
            continue;
        };
        remaining.remove(&seed);
        ordered.push(seed.clone());

        let mut placed = HashSet::from([seed]);
        while !remaining.is_empty() {
            let Some(next) = remaining
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
            else {
                tracing::warn!("Mermaid ER ordering lost remaining-node candidate");
                break;
            };
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
