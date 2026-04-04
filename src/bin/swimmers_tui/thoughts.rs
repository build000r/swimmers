use super::*;

const THOUGHT_SHIFT_LABEL: &str = "[shift]";
const THOUGHT_COMMIT_LABEL: &str = "[commit]";

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
    pub(crate) thought: String,
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) color: Color,
    pub(crate) objective_changed: bool,
    pub(crate) commit_candidate: bool,
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
            cwd: normalize_path(&session.cwd),
            pwd_label: path_tail_label(&session.cwd),
            thought,
            updated_at: session.thought_updated_at,
            color: session_display_color(session, repo_themes),
            objective_changed: session.objective_changed_at.is_some()
                && session.objective_changed_at == session.thought_updated_at,
            commit_candidate: session.commit_candidate,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThoughtFilter {
    pub(crate) cwd: Option<String>,
    pub(crate) tmux_name: Option<String>,
    pub(crate) excluded_cwds: HashSet<String>,
    pub(crate) filter_out_mode: bool,
}

impl ThoughtFilter {
    pub(crate) fn is_active(&self) -> bool {
        self.cwd.is_some() || self.tmux_name.is_some() || !self.excluded_cwds.is_empty()
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
        cwd_matches && tmux_matches
    }

    pub(crate) fn matches_session(&self, session: &SessionSummary) -> bool {
        let cwd_matches = self
            .cwd
            .as_ref()
            .map(|cwd| normalize_path(&session.cwd) == *cwd)
            .or_else(|| {
                (!self.excluded_cwds.is_empty())
                    .then_some(!self.excluded_cwds.contains(&normalize_path(&session.cwd)))
            })
            .unwrap_or(true);
        let tmux_matches = self
            .tmux_name
            .as_ref()
            .map(|tmux_name| session.tmux_name == *tmux_name)
            .unwrap_or(true);
        cwd_matches && tmux_matches
    }

    pub(crate) fn excludes_cwd(&self, cwd: &str) -> bool {
        self.excluded_cwds.contains(cwd)
    }

    pub(crate) fn clear(&mut self) {
        self.cwd = None;
        self.tmux_name = None;
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
    ToggleFilterOutMode,
    ToggleFilterOutCwd(String),
    OpenSession { session_id: String, label: String },
    LaunchCommitCodex(String),
    OpenMermaid(String),
    OpenRepoInEditor(String),
    ClearFilters,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtChipLayout {
    pub(crate) rect: Rect,
    pub(crate) cwd: String,
    pub(crate) label: String,
    pub(crate) color: Color,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtRowLayout {
    pub(crate) shift_rect: Option<Rect>,
    pub(crate) text_rect: Option<Rect>,
    pub(crate) mermaid_rect: Option<Rect>,
    pub(crate) mermaid_label: Option<String>,
    pub(crate) commit_rect: Option<Rect>,
    pub(crate) session_id: String,
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
    if width <= 4 {
        return HeaderFilterLayout::default();
    }

    let left_x = 2;
    let right_edge = width.saturating_sub(2);
    if right_edge <= left_x {
        return HeaderFilterLayout::default();
    }

    let filter_out_label = "[filter out]";
    let filter_out_width = display_width(filter_out_label);
    let clear_label = "[clear filters]";
    let clear_width = display_width(clear_label);
    let gap: u16 = 2;
    let mut available_width = right_edge.saturating_sub(left_x);

    if filter_out_width > available_width {
        return HeaderFilterLayout::default();
    }

    available_width = available_width.saturating_sub(filter_out_width);

    let show_clear =
        app.thought_filter.is_active() && available_width >= gap.saturating_add(clear_width);
    if show_clear {
        available_width = available_width.saturating_sub(gap.saturating_add(clear_width));
    }

    let chip_budget = if available_width > gap {
        available_width.saturating_sub(gap)
    } else {
        0
    };
    let mut included = Vec::new();
    let active_cwd = app.thought_filter.cwd.as_deref();
    let mut chips_width: u16 = 0;
    for summary in app.header_repo_summaries() {
        let is_include_active = active_cwd.map(|cwd| cwd == summary.cwd).unwrap_or(false);
        let is_excluded = app.thought_filter.excludes_cwd(&summary.cwd);
        let label = if is_include_active {
            "code .".to_string()
        } else {
            format!("{}x{}", summary.count, summary.label)
        };
        let width = display_width(&label);
        if width == 0 {
            continue;
        }

        let next_width = if included.is_empty() {
            width
        } else {
            chips_width.saturating_add(2).saturating_add(width)
        };
        if next_width > chip_budget {
            break;
        }

        chips_width = next_width;
        let color = if app.thought_filter.filter_out_mode {
            if is_excluded {
                Color::DarkGrey
            } else {
                summary.color
            }
        } else if active_cwd.is_some() && !is_include_active {
            Color::DarkGrey
        } else {
            summary.color
        };
        included.push((summary.cwd, label, color, width));
    }

    let mut total_width = filter_out_width;
    if show_clear {
        total_width = total_width.saturating_add(gap).saturating_add(clear_width);
    }
    if chips_width > 0 {
        total_width = total_width.saturating_add(gap).saturating_add(chips_width);
    }
    let mut cursor_x = right_edge.saturating_sub(total_width);

    let clear_filters_rect = show_clear.then_some(Rect {
        x: cursor_x,
        y: header_filter_row(),
        width: clear_width,
        height: 1,
    });
    if show_clear {
        cursor_x = cursor_x.saturating_add(clear_width).saturating_add(gap);
    }

    let filter_out_rect = Some(Rect {
        x: cursor_x,
        y: header_filter_row(),
        width: filter_out_width,
        height: 1,
    });
    cursor_x = cursor_x.saturating_add(filter_out_width);
    if chips_width > 0 {
        cursor_x = cursor_x.saturating_add(gap);
    }

    let chips = included
        .into_iter()
        .map(|(cwd, label, color, width)| {
            let rect = Rect {
                x: cursor_x,
                y: header_filter_row(),
                width,
                height: 1,
            };
            cursor_x = cursor_x.saturating_add(width).saturating_add(2);
            ThoughtChipLayout {
                rect,
                cwd,
                label,
                color,
            }
        })
        .collect::<Vec<_>>();

    HeaderFilterLayout {
        chips,
        filter_out_rect,
        clear_filters_rect,
    }
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
        renderer.draw_text(rect.x, rect.y, "[filter out]", color);
    }

    for chip in &layout.chips {
        renderer.draw_text(chip.rect.x, chip.rect.y, &chip.label, chip.color);
    }

    if let Some(rect) = layout.clear_filters_rect {
        renderer.draw_text(rect.x, rect.y, "[clear filters]", Color::Cyan);
    }
}

pub(crate) fn header_filter_action_at<C: TuiApi>(
    app: &App<C>,
    width: u16,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    let layout = build_header_filter_layout(app, width);
    if let Some(rect) = layout.filter_out_rect {
        if rect.contains(x, y) {
            return Some(ThoughtPanelAction::ToggleFilterOutMode);
        }
    }
    if let Some(rect) = layout.clear_filters_rect {
        if rect.contains(x, y) {
            return Some(ThoughtPanelAction::ClearFilters);
        }
    }

    for chip in layout.chips {
        if chip.rect.contains(x, y) {
            if app.thought_filter.filter_out_mode {
                return Some(ThoughtPanelAction::ToggleFilterOutCwd(chip.cwd));
            }
            if app
                .thought_filter
                .cwd
                .as_deref()
                .map(|cwd| cwd == chip.cwd)
                .unwrap_or(false)
            {
                return Some(ThoughtPanelAction::OpenRepoInEditor(chip.cwd));
            }
            return Some(ThoughtPanelAction::FilterByCwd(chip.cwd));
        }
    }

    None
}

pub(crate) fn display_width(text: &str) -> u16 {
    text.chars().count().min(u16::MAX as usize) as u16
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

pub(crate) fn thought_session_label(pwd_label: Option<&str>, tmux_name: &str) -> String {
    match pwd_label.map(str::trim).filter(|label| !label.is_empty()) {
        Some(pwd_label) if !tmux_name.trim().is_empty() => format!("{pwd_label}/{tmux_name}"),
        Some(pwd_label) => pwd_label.to_string(),
        None if !tmux_name.trim().is_empty() => tmux_name.to_string(),
        None => "session".to_string(),
    }
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
        if remaining.chars().count() <= max_chars {
            lines.push(remaining.to_string());
            break;
        }

        let mut char_count = 0usize;
        let mut split_at = 0usize;
        let mut last_space = None;
        for (idx, ch) in remaining.char_indices() {
            char_count += 1;
            if char_count > max_chars {
                break;
            }
            split_at = idx + ch.len_utf8();
            if ch.is_whitespace() {
                last_space = Some(idx);
            }
        }

        let break_idx = last_space.unwrap_or(split_at).max(1);
        let (line, rest) = remaining.split_at(break_idx);
        lines.push(line.trim_end().to_string());
        remaining = rest.trim_start();
    }

    lines
}

#[derive(Clone, Debug)]
pub(crate) struct ThoughtPanelEntryView {
    pub(crate) session_id: String,
    pub(crate) label: String,
    pub(crate) tmux_name: String,
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) color: Color,
    pub(crate) text: String,
    pub(crate) has_objective_shift: bool,
    pub(crate) mermaid_label: Option<String>,
    pub(crate) has_commit_candidate: bool,
}

pub(crate) const DARK_TERMINAL_BG_RGB: (u8, u8, u8) = (0x11, 0x11, 0x11);
pub(crate) const MIN_DARK_TERMINAL_CONTRAST: f64 = 4.5;
pub(crate) const DARK_TERMINAL_COLOR_SEARCH_STEPS: usize = 12;

pub(crate) fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let trimmed = value.trim();
    if trimmed.len() != 7 || !trimmed.starts_with('#') {
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
    session_theme_color(session, repo_themes)
        .unwrap_or_else(|| name_based_color(&session.tmux_name))
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

    let rgb = hsl_to_rgb_tuple(hue, saturation, lightness);
    rgb_color(adjust_for_dark_terminal(rgb))
}

fn hsl_to_rgb_tuple(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = (h % 360.0) / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime {
        hp if hp < 1.0 => (c, x, 0.0),
        hp if hp < 2.0 => (x, c, 0.0),
        hp if hp < 3.0 => (0.0, c, x),
        hp if hp < 4.0 => (0.0, x, c),
        hp if hp < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_byte = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to_byte(r1), to_byte(g1), to_byte(b1))
}

pub(crate) fn compare_thought_panel_entries(
    left: &ThoughtPanelEntryView,
    right: &ThoughtPanelEntryView,
) -> Ordering {
    left.has_objective_shift
        .cmp(&right.has_objective_shift)
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
            updated_at: entry.updated_at,
            color: app.thought_entry_display_color(entry),
            text: format!("{label}: {}", entry.thought.replace('\n', " ")),
            has_objective_shift: entry.objective_changed,
            mermaid_label: app
                .mermaid_artifacts
                .get(&entry.session_id)
                .filter(|artifact| artifact.available)
                .map(|artifact| mermaid_badge_label(artifact.slice_name.as_deref())),
            has_commit_candidate: entry.commit_candidate,
        });
    }

    for entity in app.visible_entities() {
        if thought_sessions.contains(&entity.session.session_id) {
            continue;
        }
        let Some(artifact) = app.mermaid_artifacts.get(&entity.session.session_id) else {
            continue;
        };
        let cwd_label = path_tail_label(&entity.session.cwd);
        let label = thought_session_label(cwd_label.as_deref(), &entity.session.tmux_name);
        entries.push(ThoughtPanelEntryView {
            session_id: entity.session.session_id.clone(),
            label: label.clone(),
            tmux_name: entity.session.tmux_name.clone(),
            updated_at: artifact.updated_at,
            color: session_display_color(&entity.session, &app.repo_themes),
            text: format!("{label}: mermaid diagram ready"),
            has_objective_shift: false,
            mermaid_label: Some(mermaid_badge_label(artifact.slice_name.as_deref())),
            has_commit_candidate: entity.session.commit_candidate,
        });
    }

    entries.sort_by(compare_thought_panel_entries);
    entries
}

pub(crate) fn build_rows_for_panel_entry(
    entry: &ThoughtPanelEntryView,
    thought_content: Rect,
) -> Vec<ThoughtRowLayout> {
    let shift_width = display_width(THOUGHT_SHIFT_LABEL);
    let mermaid_width = entry
        .mermaid_label
        .as_deref()
        .map(display_width)
        .unwrap_or(0);
    let commit_width = display_width(THOUGHT_COMMIT_LABEL);
    let mut reserved: u16 = 0;
    if entry.has_objective_shift {
        reserved = reserved.saturating_add(shift_width).saturating_add(1);
    }
    if entry.mermaid_label.is_some() {
        reserved = reserved.saturating_add(mermaid_width).saturating_add(1);
    }
    if entry.has_commit_candidate {
        reserved = reserved.saturating_add(commit_width).saturating_add(1);
    }
    let text_x = thought_content.x.saturating_add(reserved);
    let text_width = thought_content.width.saturating_sub(reserved) as usize;
    let wrapped = if text_width == 0 {
        vec![String::new()]
    } else {
        wrap_text(&entry.text, text_width)
    };

    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let visible_line_width = display_width(&line);
            let mut badge_x = thought_content.x;
            let shift_rect = (index == 0 && entry.has_objective_shift).then_some(Rect {
                x: badge_x,
                y: 0,
                width: shift_width,
                height: 1,
            });
            if entry.has_objective_shift {
                badge_x = badge_x.saturating_add(shift_width).saturating_add(1);
            }
            let mermaid_rect = (index == 0 && entry.mermaid_label.is_some()).then_some(Rect {
                x: badge_x,
                y: 0,
                width: mermaid_width,
                height: 1,
            });
            if entry.mermaid_label.is_some() {
                badge_x = badge_x.saturating_add(mermaid_width).saturating_add(1);
            }
            ThoughtRowLayout {
                shift_rect,
                text_rect: (visible_line_width > 0).then_some(Rect {
                    x: text_x,
                    y: 0,
                    width: visible_line_width,
                    height: 1,
                }),
                mermaid_rect,
                mermaid_label: if index == 0 {
                    entry.mermaid_label.clone()
                } else {
                    None
                },
                commit_rect: (index == 0 && entry.has_commit_candidate).then_some(Rect {
                    x: badge_x,
                    y: 0,
                    width: commit_width,
                    height: 1,
                }),
                session_id: entry.session_id.clone(),
                label: entry.label.clone(),
                tmux_name: entry.tmux_name.clone(),
                line,
                color: entry.color,
            }
        })
        .collect()
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

    renderer.draw_text(
        thought_content.x,
        thought_content.y,
        &truncate_label("clawgs", thought_content.width as usize),
        Color::Cyan,
    );

    if entry_capacity == 0 {
        return;
    }

    if let Some(message) = panel.empty_message.as_deref() {
        renderer.draw_text(
            thought_content.x,
            thought_content.y + THOUGHT_RAIL_HEADER_ROWS,
            &truncate_label(message, thought_content.width as usize),
            Color::DarkGrey,
        );
        return;
    }

    let start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    for (offset, row) in panel.rows.iter().enumerate() {
        let y = start_y + offset as u16;
        if let Some(rect) = row.shift_rect {
            renderer.draw_text(rect.x, y, THOUGHT_SHIFT_LABEL, Color::Yellow);
        }
        if let (Some(rect), Some(label)) = (row.mermaid_rect, &row.mermaid_label) {
            renderer.draw_text(rect.x, y, label, Color::Cyan);
        }
        if let Some(rect) = row.commit_rect {
            renderer.draw_text(rect.x, y, THOUGHT_COMMIT_LABEL, Color::DarkGrey);
        }
        if let Some(rect) = row.text_rect {
            renderer.draw_text(rect.x, y, &row.line, row.color);
        }
    }
}

pub(crate) fn build_thought_panel<C: TuiApi>(
    app: &App<C>,
    thought_content: Rect,
    entry_capacity: usize,
) -> ThoughtPanelLayout {
    if thought_content.width == 0 || thought_content.height == 0 {
        return ThoughtPanelLayout::default();
    }

    let entries = build_thought_panel_entries(app);
    let empty_message = if entry_capacity == 0 {
        None
    } else if entries.is_empty() {
        Some(if app.thought_filter.is_active() {
            "no thoughts match filters".to_string()
        } else {
            "waiting for clawgs...".to_string()
        })
    } else {
        None
    };

    let mut rows_rev = Vec::new();
    let mut remaining = entry_capacity;
    for entry in entries.iter().rev() {
        let entry_rows = build_rows_for_panel_entry(entry, thought_content);
        if entry_rows.is_empty() || remaining == 0 {
            continue;
        }
        if entry_rows.len() > remaining && !rows_rev.is_empty() {
            break;
        }
        let take = entry_rows.len().min(remaining);
        rows_rev.extend(entry_rows.into_iter().rev().take(take));
        remaining = remaining.saturating_sub(take);
        if remaining == 0 {
            break;
        }
    }
    rows_rev.reverse();

    ThoughtPanelLayout {
        rows: rows_rev,
        empty_message,
    }
}

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
        let text_rect = row.text_rect.map(|rect| Rect {
            x: rect.x,
            y: row_start_y + offset as u16,
            width: rect.width,
            height: rect.height,
        });
        let commit_rect = row.commit_rect.map(|rect| Rect {
            x: rect.x,
            y: row_start_y + offset as u16,
            width: rect.width,
            height: rect.height,
        });
        let mermaid_rect = row.mermaid_rect.map(|rect| Rect {
            x: rect.x,
            y: row_start_y + offset as u16,
            width: rect.width,
            height: rect.height,
        });
        if commit_rect.map(|rect| rect.contains(x, y)).unwrap_or(false) {
            return Some(ThoughtPanelAction::LaunchCommitCodex(
                row.session_id.clone(),
            ));
        }
        if mermaid_rect
            .map(|rect| rect.contains(x, y))
            .unwrap_or(false)
        {
            return Some(ThoughtPanelAction::OpenMermaid(row.session_id.clone()));
        }
        if text_rect.map(|rect| rect.contains(x, y)).unwrap_or(false) {
            return Some(ThoughtPanelAction::OpenSession {
                session_id: row.session_id.clone(),
                label: row.label.clone(),
            });
        }
    }

    None
}
