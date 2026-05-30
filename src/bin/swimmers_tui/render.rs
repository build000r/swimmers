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
    let cord_char = if unverified {
        // Sparse/dashed cord; visually distinct from `|`, `:`, `!`, `x`.
        '\''
    } else {
        match kind {
            SpriteKind::Attention => '!',
            SpriteKind::Busy => ':',
            SpriteKind::Error | SpriteKind::Exited => 'x',
            _ => '|',
        }
    };
    let cord_color = if selected {
        Color::White
    } else {
        Color::DarkGrey
    };
    for y in slot.cord_top_y..slot.ball_y {
        renderer.draw_char(slot.anchor_x, y, cord_char, cord_color);
    }

    let rows: &[&str] = if unverified {
        // Ghost body — explicit `?` overlay so an unobserved ball cannot be
        // mistaken for a confirmed one even when colors are hard to read. The
        // drop distance is still per-kind, so the operator can see what the
        // detector *thinks* the state is while also seeing it isn't trusted.
        &[" .?. ", "( ? )", " `-' "]
    } else {
        match kind {
            SpriteKind::Attention => &[" .-. ", "(_!_)"],
            SpriteKind::Active => &[" .-. ", "(   )", " `-' "],
            SpriteKind::Busy => &[" .-. ", "( * )", " `-' "],
            SpriteKind::Error => &[" .-. ", "( x )", " `-' "],
            SpriteKind::Drowsy => &[" .-. ", "(   )", "(   )", " `-' "],
            SpriteKind::Sleeping => &[" .-. ", "( z )", "(   )", " `-' "],
            SpriteKind::DeepSleep => {
                if slot.on_floor {
                    &[" .-. ", "( z )", "(   )", "(___)"]
                } else {
                    &[" .-. ", "( z )", "(   )", " `-' "]
                }
            }
            SpriteKind::Exited => {
                if slot.on_floor {
                    &[" .-. ", "( x )", "(   )", "(___)"]
                } else {
                    &[" .-. ", "( x )", "(   )", " `-' "]
                }
            }
        }
    };

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

    if let Some(selected) = app.selected() {
        let state_line = format!(
            "selected: {} [{}; {}] {}",
            selected.session.tmux_name,
            session_state_text(&selected.session),
            session_state_evidence_text(&selected.session),
            shorten_path(&selected.session.cwd, 42)
        );
        renderer.draw_text(
            2,
            start_y,
            &truncate_label(&state_line, renderer.width().saturating_sub(4) as usize),
            Color::White,
        );

        let cmd = selected
            .session
            .current_command
            .as_deref()
            .unwrap_or("idle");
        let cmd_line = format!("cmd: {}", shorten_path(cmd, 60));
        renderer.draw_text(
            2,
            start_y + 1,
            &truncate_label(&cmd_line, renderer.width().saturating_sub(4) as usize),
            Color::DarkGrey,
        );
    } else {
        renderer.draw_text(2, start_y, "selected: none", Color::DarkGrey);
    }

    let help = if app.initial_request.is_some() {
        if matches!(app.voice_state, VoiceUiState::Unsupported) {
            "request: type prompt  enter create  backspace delete  esc cancel"
        } else {
            "request: type prompt  ctrl-v voice  enter create  backspace delete  esc cancel"
        }
    } else if app.thought_config_editor.is_some() {
        "thought config: tab moves  arrows adjust  enter saves  esc cancels"
    } else if app.picker.is_some() {
        "picker: type search  enter/right select  B batch  X exclude  backspace up  esc close"
    } else if app.show_help {
        "press any key to dismiss help"
    } else {
        "arrows/hjkl move  enter open  r refresh  t config  ? help  q quit"
    };
    renderer.draw_text(
        2,
        start_y + 2,
        &truncate_label(help, renderer.width().saturating_sub(4) as usize),
        Color::Cyan,
    );

    if let Some(message) = app.visible_message() {
        renderer.draw_text(
            2,
            start_y + 3,
            &truncate_label(message, renderer.width().saturating_sub(4) as usize),
            Color::Yellow,
        );
    }
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
    if field.width < 4 || field.height < 4 {
        return;
    }

    let width = usize::from(field.width.max(1));
    let scroll = (tick as usize / 3) % width;
    let lane_count = usize::from((field.width / 18).clamp(1, 4));
    let lane_spacing = (field.width / lane_count as u16).max(1);
    let bottom_y = field.bottom().saturating_sub(1);
    for lane in 0..lane_count {
        let base_offset = (2 + lane as u16 * lane_spacing) as usize;
        let x = field
            .right()
            .saturating_sub(1)
            .saturating_sub(((base_offset + scroll) % width) as u16);
        let rise = ((tick / 4) as u16 + lane as u16 * 4) % field.height.max(1);
        let y = bottom_y.saturating_sub(rise);
        renderer.draw_char(x, y, 'o', Color::DarkCyan);
        if x + 1 < field.right() && y + 1 < field.bottom() {
            renderer.draw_char(x + 1, y + 1, '.', Color::Blue);
        }
    }

    let sparkle_count = usize::from((field.width / 24).clamp(1, 3));
    for sparkle in 0..sparkle_count {
        let x = field
            .right()
            .saturating_sub(1)
            .saturating_sub((((tick as usize / 2) + sparkle * 11) % width) as u16);
        let y_span = field.height.saturating_sub(3).max(1);
        let y = field.y + 1 + (((tick / 2) as u16 + sparkle as u16 * 6) % y_span);
        renderer.draw_char(x, y, '~', Color::DarkBlue);
        if x > field.x {
            renderer.draw_char(x - 1, y, '.', Color::DarkBlue);
        }
    }
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

fn prefix_by_display_width(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = char_display_width(ch);
        if ch_width == 0 {
            if !out.is_empty() {
                out.push(ch);
            }
            continue;
        }
        if used.saturating_add(ch_width) > max_cols {
            break;
        }
        used = used.saturating_add(ch_width);
        out.push(ch);
    }
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
    if path.contains('/') && max_chars > 3 {
        let budget = max_chars - 3;
        let mut suffix = String::new();
        for part in path.split('/').filter(|part| !part.is_empty()).rev() {
            let candidate = if suffix.is_empty() {
                format!("/{part}")
            } else {
                format!("/{part}{suffix}")
            };
            if text_display_width(&candidate) > budget {
                break;
            }
            suffix = candidate;
        }
        if !suffix.is_empty() {
            return format!("...{suffix}");
        }
    }
    if max_chars <= 3 {
        return prefix_by_display_width(path, max_chars);
    }
    let tail = tail_text(path, max_chars - 3);
    format!("...{tail}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_label_respects_terminal_columns_for_mixed_width_text() {
        let text = "ab漢字a\u{0301}xyz";
        let truncated = truncate_label(text, 7);
        assert_eq!(UnicodeWidthStr::width(truncated.as_str()), 7);
        assert!(truncated.ends_with('~'));
    }
}
