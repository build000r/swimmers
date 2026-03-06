use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use reqwest::Client;
use tokio::runtime::Runtime;

use throngterm::config::{AuthMode, Config};
use throngterm::types::{
    ErrorResponse, NativeDesktopOpenRequest, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, SessionListResponse, SessionState, SessionSummary, ThoughtState,
};

const MIN_WIDTH: u16 = 70;
const MIN_HEIGHT: u16 = 20;
const FRAME_DURATION: Duration = Duration::from_millis(100);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const MESSAGE_TTL: Duration = Duration::from_secs(5);
const SPRITE_HEIGHT: u16 = 4;
const LABEL_HEIGHT: u16 = 1;
const ENTITY_WIDTH: u16 = 12;
const ENTITY_HEIGHT: u16 = SPRITE_HEIGHT + LABEL_HEIGHT;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Reset,
        }
    }
}

struct Renderer {
    stdout: BufWriter<Stdout>,
    width: u16,
    height: u16,
    buffer: Vec<Cell>,
    last_buffer: Vec<Cell>,
    active: bool,
}

impl Renderer {
    fn new() -> io::Result<Self> {
        if !io::stdout().is_terminal() {
            return Err(io::Error::other("stdout is not a tty"));
        }

        let (width, height) = terminal::size()?;
        let buffer_size = (width as usize) * (height as usize);
        Ok(Self {
            stdout: BufWriter::new(io::stdout()),
            width,
            height,
            buffer: vec![Cell::default(); buffer_size],
            last_buffer: vec![Cell::default(); buffer_size],
            active: false,
        })
    }

    fn init(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            self.stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide,
            Clear(ClearType::All)
        )?;
        self.active = true;
        Ok(())
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        execute!(
            self.stdout,
            LeaveAlternateScreen,
            DisableMouseCapture,
            cursor::Show,
            ResetColor
        )?;
        terminal::disable_raw_mode()?;
        self.active = false;
        Ok(())
    }

    fn manual_resize(&mut self, width: u16, height: u16) -> io::Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }

        self.width = width;
        self.height = height;
        let buffer_size = (width as usize) * (height as usize);
        self.buffer = vec![Cell::default(); buffer_size];
        self.last_buffer = vec![Cell::default(); buffer_size];
        execute!(self.stdout, Clear(ClearType::All))?;
        Ok(())
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn clear(&mut self) {
        self.buffer.fill(Cell::default());
    }

    fn draw_char(&mut self, x: u16, y: u16, ch: char, fg: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = (y as usize) * (self.width as usize) + (x as usize);
        if let Some(cell) = self.buffer.get_mut(idx) {
            *cell = Cell { ch, fg };
        }
    }

    fn draw_text(&mut self, x: u16, y: u16, text: &str, fg: Color) {
        if y >= self.height {
            return;
        }
        for (offset, ch) in text.chars().enumerate() {
            let col = x.saturating_add(offset as u16);
            if col >= self.width {
                break;
            }
            self.draw_char(col, y, ch, fg);
        }
    }

    fn draw_hline(&mut self, x: u16, y: u16, width: u16, ch: char, fg: Color) {
        for dx in 0..width {
            self.draw_char(x + dx, y, ch, fg);
        }
    }

    fn draw_vline(&mut self, x: u16, y: u16, height: u16, ch: char, fg: Color) {
        for dy in 0..height {
            self.draw_char(x, y + dy, ch, fg);
        }
    }

    fn draw_box(&mut self, rect: Rect, fg: Color) {
        if rect.width < 2 || rect.height < 2 {
            return;
        }
        self.draw_char(rect.x, rect.y, '+', fg);
        self.draw_char(rect.right() - 1, rect.y, '+', fg);
        self.draw_char(rect.x, rect.bottom() - 1, '+', fg);
        self.draw_char(rect.right() - 1, rect.bottom() - 1, '+', fg);
        self.draw_hline(rect.x + 1, rect.y, rect.width - 2, '-', fg);
        self.draw_hline(rect.x + 1, rect.bottom() - 1, rect.width - 2, '-', fg);
        self.draw_vline(rect.x, rect.y + 1, rect.height - 2, '|', fg);
        self.draw_vline(rect.right() - 1, rect.y + 1, rect.height - 2, '|', fg);
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut current_color = Color::Reset;
        let mut last_pos: Option<(u16, u16)> = None;

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y as usize) * (self.width as usize) + (x as usize);
                let cell = self.buffer[idx];
                let prev = self.last_buffer[idx];
                if cell == prev {
                    continue;
                }

                if last_pos != Some((x, y)) {
                    queue!(self.stdout, cursor::MoveTo(x, y))?;
                }

                if cell.fg != current_color {
                    queue!(self.stdout, SetForegroundColor(cell.fg))?;
                    current_color = cell.fg;
                }

                queue!(self.stdout, Print(cell.ch))?;
                last_pos = Some((x.saturating_add(1), y));
            }
        }

        if current_color != Color::Reset {
            queue!(self.stdout, ResetColor)?;
        }
        self.stdout.flush()?;
        self.last_buffer.copy_from_slice(&self.buffer);
        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl Rect {
    fn right(self) -> u16 {
        self.x + self.width
    }

    fn bottom(self) -> u16 {
        self.y + self.height
    }

    fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    fn inset(self, amount: u16) -> Self {
        if self.width <= amount * 2 || self.height <= amount * 2 {
            return Self {
                x: self.x,
                y: self.y,
                width: 0,
                height: 0,
            };
        }
        Self {
            x: self.x + amount,
            y: self.y + amount,
            width: self.width - amount * 2,
            height: self.height - amount * 2,
        }
    }
}

#[derive(Clone, Copy)]
enum SpriteKind {
    Active,
    Busy,
    Drowsy,
    Sleeping,
    Attention,
    Error,
    Exited,
}

impl SpriteKind {
    fn from_session(session: &SessionSummary) -> Self {
        match session.state {
            SessionState::Busy => Self::Busy,
            SessionState::Attention => Self::Attention,
            SessionState::Error => Self::Error,
            SessionState::Exited => Self::Exited,
            SessionState::Idle => match session.thought_state {
                ThoughtState::Sleeping => Self::Sleeping,
                ThoughtState::Holding => Self::Drowsy,
                ThoughtState::Active => Self::Active,
            },
        }
    }

    fn color(self, selected: bool) -> Color {
        if selected {
            return Color::White;
        }
        match self {
            Self::Active => Color::Green,
            Self::Busy => Color::Yellow,
            Self::Drowsy => Color::DarkYellow,
            Self::Sleeping => Color::Blue,
            Self::Attention => Color::Magenta,
            Self::Error => Color::Red,
            Self::Exited => Color::DarkGrey,
        }
    }

    fn speed_scale(self) -> f32 {
        match self {
            Self::Active => 1.0,
            Self::Busy => 1.15,
            Self::Drowsy => 0.45,
            Self::Sleeping => 0.08,
            Self::Attention => 1.0,
            Self::Error => 0.5,
            Self::Exited => 0.0,
        }
    }

    fn frame(self, tick: u64) -> [&'static str; 4] {
        match self {
            Self::Active => active_frame(tick),
            Self::Busy => busy_frame(tick),
            Self::Drowsy => drowsy_frame(tick),
            Self::Sleeping => sleeping_frame(tick),
            Self::Attention => attention_frame(tick),
            Self::Error => error_frame(tick),
            Self::Exited => exited_frame(),
        }
    }
}

fn active_frame(tick: u64) -> [&'static str; 4] {
    if tick % 2 == 0 {
        [" .-^. ", "(o o)", "/|_|\\", " / \\ "]
    } else {
        [" .-^. ", "(o o)", "\\|_|/", "/   \\"]
    }
}

fn busy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 2 == 0 {
        [" .-^. ", "(O O)", "/|_|\\", " / \\ "]
    } else {
        [" .-^. ", "(O O)", "\\|_|/", "/   \\"]
    }
}

fn drowsy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 4 < 2 {
        [" .-^. ", "(- -)", "/|_|\\", " / \\ "]
    } else {
        [" .-^. ", "(- -)", "\\|_|/", " / \\ "]
    }
}

fn sleeping_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [" zZ   ", " .-^. ", "(- -)", "(___)"]
    } else {
        ["  zZ  ", " .-^. ", "(- -)", "(___)"]
    }
}

fn attention_frame(tick: u64) -> [&'static str; 4] {
    if tick % 2 == 0 {
        [" .-^. ", "(! !)", "/|_|\\", " / \\ "]
    } else {
        [" .-^. ", "(! !)", "\\|_|/", "/   \\"]
    }
}

fn error_frame(tick: u64) -> [&'static str; 4] {
    if tick % 2 == 0 {
        [" .-^. ", "(x x)", "/|_|\\", " / \\ "]
    } else {
        [" .-^. ", "(x x)", "\\|_|/", "/   \\"]
    }
}

fn exited_frame() -> [&'static str; 4] {
    ["  xxx ", " (x x)", " /|_|\\", "  / \\ "]
}

#[derive(Clone)]
struct SessionEntity {
    session: SessionSummary,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

impl SessionEntity {
    fn new(session: SessionSummary, field: Rect) -> Self {
        let hash = stable_hash(&session.session_id);
        let max_x = field.width.saturating_sub(ENTITY_WIDTH).max(1);
        let max_y = field.height.saturating_sub(ENTITY_HEIGHT).max(1);
        let x = (hash % (max_x as u64)) as f32;
        let y = ((hash / 13) % (max_y as u64)) as f32;
        let vx = velocity_component(hash, 0);
        let vy = velocity_component(hash, 1);

        Self {
            session,
            x,
            y,
            vx,
            vy,
        }
    }

    fn sprite_kind(&self) -> SpriteKind {
        SpriteKind::from_session(&self.session)
    }

    fn tick(&mut self, field: Rect) {
        let speed = self.sprite_kind().speed_scale();
        if speed == 0.0 || field.width <= ENTITY_WIDTH || field.height <= ENTITY_HEIGHT {
            return;
        }

        let max_x = field.width.saturating_sub(ENTITY_WIDTH) as f32;
        let max_y = field.height.saturating_sub(ENTITY_HEIGHT) as f32;

        self.x += self.vx * speed;
        self.y += self.vy * speed;

        if self.x <= 0.0 {
            self.x = 0.0;
            self.vx = self.vx.abs();
        } else if self.x >= max_x {
            self.x = max_x;
            self.vx = -self.vx.abs();
        }

        if self.y <= 0.0 {
            self.y = 0.0;
            self.vy = self.vy.abs();
        } else if self.y >= max_y {
            self.y = max_y;
            self.vy = -self.vy.abs();
        }
    }

    fn screen_rect(&self, field: Rect) -> Rect {
        Rect {
            x: field.x + self.x.max(0.0).round() as u16,
            y: field.y + self.y.max(0.0).round() as u16,
            width: ENTITY_WIDTH,
            height: ENTITY_HEIGHT,
        }
    }
}

struct ApiClient {
    http: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl ApiClient {
    fn from_env() -> Result<Self, String> {
        let config = Config::from_env();
        let base_url = std::env::var("THRONGTERM_TUI_URL")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
        let auth_token = match config.auth_mode {
            AuthMode::Token => config.auth_token,
            AuthMode::LocalTrust => None,
        };

        let http = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|err| format!("failed to build http client: {err}"))?;

        Ok(Self {
            http,
            base_url,
            auth_token,
        })
    }

    fn with_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    async fn fetch_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let url = format!("{}/v1/sessions", self.base_url);
        let response = self
            .with_auth(self.http.get(url))
            .send()
            .await
            .map_err(|err| format!("failed to fetch sessions: {err}"))?;

        if response.status().is_success() {
            let payload = response
                .json::<SessionListResponse>()
                .await
                .map_err(|err| format!("failed to parse sessions response: {err}"))?;
            return Ok(payload.sessions);
        }

        Err(read_error(response).await)
    }

    async fn fetch_native_status(&self) -> Result<NativeDesktopStatusResponse, String> {
        let url = format!("{}/v1/native/status", self.base_url);
        let response = self
            .with_auth(self.http.get(url))
            .send()
            .await
            .map_err(|err| format!("failed to fetch native desktop status: {err}"))?;

        if response.status().is_success() {
            return response
                .json::<NativeDesktopStatusResponse>()
                .await
                .map_err(|err| format!("failed to parse native status: {err}"));
        }

        Err(read_error(response).await)
    }

    async fn open_session(&self, session_id: &str) -> Result<NativeDesktopOpenResponse, String> {
        let url = format!("{}/v1/native/open", self.base_url);
        let response = self
            .with_auth(self.http.post(url))
            .json(&NativeDesktopOpenRequest {
                session_id: session_id.to_string(),
            })
            .send()
            .await
            .map_err(|err| format!("failed to open session: {err}"))?;

        if response.status().is_success() {
            return response
                .json::<NativeDesktopOpenResponse>()
                .await
                .map_err(|err| format!("failed to parse native open response: {err}"));
        }

        Err(read_error(response).await)
    }
}

async fn read_error(response: reqwest::Response) -> String {
    let status = response.status();
    match response.json::<ErrorResponse>().await {
        Ok(body) => body
            .message
            .unwrap_or_else(|| format!("request failed: {}", status)),
        Err(_) => format!("request failed: {}", status),
    }
}

struct App {
    runtime: Runtime,
    client: ApiClient,
    entities: Vec<SessionEntity>,
    selected_id: Option<String>,
    native_status: Option<NativeDesktopStatusResponse>,
    message: Option<(String, Instant)>,
    last_refresh: Option<Instant>,
    tick: u64,
}

impl App {
    fn new(runtime: Runtime, client: ApiClient) -> Self {
        Self {
            runtime,
            client,
            entities: Vec::new(),
            selected_id: None,
            native_status: None,
            message: None,
            last_refresh: None,
            tick: 0,
        }
    }

    fn set_message(&mut self, message: impl Into<String>) {
        self.message = Some((message.into(), Instant::now()));
    }

    fn visible_message(&self) -> Option<&str> {
        self.message.as_ref().and_then(|(message, at)| {
            if at.elapsed() <= MESSAGE_TTL {
                Some(message.as_str())
            } else {
                None
            }
        })
    }

    fn should_refresh(&self) -> bool {
        self.last_refresh
            .map(|last| last.elapsed() >= REFRESH_INTERVAL)
            .unwrap_or(true)
    }

    fn refresh(&mut self, field: Rect) {
        match self.runtime.block_on(self.client.fetch_sessions()) {
            Ok(sessions) => {
                self.merge_sessions(sessions, field);
                let count = self.entities.len();
                self.set_message(format!("refreshed {count} session{}", pluralize(count)));
            }
            Err(err) => {
                self.set_message(err);
            }
        }

        if self.native_status.is_none() {
            self.native_status = self
                .runtime
                .block_on(self.client.fetch_native_status())
                .map(Some)
                .unwrap_or_else(|err| {
                    self.set_message(err);
                    None
                });
        }

        self.last_refresh = Some(Instant::now());
    }

    fn merge_sessions(&mut self, sessions: Vec<SessionSummary>, field: Rect) {
        let mut existing = HashMap::new();
        for entity in self.entities.drain(..) {
            existing.insert(entity.session.session_id.clone(), entity);
        }

        let mut next = Vec::with_capacity(sessions.len());
        for session in sessions {
            if let Some(mut entity) = existing.remove(&session.session_id) {
                entity.session = session;
                next.push(entity);
            } else {
                next.push(SessionEntity::new(session, field));
            }
        }

        next.sort_by(|a, b| a.session.tmux_name.cmp(&b.session.tmux_name));
        self.entities = next;

        let selection_missing = self
            .selected_id
            .as_ref()
            .map(|selected| {
                !self
                    .entities
                    .iter()
                    .any(|entity| entity.session.session_id == *selected)
            })
            .unwrap_or(true);

        if selection_missing {
            self.selected_id = self
                .entities
                .first()
                .map(|entity| entity.session.session_id.clone());
        }
    }

    fn tick(&mut self, field: Rect) {
        self.tick = self.tick.wrapping_add(1);
        for entity in &mut self.entities {
            entity.tick(field);
        }
        self.resolve_collisions(field);
    }

    fn resolve_collisions(&mut self, field: Rect) {
        for idx in 0..self.entities.len() {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            for b in right {
                let a_rect = a.screen_rect(field);
                let b_rect = b.screen_rect(field);
                if intersects(a_rect, b_rect) {
                    std::mem::swap(&mut a.vx, &mut b.vx);
                    std::mem::swap(&mut a.vy, &mut b.vy);
                    a.x = (a.x - 1.0).max(0.0);
                    b.x = (b.x + 1.0).min(field.width.saturating_sub(ENTITY_WIDTH) as f32);
                }
            }
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.entities.is_empty() {
            self.selected_id = None;
            return;
        }

        let current_index = self
            .selected_id
            .as_ref()
            .and_then(|selected| {
                self.entities
                    .iter()
                    .position(|entity| entity.session.session_id == *selected)
            })
            .unwrap_or(0) as isize;

        let len = self.entities.len() as isize;
        let next_index = (current_index + delta).rem_euclid(len) as usize;
        self.selected_id = Some(self.entities[next_index].session.session_id.clone());
    }

    fn selected(&self) -> Option<&SessionEntity> {
        let selected = self.selected_id.as_ref()?;
        self.entities
            .iter()
            .find(|entity| entity.session.session_id == *selected)
    }

    fn open_selected(&mut self) {
        let Some(selected_id) = self.selected_id.clone() else {
            self.set_message("no session selected");
            return;
        };

        match self
            .runtime
            .block_on(self.client.open_session(&selected_id))
        {
            Ok(response) => {
                self.set_message(format!(
                    "{} {}",
                    response.status,
                    selected_label(self.selected().map(|entity| &entity.session.tmux_name))
                ));
            }
            Err(err) => {
                self.set_message(err);
            }
        }
    }

    fn click_open(&mut self, x: u16, y: u16, field: Rect) {
        let hit = self
            .entities
            .iter()
            .find(|entity| entity.screen_rect(field).contains(x, y))
            .map(|entity| entity.session.session_id.clone());

        if let Some(session_id) = hit {
            self.selected_id = Some(session_id);
            self.open_selected();
        }
    }

    fn render(&self, renderer: &mut Renderer) {
        renderer.clear();

        if renderer.width() < MIN_WIDTH || renderer.height() < MIN_HEIGHT {
            render_too_small(renderer);
            return;
        }

        let frame = frame_rect(renderer.width(), renderer.height());
        let field_box = field_box(renderer.width(), renderer.height());
        let field = field_box.inset(1);

        renderer.draw_box(frame, Color::DarkGrey);
        renderer.draw_text(2, 1, "throngterm tui", Color::Cyan);

        let status_text = match &self.native_status {
            Some(status) if status.supported => format!(
                "native open: {}",
                status.app.as_deref().unwrap_or("available")
            ),
            Some(status) => format!(
                "native open unavailable: {}",
                status.reason.as_deref().unwrap_or("unknown reason")
            ),
            None => "native open: checking".to_string(),
        };
        let sessions_text = format!("sessions: {}", self.entities.len());
        let right_text = format!("{sessions_text} | {status_text}");
        let right_x = renderer
            .width()
            .saturating_sub(right_text.len() as u16)
            .saturating_sub(2);
        renderer.draw_text(right_x, 1, &right_text, Color::DarkGrey);

        renderer.draw_box(field_box, Color::DarkGrey);

        if self.entities.is_empty() {
            let empty = "no tmux sessions found - press r after starting one";
            let x = field
                .x
                .saturating_add(field.width.saturating_sub(empty.len() as u16) / 2);
            let y = field.y + field.height / 2;
            renderer.draw_text(x, y, empty, Color::DarkGrey);
        }

        for entity in &self.entities {
            let rect = entity.screen_rect(field);
            let selected = self
                .selected_id
                .as_ref()
                .map(|selected| *selected == entity.session.session_id)
                .unwrap_or(false);
            render_entity(renderer, entity, rect, selected, self.tick);
        }

        render_footer(self, renderer, field_box.bottom() + 1);
    }
}

fn render_entity(
    renderer: &mut Renderer,
    entity: &SessionEntity,
    rect: Rect,
    selected: bool,
    tick: u64,
) {
    let kind = entity.sprite_kind();
    let color = kind.color(selected);
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

fn render_footer(app: &App, renderer: &mut Renderer, start_y: u16) {
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

    let help = "click/enter open  arrows or hjkl move  r refresh  q quit";
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

fn render_too_small(renderer: &mut Renderer) {
    renderer.draw_text(2, 1, "throngterm tui", Color::Cyan);
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
        "resize the terminal or use the web view",
        Color::DarkGrey,
    );
}

fn frame_rect(width: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width,
        height,
    }
}

fn field_box(width: u16, height: u16) -> Rect {
    let footer_height = 6;
    Rect {
        x: 1,
        y: 3,
        width: width.saturating_sub(2),
        height: height.saturating_sub(footer_height + 3),
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn velocity_component(hash: u64, axis: u64) -> f32 {
    let segment = ((hash >> (axis * 8)) & 0xff) as f32 / 255.0;
    let speed = 0.05 + segment * 0.09;
    if axis == 0 {
        if hash & 1 == 0 {
            speed
        } else {
            -speed
        }
    } else if hash & 2 == 0 {
        speed
    } else {
        -speed
    }
}

fn pluralize(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn truncate_label(text: &str, max_chars: usize) -> String {
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

fn shorten_path(path: &str, max_chars: usize) -> String {
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

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.right() && a.right() > b.x && a.y < b.bottom() && a.bottom() > b.y
}

fn session_state_text(session: &SessionSummary) -> &'static str {
    match session.state {
        SessionState::Idle => match session.thought_state {
            ThoughtState::Active => "active",
            ThoughtState::Holding => "drowsy",
            ThoughtState::Sleeping => "sleeping",
        },
        SessionState::Busy => "busy",
        SessionState::Attention => "attention",
        SessionState::Error => "error",
        SessionState::Exited => "exited",
    }
}

fn selected_label(name: Option<&String>) -> String {
    name.cloned().unwrap_or_else(|| "session".to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    let runtime = Runtime::new()?;
    let client = ApiClient::from_env().map_err(io::Error::other)?;
    let mut renderer = Renderer::new()?;
    renderer.init()?;

    let mut app = App::new(runtime, client);
    let field = field_box(renderer.width(), renderer.height()).inset(1);
    app.refresh(field);

    loop {
        let current_field = field_box(renderer.width(), renderer.height()).inset(1);
        if app.should_refresh() {
            app.refresh(current_field);
        }

        app.tick(current_field);
        app.render(&mut renderer);
        renderer.flush()?;

        if event::poll(FRAME_DURATION)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::Up | KeyCode::Char('k') => {
                        app.move_selection(-1);
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Down | KeyCode::Char('j') => {
                        app.move_selection(1);
                    }
                    KeyCode::Enter | KeyCode::Char('o') => {
                        app.open_selected();
                    }
                    KeyCode::Char('r') => {
                        let field = field_box(renderer.width(), renderer.height()).inset(1);
                        app.refresh(field);
                    }
                    _ => {}
                },
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                {
                    let field = field_box(renderer.width(), renderer.height()).inset(1);
                    if field.contains(mouse.column, mouse.row) {
                        app.click_open(mouse.column, mouse.row, field);
                    }
                }
                Event::Resize(width, height) => {
                    renderer.manual_resize(width, height)?;
                }
                _ => {}
            }
        }
    }

    renderer.cleanup()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_label_adds_trailing_tilde() {
        assert_eq!(truncate_label("abcdefghijkl", 6), "abcde~");
        assert_eq!(truncate_label("abc", 6), "abc");
    }

    #[test]
    fn shorten_path_keeps_tail() {
        assert_eq!(shorten_path("/a/b/c/d/e", 8), ".../d/e");
        assert_eq!(shorten_path("/short", 20), "/short");
    }

    #[test]
    fn intersects_detects_overlap() {
        let a = Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        let b = Rect {
            x: 4,
            y: 2,
            width: 5,
            height: 3,
        };
        let c = Rect {
            x: 5,
            y: 5,
            width: 2,
            height: 2,
        };
        assert!(intersects(a, b));
        assert!(!intersects(a, c));
    }
}
