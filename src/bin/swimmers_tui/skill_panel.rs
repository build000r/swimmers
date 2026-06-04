use super::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

const SKILL_PANEL_MIN_FIELD_WIDTH: u16 = 86;
const SKILL_PANEL_MIN_FIELD_HEIGHT: u16 = 10;
const SKILL_PANEL_MIN_WIDTH: u16 = 30;
const SKILL_PANEL_MAX_WIDTH: u16 = 52;
const SKILL_PANEL_GAP: u16 = 1;
const SKILL_PANEL_HEADER_ROWS: u16 = 4;
const SKILL_CONTEXT_LIMIT: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillCacheContext {
    pub(crate) cwd: String,
}

impl SkillCacheContext {
    pub(crate) fn from_session(session: &SessionSummary) -> Self {
        Self {
            cwd: normalize_path(&session.cwd),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SkillCacheEntry {
    pub(crate) context: SkillCacheContext,
    pub(crate) response: Option<SessionSkillListResponse>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillPanelContext {
    pub(crate) cwd: String,
    pub(crate) label: String,
    pub(crate) count: usize,
    pub(crate) selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillPanelSkillView {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) source_dir: String,
    pub(crate) source_label: String,
    pub(crate) source_bucket: Option<String>,
    pub(crate) layer: Option<String>,
    pub(crate) availability: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) contexts: Vec<String>,
    pub(crate) selected_context: bool,
    pub(crate) sbp_highlight: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SkillPanelRowKind {
    Source {
        source_dir: String,
        label: String,
        count: usize,
    },
    Skill(SkillPanelSkillView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SkillPanelAction {
    Source {
        source_dir: String,
        source_label: String,
    },
    Skill(SkillPanelSkillView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillPanelRowLayout {
    pub(crate) rect: Rect,
    pub(crate) line: String,
    pub(crate) color: Color,
    pub(crate) kind: SkillPanelRowKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillPanelLayout {
    pub(crate) tank_field: Rect,
    pub(crate) panel_rect: Option<Rect>,
    pub(crate) content_rect: Option<Rect>,
    pub(crate) header: String,
    pub(crate) context_line: String,
    pub(crate) status_line: String,
    pub(crate) rows: Vec<SkillPanelRowLayout>,
    pub(crate) hidden_rows: usize,
}

struct SkillPanelRows {
    rows: Vec<SkillPanelRowLayout>,
    hidden_rows: usize,
}

pub(crate) fn skill_panel_contexts<C: TuiApi>(app: &App<C>) -> Vec<SkillPanelContext> {
    let selected_id = app.selected_id.as_deref();
    let mut order = Vec::<String>::new();
    let mut contexts = BTreeMap::<String, SkillPanelContext>::new();

    for entity in app.visible_entities() {
        let cwd = normalize_path(&entity.session.cwd);
        if !contexts.contains_key(&cwd) {
            order.push(cwd.clone());
        }
        let label = path_tail_label(&cwd).unwrap_or_else(|| cwd.clone());
        let selected = selected_id
            .map(|id| id == entity.session.session_id)
            .unwrap_or(false);
        let entry = contexts.entry(cwd.clone()).or_insert(SkillPanelContext {
            cwd,
            label,
            count: 0,
            selected: false,
        });
        entry.count += 1;
        entry.selected |= selected;
    }

    order.sort_by_key(|cwd| {
        contexts
            .get(cwd)
            .map(|context| {
                (
                    !context.selected,
                    context.label.clone(),
                    context.cwd.clone(),
                )
            })
            .unwrap_or((true, String::new(), cwd.clone()))
    });
    order
        .into_iter()
        .filter_map(|cwd| contexts.remove(&cwd))
        .collect()
}

pub(crate) fn skill_panel_rect_for_field(field: Rect) -> Option<(Rect, Rect)> {
    if field.width < SKILL_PANEL_MIN_FIELD_WIDTH || field.height < SKILL_PANEL_MIN_FIELD_HEIGHT {
        return None;
    }
    let width = (field.width / 3)
        .clamp(SKILL_PANEL_MIN_WIDTH, SKILL_PANEL_MAX_WIDTH)
        .min(field.width.saturating_sub(ENTITY_WIDTH + 5));
    if width < SKILL_PANEL_MIN_WIDTH {
        return None;
    }
    let panel_rect = Rect {
        x: field.right().saturating_sub(width),
        y: field.y,
        width,
        height: field.height,
    };
    let tank_field = Rect {
        x: field.x,
        y: field.y,
        width: field
            .width
            .saturating_sub(width.saturating_add(SKILL_PANEL_GAP)),
        height: field.height,
    };
    (tank_field.width >= ENTITY_WIDTH + 4).then_some((tank_field, panel_rect))
}

pub(crate) fn build_skill_panel<C: TuiApi>(app: &App<C>, field: Rect) -> SkillPanelLayout {
    let Some((tank_field, panel_rect)) = skill_panel_rect_for_field(field) else {
        return SkillPanelLayout {
            tank_field: field,
            ..SkillPanelLayout::default()
        };
    };

    let contexts = skill_panel_contexts(app);
    if contexts.is_empty() {
        return SkillPanelLayout {
            tank_field: field,
            ..SkillPanelLayout::default()
        };
    }

    let content_rect = panel_rect.inset(1);
    let context_line = skill_context_line(&contexts);
    let status_line = skill_panel_status_line(app);
    let skills = collect_skill_views(app);
    let SkillPanelRows { rows, hidden_rows } = build_skill_panel_rows(skills, content_rect);

    SkillPanelLayout {
        tank_field,
        panel_rect: Some(panel_rect),
        content_rect: Some(content_rect),
        header: "skills via SBP".to_string(),
        context_line,
        status_line,
        rows,
        hidden_rows,
    }
}

fn build_skill_panel_rows(skills: Vec<SkillPanelSkillView>, content_rect: Rect) -> SkillPanelRows {
    let source_counts = skill_panel_source_counts(&skills);
    let mut builder = SkillPanelRowBuilder::new(content_rect);
    for skill in skills {
        let count = source_counts
            .get(&skill.source_dir)
            .copied()
            .unwrap_or_default();
        builder.push_skill(skill, count);
    }
    builder.finish()
}

fn skill_panel_source_counts(skills: &[SkillPanelSkillView]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<String, usize>::new();
    for skill in skills {
        *counts.entry(skill.source_dir.clone()).or_default() += 1;
    }
    counts
}

struct SkillPanelRowBuilder {
    content_rect: Rect,
    max_rows: usize,
    rows: Vec<SkillPanelRowLayout>,
    hidden_rows: usize,
    current_source: String,
    y: u16,
}

impl SkillPanelRowBuilder {
    fn new(content_rect: Rect) -> Self {
        Self {
            content_rect,
            max_rows: content_rect.height.saturating_sub(SKILL_PANEL_HEADER_ROWS) as usize,
            rows: Vec::new(),
            hidden_rows: 0,
            current_source: String::new(),
            y: content_rect.y.saturating_add(SKILL_PANEL_HEADER_ROWS),
        }
    }

    fn push_skill(&mut self, skill: SkillPanelSkillView, source_count: usize) {
        if self.is_full() {
            self.hidden_rows += 1;
            return;
        }
        if self.current_source != skill.source_dir {
            self.current_source = skill.source_dir.clone();
            self.push_source_row(&skill, source_count);
            if self.is_full() {
                self.hidden_rows += 1;
                return;
            }
        }
        self.push_skill_row(skill);
    }

    fn push_source_row(&mut self, skill: &SkillPanelSkillView, count: usize) {
        self.rows.push(skill_panel_source_row_layout(
            self.content_rect,
            self.y,
            skill,
            count,
        ));
        self.advance();
    }

    fn push_skill_row(&mut self, skill: SkillPanelSkillView) {
        self.rows.push(skill_panel_skill_row_layout(
            self.content_rect,
            self.y,
            skill,
        ));
        self.advance();
    }

    fn is_full(&self) -> bool {
        self.rows.len() >= self.max_rows
    }

    fn advance(&mut self) {
        self.y = self.y.saturating_add(1);
    }

    fn finish(self) -> SkillPanelRows {
        SkillPanelRows {
            rows: self.rows,
            hidden_rows: self.hidden_rows,
        }
    }
}

fn skill_panel_source_row_layout(
    content_rect: Rect,
    y: u16,
    skill: &SkillPanelSkillView,
    count: usize,
) -> SkillPanelRowLayout {
    let line = truncate_label(
        &format!("src: {} ({count})", skill.source_label),
        content_rect.width as usize,
    );
    SkillPanelRowLayout {
        rect: skill_panel_row_rect(content_rect, y, &line),
        line,
        color: Color::DarkGrey,
        kind: SkillPanelRowKind::Source {
            source_dir: skill.source_dir.clone(),
            label: skill.source_label.clone(),
            count,
        },
    }
}

fn skill_panel_skill_row_layout(
    content_rect: Rect,
    y: u16,
    skill: SkillPanelSkillView,
) -> SkillPanelRowLayout {
    let line = truncate_label(&skill_row_line(&skill), content_rect.width as usize);
    SkillPanelRowLayout {
        rect: skill_panel_row_rect(content_rect, y, &line),
        line,
        color: skill_panel_skill_row_color(&skill),
        kind: SkillPanelRowKind::Skill(skill),
    }
}

fn skill_panel_row_rect(content_rect: Rect, y: u16, line: &str) -> Rect {
    Rect {
        x: content_rect.x,
        y,
        width: display_width(line),
        height: 1,
    }
}

fn skill_panel_skill_row_color(skill: &SkillPanelSkillView) -> Color {
    if skill.sbp_highlight {
        Color::Yellow
    } else if skill.selected_context {
        Color::White
    } else {
        Color::Cyan
    }
}

pub(crate) fn render_skill_panel<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, field: Rect) {
    let layout = build_skill_panel(app, field);
    let (Some(panel_rect), Some(content_rect)) = (layout.panel_rect, layout.content_rect) else {
        return;
    };
    renderer.draw_box(panel_rect, Color::DarkGrey);
    renderer.draw_text(content_rect.x, content_rect.y, &layout.header, Color::Cyan);
    renderer.draw_text(
        content_rect.x,
        content_rect.y.saturating_add(1),
        &truncate_label(&layout.context_line, content_rect.width as usize),
        Color::White,
    );
    renderer.draw_text(
        content_rect.x,
        content_rect.y.saturating_add(2),
        &truncate_label(&layout.status_line, content_rect.width as usize),
        Color::Yellow,
    );
    for row in &layout.rows {
        renderer.draw_text(row.rect.x, row.rect.y, &row.line, row.color);
    }
    render_skill_panel_hidden_rows(renderer, content_rect, layout.hidden_rows);
}

fn render_skill_panel_hidden_rows(renderer: &mut Renderer, content_rect: Rect, hidden_rows: usize) {
    if hidden_rows == 0 {
        return;
    }
    let hidden = format!("+{hidden_rows} more");
    renderer.draw_text(
        content_rect.x,
        content_rect.bottom().saturating_sub(1),
        &truncate_label(&hidden, content_rect.width as usize),
        Color::DarkGrey,
    );
}

pub(crate) fn skill_panel_action_at<C: TuiApi>(
    app: &App<C>,
    field: Rect,
    x: u16,
    y: u16,
) -> Option<SkillPanelAction> {
    let layout = build_skill_panel(app, field);
    let panel_rect = layout.panel_rect?;
    if !panel_rect.contains(x, y) {
        return None;
    }
    layout.rows.into_iter().find_map(|row| {
        if !row.rect.contains(x, y) {
            return None;
        }
        match row.kind {
            SkillPanelRowKind::Skill(skill) => Some(SkillPanelAction::Skill(skill)),
            SkillPanelRowKind::Source {
                source_dir, label, ..
            } => Some(SkillPanelAction::Source {
                source_dir,
                source_label: label,
            }),
        }
    })
}

pub(crate) fn skill_atlas_mermaid_source<C: TuiApi>(
    app: &App<C>,
    focus: &SkillPanelAction,
) -> String {
    let skills = collect_skill_views(app);
    let contexts = skill_panel_contexts(app);
    let mut source = skill_atlas_mermaid_header(focus);
    append_skill_atlas_repo_nodes(&mut source, &contexts);
    let source_indexes = append_skill_atlas_source_nodes(&mut source, &skills, focus);
    append_skill_atlas_skill_nodes(&mut source, &skills, &source_indexes, focus);
    source
}

fn skill_atlas_mermaid_header(focus: &SkillPanelAction) -> String {
    let focus_title = mermaid_label(&skill_atlas_focus_title(focus));
    let mut source = String::from("flowchart TB\n");
    source.push_str("  atlas[\"Skill Atlas\\nSBP skill context\"]\n");
    source.push_str(&format!("  focus[\"FOCUS\\n{focus_title}\"]\n"));
    source.push_str("  repos[\"active repo contexts\"]\n");
    source.push_str("  sources[\"source directories\"]\n");
    source.push_str("  atlas --> focus\n");
    source.push_str("  atlas --> repos\n");
    source.push_str("  atlas --> sources\n");
    source
}

fn append_skill_atlas_repo_nodes(source: &mut String, contexts: &[SkillPanelContext]) {
    for (idx, context) in contexts.iter().enumerate().take(8) {
        append_skill_atlas_repo_node(source, idx, context);
    }
    append_skill_atlas_repo_overflow(source, contexts.len());
}

fn append_skill_atlas_repo_node(source: &mut String, idx: usize, context: &SkillPanelContext) {
    let label = mermaid_label(&skill_atlas_repo_label(context));
    source.push_str(&format!("  repo{idx}[\"{label}\"]\n"));
    source.push_str(&format!("  repos --> repo{idx}\n"));
}

fn append_skill_atlas_repo_overflow(source: &mut String, context_count: usize) {
    if context_count > 8 {
        source.push_str(&format!(
            "  repo_more[\"+{} more repo contexts\"]\n  repos --> repo_more\n",
            context_count - 8
        ));
    }
}

fn skill_atlas_repo_label(context: &SkillPanelContext) -> String {
    let selected = if context.selected { " *" } else { "" };
    format!(
        "{}{}\\n{} session{}",
        context.label,
        selected,
        context.count,
        skill_atlas_plural_suffix(context.count)
    )
}

fn append_skill_atlas_source_nodes(
    source: &mut String,
    skills: &[SkillPanelSkillView],
    focus: &SkillPanelAction,
) -> BTreeMap<String, usize> {
    let mut source_indexes = BTreeMap::<String, usize>::new();
    for skill in skills {
        if source_indexes.contains_key(&skill.source_dir) {
            continue;
        }
        let idx = source_indexes.len();
        source_indexes.insert(skill.source_dir.clone(), idx);
        append_skill_atlas_source_node(source, idx, skill, focus);
    }
    source_indexes
}

fn append_skill_atlas_source_node(
    source: &mut String,
    idx: usize,
    skill: &SkillPanelSkillView,
    focus: &SkillPanelAction,
) {
    let label = mermaid_label(&skill.source_label);
    source.push_str(&format!("  src{idx}[\"{label}\"]\n"));
    source.push_str(&format!("  sources --> src{idx}\n"));
    if skill_atlas_focus_matches_source(focus, &skill.source_dir) {
        source.push_str(&format!("  focus --> src{idx}\n"));
    }
}

fn append_skill_atlas_skill_nodes(
    source: &mut String,
    skills: &[SkillPanelSkillView],
    source_indexes: &BTreeMap<String, usize>,
    focus: &SkillPanelAction,
) {
    for (idx, skill) in skills.iter().enumerate().take(48) {
        append_skill_atlas_skill_node(source, idx, skill, source_indexes, focus);
    }
    append_skill_atlas_skill_overflow(source, skills.len());
}

fn append_skill_atlas_skill_node(
    source: &mut String,
    idx: usize,
    skill: &SkillPanelSkillView,
    source_indexes: &BTreeMap<String, usize>,
    focus: &SkillPanelAction,
) {
    let label = mermaid_label(&skill_atlas_skill_label(skill));
    source.push_str(&format!("  skill{idx}[\"{label}\"]\n"));
    if let Some(source_idx) = source_indexes.get(&skill.source_dir) {
        source.push_str(&format!("  src{source_idx} --> skill{idx}\n"));
    }
    if skill_atlas_focus_matches_skill(focus, skill) {
        source.push_str(&format!("  focus --> skill{idx}\n"));
    }
}

fn append_skill_atlas_skill_overflow(source: &mut String, skill_count: usize) {
    if skill_count > 48 {
        source.push_str(&format!(
            "  skill_more[\"+{} more skills\"]\n  sources --> skill_more\n",
            skill_count - 48
        ));
    }
}

fn skill_atlas_skill_label(skill: &SkillPanelSkillView) -> String {
    let marker = if skill.sbp_highlight { "[SBP] " } else { "" };
    let selected = if skill.selected_context { " *" } else { "" };
    format!(
        "{marker}{}{}\\n{}\\n{}",
        skill.name,
        selected,
        skill_atlas_skill_state(skill),
        skill_atlas_skill_contexts_label(skill)
    )
}

fn skill_atlas_skill_state(skill: &SkillPanelSkillView) -> &str {
    skill
        .state
        .as_deref()
        .or(skill.availability.as_deref())
        .unwrap_or("ok")
}

fn skill_atlas_skill_contexts_label(skill: &SkillPanelSkillView) -> String {
    if skill.contexts.is_empty() {
        "no active repo".to_string()
    } else {
        skill.contexts.join(", ")
    }
}

fn skill_atlas_plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn skill_atlas_focus_matches_source(focus: &SkillPanelAction, candidate_source_dir: &str) -> bool {
    matches!(focus, SkillPanelAction::Source { source_dir, .. } if source_dir == candidate_source_dir)
}

fn skill_atlas_focus_matches_skill(
    focus: &SkillPanelAction,
    candidate: &SkillPanelSkillView,
) -> bool {
    matches!(focus, SkillPanelAction::Skill(focused) if focused.name == candidate.name && focused.source_dir == candidate.source_dir)
}

pub(crate) fn skill_atlas_plan_text<C: TuiApi>(app: &App<C>, focus: &SkillPanelAction) -> String {
    let skills = collect_skill_views(app);
    let contexts = skill_panel_contexts(app);
    let mut text = skill_atlas_plan_header(focus);
    append_skill_atlas_contexts_text(&mut text, &contexts);
    append_skill_atlas_skills_text(&mut text, &skills);
    text
}

fn skill_atlas_plan_header(focus: &SkillPanelAction) -> String {
    format!(
        "# Skill Atlas\n\nFocus: {}\n\n",
        skill_atlas_focus_title(focus)
    )
}

fn append_skill_atlas_contexts_text(text: &mut String, contexts: &[SkillPanelContext]) {
    text.push_str("## Active Repo Contexts\n\n");
    for context in contexts {
        text.push_str(&skill_atlas_context_text_line(context));
    }
    text.push_str("\n## Skills By Source Directory\n\n");
}

fn skill_atlas_context_text_line(context: &SkillPanelContext) -> String {
    let selected = if context.selected { " selected" } else { "" };
    format!(
        "- {}{}: {} session{}\n",
        context.label,
        selected,
        context.count,
        skill_atlas_plural_suffix(context.count)
    )
}

fn append_skill_atlas_skills_text(text: &mut String, skills: &[SkillPanelSkillView]) {
    let mut current_source = "";
    for skill in skills {
        append_skill_atlas_source_heading_text(text, &mut current_source, skill);
        append_skill_atlas_skill_text(text, skill);
    }
}

fn append_skill_atlas_source_heading_text<'a>(
    text: &mut String,
    current_source: &mut &'a str,
    skill: &'a SkillPanelSkillView,
) {
    if *current_source != skill.source_dir.as_str() {
        *current_source = &skill.source_dir;
        text.push_str(&format!("### {}\n\n", skill.source_label));
    }
}

fn append_skill_atlas_skill_text(text: &mut String, skill: &SkillPanelSkillView) {
    let marker = skill_atlas_skill_text_marker(skill);
    let state = skill_atlas_skill_state(skill);
    let contexts = skill_atlas_skill_contexts_label(skill);
    text.push_str(&format!(
        "- {}{}: {} ({})\n",
        skill.name, marker, state, contexts
    ));
    append_skill_atlas_skill_description_text(text, skill);
}

fn skill_atlas_skill_text_marker(skill: &SkillPanelSkillView) -> &'static str {
    if skill.sbp_highlight {
        " [SBP]"
    } else {
        ""
    }
}

fn append_skill_atlas_skill_description_text(text: &mut String, skill: &SkillPanelSkillView) {
    if let Some(description) = skill.description.as_deref() {
        text.push_str(&format!("  {}\n", description.trim()));
    }
}

pub(crate) fn skill_atlas_focus_title(focus: &SkillPanelAction) -> String {
    match focus {
        SkillPanelAction::Skill(skill) => format!("skill {}", skill.name),
        SkillPanelAction::Source { source_label, .. } => format!("source {}", source_label),
    }
}

pub(crate) fn skill_atlas_focus_path(focus: &SkillPanelAction) -> Option<String> {
    match focus {
        SkillPanelAction::Skill(skill) => skill
            .path
            .clone()
            .or_else(|| Some(skill.source_dir.clone())),
        SkillPanelAction::Source { source_dir, .. } => Some(source_dir.clone()),
    }
}

fn collect_skill_views<C: TuiApi>(app: &App<C>) -> Vec<SkillPanelSkillView> {
    let selected_id = app.selected_id.as_deref();
    let mut views = BTreeMap::<(String, String), SkillPanelSkillView>::new();
    for entity in app.visible_entities() {
        let Some(response) =
            eligible_skill_response(app.session_skill_cache.get(&entity.session.session_id))
        else {
            continue;
        };
        let context_label = skill_context_label(&entity.session.cwd);
        let selected = selected_id
            .map(|id| id == entity.session.session_id)
            .unwrap_or(false);

        for skill in &response.skills {
            let entry = views
                .entry(skill_view_key(skill))
                .or_insert_with(|| seed_skill_view(skill));
            merge_skill_context(entry, &context_label, selected, skill);
        }
    }

    let mut out = views.into_values().collect::<Vec<_>>();
    out.sort_by(compare_skill_views);
    out
}

fn eligible_skill_response(cache: Option<&SkillCacheEntry>) -> Option<&SessionSkillListResponse> {
    cache?
        .response
        .as_ref()
        .filter(|response| response.available)
}

fn skill_context_label(cwd: &str) -> String {
    path_tail_label(cwd).unwrap_or_else(|| normalize_path(cwd))
}

fn skill_view_key(skill: &SessionSkillSummary) -> (String, String) {
    (skill_source_dir(skill), skill.name.to_ascii_lowercase())
}

fn seed_skill_view(skill: &SessionSkillSummary) -> SkillPanelSkillView {
    let source_dir = skill_source_dir(skill);
    SkillPanelSkillView {
        name: skill.name.clone(),
        description: skill.description.clone(),
        source_label: compact_source_label(&source_dir),
        source_dir,
        source_bucket: skill.source_bucket.clone(),
        layer: skill.layer.clone(),
        availability: skill.availability.clone(),
        state: skill.state.clone(),
        path: skill.path.clone().or_else(|| skill.source.clone()),
        contexts: Vec::new(),
        selected_context: false,
        sbp_highlight: skill_is_sbp_highlight(skill),
    }
}

fn merge_skill_context(
    view: &mut SkillPanelSkillView,
    context_label: &str,
    selected: bool,
    skill: &SessionSkillSummary,
) {
    if !view.contexts.iter().any(|context| context == context_label) {
        view.contexts.push(context_label.to_string());
    }
    view.selected_context |= selected;
    view.sbp_highlight |= skill_is_sbp_highlight(skill);
}

fn compare_skill_views(left: &SkillPanelSkillView, right: &SkillPanelSkillView) -> Ordering {
    right
        .sbp_highlight
        .cmp(&left.sbp_highlight)
        .then_with(|| right.selected_context.cmp(&left.selected_context))
        .then_with(|| left.source_label.cmp(&right.source_label))
        .then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
}

fn skill_context_line(contexts: &[SkillPanelContext]) -> String {
    let mut labels = Vec::new();
    for context in contexts.iter().take(SKILL_CONTEXT_LIMIT) {
        let mut label = context.label.clone();
        if context.count > 1 {
            label.push_str(&format!(" x{}", context.count));
        }
        if context.selected {
            label.push('*');
        }
        labels.push(label);
    }
    if contexts.len() > SKILL_CONTEXT_LIMIT {
        labels.push(format!("+{}", contexts.len() - SKILL_CONTEXT_LIMIT));
    }
    format!("ctx: {}", labels.join(", "))
}

fn skill_panel_status_line<C: TuiApi>(app: &App<C>) -> String {
    let contexts = skill_panel_contexts(app);
    let cached = app
        .visible_entities()
        .into_iter()
        .filter(|entity| {
            app.session_skill_cache
                .contains_key(&entity.session.session_id)
        })
        .map(|entity| normalize_path(&entity.session.cwd))
        .collect::<BTreeSet<_>>()
        .len();
    if cached == 0 {
        return "SBP loading".to_string();
    }
    format!("SBP active {cached}/{} repos", contexts.len())
}

fn skill_row_line(skill: &SkillPanelSkillView) -> String {
    let marker = if skill.sbp_highlight { "[SBP] " } else { "" };
    let context = if skill.contexts.is_empty() {
        "none".to_string()
    } else {
        skill.contexts.join(",")
    };
    let state = skill
        .state
        .as_deref()
        .or(skill.availability.as_deref())
        .unwrap_or("ok");
    format!("  {marker}{} {state} {context}", skill.name)
}

fn skill_source_dir(skill: &SessionSkillSummary) -> String {
    skill
        .source
        .as_deref()
        .or(skill.path.as_deref())
        .or(skill.source_bucket.as_deref())
        .unwrap_or("unknown")
        .trim()
        .to_string()
}

fn compact_source_label(source: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let compact = if !home.is_empty() && source.starts_with(&home) {
        format!("~{}", &source[home.len()..])
    } else {
        source.to_string()
    };
    compact
}

fn skill_is_sbp_highlight(skill: &SessionSkillSummary) -> bool {
    let haystack = [
        skill.name.as_str(),
        skill.source_bucket.as_deref().unwrap_or_default(),
        skill.source.as_deref().unwrap_or_default(),
        skill.layer.as_deref().unwrap_or_default(),
    ]
    .join(" ")
    .to_ascii_lowercase();
    haystack.contains("sbp")
        || haystack.contains("skillbox")
        || skill.name.eq_ignore_ascii_case("sbp")
}

fn mermaid_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_skill(
        name: &str,
        source_bucket: &str,
        source: &str,
        path: &str,
    ) -> SessionSkillSummary {
        SessionSkillSummary {
            name: name.to_string(),
            description: Some(format!("{name} description")),
            state: Some("ok".to_string()),
            availability: Some("installed".to_string()),
            layer: Some("project:codex".to_string()),
            source_bucket: Some(source_bucket.to_string()),
            source: Some(source.to_string()),
            path: Some(path.to_string()),
        }
    }

    fn test_skill_response(available: bool) -> SessionSkillListResponse {
        SessionSkillListResponse {
            session_id: "sess".to_string(),
            source: "sbp".to_string(),
            cwd: "/repo/swimmers".to_string(),
            available,
            query: None,
            skills: Vec::new(),
            issues: Vec::new(),
            message: None,
        }
    }

    fn test_cache(response: Option<SessionSkillListResponse>) -> SkillCacheEntry {
        SkillCacheEntry {
            context: SkillCacheContext {
                cwd: "/repo/swimmers".to_string(),
            },
            response,
        }
    }

    #[test]
    fn skill_panel_eligible_response_requires_available_cached_response() {
        let missing_response = test_cache(None);
        let unavailable_response = test_cache(Some(test_skill_response(false)));
        let available_response = test_cache(Some(test_skill_response(true)));

        assert!(eligible_skill_response(None).is_none());
        assert!(eligible_skill_response(Some(&missing_response)).is_none());
        assert!(eligible_skill_response(Some(&unavailable_response)).is_none());
        assert!(eligible_skill_response(Some(&available_response)).is_some());
    }

    #[test]
    fn skill_panel_view_merge_preserves_first_entry_and_ors_duplicate_flags() {
        let first = test_skill(
            "UI",
            "skills-private",
            "/fixture/repos/skills-private/ui",
            "/repo/.codex/skills/ui",
        );
        let duplicate = test_skill(
            "ui",
            "skillbox",
            "/fixture/repos/skills-private/ui",
            "/repo/.codex/skills/ui-duplicate",
        );
        let mut views = BTreeMap::<(String, String), SkillPanelSkillView>::new();

        for (skill, context_label, selected) in
            [(&first, "swimmers", false), (&duplicate, "swimmers", true)]
        {
            let entry = views
                .entry(skill_view_key(skill))
                .or_insert_with(|| seed_skill_view(skill));
            merge_skill_context(entry, context_label, selected, skill);
        }

        let view = views
            .get(&(
                "/fixture/repos/skills-private/ui".to_string(),
                "ui".to_string(),
            ))
            .expect("merged skill view");
        assert_eq!(view.name, "UI");
        assert_eq!(view.description.as_deref(), Some("UI description"));
        assert_eq!(view.source_bucket.as_deref(), Some("skills-private"));
        assert_eq!(view.path.as_deref(), Some("/repo/.codex/skills/ui"));
        assert_eq!(view.contexts, vec!["swimmers"]);
        assert!(view.selected_context);
        assert!(view.sbp_highlight);
    }

    #[test]
    fn skill_panel_view_sort_keeps_highlight_selected_source_and_name_order() {
        let mut views = vec![
            SkillPanelSkillView {
                name: "zeta".to_string(),
                source_label: "beta".to_string(),
                source_dir: "/beta/zeta".to_string(),
                description: None,
                source_bucket: None,
                layer: None,
                availability: None,
                state: None,
                path: None,
                contexts: Vec::new(),
                selected_context: false,
                sbp_highlight: false,
            },
            SkillPanelSkillView {
                name: "alpha".to_string(),
                source_label: "gamma".to_string(),
                source_dir: "/gamma/alpha".to_string(),
                description: None,
                source_bucket: None,
                layer: None,
                availability: None,
                state: None,
                path: None,
                contexts: Vec::new(),
                selected_context: true,
                sbp_highlight: false,
            },
            SkillPanelSkillView {
                name: "skillbox".to_string(),
                source_label: "omega".to_string(),
                source_dir: "/omega/skillbox".to_string(),
                description: None,
                source_bucket: None,
                layer: None,
                availability: None,
                state: None,
                path: None,
                contexts: Vec::new(),
                selected_context: false,
                sbp_highlight: true,
            },
            SkillPanelSkillView {
                name: "Beta".to_string(),
                source_label: "alpha".to_string(),
                source_dir: "/alpha/beta".to_string(),
                description: None,
                source_bucket: None,
                layer: None,
                availability: None,
                state: None,
                path: None,
                contexts: Vec::new(),
                selected_context: false,
                sbp_highlight: false,
            },
            SkillPanelSkillView {
                name: "alpha".to_string(),
                source_label: "alpha".to_string(),
                source_dir: "/alpha/alpha".to_string(),
                description: None,
                source_bucket: None,
                layer: None,
                availability: None,
                state: None,
                path: None,
                contexts: Vec::new(),
                selected_context: false,
                sbp_highlight: false,
            },
        ];

        views.sort_by(compare_skill_views);

        assert_eq!(
            views
                .iter()
                .map(|view| view.name.as_str())
                .collect::<Vec<_>>(),
            vec!["skillbox", "alpha", "alpha", "Beta", "zeta"]
        );
    }
}
