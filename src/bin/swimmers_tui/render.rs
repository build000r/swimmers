use super::*;

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
    let desired_width = (count as u16)
        .saturating_mul(5)
        .saturating_add(12)
        .clamp(18, field.width.saturating_sub(2));
    Rect {
        x: field.x + field.width.saturating_sub(desired_width) / 2,
        y: field.y,
        width: desired_width,
        height: BALLS_BODY_HEIGHT,
    }
}

fn balls_center_out_indices(count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }

    let mut order = Vec::with_capacity(count);
    let left_center = (count - 1) / 2;
    let right_center = count / 2;
    order.push(left_center);
    if right_center != left_center {
        order.push(right_center);
    }

    let mut offset = 1usize;
    while order.len() < count {
        if let Some(index) = left_center.checked_sub(offset) {
            order.push(index);
        }
        let right = right_center + offset;
        if right < count {
            order.push(right);
        }
        offset += 1;
    }
    order
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
    let placement = balls_center_out_indices(entities.len());

    let mut ordered = entities.to_vec();
    ordered.sort_by(|left, right| compare_sleepiness(&left.session, &right.session));

    let floor_y = field.bottom().saturating_sub(1);
    let cord_top_y = body.bottom();
    let mut slots = Vec::with_capacity(ordered.len());
    for (age_rank, entity) in ordered.into_iter().enumerate() {
        let placement_index = *placement.get(age_rank).unwrap_or(&age_rank);
        let anchor_x = *centers.get(placement_index).unwrap_or(&centers[0]);
        let kind = entity.sprite_kind();
        let ball_height = balls_theme_ball_height(kind);
        let mut drop =
            balls_theme_base_drop(kind) + balls_theme_age_bonus(age_rank, entities.len(), kind);
        if matches!(kind, SpriteKind::DeepSleep | SpriteKind::Exited) {
            drop = drop.max(field.height.saturating_sub(BALLS_BODY_HEIGHT + ball_height));
        }
        let ball_y = (cord_top_y + drop).min(floor_y.saturating_add(1).saturating_sub(ball_height));
        let ball_x = anchor_x
            .saturating_sub(BALLS_BALL_WIDTH / 2)
            .clamp(field.x, field.right().saturating_sub(BALLS_BALL_WIDTH));
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
    let cord_char = match kind {
        SpriteKind::Attention => '!',
        SpriteKind::Busy => ':',
        SpriteKind::Error | SpriteKind::Exited => 'x',
        _ => '|',
    };
    let cord_color = if selected {
        Color::White
    } else {
        Color::DarkGrey
    };
    for y in slot.cord_top_y..slot.ball_y {
        renderer.draw_char(slot.anchor_x, y, cord_char, cord_color);
    }

    let rows: &[&str] = match kind {
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

pub(crate) fn render_footer<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, start_y: u16) {
    if start_y >= renderer.height() {
        return;
    }

    if let Some(selected) = app.selected() {
        let state_line = format!(
            "selected: {} [{}] {}",
            selected.session.tmux_name,
            session_state_text(&selected.session),
            shorten_path(&selected.session.cwd, 42)
        );
        renderer.draw_text(
            2,
            start_y,
            &truncate_label(&state_line, (renderer.width() - 4) as usize),
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
            &truncate_label(&cmd_line, (renderer.width() - 4) as usize),
            Color::DarkGrey,
        );
    } else {
        renderer.draw_text(2, start_y, "selected: none", Color::DarkGrey);
    }

    let help = if app.initial_request.is_some() {
        "request: type prompt  enter create hidden  backspace delete  esc cancel"
    } else if app.thought_config_editor.is_some() {
        "thought config: tab moves  arrows adjust  enter saves  esc cancels"
    } else if app.picker.is_some() {
        "picker: enter/right select  up/down or jk move  h/backspace up  e env  a all  esc close"
    } else {
        "click empty field spawn  click/enter open  arrows or hjkl move  n native target  m ghostty mode  s sprite theme  t thought cfg  r refresh  q quit"
    };
    renderer.draw_text(
        2,
        start_y + 2,
        &truncate_label(help, (renderer.width() - 4) as usize),
        Color::Cyan,
    );

    if let Some(message) = app.visible_message() {
        renderer.draw_text(
            2,
            start_y + 3,
            &truncate_label(message, (renderer.width() - 4) as usize),
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

pub(crate) fn truncate_label(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return chars.into_iter().collect();
    }
    if max_chars == 0 {
        return String::new();
    }
    chars.truncate(max_chars.saturating_sub(1));
    let mut out: String = chars.into_iter().collect();
    out.push('~');
    out
}

pub(crate) fn tail_text(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return chars.into_iter().collect();
    }
    if max_chars == 0 {
        return String::new();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

pub(crate) fn shorten_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
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
            if candidate.chars().count() > budget {
                break;
            }
            suffix = candidate;
        }
        if !suffix.is_empty() {
            return format!("...{suffix}");
        }
    }
    let chars = path.chars().collect::<Vec<_>>();
    if max_chars <= 3 {
        return chars.into_iter().take(max_chars).collect();
    }
    let tail = chars
        .into_iter()
        .rev()
        .take(max_chars - 3)
        .collect::<Vec<_>>();
    let tail = tail.into_iter().rev().collect::<String>();
    format!("...{tail}")
}

pub(crate) fn session_state_text(session: &SessionSummary) -> &'static str {
    match session.state {
        SessionState::Idle | SessionState::Attention => match session.rest_state {
            RestState::Active => match session.state {
                SessionState::Attention => "attention",
                SessionState::Idle => "active",
                _ => unreachable!("only idle/attention reach active rest-state branch"),
            },
            RestState::Drowsy => "drowsy",
            RestState::Sleeping => "sleeping",
            RestState::DeepSleep => "deep sleep",
        },
        SessionState::Busy => "busy",
        SessionState::Error => "error",
        SessionState::Exited => "exited",
    }
}

pub(crate) fn selected_label(name: Option<&String>) -> String {
    name.cloned().unwrap_or_else(|| "session".to_string())
}
