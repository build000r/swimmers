use super::*;

pub(crate) fn render_entity(
    renderer: &mut Renderer,
    entity: &SessionEntity,
    rect: Rect,
    selected: bool,
    tick: u64,
    repo_themes: &HashMap<String, RepoTheme>,
) {
    let kind = entity.sprite_kind();
    let color = session_display_color(&entity.session, repo_themes);
    let sprite = kind.frame(tick);
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
        "click empty field spawn  click/enter open  arrows or hjkl move  n native target  m ghostty mode  t thought cfg  r refresh  q quit"
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
