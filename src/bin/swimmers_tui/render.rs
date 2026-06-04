use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BALLS_BODY_HEIGHT: u16 = 3;
const BALLS_BALL_WIDTH: u16 = 5;

#[cfg(test)]
pub(crate) fn render_entity(
    renderer: &mut Renderer,
    entity: &SessionEntity,
    rect: Rect,
    selected: bool,
    tick: u64,
    repo_themes: &HashMap<String, RepoTheme>,
) {
    render_entity_with_theme(
        renderer,
        entity,
        rect,
        selected,
        tick,
        repo_themes,
        SpriteTheme::Fish,
    );
}

#[derive(Clone, Copy)]
pub(crate) struct BallsThemeSlot<'a> {
    pub(crate) entity: &'a SessionEntity,
    pub(crate) anchor_x: u16,
    pub(crate) cord_top_y: u16,
    pub(crate) ball_x: u16,
    pub(crate) ball_y: u16,
    pub(crate) ball_width: u16,
    pub(crate) ball_height: u16,
    pub(crate) on_floor: bool,
}

impl<'a> BallsThemeSlot<'a> {
    pub(crate) fn hit_rect(self) -> Rect {
        Rect {
            x: self.ball_x,
            y: self.cord_top_y,
            width: self.ball_width,
            height: self
                .ball_y
                .saturating_add(self.ball_height)
                .saturating_sub(self.cord_top_y),
        }
    }
}

fn balls_theme_body_rect(count: usize, field: Rect) -> Rect {
    let max_width = field.width.saturating_sub(2);
    let desired_width = (count as u16)
        .saturating_mul(5)
        .saturating_add(12)
        .min(max_width)
        .max(max_width.min(18));
    Rect {
        x: field.x + field.width.saturating_sub(desired_width) / 2,
        y: field.y,
        width: desired_width,
        height: BALLS_BODY_HEIGHT,
    }
}

fn balls_theme_centers(count: usize, body: Rect) -> Vec<u16> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![body.x + body.width / 2];
    }

    let left = body.x.saturating_add(2);
    let span = body.width.saturating_sub(5);
    (0..count)
        .map(|index| left + ((index as u32 * span as u32) / (count as u32 - 1)) as u16)
        .collect()
}

fn balls_theme_ball_height(kind: SpriteKind) -> u16 {
    match kind {
        SpriteKind::Attention => 2,
        SpriteKind::Active | SpriteKind::Busy | SpriteKind::Error => 3,
        SpriteKind::Drowsy | SpriteKind::Sleeping | SpriteKind::DeepSleep | SpriteKind::Exited => 4,
    }
}

fn balls_theme_base_drop(kind: SpriteKind) -> u16 {
    match kind {
        SpriteKind::Attention => 1,
        SpriteKind::Active => 3,
        SpriteKind::Busy => 4,
        SpriteKind::Error => 5,
        SpriteKind::Drowsy => 7,
        SpriteKind::Sleeping => 10,
        SpriteKind::DeepSleep | SpriteKind::Exited => 12,
    }
}

fn balls_theme_age_bonus(age_rank: usize, count: usize, kind: SpriteKind) -> u16 {
    if count <= 1 || matches!(kind, SpriteKind::Attention) {
        return 0;
    }
    let max_rank = count - 1;
    let max_bonus = 3usize;
    ((max_rank.saturating_sub(age_rank)) * max_bonus / max_rank) as u16
}

pub(crate) fn balls_theme_slots<'a>(
    entities: &[&'a SessionEntity],
    field: Rect,
) -> Vec<BallsThemeSlot<'a>> {
    if entities.is_empty() || field.height <= BALLS_BODY_HEIGHT + 2 {
        return Vec::new();
    }

    let body = balls_theme_body_rect(entities.len(), field);
    let centers = balls_theme_centers(entities.len(), body);
    // `balls_theme_centers` returns one entry per entity, so this should
    // always succeed for non-empty entities; fall back to the body x to keep
    // the `&centers[0]` indexing footgun out of the hot path entirely.
    let Some(&first_center) = centers.first() else {
        return Vec::new();
    };

    let mut ordered = entities.to_vec();
    ordered.sort_by(|left, right| compare_tmux_natural(&left.session, &right.session));

    let mut sleepiness_order = entities.to_vec();
    sleepiness_order.sort_by(|left, right| compare_sleepiness(&left.session, &right.session));
    let sleepiness_ranks = sleepiness_order
        .iter()
        .enumerate()
        .map(|(rank, entity)| (entity.session.session_id.as_str(), rank))
        .collect::<HashMap<_, _>>();

    let floor_y = field.bottom().saturating_sub(1);
    let cord_top_y = body.bottom();
    let mut slots = Vec::with_capacity(ordered.len());
    for (slot_index, entity) in ordered.into_iter().enumerate() {
        let anchor_x = centers.get(slot_index).copied().unwrap_or(first_center);
        let kind = entity.sprite_kind();
        let ball_height = balls_theme_ball_height(kind);
        let age_rank = sleepiness_ranks
            .get(entity.session.session_id.as_str())
            .copied()
            .unwrap_or(slot_index);
        let mut drop =
            balls_theme_base_drop(kind) + balls_theme_age_bonus(age_rank, entities.len(), kind);
        if matches!(kind, SpriteKind::DeepSleep | SpriteKind::Exited) {
            drop = drop.max(field.height.saturating_sub(BALLS_BODY_HEIGHT + ball_height));
        }
        let ball_y = (cord_top_y + drop).min(floor_y.saturating_add(1).saturating_sub(ball_height));
        let max_ball_x = field.right().saturating_sub(BALLS_BALL_WIDTH);
        let ball_x = if max_ball_x >= field.x {
            anchor_x
                .saturating_sub(BALLS_BALL_WIDTH / 2)
                .clamp(field.x, max_ball_x)
        } else {
            field.x
        };
        slots.push(BallsThemeSlot {
            entity,
            anchor_x,
            cord_top_y,
            ball_x,
            ball_y,
            ball_width: BALLS_BALL_WIDTH,
            ball_height,
            on_floor: ball_y.saturating_add(ball_height) >= field.bottom(),
        });
    }
    slots
}

pub(crate) fn balls_theme_hit_test<'a>(
    entities: &[&'a SessionEntity],
    field: Rect,
    x: u16,
    y: u16,
) -> Option<&'a SessionEntity> {
    balls_theme_slots(entities, field)
        .into_iter()
        .find(|slot| slot.hit_rect().contains(x, y))
        .map(|slot| slot.entity)
}

fn render_balls_theme_body(
    renderer: &mut Renderer,
    body: Rect,
    slots: &[BallsThemeSlot<'_>],
    tick: u64,
) {
    if body.width < 6 {
        return;
    }

    let ripple = if tick % 12 < 6 { '~' } else { '^' };
    renderer.draw_char(body.x, body.y, '.', Color::DarkGrey);
    renderer.draw_hline(body.x + 1, body.y, body.width - 2, ripple, Color::DarkGrey);
    renderer.draw_char(body.right() - 1, body.y, '.', Color::DarkGrey);
    renderer.draw_char(body.x, body.y + 1, '/', Color::DarkGrey);
    renderer.draw_hline(body.x + 1, body.y + 1, body.width - 2, ' ', Color::DarkGrey);
    renderer.draw_char(body.right() - 1, body.y + 1, '\\', Color::DarkGrey);
    renderer.draw_char(body.x, body.bottom() - 1, '(', Color::DarkGrey);
    renderer.draw_hline(
        body.x + 1,
        body.bottom() - 1,
        body.width - 2,
        '_',
        Color::DarkGrey,
    );
    renderer.draw_char(body.right() - 1, body.bottom() - 1, ')', Color::DarkGrey);

    for slot in slots {
        if slot.anchor_x > body.x && slot.anchor_x < body.right().saturating_sub(1) {
            renderer.draw_char(slot.anchor_x, body.bottom() - 1, 'v', Color::DarkGrey);
        }
    }
}

fn balls_theme_cord_style(
    kind: SpriteKind,
    unverified: bool,
    selected: bool,
) -> BallsThemeCordStyle {
    BallsThemeCordStyle {
        ch: balls_theme_cord_char(kind, unverified),
        color: if selected {
            Color::White
        } else {
            Color::DarkGrey
        },
    }
}

fn balls_theme_cord_char(kind: SpriteKind, unverified: bool) -> char {
    if unverified {
        // Sparse/dashed cord; visually distinct from `|`, `:`, `!`, `x`.
        '\''
    } else {
        BALLS_CORD_CHARS_BY_KIND[kind as usize]
    }
}

fn balls_theme_body_rows(
    kind: SpriteKind,
    unverified: bool,
    on_floor: bool,
) -> &'static [&'static str] {
    if unverified {
        // Ghost body with an explicit `?` overlay; drop distance still comes
        // from the detected kind so uncertainty does not erase state shape.
        return BALLS_UNVERIFIED_BODY_ROWS;
    }
    balls_theme_verified_body_rows(kind, on_floor)
}

fn balls_theme_verified_body_rows(kind: SpriteKind, on_floor: bool) -> &'static [&'static str] {
    let rows_by_kind = if on_floor {
        &BALLS_FLOOR_BODY_ROWS_BY_KIND
    } else {
        &BALLS_AIR_BODY_ROWS_BY_KIND
    };
    rows_by_kind[kind as usize]
}

fn render_balls_theme_ball(
    renderer: &mut Renderer,
    slot: BallsThemeSlot<'_>,
    selected: bool,
    color: Color,
) {
    let kind = slot.entity.sprite_kind();
    // Whether to overlay an "uncertain" mark on the ball. We reuse the same
    // predicate that already adds `?` to the text label so the visual signal
    // and the label suffix stay in lock-step. Observed medium-confidence
    // detector output is still useful live state; cache fallbacks and unhealthy
    // transport are the cases where an operator should not trust a
    // confident-looking ball.
    let unverified = session_state_evidence_unverified(&slot.entity.session);
    let cord = balls_theme_cord_style(kind, unverified, selected);
    for y in slot.cord_top_y..slot.ball_y {
        renderer.draw_char(slot.anchor_x, y, cord.ch, cord.color);
    }

    let rows = balls_theme_body_rows(kind, unverified, slot.on_floor);
    for (dy, row) in rows.iter().enumerate() {
        renderer.draw_text(slot.ball_x, slot.ball_y + dy as u16, row, color);
    }

    if selected {
        renderer.draw_char(
            slot.anchor_x,
            slot.cord_top_y.saturating_sub(1),
            '^',
            Color::White,
        );
    }
}

pub(crate) fn render_balls_theme(
    renderer: &mut Renderer,
    field: Rect,
    entities: &[&SessionEntity],
    selected_id: Option<&str>,
    repo_themes: &HashMap<String, RepoTheme>,
    tick: u64,
) {
    if entities.is_empty() {
        return;
    }

    let body = balls_theme_body_rect(entities.len(), field);
    let slots = balls_theme_slots(entities, field);
    render_balls_theme_body(renderer, body, &slots, tick);
    renderer.draw_hline(
        field.x,
        field.bottom().saturating_sub(1),
        field.width,
        '_',
        Color::DarkGrey,
    );
    for slot in slots {
        let selected = selected_id
            .map(|selected_id| selected_id == slot.entity.session.session_id)
            .unwrap_or(false);
        let color = session_display_color(&slot.entity.session, repo_themes);
        render_balls_theme_ball(renderer, slot, selected, color);
    }
}

pub(crate) fn render_entity_with_theme(
    renderer: &mut Renderer,
    entity: &SessionEntity,
    rect: Rect,
    selected: bool,
    tick: u64,
    repo_themes: &HashMap<String, RepoTheme>,
    theme: SpriteTheme,
) {
    let kind = entity.sprite_kind();
    let color = session_display_color(&entity.session, repo_themes);

    let sprite = kind.frame_with_theme(tick, theme);
    for (dy, line) in sprite.iter().enumerate() {
        renderer.draw_text(rect.x, rect.y + dy as u16, line, color);
    }

    // Mirror the dangling-ball ghost mark on the fish/jelly themes. Every
    // sprite frame is 12 cols wide and reserves the trailing column as
    // breathing room (see entity.rs frame data), so overdrawing a `?` at the
    // top-right corner is a safe, theme-agnostic uncertainty marker.
    if session_state_evidence_unverified(&entity.session) {
        renderer.draw_char(
            rect.x + ENTITY_WIDTH.saturating_sub(2),
            rect.y,
            '?',
            Color::DarkGrey,
        );
    }

    if selected {
        if rect.x > 0 {
            renderer.draw_char(rect.x - 1, rect.y + 1, '>', Color::White);
        }
        let label = truncate_label(&entity.session.tmux_name, ENTITY_WIDTH as usize);
        renderer.draw_text(rect.x, rect.y + SPRITE_HEIGHT, &label, Color::White);
    } else {
        let label = truncate_label(&entity.session.tmux_name, ENTITY_WIDTH as usize);
        renderer.draw_text(rect.x, rect.y + SPRITE_HEIGHT, &label, Color::DarkGrey);
    }
}

pub(crate) fn render_help_overlay(renderer: &mut Renderer, field: Rect) {
    let lines: &[(&str, &str)] = &[
        ("arrows/hjkl", "move selection"),
        ("enter/click", "open selected session"),
        ("click field", "spawn new session"),
        ("n", "cycle terminal handoff target"),
        ("m", "cycle Ghostty placement"),
        ("s", "cycle sprite theme"),
        ("t", "thought config editor"),
        ("tab", "toggle thought grouping"),
        (">", "toggle asleep / all view"),
        ("A", "reattach selected session"),
        ("r", "refresh sessions"),
        ("q / esc", "quit"),
        ("?", "this help"),
    ];

    let key_col_width = 12u16;
    let content_width = lines
        .iter()
        .fold(0u16, |acc, (_key, desc)| {
            acc.max(key_col_width + display_width(desc))
        })
        .saturating_add(4);
    let content_height = lines.len() as u16 + 4;

    let overlay_width = content_width.min(field.width.saturating_sub(2));
    let overlay_height = content_height.min(field.height.saturating_sub(2));
    if overlay_width < 20 || overlay_height < 6 {
        return;
    }

    let x = field.x + field.width.saturating_sub(overlay_width) / 2;
    let y = field.y + field.height.saturating_sub(overlay_height) / 2;
    let overlay = Rect {
        x,
        y,
        width: overlay_width,
        height: overlay_height,
    };

    renderer.fill_rect(overlay, ' ', Color::DarkGrey);
    renderer.draw_box(overlay, Color::Cyan);
    renderer.draw_text(x + 2, y + 1, "keybindings", Color::Cyan);

    let max_desc_width = overlay_width.saturating_sub(key_col_width + 4) as usize;
    for (row, (key, desc)) in lines.iter().enumerate() {
        let row_y = y + 3 + row as u16;
        if row_y >= overlay.bottom().saturating_sub(1) {
            break;
        }
        renderer.draw_text(x + 2, row_y, key, Color::White);
        renderer.draw_text(
            x + 2 + key_col_width,
            row_y,
            &truncate_label(desc, max_desc_width),
            Color::DarkGrey,
        );
    }
}

pub(crate) fn render_footer<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, start_y: u16) {
    if start_y >= renderer.height() {
        return;
    }

    let max_width = footer_text_width(renderer);
    render_footer_selection(app, renderer, start_y, max_width);
    render_footer_line(
        renderer,
        start_y + 2,
        footer_help_text(app),
        Color::Cyan,
        max_width,
    );

    if let Some(message) = app.visible_message() {
        render_footer_line(renderer, start_y + 3, message, Color::Yellow, max_width);
    }
}

fn footer_text_width(renderer: &Renderer) -> usize {
    renderer.width().saturating_sub(4) as usize
}

fn render_footer_selection<C: TuiApi>(
    app: &App<C>,
    renderer: &mut Renderer,
    start_y: u16,
    max_width: usize,
) {
    let Some(selected) = app.selected() else {
        renderer.draw_text(2, start_y, "selected: none", Color::DarkGrey);
        return;
    };

    render_footer_line(
        renderer,
        start_y,
        &footer_selected_state_line(selected),
        Color::White,
        max_width,
    );
    render_footer_line(
        renderer,
        start_y + 1,
        &footer_selected_command_line(selected),
        Color::DarkGrey,
        max_width,
    );
}

fn footer_selected_state_line(selected: &SessionEntity) -> String {
    format!(
        "selected: {} [{}; {}] {}",
        selected.session.tmux_name,
        session_state_text(&selected.session),
        session_state_evidence_text(&selected.session),
        shorten_path(&selected.session.cwd, 42)
    )
}

fn footer_selected_command_line(selected: &SessionEntity) -> String {
    let cmd = selected
        .session
        .current_command
        .as_deref()
        .unwrap_or("idle");
    format!("cmd: {}", shorten_path(cmd, 60))
}

fn footer_help_text<C: TuiApi>(app: &App<C>) -> &'static str {
    if app.initial_request.is_some() {
        return request_footer_help_text(&app.voice_state);
    }
    footer_mode_help_text(FooterHelpState::from_app(app))
}

#[derive(Clone, Copy)]
struct FooterHelpState {
    thought_config_editor: bool,
    picker: bool,
    show_help: bool,
}

impl FooterHelpState {
    fn from_app<C: TuiApi>(app: &App<C>) -> Self {
        Self {
            thought_config_editor: app.thought_config_editor.is_some(),
            picker: app.picker.is_some(),
            show_help: app.show_help,
        }
    }
}

fn footer_mode_help_text(state: FooterHelpState) -> &'static str {
    if state.thought_config_editor {
        return "thought config: tab moves  arrows adjust  enter saves  esc cancels";
    }
    if state.picker {
        return "picker: type search  enter/right select  B batch  X exclude  backspace up  esc close";
    }
    if state.show_help {
        return "press any key to dismiss help";
    }
    "arrows/hjkl move  enter open  r refresh  t config  ? help  q quit"
}

fn request_footer_help_text(voice_state: &VoiceUiState) -> &'static str {
    match voice_state {
        VoiceUiState::Unsupported => {
            "request: type prompt  enter create  backspace delete  esc cancel"
        }
        _ => "request: type prompt  ctrl-v voice  enter create  backspace delete  esc cancel",
    }
}

fn render_footer_line(renderer: &mut Renderer, y: u16, text: &str, color: Color, max_width: usize) {
    renderer.draw_text(2, y, &truncate_label(text, max_width), color);
}

pub(crate) fn render_too_small(renderer: &mut Renderer) {
    renderer.draw_text(2, 1, "swimmers tui", Color::Cyan);
    renderer.draw_text(
        2,
        3,
        &format!(
            "terminal too small - need at least {}x{}",
            MIN_WIDTH, MIN_HEIGHT
        ),
        Color::Red,
    );
    renderer.draw_text(
        2,
        5,
        "resize the terminal and reopen the TUI",
        Color::DarkGrey,
    );
}

pub(crate) fn render_aquarium_background(renderer: &mut Renderer, field: Rect, tick: u64) {
    if aquarium_background_too_small(field) {
        return;
    }

    let width = aquarium_scroll_width(field);
    render_aquarium_bubbles(renderer, field, tick, width);
    render_aquarium_sparkles(renderer, field, tick, width);
}

pub(crate) fn pluralize(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn text_display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn trim_trailing_joiners(text: &mut String) {
    while text.ends_with('\u{200c}') || text.ends_with('\u{200d}') {
        text.pop();
    }
}

#[derive(Clone, Copy)]
struct DisplayWidthBudget {
    max_cols: usize,
    used_cols: usize,
}

impl DisplayWidthBudget {
    fn new(max_cols: usize) -> Self {
        Self {
            max_cols,
            used_cols: 0,
        }
    }

    fn consume(&mut self, width: usize) -> bool {
        if self.used_cols.saturating_add(width) > self.max_cols {
            return false;
        }
        self.used_cols = self.used_cols.saturating_add(width);
        true
    }
}

fn collect_display_width_prefix(text: &str, mut budget: DisplayWidthBudget) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if !push_display_width_prefix_char(&mut out, &mut budget, ch) {
            break;
        }
    }
    out
}

fn push_display_width_prefix_char(
    out: &mut String,
    budget: &mut DisplayWidthBudget,
    ch: char,
) -> bool {
    let ch_width = char_display_width(ch);
    if ch_width == 0 {
        push_non_leading_zero_width_char(out, ch);
        return true;
    }
    if !budget.consume(ch_width) {
        return false;
    }
    out.push(ch);
    true
}

fn push_non_leading_zero_width_char(out: &mut String, ch: char) {
    if !out.is_empty() {
        out.push(ch);
    }
}

fn prefix_by_display_width(text: &str, max_cols: usize) -> String {
    let mut out = collect_display_width_prefix(text, DisplayWidthBudget::new(max_cols));
    trim_trailing_joiners(&mut out);
    out
}

fn suffix_by_display_width(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    let mut start = text.len();
    let mut used = 0usize;
    let mut has_base = false;
    for (idx, ch) in text.char_indices().rev() {
        let ch_width = char_display_width(ch);
        if ch_width == 0 {
            if has_base {
                start = idx;
            }
            continue;
        }
        if used.saturating_add(ch_width) > max_cols {
            break;
        }
        used = used.saturating_add(ch_width);
        has_base = true;
        start = idx;
    }

    let mut out = text[start..].to_string();
    trim_trailing_joiners(&mut out);
    out
}

pub(crate) fn truncate_label(text: &str, max_chars: usize) -> String {
    if text_display_width(text) <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let marker = "~";
    let marker_width = text_display_width(marker);
    if max_chars <= marker_width {
        return prefix_by_display_width(marker, max_chars);
    }
    let mut out = prefix_by_display_width(text, max_chars.saturating_sub(marker_width));
    out.push('~');
    out
}

pub(crate) fn tail_text(text: &str, max_chars: usize) -> String {
    if text_display_width(text) <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    suffix_by_display_width(text, max_chars)
}

pub(crate) fn shorten_path(path: &str, max_chars: usize) -> String {
    if text_display_width(path) <= max_chars {
        return path.to_string();
    }
    if max_chars <= 3 {
        return prefix_by_display_width(path, max_chars);
    }

    let budget = max_chars - 3;
    if let Some(suffix) = path_segment_suffix(path, budget) {
        return format!("...{suffix}");
    }

    let tail = tail_text(path, budget);
    format!("...{tail}")
}

fn path_segment_suffix(path: &str, budget: usize) -> Option<String> {
    if !path.contains('/') {
        return None;
    }
    non_empty_string(path_segment_suffix_with_budget(path, budget))
}

fn path_segment_suffix_with_budget(path: &str, budget: usize) -> String {
    let mut suffix = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()).rev() {
        let candidate = path_suffix_candidate(part, &suffix);
        if text_display_width(&candidate) > budget {
            break;
        }
        suffix = candidate;
    }
    suffix
}

fn path_suffix_candidate(part: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        format!("/{part}")
    } else {
        format!("/{part}{suffix}")
    }
}

fn non_empty_string(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn aquarium_background_too_small(field: Rect) -> bool {
    field.width < 4 || field.height < 4
}

fn aquarium_scroll_width(field: Rect) -> usize {
    usize::from(field.width.max(1))
}

fn render_aquarium_bubbles(renderer: &mut Renderer, field: Rect, tick: u64, width: usize) {
    let scroll = aquarium_bubble_scroll(tick, width);
    let lane_count = aquarium_lane_count(field);
    let lane_spacing = aquarium_lane_spacing(field, lane_count);
    let bottom_y = field.bottom().saturating_sub(1);
    for lane in 0..lane_count {
        let (x, y) =
            aquarium_bubble_position(field, tick, width, lane, lane_spacing, scroll, bottom_y);
        renderer.draw_char(x, y, 'o', Color::DarkCyan);
        if x + 1 < field.right() && y + 1 < field.bottom() {
            renderer.draw_char(x + 1, y + 1, '.', Color::Blue);
        }
    }
}

fn render_aquarium_sparkles(renderer: &mut Renderer, field: Rect, tick: u64, width: usize) {
    for sparkle in 0..aquarium_sparkle_count(field) {
        let (x, y) = aquarium_sparkle_position(field, tick, width, sparkle);
        renderer.draw_char(x, y, '~', Color::DarkBlue);
        if x > field.x {
            renderer.draw_char(x - 1, y, '.', Color::DarkBlue);
        }
    }
}

fn aquarium_lane_count(field: Rect) -> usize {
    usize::from((field.width / 18).clamp(1, 4))
}

fn aquarium_lane_spacing(field: Rect, lane_count: usize) -> u16 {
    (field.width / lane_count as u16).max(1)
}

fn aquarium_bubble_scroll(tick: u64, width: usize) -> usize {
    (tick as usize / 3) % width
}

fn aquarium_bubble_position(
    field: Rect,
    tick: u64,
    width: usize,
    lane: usize,
    lane_spacing: u16,
    scroll: usize,
    bottom_y: u16,
) -> (u16, u16) {
    let base_offset = (2 + lane as u16 * lane_spacing) as usize;
    let x = aquarium_scrolled_x(field, width, base_offset + scroll);
    let rise = ((tick / 4) as u16 + lane as u16 * 4) % field.height.max(1);
    (x, bottom_y.saturating_sub(rise))
}

fn aquarium_sparkle_count(field: Rect) -> usize {
    usize::from((field.width / 24).clamp(1, 3))
}

fn aquarium_sparkle_position(field: Rect, tick: u64, width: usize, sparkle: usize) -> (u16, u16) {
    let x = aquarium_scrolled_x(field, width, (tick as usize / 2) + sparkle * 11);
    let y_span = field.height.saturating_sub(3).max(1);
    let y = field.y + 1 + (((tick / 2) as u16 + sparkle as u16 * 6) % y_span);
    (x, y)
}

fn aquarium_scrolled_x(field: Rect, width: usize, offset: usize) -> u16 {
    field
        .right()
        .saturating_sub(1)
        .saturating_sub((offset % width) as u16)
}

pub(crate) fn session_state_text(session: &SessionSummary) -> &'static str {
    let unverified = session_state_evidence_unverified(session);
    SpriteKind::from_session(session).state_label(unverified)
}

pub(crate) fn session_state_evidence_text(session: &SessionSummary) -> String {
    let confidence = match session.state_evidence.confidence {
        StateConfidence::Low => "low",
        StateConfidence::Medium => "medium",
        StateConfidence::High => "high",
    };
    let freshness = if session.state_evidence.observed_at.is_some() {
        "observed"
    } else {
        "unobserved"
    };
    format!(
        "{} {} {}",
        confidence, freshness, session.state_evidence.cause
    )
}

pub(crate) fn selected_label(name: Option<&String>) -> String {
    name.cloned().unwrap_or_else(|| "session".to_string())
}

const BALLS_UNVERIFIED_BODY_ROWS: &[&str] = &[" .?. ", "( ? )", " `-' "];
const BALLS_ATTENTION_BODY_ROWS: &[&str] = &[" .-. ", "(_!_)"];
const BALLS_ACTIVE_BODY_ROWS: &[&str] = &[" .-. ", "(   )", " `-' "];
const BALLS_BUSY_BODY_ROWS: &[&str] = &[" .-. ", "( * )", " `-' "];
const BALLS_ERROR_BODY_ROWS: &[&str] = &[" .-. ", "( x )", " `-' "];
const BALLS_DROWSY_BODY_ROWS: &[&str] = &[" .-. ", "(   )", "(   )", " `-' "];
const BALLS_SLEEPING_BODY_ROWS: &[&str] = &[" .-. ", "( z )", "(   )", " `-' "];
const BALLS_DEEP_SLEEP_FLOOR_BODY_ROWS: &[&str] = &[" .-. ", "( z )", "(   )", "(___)"];
const BALLS_EXITED_FLOOR_BODY_ROWS: &[&str] = &[" .-. ", "( x )", "(   )", "(___)"];
// Table order mirrors `SpriteKind`; the balls planner tests cover every entry.
const BALLS_CORD_CHARS_BY_KIND: [char; 8] = ['|', ':', '|', '|', '|', '!', 'x', 'x'];
const BALLS_AIR_BODY_ROWS_BY_KIND: [&[&str]; 8] = [
    BALLS_ACTIVE_BODY_ROWS,
    BALLS_BUSY_BODY_ROWS,
    BALLS_DROWSY_BODY_ROWS,
    BALLS_SLEEPING_BODY_ROWS,
    BALLS_SLEEPING_BODY_ROWS,
    BALLS_ATTENTION_BODY_ROWS,
    BALLS_ERROR_BODY_ROWS,
    BALLS_ERROR_BODY_ROWS,
];
const BALLS_FLOOR_BODY_ROWS_BY_KIND: [&[&str]; 8] = [
    BALLS_ACTIVE_BODY_ROWS,
    BALLS_BUSY_BODY_ROWS,
    BALLS_DROWSY_BODY_ROWS,
    BALLS_SLEEPING_BODY_ROWS,
    BALLS_DEEP_SLEEP_FLOOR_BODY_ROWS,
    BALLS_ATTENTION_BODY_ROWS,
    BALLS_ERROR_BODY_ROWS,
    BALLS_EXITED_FLOOR_BODY_ROWS,
];

#[derive(Clone, Copy)]
struct BallsThemeCordStyle {
    ch: char,
    color: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_renderer(width: u16, height: u16) -> Renderer {
        let buffer = vec![Cell::default(); width as usize * height as usize];
        Renderer {
            stdout: BufWriter::new(io::stdout()),
            width,
            height,
            buffer: buffer.clone(),
            last_buffer: buffer,
            terminal_state: TerminalState::default(),
        }
    }

    fn cell_at(renderer: &Renderer, x: u16, y: u16) -> Cell {
        renderer.buffer[y as usize * renderer.width as usize + x as usize]
    }

    #[test]
    fn balls_cord_style_plans_verified_and_unverified_glyphs() {
        let cases = [
            (SpriteKind::Attention, false, false, '!', Color::DarkGrey),
            (SpriteKind::Busy, false, false, ':', Color::DarkGrey),
            (SpriteKind::Error, false, false, 'x', Color::DarkGrey),
            (SpriteKind::Exited, false, false, 'x', Color::DarkGrey),
            (SpriteKind::Active, false, false, '|', Color::DarkGrey),
            (SpriteKind::Drowsy, false, false, '|', Color::DarkGrey),
            (SpriteKind::DeepSleep, false, false, '|', Color::DarkGrey),
            (SpriteKind::Sleeping, true, false, '\'', Color::DarkGrey),
            (SpriteKind::Active, false, true, '|', Color::White),
        ];

        for (kind, unverified, selected, expected_ch, expected_color) in cases {
            let style = balls_theme_cord_style(kind, unverified, selected);

            assert_eq!(style.ch, expected_ch, "{kind:?} cord glyph");
            assert_eq!(style.color, expected_color, "{kind:?} cord color");
        }
    }

    #[test]
    fn balls_body_rows_plan_all_sprite_kinds_and_floor_variants() {
        let cases = [
            (
                SpriteKind::Attention,
                false,
                BALLS_ATTENTION_BODY_ROWS,
                BALLS_ATTENTION_BODY_ROWS,
            ),
            (
                SpriteKind::Active,
                false,
                BALLS_ACTIVE_BODY_ROWS,
                BALLS_ACTIVE_BODY_ROWS,
            ),
            (
                SpriteKind::Busy,
                false,
                BALLS_BUSY_BODY_ROWS,
                BALLS_BUSY_BODY_ROWS,
            ),
            (
                SpriteKind::Error,
                false,
                BALLS_ERROR_BODY_ROWS,
                BALLS_ERROR_BODY_ROWS,
            ),
            (
                SpriteKind::Drowsy,
                false,
                BALLS_DROWSY_BODY_ROWS,
                BALLS_DROWSY_BODY_ROWS,
            ),
            (
                SpriteKind::Sleeping,
                false,
                BALLS_SLEEPING_BODY_ROWS,
                BALLS_SLEEPING_BODY_ROWS,
            ),
            (
                SpriteKind::DeepSleep,
                false,
                BALLS_SLEEPING_BODY_ROWS,
                BALLS_DEEP_SLEEP_FLOOR_BODY_ROWS,
            ),
            (
                SpriteKind::Exited,
                false,
                BALLS_ERROR_BODY_ROWS,
                BALLS_EXITED_FLOOR_BODY_ROWS,
            ),
        ];

        for (kind, unverified, airborne_rows, floor_rows) in cases {
            assert_eq!(
                balls_theme_body_rows(kind, unverified, false),
                airborne_rows,
                "{kind:?} airborne rows"
            );
            assert_eq!(
                balls_theme_body_rows(kind, unverified, true),
                floor_rows,
                "{kind:?} floor rows"
            );
            assert_eq!(
                balls_theme_body_rows(kind, true, false),
                BALLS_UNVERIFIED_BODY_ROWS,
                "{kind:?} unverified airborne rows"
            );
            assert_eq!(
                balls_theme_body_rows(kind, true, true),
                BALLS_UNVERIFIED_BODY_ROWS,
                "{kind:?} unverified floor rows"
            );
        }
    }

    #[test]
    fn prefix_by_display_width_handles_mixed_width_and_zero_width_text() {
        let cases = [
            ("abcdef", 0, ""),
            ("abcdef", 3, "abc"),
            ("漢字a", 4, "漢字"),
            ("漢字a", 5, "漢字a"),
            ("a\u{0301}漢b", 3, "a\u{0301}漢"),
            ("a\u{0301}漢b", 2, "a\u{0301}"),
            ("\u{0301}a", 1, "a"),
            ("漢\u{0301}a", 1, ""),
            ("a\u{200d}b", 1, "a"),
        ];

        for (text, max_cols, expected) in cases {
            let prefix = prefix_by_display_width(text, max_cols);

            assert_eq!(prefix, expected, "{text:?} at {max_cols} cols");
            assert!(
                UnicodeWidthStr::width(prefix.as_str()) <= max_cols,
                "{prefix:?} exceeded {max_cols} cols"
            );
        }
    }

    #[test]
    fn truncate_label_respects_terminal_columns_for_mixed_width_text() {
        let text = "ab漢字a\u{0301}xyz";
        let truncated = truncate_label(text, 7);
        assert_eq!(UnicodeWidthStr::width(truncated.as_str()), 7);
        assert!(truncated.ends_with('~'));
    }

    #[test]
    fn shorten_path_preserves_segment_suffix_and_tail_fallbacks() {
        assert_eq!(shorten_path("/a/b/c/d/e", 8), ".../d/e");
        assert_eq!(shorten_path("/reallylongsegment", 10), "...segment");
        assert_eq!(shorten_path("abcdef", 3), "abc");
        assert_eq!(shorten_path("abcdefa\u{0301}", 5), "...fa\u{0301}");
    }

    #[test]
    fn shorten_path_respects_display_width_for_path_segments() {
        let shortened = shorten_path("/alpha/漢字", 8);

        assert_eq!(shortened, ".../漢字");
        assert_eq!(UnicodeWidthStr::width(shortened.as_str()), 8);
    }

    #[test]
    fn aquarium_background_glyphs_preserve_positions_and_colors() {
        let field = Rect {
            x: 2,
            y: 3,
            width: 40,
            height: 10,
        };
        let tick = 12;
        let mut renderer = test_renderer(50, 20);

        render_aquarium_background(&mut renderer, field, tick);

        assert_eq!(
            cell_at(&renderer, 35, 9),
            Cell {
                ch: 'o',
                fg: Color::DarkCyan
            }
        );
        assert_eq!(
            cell_at(&renderer, 36, 10),
            Cell {
                ch: '.',
                fg: Color::Blue
            }
        );
        assert_eq!(
            cell_at(&renderer, 15, 5),
            Cell {
                ch: 'o',
                fg: Color::DarkCyan
            }
        );
        assert_eq!(
            cell_at(&renderer, 16, 6),
            Cell {
                ch: '.',
                fg: Color::Blue
            }
        );
        assert_eq!(
            cell_at(&renderer, 35, 10),
            Cell {
                ch: '~',
                fg: Color::DarkBlue
            }
        );
        assert_eq!(
            cell_at(&renderer, 34, 10),
            Cell {
                ch: '.',
                fg: Color::DarkBlue
            }
        );
    }

    #[test]
    fn footer_mode_help_text_preserves_priority_order() {
        assert_eq!(
            footer_mode_help_text(FooterHelpState {
                thought_config_editor: true,
                picker: true,
                show_help: true,
            }),
            "thought config: tab moves  arrows adjust  enter saves  esc cancels"
        );
        assert_eq!(
            footer_mode_help_text(FooterHelpState {
                thought_config_editor: false,
                picker: true,
                show_help: true,
            }),
            "picker: type search  enter/right select  B batch  X exclude  backspace up  esc close"
        );
        assert_eq!(
            footer_mode_help_text(FooterHelpState {
                thought_config_editor: false,
                picker: false,
                show_help: true,
            }),
            "press any key to dismiss help"
        );
        assert_eq!(
            footer_mode_help_text(FooterHelpState {
                thought_config_editor: false,
                picker: false,
                show_help: false,
            }),
            "arrows/hjkl move  enter open  r refresh  t config  ? help  q quit"
        );
    }

    #[test]
    fn request_footer_help_text_hides_voice_only_when_unsupported() {
        assert_eq!(
            request_footer_help_text(&VoiceUiState::Unsupported),
            "request: type prompt  enter create  backspace delete  esc cancel"
        );
        for state in [
            VoiceUiState::Ready,
            VoiceUiState::Recording,
            VoiceUiState::Transcribing,
            VoiceUiState::Failed("denied".to_string()),
        ] {
            assert_eq!(
                request_footer_help_text(&state),
                "request: type prompt  ctrl-v voice  enter create  backspace delete  esc cancel"
            );
        }
    }
}
