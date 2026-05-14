use super::*;
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
    let mut rows = Vec::new();

    let max_rows = content_rect.height.saturating_sub(SKILL_PANEL_HEADER_ROWS) as usize;
    let mut hidden_rows = 0usize;
    let mut current_source = String::new();
    let mut y = content_rect.y.saturating_add(SKILL_PANEL_HEADER_ROWS);
    for skill in skills {
        if rows.len() >= max_rows {
            hidden_rows += 1;
            continue;
        }
        if current_source != skill.source_dir {
            current_source = skill.source_dir.clone();
            let count = collect_skill_views(app)
                .iter()
                .filter(|candidate| candidate.source_dir == current_source)
                .count();
            let line = truncate_label(
                &format!("src: {} ({count})", skill.source_label),
                content_rect.width as usize,
            );
            rows.push(SkillPanelRowLayout {
                rect: Rect {
                    x: content_rect.x,
                    y,
                    width: display_width(&line),
                    height: 1,
                },
                line,
                color: Color::DarkGrey,
                kind: SkillPanelRowKind::Source {
                    source_dir: current_source.clone(),
                    label: skill.source_label.clone(),
                    count,
                },
            });
            y = y.saturating_add(1);
            if rows.len() >= max_rows {
                hidden_rows += 1;
                continue;
            }
        }

        let line = truncate_label(&skill_row_line(&skill), content_rect.width as usize);
        let color = if skill.sbp_highlight {
            Color::Yellow
        } else if skill.selected_context {
            Color::White
        } else {
            Color::Cyan
        };
        rows.push(SkillPanelRowLayout {
            rect: Rect {
                x: content_rect.x,
                y,
                width: display_width(&line),
                height: 1,
            },
            line,
            color,
            kind: SkillPanelRowKind::Skill(skill),
        });
        y = y.saturating_add(1);
    }

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

pub(crate) fn render_skill_panel<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, field: Rect) {
    let layout = build_skill_panel(app, field);
    let Some(panel_rect) = layout.panel_rect else {
        return;
    };
    let Some(content_rect) = layout.content_rect else {
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
    if layout.hidden_rows > 0 {
        let hidden = format!("+{} more", layout.hidden_rows);
        renderer.draw_text(
            content_rect.x,
            content_rect.bottom().saturating_sub(1),
            &truncate_label(&hidden, content_rect.width as usize),
            Color::DarkGrey,
        );
    }
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
    let focus_title = mermaid_label(&skill_atlas_focus_title(focus));
    let mut source = String::from("flowchart TB\n");
    source.push_str("  atlas[\"Skill Atlas\\nSBP skill context\"]\n");
    source.push_str(&format!("  focus[\"FOCUS\\n{focus_title}\"]\n"));
    source.push_str("  repos[\"active repo contexts\"]\n");
    source.push_str("  sources[\"source directories\"]\n");
    source.push_str("  atlas --> focus\n");
    source.push_str("  atlas --> repos\n");
    source.push_str("  atlas --> sources\n");

    for (idx, context) in contexts.iter().enumerate().take(8) {
        let selected = if context.selected { " *" } else { "" };
        let label = mermaid_label(&format!(
            "{}{}\\n{} session{}",
            context.label,
            selected,
            context.count,
            if context.count == 1 { "" } else { "s" }
        ));
        source.push_str(&format!("  repo{idx}[\"{label}\"]\n"));
        source.push_str(&format!("  repos --> repo{idx}\n"));
    }
    if contexts.len() > 8 {
        source.push_str(&format!(
            "  repo_more[\"+{} more repo contexts\"]\n  repos --> repo_more\n",
            contexts.len() - 8
        ));
    }

    let mut source_indexes = BTreeMap::<String, usize>::new();
    for skill in &skills {
        if !source_indexes.contains_key(&skill.source_dir) {
            let idx = source_indexes.len();
            source_indexes.insert(skill.source_dir.clone(), idx);
            let label = mermaid_label(&skill.source_label);
            source.push_str(&format!("  src{idx}[\"{label}\"]\n"));
            source.push_str(&format!("  sources --> src{idx}\n"));
            if matches!(focus, SkillPanelAction::Source { source_dir, .. } if source_dir == &skill.source_dir)
            {
                source.push_str(&format!("  focus --> src{idx}\n"));
            }
        }
    }

    for (idx, skill) in skills.iter().enumerate().take(48) {
        let marker = if skill.sbp_highlight { "[SBP] " } else { "" };
        let selected = if skill.selected_context { " *" } else { "" };
        let state = skill
            .state
            .as_deref()
            .or(skill.availability.as_deref())
            .unwrap_or("ok");
        let contexts = if skill.contexts.is_empty() {
            "no active repo".to_string()
        } else {
            skill.contexts.join(", ")
        };
        let label = mermaid_label(&format!(
            "{marker}{}{}\\n{}\\n{}",
            skill.name, selected, state, contexts
        ));
        source.push_str(&format!("  skill{idx}[\"{label}\"]\n"));
        if let Some(source_idx) = source_indexes.get(&skill.source_dir) {
            source.push_str(&format!("  src{source_idx} --> skill{idx}\n"));
        }
        if matches!(focus, SkillPanelAction::Skill(focused) if focused.name == skill.name && focused.source_dir == skill.source_dir)
        {
            source.push_str(&format!("  focus --> skill{idx}\n"));
        }
    }
    if skills.len() > 48 {
        source.push_str(&format!(
            "  skill_more[\"+{} more skills\"]\n  sources --> skill_more\n",
            skills.len() - 48
        ));
    }

    source
}

pub(crate) fn skill_atlas_plan_text<C: TuiApi>(app: &App<C>, focus: &SkillPanelAction) -> String {
    let skills = collect_skill_views(app);
    let contexts = skill_panel_contexts(app);
    let mut text = String::new();
    text.push_str("# Skill Atlas\n\n");
    text.push_str(&format!("Focus: {}\n\n", skill_atlas_focus_title(focus)));
    text.push_str("## Active Repo Contexts\n\n");
    for context in &contexts {
        let selected = if context.selected { " selected" } else { "" };
        text.push_str(&format!(
            "- {}{}: {} session{}\n",
            context.label,
            selected,
            context.count,
            if context.count == 1 { "" } else { "s" }
        ));
    }
    text.push_str("\n## Skills By Source Directory\n\n");
    let mut current_source = "";
    for skill in &skills {
        if current_source != skill.source_dir {
            current_source = &skill.source_dir;
            text.push_str(&format!("### {}\n\n", skill.source_label));
        }
        let marker = if skill.sbp_highlight { " [SBP]" } else { "" };
        let state = skill
            .state
            .as_deref()
            .or(skill.availability.as_deref())
            .unwrap_or("ok");
        let contexts = if skill.contexts.is_empty() {
            "no active repo".to_string()
        } else {
            skill.contexts.join(", ")
        };
        text.push_str(&format!(
            "- {}{}: {} ({})\n",
            skill.name, marker, state, contexts
        ));
        if let Some(description) = skill.description.as_deref() {
            text.push_str(&format!("  {}\n", description.trim()));
        }
    }
    text
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
        let Some(cache) = app.session_skill_cache.get(&entity.session.session_id) else {
            continue;
        };
        let Some(response) = cache
            .response
            .as_ref()
            .filter(|response| response.available)
        else {
            continue;
        };
        let context_label = path_tail_label(&entity.session.cwd)
            .unwrap_or_else(|| normalize_path(&entity.session.cwd));
        let selected = selected_id
            .map(|id| id == entity.session.session_id)
            .unwrap_or(false);

        for skill in &response.skills {
            let source_dir = skill_source_dir(skill);
            let key = (source_dir.clone(), skill.name.to_ascii_lowercase());
            let entry = views.entry(key).or_insert_with(|| SkillPanelSkillView {
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
            });
            if !entry.contexts.contains(&context_label) {
                entry.contexts.push(context_label.clone());
            }
            entry.selected_context |= selected;
            entry.sbp_highlight |= skill_is_sbp_highlight(skill);
        }
    }

    let mut out = views.into_values().collect::<Vec<_>>();
    out.sort_by(|left, right| {
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
    });
    out
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
