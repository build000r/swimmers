use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::future::BoxFuture;
use reqwest::Client;
use tokio::runtime::Runtime;

use throngterm::config::{AuthMode, Config};
use throngterm::repo_theme::{discover_repo_theme, existing_repo_theme};
use throngterm::types::{
    CreateSessionRequest, CreateSessionResponse, DirEntry, DirListResponse, ErrorResponse,
    NativeDesktopOpenRequest, NativeDesktopOpenResponse, NativeDesktopStatusResponse, RepoTheme,
    RestState, SessionListResponse, SessionState, SessionSummary, SpawnTool,
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
const PICKER_WIDTH: u16 = 46;
const PICKER_MAX_HEIGHT: u16 = 16;
const INITIAL_REQUEST_WIDTH: u16 = 58;
const INITIAL_REQUEST_HEIGHT: u16 = 7;
const THOUGHT_RAIL_MIN_WIDTH: u16 = 100;
const THOUGHT_RAIL_MIN_PANEL_WIDTH: u16 = 24;
const THOUGHT_RAIL_GAP: u16 = 1;
const THOUGHT_RAIL_RATIO_DENOMINATOR: u16 = 3;
const THOUGHT_RAIL_HEADER_ROWS: u16 = 1;
const THOUGHT_RAIL_DEFAULT_RATIO: f32 = 1.0 / THOUGHT_RAIL_RATIO_DENOMINATOR as f32;
const THOUGHT_RAIL_DRAG_HITBOX_WIDTH: u16 = 3;

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

fn enter_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide,
        Clear(ClearType::All)
    )
}

fn leave_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show,
        ResetColor
    )
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
        enter_terminal_ui(&mut self.stdout)?;
        self.active = true;
        Ok(())
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        leave_terminal_ui(&mut self.stdout)?;
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

    fn fill_rect(&mut self, rect: Rect, ch: char, fg: Color) {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                self.draw_char(x, y, ch, fg);
            }
        }
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
struct WorkspaceLayout {
    workspace_box: Rect,
    overview_box: Rect,
    overview_field: Rect,
    thought_box: Option<Rect>,
    thought_content: Option<Rect>,
    split_divider: Option<Rect>,
    split_hitbox: Option<Rect>,
    footer_start_y: u16,
}

impl WorkspaceLayout {
    fn for_terminal(width: u16, height: u16) -> Self {
        Self::for_terminal_with_ratio(width, height, THOUGHT_RAIL_DEFAULT_RATIO)
    }

    fn for_terminal_with_ratio(width: u16, height: u16, thought_ratio: f32) -> Self {
        let workspace_box = field_box(width, height);
        let footer_start_y = workspace_box.bottom() + 1;
        let inner = workspace_box.inset(1);

        let split_allowed = width >= THOUGHT_RAIL_MIN_WIDTH
            && inner.height >= 3
            && inner.width >= THOUGHT_RAIL_MIN_PANEL_WIDTH + THOUGHT_RAIL_GAP + ENTITY_WIDTH + 4;
        if !split_allowed {
            return Self {
                workspace_box,
                overview_box: workspace_box,
                overview_field: inner,
                thought_box: None,
                thought_content: None,
                split_divider: None,
                split_hitbox: None,
                footer_start_y,
            };
        }

        let min_overview_width = ENTITY_WIDTH + 4;
        let sanitized_ratio = if thought_ratio.is_finite() {
            thought_ratio.clamp(0.0, 1.0)
        } else {
            THOUGHT_RAIL_DEFAULT_RATIO
        };
        let ideal_thought_width = ((inner.width as f32) * sanitized_ratio).floor() as u16;
        let ideal_thought_width = ideal_thought_width.max(THOUGHT_RAIL_MIN_PANEL_WIDTH);
        let max_thought_width = inner
            .width
            .saturating_sub(THOUGHT_RAIL_GAP + min_overview_width);
        let thought_width = ideal_thought_width.min(max_thought_width);
        let overview_width = inner.width.saturating_sub(thought_width + THOUGHT_RAIL_GAP);
        if overview_width < min_overview_width {
            return Self {
                workspace_box,
                overview_box: workspace_box,
                overview_field: inner,
                thought_box: None,
                thought_content: None,
                split_divider: None,
                split_hitbox: None,
                footer_start_y,
            };
        }

        let thought_box = Rect {
            x: inner.x,
            y: inner.y,
            width: thought_width,
            height: inner.height,
        };
        let overview_box = Rect {
            x: thought_box.right() + THOUGHT_RAIL_GAP,
            y: inner.y,
            width: overview_width,
            height: inner.height,
        };
        let split_divider = Rect {
            x: thought_box.right(),
            y: inner.y,
            width: THOUGHT_RAIL_GAP,
            height: inner.height,
        };
        let split_hitbox = Rect {
            x: split_divider.x.saturating_sub(1),
            y: inner.y,
            width: THOUGHT_RAIL_DRAG_HITBOX_WIDTH,
            height: inner.height,
        };

        Self {
            workspace_box,
            overview_box,
            overview_field: overview_box.inset(1),
            thought_box: Some(thought_box),
            thought_content: Some(thought_box.inset(1)),
            split_divider: Some(split_divider),
            split_hitbox: Some(split_hitbox),
            footer_start_y,
        }
    }

    fn thought_entry_capacity(self) -> usize {
        self.thought_content
            .map(|content| content.height.saturating_sub(THOUGHT_RAIL_HEADER_ROWS) as usize)
            .unwrap_or(0)
    }

    fn thought_ratio_for_divider_x(self, x: u16) -> Option<f32> {
        let thought_box = self.thought_box?;
        let inner = self.workspace_box.inset(1);
        let min_overview_width = ENTITY_WIDTH + 4;
        let max_thought_width = inner
            .width
            .saturating_sub(THOUGHT_RAIL_GAP + min_overview_width);
        if max_thought_width < THOUGHT_RAIL_MIN_PANEL_WIDTH || inner.width == 0 {
            return None;
        }

        let requested_width = x.saturating_sub(thought_box.x);
        let thought_width = requested_width.clamp(THOUGHT_RAIL_MIN_PANEL_WIDTH, max_thought_width);
        Some(thought_width as f32 / inner.width as f32)
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
            SessionState::Error => Self::Error,
            SessionState::Exited => Self::Exited,
            SessionState::Idle | SessionState::Attention => match session.rest_state {
                RestState::DeepSleep | RestState::Sleeping => Self::Sleeping,
                RestState::Drowsy => Self::Drowsy,
                RestState::Active => match session.state {
                    SessionState::Attention => Self::Attention,
                    SessionState::Idle => Self::Active,
                    _ => unreachable!("only idle/attention reach active rest-state branch"),
                },
            },
        }
    }

    fn color(self) -> Color {
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
            Self::Drowsy => 0.0,
            Self::Sleeping => 0.0,
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

fn drowsy_frame(_tick: u64) -> [&'static str; 4] {
    [" .-^. ", "(- -)", " /|_| ", " _/ \\_"]
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

    fn is_sleeping(&self) -> bool {
        matches!(self.sprite_kind(), SpriteKind::Sleeping)
    }

    fn is_stationary(&self) -> bool {
        matches!(
            self.sprite_kind(),
            SpriteKind::Drowsy | SpriteKind::Sleeping | SpriteKind::Exited
        )
    }

    fn set_grid_position(&mut self, column: usize, row: usize) {
        self.x = column as f32 * ENTITY_WIDTH as f32;
        self.y = row as f32 * ENTITY_HEIGHT as f32;
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
}

trait TuiApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>>;
    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>>;
    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
    ) -> BoxFuture<'_, Result<DirListResponse, String>>;
    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>>;
}

impl TuiApi for ApiClient {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        Box::pin(async move {
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
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        Box::pin(async move {
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
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/native/open", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .json(&NativeDesktopOpenRequest { session_id })
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
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let path = path.map(|value| value.to_string());
        Box::pin(async move {
            let url = format!("{}/v1/dirs", self.base_url);
            let mut request = self.http.get(url);
            if let Some(path) = path {
                request = request.query(&[("path", path)]);
            }
            if managed_only {
                request = request.query(&[("managed_only", true)]);
            }

            let response = self
                .with_auth(request)
                .send()
                .await
                .map_err(|err| format!("failed to list dirs: {err}"))?;

            if response.status().is_success() {
                return response
                    .json::<DirListResponse>()
                    .await
                    .map_err(|err| format!("failed to parse dirs response: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        let cwd = cwd.to_string();
        Box::pin(async move {
            let url = format!("{}/v1/sessions", self.base_url);
            let response = self
                .with_auth(self.http.post(url))
                .json(&CreateSessionRequest {
                    name: None,
                    cwd: Some(cwd),
                    spawn_tool: Some(spawn_tool),
                    initial_request,
                })
                .send()
                .await
                .map_err(|err| format!("failed to create session: {err}"))?;

            if response.status().is_success() {
                return response
                    .json::<CreateSessionResponse>()
                    .await
                    .map_err(|err| format!("failed to parse create session response: {err}"));
            }

            Err(read_error(response).await)
        })
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PickerSelection {
    SpawnHere,
    Entry(usize),
}

#[derive(Clone)]
struct PickerState {
    anchor_x: u16,
    anchor_y: u16,
    base_path: String,
    current_path: String,
    entries: Vec<DirEntry>,
    current_theme_color: Option<Color>,
    entry_theme_colors: Vec<Option<Color>>,
    managed_only: bool,
    selection: PickerSelection,
    scroll: usize,
}

impl PickerState {
    fn new(anchor_x: u16, anchor_y: u16, response: DirListResponse, managed_only: bool) -> Self {
        Self {
            anchor_x,
            anchor_y,
            base_path: response.path.clone(),
            current_path: response.path,
            entries: response.entries,
            current_theme_color: None,
            entry_theme_colors: Vec::new(),
            managed_only,
            selection: PickerSelection::SpawnHere,
            scroll: 0,
        }
    }

    fn apply_response(&mut self, response: DirListResponse) {
        self.current_path = response.path;
        self.entries = response.entries;
        self.current_theme_color = None;
        self.entry_theme_colors.clear();
        self.selection = PickerSelection::SpawnHere;
        self.scroll = 0;
    }

    fn sync_theme_colors(&mut self, repo_themes: &mut HashMap<String, RepoTheme>) {
        self.current_theme_color = picker_theme_color_for_path(&self.current_path, repo_themes);
        self.entry_theme_colors = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.path_for_entry(index)
                    .and_then(|path| picker_theme_color_for_path(&path, repo_themes))
            })
            .collect();
    }

    fn at_root(&self) -> bool {
        normalize_path(&self.current_path) == normalize_path(&self.base_path)
    }

    fn parent_path(&self) -> Option<String> {
        if self.at_root() {
            return None;
        }

        let normalized = normalize_path(&self.current_path);
        let path = std::path::Path::new(&normalized);
        path.parent().map(|parent| {
            let raw = parent.to_string_lossy();
            if raw.is_empty() {
                "/".to_string()
            } else {
                raw.into_owned()
            }
        })
    }

    fn relative_label(&self) -> String {
        let base = normalize_path(&self.base_path);
        let current = normalize_path(&self.current_path);
        if current == base {
            return "/".to_string();
        }
        current
            .strip_prefix(&base)
            .filter(|suffix| !suffix.is_empty())
            .map(|suffix| suffix.to_string())
            .unwrap_or(current)
    }

    fn path_for_entry(&self, index: usize) -> Option<String> {
        let entry = self.entries.get(index)?;
        Some(join_path(&self.current_path, &entry.name))
    }

    fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.entries.is_empty() && matches!(self.selection, PickerSelection::SpawnHere) {
            return;
        }

        let total = self.entries.len() as isize + 1;
        let current = match self.selection {
            PickerSelection::SpawnHere => 0,
            PickerSelection::Entry(index) => index as isize + 1,
        };
        let next = (current + delta).clamp(0, total.saturating_sub(1));
        self.selection = if next == 0 {
            PickerSelection::SpawnHere
        } else {
            PickerSelection::Entry((next - 1) as usize)
        };
        self.ensure_selection_visible(visible_rows);
    }

    fn ensure_selection_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            self.scroll = 0;
            return;
        }
        let PickerSelection::Entry(index) = self.selection else {
            self.scroll = 0;
            return;
        };

        if index < self.scroll {
            self.scroll = index;
            return;
        }

        let last_visible = self.scroll + visible_rows.saturating_sub(1);
        if index > last_visible {
            self.scroll = index + 1 - visible_rows;
        }
    }
}

#[derive(Clone, Copy)]
enum PickerAction {
    Close,
    Up,
    ToggleManaged(bool),
    ActivateCurrentPath,
    ActivateEntry(usize),
}

#[derive(Clone, Copy)]
struct PickerLayout {
    frame: Rect,
    content: Rect,
    back_button: Option<Rect>,
    close_button: Rect,
    env_button: Rect,
    all_button: Rect,
    spawn_here_button: Rect,
    first_entry_y: u16,
    visible_entry_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InitialRequestState {
    cwd: String,
    value: String,
}

impl InitialRequestState {
    fn new(cwd: String) -> Self {
        Self {
            cwd,
            value: String::new(),
        }
    }

    fn trimmed_value(&self) -> Option<String> {
        let trimmed = self.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[derive(Clone, Copy)]
struct InitialRequestLayout {
    frame: Rect,
    content: Rect,
    input_y: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThoughtFingerprint {
    thought: String,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThoughtLogEntry {
    session_id: String,
    tmux_name: String,
    cwd: String,
    pwd_label: Option<String>,
    thought: String,
    updated_at: Option<DateTime<Utc>>,
    color: Color,
}

impl ThoughtLogEntry {
    fn from_session(
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
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ThoughtFilter {
    cwd: Option<String>,
    tmux_name: Option<String>,
}

impl ThoughtFilter {
    fn is_active(&self) -> bool {
        self.cwd.is_some() || self.tmux_name.is_some()
    }

    fn matches(&self, entry: &ThoughtLogEntry) -> bool {
        let cwd_matches = self
            .cwd
            .as_ref()
            .map(|cwd| entry.cwd == *cwd)
            .unwrap_or(true);
        let tmux_matches = self
            .tmux_name
            .as_ref()
            .map(|tmux_name| entry.tmux_name == *tmux_name)
            .unwrap_or(true);
        cwd_matches && tmux_matches
    }

    fn matches_session(&self, session: &SessionSummary) -> bool {
        let cwd_matches = self
            .cwd
            .as_ref()
            .map(|cwd| normalize_path(&session.cwd) == *cwd)
            .unwrap_or(true);
        let tmux_matches = self
            .tmux_name
            .as_ref()
            .map(|tmux_name| session.tmux_name == *tmux_name)
            .unwrap_or(true);
        cwd_matches && tmux_matches
    }

    fn clear(&mut self) {
        self.cwd = None;
        self.tmux_name = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThoughtRepoSummary {
    cwd: String,
    label: String,
    count: usize,
    color: Color,
    last_seen: usize,
}

fn normalize_thought_text(thought: Option<&str>) -> Option<String> {
    let thought = thought?.trim();
    if thought.is_empty() {
        return None;
    }
    Some(thought.to_string())
}

fn should_append_thought(
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

fn compare_thought_log_entries(left: &ThoughtLogEntry, right: &ThoughtLogEntry) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.tmux_name.cmp(&right.tmux_name))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

struct App<C: TuiApi> {
    runtime: Runtime,
    client: C,
    entities: Vec<SessionEntity>,
    thought_log: Vec<ThoughtLogEntry>,
    thought_filter: ThoughtFilter,
    last_logged_thoughts: HashMap<String, ThoughtFingerprint>,
    repo_themes: HashMap<String, RepoTheme>,
    selected_id: Option<String>,
    native_status: Option<NativeDesktopStatusResponse>,
    picker: Option<PickerState>,
    initial_request: Option<InitialRequestState>,
    message: Option<(String, Instant)>,
    last_refresh: Option<Instant>,
    thought_panel_ratio: f32,
    split_drag_active: bool,
    tick: u64,
}

impl<C: TuiApi> App<C> {
    fn new(runtime: Runtime, client: C) -> Self {
        Self {
            runtime,
            client,
            entities: Vec::new(),
            thought_log: Vec::new(),
            thought_filter: ThoughtFilter::default(),
            last_logged_thoughts: HashMap::new(),
            repo_themes: HashMap::new(),
            selected_id: None,
            native_status: None,
            picker: None,
            initial_request: None,
            message: None,
            last_refresh: None,
            thought_panel_ratio: THOUGHT_RAIL_DEFAULT_RATIO,
            split_drag_active: false,
            tick: 0,
        }
    }

    fn layout_for_terminal(&self, width: u16, height: u16) -> WorkspaceLayout {
        WorkspaceLayout::for_terminal_with_ratio(width, height, self.thought_panel_ratio)
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

    fn native_status_text(&self) -> String {
        match &self.native_status {
            Some(status) if status.supported => format!(
                "native open: {}",
                status.app.as_deref().unwrap_or("available")
            ),
            Some(status) => format!(
                "native open unavailable: {}",
                status.reason.as_deref().unwrap_or("unknown reason")
            ),
            None => "native open: checking".to_string(),
        }
    }

    fn header_right_text(&self) -> String {
        self.thought_filter
            .tmux_name
            .as_deref()
            .map(|tmux_name| format!("num={tmux_name} | {}", self.native_status_text()))
            .unwrap_or_else(|| self.native_status_text())
    }

    fn visible_entities(&self) -> Vec<&SessionEntity> {
        self.entities
            .iter()
            .filter(|entity| self.thought_filter.matches_session(&entity.session))
            .collect()
    }

    fn reconcile_selection(&mut self) {
        let selected_visible = self
            .selected_id
            .as_ref()
            .map(|selected| {
                self.entities.iter().any(|entity| {
                    entity.session.session_id == *selected
                        && self.thought_filter.matches_session(&entity.session)
                })
            })
            .unwrap_or(false);

        if !selected_visible {
            self.selected_id = self
                .entities
                .iter()
                .find(|entity| self.thought_filter.matches_session(&entity.session))
                .map(|entity| entity.session.session_id.clone());
        }
    }

    fn trim_thought_log(&mut self, capacity: usize) {
        if capacity == 0 || self.thought_log.len() <= capacity {
            return;
        }

        let drop_count = self.thought_log.len() - capacity;
        self.thought_log.drain(0..drop_count);
    }

    fn upsert_thought_log_entries(
        &mut self,
        entries: impl IntoIterator<Item = ThoughtLogEntry>,
        capacity: usize,
    ) {
        for entry in entries {
            if let Some(index) = self
                .thought_log
                .iter()
                .position(|existing| existing.session_id == entry.session_id)
            {
                self.thought_log.remove(index);
            }
            self.thought_log.push(entry);
        }
        self.thought_log.sort_by(compare_thought_log_entries);
        self.trim_thought_log(capacity);
    }

    fn visible_thought_entries(&self, capacity: usize) -> Vec<&ThoughtLogEntry> {
        if capacity == 0 {
            return Vec::new();
        }

        let filtered = self
            .thought_log
            .iter()
            .filter(|entry| self.thought_filter.matches(entry))
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(capacity);
        filtered[start..].to_vec()
    }

    fn thought_entry_display_color(&self, entry: &ThoughtLogEntry) -> Color {
        self.entities
            .iter()
            .find(|entity| entity.session.session_id == entry.session_id)
            .map(|entity| session_display_color(&entity.session, &self.repo_themes))
            .unwrap_or(entry.color)
    }

    fn thought_repo_summaries(&self) -> Vec<ThoughtRepoSummary> {
        let mut grouped = HashMap::<String, ThoughtRepoSummary>::new();
        for (index, entry) in self.thought_log.iter().enumerate() {
            let Some(label) = entry.pwd_label.as_ref() else {
                continue;
            };
            let color = self.thought_entry_display_color(entry);

            let summary = grouped
                .entry(entry.cwd.clone())
                .or_insert_with(|| ThoughtRepoSummary {
                    cwd: entry.cwd.clone(),
                    label: label.clone(),
                    count: 0,
                    color,
                    last_seen: index,
                });
            summary.count += 1;
            summary.color = color;
            summary.last_seen = index;
        }

        let mut summaries = grouped.into_values().collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_seen
                .cmp(&left.last_seen)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.cwd.cmp(&right.cwd))
        });
        summaries
    }

    fn active_thought_filter_text(&self) -> String {
        if !self.thought_filter.is_active() {
            return "filter: none".to_string();
        }

        let mut parts = Vec::new();
        if let Some(cwd) = self.thought_filter.cwd.as_deref() {
            parts.push(format!(
                "pwd={}",
                path_tail_label(cwd).unwrap_or_else(|| cwd.to_string())
            ));
        }
        if let Some(tmux_name) = self.thought_filter.tmux_name.as_deref() {
            parts.push(format!("num={tmux_name}"));
        }
        format!("filter: {}", parts.join(", "))
    }

    fn set_thought_filter_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = Some(cwd);
        self.reconcile_selection();
    }

    fn set_thought_filter_tmux_name(&mut self, tmux_name: String) {
        self.thought_filter.tmux_name = Some(tmux_name);
        self.reconcile_selection();
    }

    fn clear_thought_filters(&mut self) {
        self.thought_filter.clear();
        self.reconcile_selection();
    }

    fn reconcile_thought_log_sessions(&mut self, sessions: &[SessionSummary]) {
        let session_by_id = sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();

        self.thought_log
            .retain(|entry| session_by_id.contains_key(entry.session_id.as_str()));
        self.last_logged_thoughts
            .retain(|session_id, _| session_by_id.contains_key(session_id.as_str()));

        for entry in &mut self.thought_log {
            let Some(session) = session_by_id.get(entry.session_id.as_str()) else {
                continue;
            };
            entry.tmux_name = session.tmux_name.clone();
            entry.cwd = normalize_path(&session.cwd);
            entry.pwd_label = path_tail_label(&session.cwd);
            entry.color = session_display_color(session, &self.repo_themes);
        }

        self.thought_log.sort_by(compare_thought_log_entries);
    }

    fn capture_thought_updates(&mut self, sessions: &[SessionSummary], thought_capacity: usize) {
        let mut pending = Vec::new();
        for session in sessions {
            let Some(thought) = normalize_thought_text(session.thought.as_deref()) else {
                continue;
            };

            let incoming = ThoughtFingerprint {
                thought: thought.clone(),
                updated_at: session.thought_updated_at,
            };
            if !should_append_thought(
                self.last_logged_thoughts.get(&session.session_id),
                &incoming,
            ) {
                continue;
            }

            self.last_logged_thoughts
                .insert(session.session_id.clone(), incoming);
            pending.push(ThoughtLogEntry::from_session(
                session,
                thought,
                &self.repo_themes,
            ));
        }

        pending.sort_by(compare_thought_log_entries);

        if !pending.is_empty() {
            self.upsert_thought_log_entries(pending, thought_capacity);
        }
    }

    fn refresh(&mut self, layout: WorkspaceLayout) {
        match self.runtime.block_on(self.client.fetch_sessions()) {
            Ok(sessions) => {
                self.sync_repo_themes(&sessions);
                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);
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
        self.layout_sleeping_entities(field);
        self.reconcile_selection();
    }

    fn upsert_session(&mut self, session: SessionSummary, field: Rect) {
        let mut sessions: Vec<SessionSummary> = self
            .entities
            .iter()
            .map(|entity| entity.session.clone())
            .collect();
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.session_id == session.session_id)
        {
            *existing = session;
        } else {
            sessions.push(session);
        }
        self.merge_sessions(sessions, field);
    }

    fn sync_repo_themes(&mut self, sessions: &[SessionSummary]) {
        let mut next = HashMap::new();
        for session in sessions {
            let Some((theme_id, theme)) = discover_repo_theme(&session.cwd) else {
                continue;
            };
            next.insert(theme_id, theme);
        }
        self.repo_themes = next;
    }

    fn remember_repo_theme(&mut self, session: &SessionSummary, theme: Option<RepoTheme>) {
        if let (Some(theme_id), Some(theme)) = (session.repo_theme_id.as_ref(), theme) {
            self.repo_themes.insert(theme_id.clone(), theme);
            return;
        }

        if let Some((theme_id, resolved)) = discover_repo_theme(&session.cwd) {
            self.repo_themes.insert(theme_id, resolved);
        }
    }

    fn tick(&mut self, field: Rect) {
        self.tick = self.tick.wrapping_add(1);
        self.layout_sleeping_entities(field);
        for entity in &mut self.entities {
            entity.tick(field);
        }
        self.resolve_collisions(field);
    }

    fn layout_sleeping_entities(&mut self, field: Rect) {
        let rows = sleep_grid_rows(field);
        let mut sleeping_indices = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| entity.is_sleeping().then_some(index))
            .collect::<Vec<_>>();

        sleeping_indices.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });

        for (slot, entity_index) in sleeping_indices.into_iter().enumerate() {
            let row = slot % rows;
            let column = slot / rows;
            self.entities[entity_index].set_grid_position(column, row);
        }
    }

    fn resolve_collisions(&mut self, field: Rect) {
        for idx in 0..self.entities.len() {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            for b in right {
                let a_rect = a.screen_rect(field);
                let b_rect = b.screen_rect(field);
                if intersects(a_rect, b_rect) {
                    match (a.is_stationary(), b.is_stationary()) {
                        (true, true) => {}
                        (true, false) => separate_from_fixed_entity(b, a_rect, field),
                        (false, true) => separate_from_fixed_entity(a, b_rect, field),
                        (false, false) => {
                            std::mem::swap(&mut a.vx, &mut b.vx);
                            std::mem::swap(&mut a.vy, &mut b.vy);
                            a.x = (a.x - 1.0).max(0.0);
                            b.x = (b.x + 1.0).min(field.width.saturating_sub(ENTITY_WIDTH) as f32);
                        }
                    }
                }
            }
        }
    }

    fn move_selection(&mut self, delta: isize, field: Rect) {
        if let Some(picker) = &mut self.picker {
            let layout = picker_layout(picker, field);
            picker.move_selection(delta, layout.visible_entry_rows);
            return;
        }

        if self.entities.is_empty() {
            self.selected_id = None;
            return;
        }

        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.selected_id = None;
            return;
        }

        let current_index = self
            .selected_id
            .as_ref()
            .and_then(|selected| {
                visible_entities
                    .iter()
                    .position(|entity| entity.session.session_id == *selected)
            })
            .unwrap_or(0) as isize;

        let len = visible_entities.len() as isize;
        let next_index = (current_index + delta).rem_euclid(len) as usize;
        self.selected_id = Some(visible_entities[next_index].session.session_id.clone());
    }

    fn selected(&self) -> Option<&SessionEntity> {
        let selected = self.selected_id.as_ref()?;
        self.entities.iter().find(|entity| {
            entity.session.session_id == *selected
                && self.thought_filter.matches_session(&entity.session)
        })
    }

    fn close_picker(&mut self) {
        self.picker = None;
        self.initial_request = None;
    }

    fn open_picker(&mut self, x: u16, y: u16) {
        match self.runtime.block_on(self.client.list_dirs(None, true)) {
            Ok(response) => {
                let mut picker = PickerState::new(x, y, response, true);
                picker.sync_theme_colors(&mut self.repo_themes);
                self.picker = Some(picker);
            }
            Err(err) => {
                self.set_message(err);
                self.picker = None;
            }
        }
    }

    fn picker_reload(&mut self, path: Option<String>, managed_only: bool) {
        let target = path.clone();
        match self
            .runtime
            .block_on(self.client.list_dirs(target.as_deref(), managed_only))
        {
            Ok(response) => {
                if let Some(picker) = &mut self.picker {
                    picker.managed_only = managed_only;
                    picker.apply_response(response);
                    picker.sync_theme_colors(&mut self.repo_themes);
                }
            }
            Err(err) => self.set_message(err),
        }
    }

    fn picker_up(&mut self) {
        let Some(parent_path) = self.picker.as_ref().and_then(PickerState::parent_path) else {
            return;
        };
        let managed_only = self
            .picker
            .as_ref()
            .map(|picker| picker.managed_only)
            .unwrap_or(true);
        self.picker_reload(Some(parent_path), managed_only);
    }

    fn picker_set_managed_only(&mut self, managed_only: bool) {
        let Some(picker) = &self.picker else {
            return;
        };
        if picker.managed_only == managed_only {
            return;
        }
        self.picker_reload(Some(picker.current_path.clone()), managed_only);
    }

    fn open_initial_request(&mut self, cwd: String) {
        self.initial_request = Some(InitialRequestState::new(cwd));
    }

    fn close_initial_request(&mut self) {
        self.initial_request = None;
    }

    fn handle_initial_request_key(&mut self, key: KeyEvent, field: Rect) {
        match key.code {
            KeyCode::Esc => self.close_initial_request(),
            KeyCode::Enter => self.submit_initial_request(field),
            KeyCode::Backspace => {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.pop();
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.push(ch);
                }
            }
            _ => {}
        }
    }

    fn submit_initial_request(&mut self, field: Rect) {
        let Some(initial_request) = self
            .initial_request
            .as_ref()
            .and_then(InitialRequestState::trimmed_value)
        else {
            self.set_message("enter an initial request");
            return;
        };
        let Some(cwd) = self.initial_request.as_ref().map(|state| state.cwd.clone()) else {
            return;
        };
        self.spawn_session(&cwd, Some(initial_request), field);
    }

    fn picker_activate_selection(&mut self, _field: Rect) {
        let Some((selection, current_path, entry_path, has_children)) =
            self.picker.as_ref().map(|picker| match picker.selection {
                PickerSelection::SpawnHere => (
                    PickerSelection::SpawnHere,
                    picker.current_path.clone(),
                    None,
                    false,
                ),
                PickerSelection::Entry(index) => (
                    PickerSelection::Entry(index),
                    picker.current_path.clone(),
                    picker.path_for_entry(index),
                    picker
                        .entries
                        .get(index)
                        .map(|entry| entry.has_children)
                        .unwrap_or(false),
                ),
            })
        else {
            return;
        };

        match selection {
            PickerSelection::SpawnHere => self.open_initial_request(current_path),
            PickerSelection::Entry(_) if has_children => {
                if let Some(path) = entry_path {
                    let managed_only = self
                        .picker
                        .as_ref()
                        .map(|picker| picker.managed_only)
                        .unwrap_or(true);
                    self.picker_reload(Some(path), managed_only);
                }
            }
            PickerSelection::Entry(_) => {
                if let Some(path) = entry_path {
                    self.open_initial_request(path);
                }
            }
        }
    }

    fn spawn_session(&mut self, cwd: &str, initial_request: Option<String>, field: Rect) {
        match self.runtime.block_on(self.client.create_session(
            cwd,
            SpawnTool::Codex,
            initial_request,
        )) {
            Ok(response) => {
                let repo_theme = response.repo_theme.clone();
                let session = response.session;
                let session_id = session.session_id.clone();
                let tmux_name = session.tmux_name.clone();
                self.remember_repo_theme(&session, repo_theme);
                self.upsert_session(session, field);
                self.selected_id = Some(session_id);
                self.reconcile_selection();
                self.close_picker();
                self.set_message(format!("created {tmux_name}"));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn open_session_for_label(&mut self, session_id: &str, label: &str) {
        match self.runtime.block_on(self.client.open_session(session_id)) {
            Ok(response) => {
                self.set_message(format!("{} {}", response.status, label));
            }
            Err(err) => self.set_message(err),
        }
    }

    fn open_selected(&mut self) {
        let Some((selected_id, label)) = self.selected().map(|entity| {
            (
                entity.session.session_id.clone(),
                selected_label(Some(&entity.session.tmux_name)),
            )
        }) else {
            self.set_message("no session selected");
            return;
        };

        self.open_session_for_label(&selected_id, &label);
    }

    fn handle_thought_click(
        &mut self,
        x: u16,
        y: u16,
        thought_content: Rect,
        entry_capacity: usize,
    ) {
        if let Some(action) = thought_panel_action_at(self, thought_content, entry_capacity, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    fn handle_header_filter_click(&mut self, renderer_width: u16, x: u16, y: u16) {
        if let Some(action) = header_filter_action_at(self, renderer_width, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    fn apply_thought_filter_action(&mut self, action: ThoughtPanelAction) {
        match action {
            ThoughtPanelAction::FilterByCwd(cwd) => self.set_thought_filter_cwd(cwd),
            ThoughtPanelAction::FilterByTmuxName(tmux_name) => {
                self.set_thought_filter_tmux_name(tmux_name);
            }
            ThoughtPanelAction::OpenRepoInEditor(cwd) => self.open_repo_in_editor(&cwd),
            ThoughtPanelAction::ClearFilters => self.clear_thought_filters(),
        }
    }

    fn open_repo_in_editor(&mut self, cwd: &str) {
        let repo_label = path_tail_label(cwd).unwrap_or_else(|| cwd.to_string());
        match ProcessCommand::new("code")
            .arg(".")
            .current_dir(cwd)
            .spawn()
        {
            Ok(_) => self.set_message(format!("code . -> {repo_label}")),
            Err(err) => self.set_message(format!("failed to run code .: {err}")),
        }
    }

    fn start_split_drag(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let resized = self.resize_thought_panel(layout, x);
        self.split_drag_active = resized;
        resized
    }

    fn drag_split(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        if !self.split_drag_active {
            return false;
        }
        self.resize_thought_panel(layout, x)
    }

    fn stop_split_drag(&mut self) {
        self.split_drag_active = false;
    }

    fn resize_thought_panel(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let Some(ratio) = layout.thought_ratio_for_divider_x(x) else {
            return false;
        };
        self.thought_panel_ratio = ratio;
        true
    }

    fn handle_field_click(&mut self, x: u16, y: u16, field: Rect) {
        if self.initial_request.is_some() {
            return;
        }

        if let Some(picker) = &self.picker {
            let layout = picker_layout(picker, field);
            if layout.frame.contains(x, y) {
                if let Some(action) = picker_action_at(picker, layout, x, y) {
                    self.handle_picker_action(action, field);
                }
                return;
            }
            self.close_picker();
            return;
        }

        let hit = self
            .visible_entities()
            .into_iter()
            .find(|entity| entity.screen_rect(field).contains(x, y))
            .map(|entity| entity.session.session_id.clone());

        if let Some(session_id) = hit {
            self.selected_id = Some(session_id);
            self.open_selected();
            return;
        }

        self.open_picker(x, y);
    }

    fn handle_picker_action(&mut self, action: PickerAction, field: Rect) {
        match action {
            PickerAction::Close => self.close_picker(),
            PickerAction::Up => self.picker_up(),
            PickerAction::ToggleManaged(managed_only) => {
                self.picker_set_managed_only(managed_only);
            }
            PickerAction::ActivateCurrentPath => self.spawn_session_from_picker(field),
            PickerAction::ActivateEntry(index) => self.activate_picker_entry(index, field),
        }
    }

    fn spawn_session_from_picker(&mut self, _field: Rect) {
        let Some(path) = self
            .picker
            .as_ref()
            .map(|picker| picker.current_path.clone())
        else {
            return;
        };
        self.open_initial_request(path);
    }

    fn activate_picker_entry(&mut self, index: usize, _field: Rect) {
        let Some((path, has_children, managed_only)) = self.picker.as_ref().and_then(|picker| {
            Some((
                picker.path_for_entry(index)?,
                picker.entries.get(index)?.has_children,
                picker.managed_only,
            ))
        }) else {
            return;
        };

        if has_children {
            self.picker_reload(Some(path), managed_only);
        } else {
            self.open_initial_request(path);
        }
    }

    fn render(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.clear();

        if renderer.width() < MIN_WIDTH || renderer.height() < MIN_HEIGHT {
            render_too_small(renderer);
            return;
        }

        let frame = frame_rect(renderer.width(), renderer.height());

        renderer.draw_box(frame, Color::DarkGrey);
        renderer.draw_text(2, 1, "throngterm tui", Color::Cyan);

        let max_right_width = renderer.width().saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let right_x = renderer
            .width()
            .saturating_sub(display_width(&right_text))
            .saturating_sub(2);
        renderer.draw_text(right_x, 1, &right_text, Color::DarkGrey);
        render_header_filter_strip(self, renderer, renderer.width());

        renderer.draw_box(layout.workspace_box, Color::DarkGrey);

        if let (Some(thought_box), Some(thought_content)) =
            (layout.thought_box, layout.thought_content)
        {
            renderer.draw_box(thought_box, Color::DarkGrey);
            renderer.draw_box(layout.overview_box, Color::DarkGrey);
            if let Some(split_divider) = layout.split_divider {
                let divider_color = if self.split_drag_active {
                    Color::Cyan
                } else {
                    Color::DarkGrey
                };
                renderer.draw_vline(
                    split_divider.x,
                    split_divider.y,
                    split_divider.height,
                    ':',
                    divider_color,
                );
            }
            render_thought_panel(
                self,
                renderer,
                thought_content,
                layout.thought_entry_capacity(),
            );
        }

        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            let empty = if self.entities.is_empty() {
                "no tmux sessions found - press r after starting one"
            } else if self.thought_filter.is_active() {
                "no thronglets match filters"
            } else {
                "no tmux sessions found - press r after starting one"
            };
            let x = layout.overview_field.x.saturating_add(
                layout
                    .overview_field
                    .width
                    .saturating_sub(empty.len() as u16)
                    / 2,
            );
            let y = layout.overview_field.y + layout.overview_field.height / 2;
            renderer.draw_text(x, y, empty, Color::DarkGrey);
        }

        for entity in visible_entities {
            let rect = entity.screen_rect(layout.overview_field);
            let selected = self
                .selected_id
                .as_ref()
                .map(|selected| *selected == entity.session.session_id)
                .unwrap_or(false);
            render_entity(
                renderer,
                entity,
                rect,
                selected,
                self.tick,
                &self.repo_themes,
            );
        }

        if let Some(picker) = &self.picker {
            render_picker(renderer, picker, layout.overview_field);
        }
        if let Some(initial_request) = &self.initial_request {
            render_initial_request(renderer, initial_request, layout.overview_field);
        }

        render_footer(self, renderer, layout.footer_start_y);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ThoughtPanelAction {
    FilterByCwd(String),
    FilterByTmuxName(String),
    OpenRepoInEditor(String),
    ClearFilters,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThoughtChipLayout {
    rect: Rect,
    cwd: String,
    label: String,
    color: Color,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThoughtRowLayout {
    row_rect: Option<Rect>,
    tmux_name: String,
    line: String,
    color: Color,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ThoughtPanelLayout {
    rows: Vec<ThoughtRowLayout>,
    empty_message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HeaderFilterLayout {
    chips: Vec<ThoughtChipLayout>,
    clear_filters_rect: Option<Rect>,
}

fn header_filter_row() -> u16 {
    2
}

fn build_header_filter_layout<C: TuiApi>(app: &App<C>, width: u16) -> HeaderFilterLayout {
    if width <= 4 {
        return HeaderFilterLayout::default();
    }

    let left_x = 2;
    let right_edge = width.saturating_sub(2);
    if right_edge <= left_x {
        return HeaderFilterLayout::default();
    }

    let clear_label = "[clear filters]";
    let clear_width = display_width(clear_label);
    let clear_gap = 2;
    let mut available_width = right_edge.saturating_sub(left_x);
    let mut clear_filters_rect = None;
    if app.thought_filter.is_active() {
        if clear_width <= available_width {
            available_width = available_width.saturating_sub(clear_width);
            if available_width >= clear_gap {
                available_width = available_width.saturating_sub(clear_gap);
            } else {
                available_width = 0;
            }
        } else {
            return HeaderFilterLayout {
                chips: Vec::new(),
                clear_filters_rect: Some(Rect {
                    x: right_edge.saturating_sub(clear_width),
                    y: header_filter_row(),
                    width: clear_width,
                    height: 1,
                }),
            };
        }
    }

    let mut included = Vec::new();
    let active_cwd = app.thought_filter.cwd.as_deref();
    let mut chips_width: u16 = 0;
    for summary in app.thought_repo_summaries() {
        let is_active = active_cwd.map(|cwd| cwd == summary.cwd).unwrap_or(false);
        let label = if is_active {
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
        if next_width > available_width {
            break;
        }

        chips_width = next_width;
        let color = if active_cwd.is_some() && !is_active {
            Color::DarkGrey
        } else {
            summary.color
        };
        included.push((summary.cwd, label, color, width));
    }

    let total_width = chips_width.saturating_add(if app.thought_filter.is_active() {
        clear_gap + clear_width
    } else {
        0
    });
    let mut chip_x = right_edge.saturating_sub(total_width);
    let chips = included
        .into_iter()
        .map(|(cwd, label, color, width)| {
            let rect = Rect {
                x: chip_x,
                y: header_filter_row(),
                width,
                height: 1,
            };
            chip_x = chip_x.saturating_add(width).saturating_add(2);
            ThoughtChipLayout {
                rect,
                cwd,
                label,
                color,
            }
        })
        .collect::<Vec<_>>();

    if app.thought_filter.is_active() {
        clear_filters_rect = Some(Rect {
            x: right_edge.saturating_sub(clear_width),
            y: header_filter_row(),
            width: clear_width,
            height: 1,
        });
    }

    HeaderFilterLayout {
        chips,
        clear_filters_rect,
    }
}

fn render_header_filter_strip<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, width: u16) {
    let layout = build_header_filter_layout(app, width);
    for chip in &layout.chips {
        renderer.draw_text(chip.rect.x, chip.rect.y, &chip.label, chip.color);
    }

    if let Some(rect) = layout.clear_filters_rect {
        renderer.draw_text(rect.x, rect.y, "[clear filters]", Color::Cyan);
    }
}

fn header_filter_action_at<C: TuiApi>(
    app: &App<C>,
    width: u16,
    x: u16,
    y: u16,
) -> Option<ThoughtPanelAction> {
    let layout = build_header_filter_layout(app, width);
    if let Some(rect) = layout.clear_filters_rect {
        if rect.contains(x, y) {
            return Some(ThoughtPanelAction::ClearFilters);
        }
    }

    for chip in layout.chips {
        if chip.rect.contains(x, y) {
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

fn display_width(text: &str) -> u16 {
    text.chars().count().min(u16::MAX as usize) as u16
}

fn path_tail_label(path: &str) -> Option<String> {
    let normalized = normalize_path(path.trim());
    if normalized == "/" {
        return None;
    }

    normalized
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
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

fn format_thought_lines(entry: &ThoughtLogEntry, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return Vec::new();
    }

    let thought = entry.thought.replace('\n', " ");
    let line = format!("{}: {}", entry.tmux_name, thought);
    wrap_text(&line, max_chars)
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let trimmed = value.trim();
    if trimmed.len() != 7 || !trimmed.starts_with('#') {
        return None;
    }

    let r = u8::from_str_radix(&trimmed[1..3], 16).ok()?;
    let g = u8::from_str_radix(&trimmed[3..5], 16).ok()?;
    let b = u8::from_str_radix(&trimmed[5..7], 16).ok()?;
    Some(Color::Rgb { r, g, b })
}

fn session_theme_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Option<Color> {
    let theme_id = session.repo_theme_id.as_ref()?;
    let theme = repo_themes.get(theme_id)?;
    parse_hex_color(&theme.body)
}

fn session_display_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Color {
    session_theme_color(session, repo_themes)
        .unwrap_or_else(|| SpriteKind::from_session(session).color())
}

fn render_thought_panel<C: TuiApi>(
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
        renderer.draw_text(
            thought_content.x,
            start_y + offset as u16,
            &row.line,
            row.color,
        );
    }
}

fn build_thought_panel<C: TuiApi>(
    app: &App<C>,
    thought_content: Rect,
    entry_capacity: usize,
) -> ThoughtPanelLayout {
    if thought_content.width == 0 || thought_content.height == 0 {
        return ThoughtPanelLayout::default();
    }

    let entries = app.visible_thought_entries(entry_capacity);
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

    let mut rows = entries
        .into_iter()
        .flat_map(|entry| {
            let color = app.thought_entry_display_color(entry);
            format_thought_lines(entry, thought_content.width as usize)
                .into_iter()
                .map(move |line| {
                    let visible_line_width = display_width(&line);
                    ThoughtRowLayout {
                        row_rect: (visible_line_width > 0).then_some(Rect {
                            x: thought_content.x,
                            y: 0,
                            width: visible_line_width,
                            height: 1,
                        }),
                        tmux_name: entry.tmux_name.clone(),
                        line,
                        color,
                    }
                })
        })
        .collect::<Vec<_>>();

    if rows.len() > entry_capacity {
        let start = rows.len().saturating_sub(entry_capacity);
        rows = rows.split_off(start);
    }

    ThoughtPanelLayout {
        rows,
        empty_message,
    }
}

fn thought_panel_action_at<C: TuiApi>(
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
    for (offset, row) in panel.rows.into_iter().enumerate() {
        let Some(rect) = row.row_rect else {
            continue;
        };
        let rect = Rect {
            x: rect.x,
            y: row_start_y + offset as u16,
            width: rect.width,
            height: rect.height,
        };
        if rect.contains(x, y) {
            return Some(ThoughtPanelAction::FilterByTmuxName(row.tmux_name));
        }
    }

    None
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn join_path(base: &str, name: &str) -> String {
    let base = normalize_path(base);
    let name = name.trim_matches('/');
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn picker_layout(picker: &PickerState, field: Rect) -> PickerLayout {
    let width = PICKER_WIDTH.min(field.width);
    let max_height = PICKER_MAX_HEIGHT.min(field.height);
    let header_rows = 4;
    let entry_capacity = max_height.saturating_sub(2 + header_rows).max(1);
    let list_rows = picker.entries.len().max(1).min(entry_capacity as usize) as u16;
    let height = 2 + header_rows + list_rows;

    let max_x = field.right().saturating_sub(width);
    let max_y = field.bottom().saturating_sub(height);

    let mut x = picker.anchor_x;
    if x + width > field.right() {
        x = picker.anchor_x.saturating_sub(width.saturating_sub(1));
    }
    x = x.max(field.x).min(max_x);

    let mut y = picker.anchor_y;
    if y + height > field.bottom() {
        y = picker.anchor_y.saturating_sub(height.saturating_sub(1));
    }
    y = y.max(field.y).min(max_y);

    let frame = Rect {
        x,
        y,
        width,
        height,
    };
    let content = frame.inset(1);
    let close_button = Rect {
        x: content.right().saturating_sub(3),
        y: content.y,
        width: 3,
        height: 1,
    };
    let back_button = if picker.at_root() {
        None
    } else {
        Some(Rect {
            x: content.x,
            y: content.y + 1,
            width: 4,
            height: 1,
        })
    };
    let env_button = Rect {
        x: content.x,
        y: content.y + 2,
        width: 13,
        height: 1,
    };
    let all_button = Rect {
        x: (content.x + 15).min(content.right().saturating_sub(13)),
        y: content.y + 2,
        width: 13,
        height: 1,
    };

    PickerLayout {
        frame,
        content,
        back_button,
        close_button,
        env_button,
        all_button,
        spawn_here_button: Rect {
            x: content.x,
            y: content.y + 3,
            width: content.width,
            height: 1,
        },
        first_entry_y: content.y + 4,
        visible_entry_rows: list_rows as usize,
    }
}

fn picker_action_at(
    picker: &PickerState,
    layout: PickerLayout,
    x: u16,
    y: u16,
) -> Option<PickerAction> {
    if layout.close_button.contains(x, y) {
        return Some(PickerAction::Close);
    }
    if layout
        .back_button
        .map(|button| button.contains(x, y))
        .unwrap_or(false)
    {
        return Some(PickerAction::Up);
    }
    if layout.env_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(true));
    }
    if layout.all_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(false));
    }
    if layout.spawn_here_button.contains(x, y) {
        return Some(PickerAction::ActivateCurrentPath);
    }
    if y >= layout.first_entry_y
        && y < layout.first_entry_y + layout.visible_entry_rows as u16
        && x >= layout.content.x
        && x < layout.content.right()
    {
        let index = picker.scroll + (y - layout.first_entry_y) as usize;
        if index < picker.entries.len() {
            return Some(PickerAction::ActivateEntry(index));
        }
    }
    None
}

fn picker_theme_color_for_path(
    path: &str,
    repo_themes: &mut HashMap<String, RepoTheme>,
) -> Option<Color> {
    let (theme_id, theme) = existing_repo_theme(path)?;
    let color = parse_hex_color(&theme.body)?;
    repo_themes.insert(theme_id, theme);
    Some(color)
}

fn render_picker(renderer: &mut Renderer, picker: &PickerState, field: Rect) {
    let layout = picker_layout(picker, field);
    let picker_color = picker.current_theme_color.unwrap_or(Color::White);
    let picker_accent = picker.current_theme_color.unwrap_or(Color::Cyan);
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, picker_color);

    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        "spawn codex",
        picker_accent,
    );
    renderer.draw_text(
        layout.close_button.x,
        layout.close_button.y,
        "[x]",
        Color::DarkGrey,
    );

    let path_x = layout
        .back_button
        .map(|button| {
            renderer.draw_text(button.x, button.y, "[..]", Color::DarkGrey);
            button.right().saturating_add(1)
        })
        .unwrap_or(layout.content.x);
    let path_width = layout.content.right().saturating_sub(path_x) as usize;
    let path_label = truncate_label(&picker.relative_label(), path_width);
    renderer.draw_text(path_x, layout.content.y + 1, &path_label, picker_color);

    renderer.draw_text(
        layout.env_button.x,
        layout.env_button.y,
        "[env managed]",
        if picker.managed_only {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    renderer.draw_text(
        layout.all_button.x,
        layout.all_button.y,
        "[all folders]",
        if picker.managed_only {
            Color::DarkGrey
        } else {
            Color::White
        },
    );

    let spawn_color = if matches!(picker.selection, PickerSelection::SpawnHere) {
        picker.current_theme_color.unwrap_or(Color::White)
    } else {
        picker.current_theme_color.unwrap_or(Color::Yellow)
    };
    let spawn_line = format!(
        "{} + spawn here",
        if matches!(picker.selection, PickerSelection::SpawnHere) {
            ">"
        } else {
            " "
        }
    );
    renderer.draw_text(
        layout.spawn_here_button.x,
        layout.spawn_here_button.y,
        &truncate_label(&spawn_line, layout.spawn_here_button.width as usize),
        spawn_color,
    );

    if picker.entries.is_empty() {
        renderer.draw_text(
            layout.content.x,
            layout.first_entry_y,
            "  empty",
            Color::DarkGrey,
        );
        return;
    }

    for row in 0..layout.visible_entry_rows {
        let index = picker.scroll + row;
        if index >= picker.entries.len() {
            break;
        }
        let entry = &picker.entries[index];
        let marker = if picker.selection == PickerSelection::Entry(index) {
            ">"
        } else {
            " "
        };
        let icon = if entry.has_children { ">" } else { "+" };
        let running = match entry.is_running {
            Some(true) => " *",
            Some(false) => " -",
            None => "",
        };
        let line = format!("{marker} {icon} {}{}", entry.name, running);
        let themed_color = picker.entry_theme_colors.get(index).copied().flatten();
        let color = if picker.selection == PickerSelection::Entry(index) {
            themed_color.unwrap_or(Color::White)
        } else if let Some(theme_color) = themed_color {
            theme_color
        } else if entry.has_children {
            Color::Cyan
        } else {
            Color::DarkGrey
        };
        renderer.draw_text(
            layout.content.x,
            layout.first_entry_y + row as u16,
            &truncate_label(&line, layout.content.width as usize),
            color,
        );
    }
}

fn initial_request_layout(field: Rect) -> InitialRequestLayout {
    let width = INITIAL_REQUEST_WIDTH.min(field.width);
    let height = INITIAL_REQUEST_HEIGHT.min(field.height);
    let x = field.x + field.width.saturating_sub(width) / 2;
    let y = field.y + field.height.saturating_sub(height) / 2;
    let frame = Rect {
        x,
        y,
        width,
        height,
    };
    let content = frame.inset(1);

    InitialRequestLayout {
        frame,
        content,
        input_y: content.y + 3,
    }
}

fn render_initial_request(
    renderer: &mut Renderer,
    initial_request: &InitialRequestState,
    field: Rect,
) {
    let layout = initial_request_layout(field);
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, Color::White);
    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        "initial request",
        Color::Cyan,
    );

    let cwd_line = format!(
        "cwd: {}",
        shorten_path(
            &initial_request.cwd,
            layout.content.width.saturating_sub(5) as usize,
        )
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 1,
        &truncate_label(&cwd_line, layout.content.width as usize),
        Color::DarkGrey,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 2,
        "enter creates hidden thronglet  esc cancels",
        Color::DarkGrey,
    );

    let input_x = layout.content.x;
    renderer.draw_text(input_x, layout.input_y, "> ", Color::White);
    let available = layout.content.width.saturating_sub(3) as usize;
    let (text, color) = if initial_request.value.is_empty() {
        ("type initial request".to_string(), Color::DarkGrey)
    } else {
        (tail_text(&initial_request.value, available), Color::White)
    };
    let visible = truncate_label(&text, available);
    renderer.draw_text(input_x + 2, layout.input_y, &visible, color);
    let cursor_x = if initial_request.value.is_empty() {
        input_x + 2
    } else {
        input_x + 2 + visible.chars().count() as u16
    };
    if cursor_x < layout.content.right() {
        renderer.draw_char(cursor_x, layout.input_y, '|', Color::Yellow);
    }
}

fn render_entity(
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

fn render_footer<C: TuiApi>(app: &App<C>, renderer: &mut Renderer, start_y: u16) {
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
    } else if app.picker.is_some() {
        "picker: enter/right select  up/down or jk move  h/backspace up  e env  a all  esc close"
    } else {
        "click empty field spawn  click/enter open  arrows or hjkl move  r refresh  q quit"
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

fn sleep_grid_rows(field: Rect) -> usize {
    usize::from((field.height / ENTITY_HEIGHT).max(1))
}

fn compare_sleepiness(left: &SessionSummary, right: &SessionSummary) -> Ordering {
    left.last_activity_at
        .cmp(&right.last_activity_at)
        .then_with(|| left.tmux_name.cmp(&right.tmux_name))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

fn separate_from_fixed_entity(entity: &mut SessionEntity, obstacle: Rect, field: Rect) {
    let max_x = field.width.saturating_sub(ENTITY_WIDTH);
    let max_y = field.height.saturating_sub(ENTITY_HEIGHT);
    let entity_rect = entity.screen_rect(field);
    let entity_center_x = u32::from(entity_rect.x) + u32::from(entity_rect.width / 2);
    let obstacle_center_x = u32::from(obstacle.x) + u32::from(obstacle.width / 2);
    let entity_center_y = u32::from(entity_rect.y) + u32::from(entity_rect.height / 2);
    let obstacle_center_y = u32::from(obstacle.y) + u32::from(obstacle.height / 2);
    let obstacle_rel_x = obstacle.x.saturating_sub(field.x);
    let obstacle_rel_y = obstacle.y.saturating_sub(field.y);
    let obstacle_rel_right = obstacle_rel_x.saturating_add(obstacle.width);
    let obstacle_rel_bottom = obstacle_rel_y.saturating_add(obstacle.height);

    entity.vx = -entity.vx;
    entity.vy = -entity.vy;
    entity.x = if entity_center_x < obstacle_center_x {
        obstacle_rel_x.saturating_sub(ENTITY_WIDTH) as f32
    } else {
        obstacle_rel_right.min(max_x) as f32
    };
    entity.y = if entity_center_y < obstacle_center_y {
        obstacle_rel_y.saturating_sub(ENTITY_HEIGHT) as f32
    } else {
        obstacle_rel_bottom.min(max_y) as f32
    };
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

fn tail_text(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return chars.into_iter().collect();
    }
    if max_chars == 0 {
        return String::new();
    }
    chars[chars.len() - max_chars..].iter().collect()
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
    let initial_layout = app.layout_for_terminal(renderer.width(), renderer.height());
    app.refresh(initial_layout);

    loop {
        let layout = app.layout_for_terminal(renderer.width(), renderer.height());
        if layout.split_divider.is_none() {
            app.stop_split_drag();
        }
        app.trim_thought_log(layout.thought_entry_capacity());

        if app.should_refresh() {
            app.refresh(layout);
        }

        app.tick(layout.overview_field);
        app.render(&mut renderer, layout);
        renderer.flush()?;

        if event::poll(FRAME_DURATION)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.initial_request.is_some() {
                        app.handle_initial_request_key(key, layout.overview_field);
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Esc => {
                            if app.picker.is_some() {
                                app.close_picker();
                            } else {
                                break;
                            }
                        }
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                            if app.picker.is_some() {
                                app.picker_up();
                            } else {
                                app.move_selection(-1, layout.overview_field);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.move_selection(-1, layout.overview_field);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.move_selection(1, layout.overview_field);
                        }
                        KeyCode::Right
                        | KeyCode::Char('l')
                        | KeyCode::Enter
                        | KeyCode::Char('o') => {
                            if app.picker.is_some() {
                                app.picker_activate_selection(layout.overview_field);
                            } else {
                                app.open_selected();
                            }
                        }
                        KeyCode::Char('e') => {
                            app.picker_set_managed_only(true);
                        }
                        KeyCode::Char('a') => {
                            app.picker_set_managed_only(false);
                        }
                        KeyCode::Char('r') => {
                            if let Some((path, managed_only)) = app
                                .picker
                                .as_ref()
                                .map(|picker| (picker.current_path.clone(), picker.managed_only))
                            {
                                app.picker_reload(Some(path), managed_only);
                            } else {
                                app.refresh(layout);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                {
                    if app.initial_request.is_some() {
                        continue;
                    }
                    if layout
                        .split_hitbox
                        .map(|hitbox| hitbox.contains(mouse.column, mouse.row))
                        .unwrap_or(false)
                    {
                        app.start_split_drag(layout, mouse.column);
                    } else if header_filter_action_at(
                        &app,
                        renderer.width(),
                        mouse.column,
                        mouse.row,
                    )
                    .is_some()
                    {
                        app.handle_header_filter_click(renderer.width(), mouse.column, mouse.row);
                    } else if let Some(thought_box) = layout.thought_box {
                        if thought_box.contains(mouse.column, mouse.row) {
                            if let Some(thought_content) = layout.thought_content {
                                app.handle_thought_click(
                                    mouse.column,
                                    mouse.row,
                                    thought_content,
                                    layout.thought_entry_capacity(),
                                );
                            }
                        } else if layout.overview_field.contains(mouse.column, mouse.row) {
                            app.handle_field_click(mouse.column, mouse.row, layout.overview_field);
                        }
                    } else if layout.overview_field.contains(mouse.column, mouse.row) {
                        app.handle_field_click(mouse.column, mouse.row, layout.overview_field);
                    }
                }
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) =>
                {
                    if app.drag_split(layout, mouse.column) {
                        continue;
                    }
                }
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) =>
                {
                    app.stop_split_drag();
                }
                Event::Resize(width, height) => {
                    app.stop_split_drag();
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
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use tempfile::tempdir;
    use throngterm::types::{ThoughtSource, ThoughtState, TransportHealth};

    #[derive(Default)]
    struct MockApiState {
        fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
        native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
        open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
        list_dirs_results: VecDeque<Result<DirListResponse, String>>,
        create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
        open_calls: Vec<String>,
        list_calls: Vec<(Option<String>, bool)>,
        create_calls: Vec<(String, SpawnTool, Option<String>)>,
    }

    #[derive(Clone, Default)]
    struct MockApi {
        state: Arc<Mutex<MockApiState>>,
    }

    impl MockApi {
        fn new() -> Self {
            Self::default()
        }

        fn push_fetch_sessions(&self, result: Result<Vec<SessionSummary>, String>) {
            self.state
                .lock()
                .unwrap()
                .fetch_sessions_results
                .push_back(result);
        }

        fn push_list_dirs(&self, result: Result<DirListResponse, String>) {
            self.state
                .lock()
                .unwrap()
                .list_dirs_results
                .push_back(result);
        }

        fn push_create_session(&self, result: Result<CreateSessionResponse, String>) {
            self.state
                .lock()
                .unwrap()
                .create_session_results
                .push_back(result);
        }

        fn push_open_session(&self, result: Result<NativeDesktopOpenResponse, String>) {
            self.state
                .lock()
                .unwrap()
                .open_session_results
                .push_back(result);
        }

        fn list_calls(&self) -> Vec<(Option<String>, bool)> {
            self.state.lock().unwrap().list_calls.clone()
        }

        fn create_calls(&self) -> Vec<(String, SpawnTool, Option<String>)> {
            self.state.lock().unwrap().create_calls.clone()
        }

        fn open_calls(&self) -> Vec<String> {
            self.state.lock().unwrap().open_calls.clone()
        }
    }

    impl TuiApi for MockApi {
        fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
            let state = self.state.clone();
            Box::pin(async move {
                state
                    .lock()
                    .unwrap()
                    .fetch_sessions_results
                    .pop_front()
                    .unwrap_or_else(|| Ok(Vec::new()))
            })
        }

        fn fetch_native_status(
            &self,
        ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
            let state = self.state.clone();
            Box::pin(async move {
                state
                    .lock()
                    .unwrap()
                    .native_status_results
                    .pop_front()
                    .unwrap_or_else(|| {
                        Ok(NativeDesktopStatusResponse {
                            supported: true,
                            platform: Some("test".to_string()),
                            app: Some("test".to_string()),
                            reason: None,
                        })
                    })
            })
        }

        fn open_session(
            &self,
            session_id: &str,
        ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
            let state = self.state.clone();
            let session_id = session_id.to_string();
            Box::pin(async move {
                let mut state = state.lock().unwrap();
                state.open_calls.push(session_id);
                state
                    .open_session_results
                    .pop_front()
                    .unwrap_or_else(|| Err("unexpected open_session".to_string()))
            })
        }

        fn list_dirs(
            &self,
            path: Option<&str>,
            managed_only: bool,
        ) -> BoxFuture<'_, Result<DirListResponse, String>> {
            let state = self.state.clone();
            let path = path.map(|value| value.to_string());
            Box::pin(async move {
                let mut state = state.lock().unwrap();
                state.list_calls.push((path, managed_only));
                state
                    .list_dirs_results
                    .pop_front()
                    .unwrap_or_else(|| Err("unexpected list_dirs".to_string()))
            })
        }

        fn create_session(
            &self,
            cwd: &str,
            spawn_tool: SpawnTool,
            initial_request: Option<String>,
        ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
            let state = self.state.clone();
            let cwd = cwd.to_string();
            Box::pin(async move {
                let mut state = state.lock().unwrap();
                state.create_calls.push((cwd, spawn_tool, initial_request));
                state
                    .create_session_results
                    .pop_front()
                    .unwrap_or_else(|| Err("unexpected create_session".to_string()))
            })
        }
    }

    fn test_runtime() -> Runtime {
        Runtime::new().expect("test runtime")
    }

    fn test_field() -> Rect {
        Rect {
            x: 1,
            y: 3,
            width: 78,
            height: 14,
        }
    }

    fn test_layout(width: u16, height: u16) -> WorkspaceLayout {
        WorkspaceLayout::for_terminal(width, height)
    }

    fn test_layout_with_ratio(width: u16, height: u16, thought_ratio: f32) -> WorkspaceLayout {
        WorkspaceLayout::for_terminal_with_ratio(width, height, thought_ratio)
    }

    fn make_app(api: MockApi) -> App<MockApi> {
        App::new(test_runtime(), api)
    }

    fn test_renderer(width: u16, height: u16) -> Renderer {
        let buffer_size = (width as usize) * (height as usize);
        Renderer {
            stdout: BufWriter::new(io::stdout()),
            width,
            height,
            buffer: vec![Cell::default(); buffer_size],
            last_buffer: vec![Cell::default(); buffer_size],
            active: false,
        }
    }

    #[test]
    fn leave_terminal_ui_disables_mouse_before_leaving_alt_screen() {
        let mut output = Vec::new();

        leave_terminal_ui(&mut output).expect("leave terminal UI should write ANSI codes");

        assert_eq!(
            String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
            concat!(
                "\u{1b}[?1006l",
                "\u{1b}[?1015l",
                "\u{1b}[?1003l",
                "\u{1b}[?1002l",
                "\u{1b}[?1000l",
                "\u{1b}[?1049l",
                "\u{1b}[?25h",
                "\u{1b}[0m",
            )
        );
    }

    #[test]
    fn cleanup_is_noop_when_renderer_is_inactive() {
        let mut renderer = test_renderer(80, 24);

        renderer.cleanup().expect("inactive cleanup should succeed");

        assert!(!renderer.active);
    }

    fn cell_at(renderer: &Renderer, x: u16, y: u16) -> Cell {
        renderer.buffer[(y as usize) * (renderer.width as usize) + (x as usize)]
    }

    fn row_text(renderer: &Renderer, y: u16) -> String {
        (0..renderer.width)
            .map(|x| cell_at(renderer, x, y).ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn visible_entity_ids(app: &App<MockApi>) -> Vec<String> {
        app.visible_entities()
            .into_iter()
            .map(|entity| entity.session.session_id.clone())
            .collect()
    }

    fn session_summary(session_id: &str, tmux_name: &str, cwd: &str) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: tmux_name.to_string(),
            state: SessionState::Idle,
            current_command: None,
            cwd: cwd.to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
            repo_theme_id: None,
        }
    }

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn session_summary_with_thought(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        thought: &str,
        updated_at: &str,
    ) -> SessionSummary {
        let mut session = session_summary(session_id, tmux_name, cwd);
        session.thought = Some(thought.to_string());
        session.thought_state = ThoughtState::Active;
        session.rest_state = RestState::Active;
        session.thought_updated_at = Some(timestamp(updated_at));
        session
    }

    fn sleeping_session(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        last_activity_at: &str,
    ) -> SessionSummary {
        let mut session = session_summary(session_id, tmux_name, cwd);
        session.thought_state = ThoughtState::Sleeping;
        session.rest_state = RestState::Sleeping;
        session.last_activity_at = timestamp(last_activity_at);
        session
    }

    fn attention_session(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        rest_state: RestState,
        last_activity_at: &str,
    ) -> SessionSummary {
        let mut session = session_summary(session_id, tmux_name, cwd);
        session.state = SessionState::Attention;
        session.rest_state = rest_state;
        session.thought_state = match rest_state {
            RestState::Sleeping | RestState::DeepSleep => ThoughtState::Sleeping,
            RestState::Active | RestState::Drowsy => ThoughtState::Holding,
        };
        session.last_activity_at = timestamp(last_activity_at);
        session
    }

    fn repo_theme(body: &str) -> RepoTheme {
        RepoTheme {
            body: body.to_string(),
            outline: "#222222".to_string(),
            accent: "#111111".to_string(),
            shirt: "#333333".to_string(),
        }
    }

    fn dir_response(path: &str, names: &[(&str, bool)]) -> DirListResponse {
        DirListResponse {
            path: path.to_string(),
            entries: names
                .iter()
                .map(|(name, has_children)| DirEntry {
                    name: (*name).to_string(),
                    has_children: *has_children,
                    is_running: None,
                })
                .collect(),
        }
    }

    fn write_repo_theme_file(path: &std::path::Path, body: &str) {
        let throngterm_dir = path.join(".throngterm");
        fs::create_dir_all(&throngterm_dir).expect("create .throngterm");
        let contents = format!(
            concat!(
                "{{\n",
                "  \"palette\": {{\n",
                "    \"body\": \"{}\",\n",
                "    \"outline\": \"#3D2F24\",\n",
                "    \"accent\": \"#1D1914\",\n",
                "    \"shirt\": \"#AA9370\"\n",
                "  }}\n",
                "}}\n"
            ),
            body,
        );
        fs::write(throngterm_dir.join("colors.json"), contents).expect("write colors.json");
    }

    fn create_response(session_id: &str, tmux_name: &str, cwd: &str) -> CreateSessionResponse {
        CreateSessionResponse {
            session: session_summary(session_id, tmux_name, cwd),
            sprite_pack: None,
            repo_theme: None,
        }
    }

    fn create_response_with_theme(
        session: SessionSummary,
        repo_theme: RepoTheme,
    ) -> CreateSessionResponse {
        CreateSessionResponse {
            session,
            sprite_pack: None,
            repo_theme: Some(repo_theme),
        }
    }

    fn entity_at(
        field: Rect,
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        x: u16,
        y: u16,
    ) -> SessionEntity {
        let mut entity = SessionEntity::new(session_summary(session_id, tmux_name, cwd), field);
        entity.x = x.saturating_sub(field.x) as f32;
        entity.y = y.saturating_sub(field.y) as f32;
        entity
    }

    fn entity_rect_for(app: &App<MockApi>, session_id: &str, field: Rect) -> Rect {
        app.entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .expect("entity should exist")
            .screen_rect(field)
    }

    fn sleep_grid_rect(field: Rect, slot: usize) -> Rect {
        let rows = sleep_grid_rows(field);
        Rect {
            x: field.x + (slot / rows) as u16 * ENTITY_WIDTH,
            y: field.y + (slot % rows) as u16 * ENTITY_HEIGHT,
            width: ENTITY_WIDTH,
            height: ENTITY_HEIGHT,
        }
    }

    #[test]
    fn wide_layout_enables_global_thought_rail() {
        let layout = test_layout(120, 32);

        assert!(layout.thought_box.is_some());
        assert!(layout.thought_content.is_some());
        assert!(layout.thought_entry_capacity() > 0);
        assert!(layout.overview_box.x > layout.workspace_box.x);
    }

    #[test]
    fn narrow_layout_keeps_single_overview_field() {
        let layout = test_layout(96, 24);

        assert!(layout.thought_box.is_none());
        assert!(layout.thought_content.is_none());
        assert_eq!(layout.thought_entry_capacity(), 0);
        assert_eq!(layout.overview_box.x, layout.workspace_box.x);
        assert_eq!(layout.overview_field, layout.workspace_box.inset(1));
    }

    #[test]
    fn custom_split_ratio_changes_thought_rail_width() {
        let default_layout = test_layout(120, 32);
        let wider_layout = test_layout_with_ratio(120, 32, 0.5);

        assert_eq!(
            default_layout.split_divider.map(|divider| divider.width),
            Some(THOUGHT_RAIL_GAP)
        );
        assert!(
            wider_layout
                .thought_box
                .expect("wide layout should include thought rail")
                .width
                > default_layout
                    .thought_box
                    .expect("default layout should include thought rail")
                    .width
        );
        assert!(
            wider_layout.overview_field.width < default_layout.overview_field.width,
            "widening the clawgs rail should shrink the throngterm field"
        );
    }

    #[test]
    fn divider_drag_updates_thought_rail_ratio() {
        let api = MockApi::new();
        let mut app = make_app(api);
        let initial_layout = app.layout_for_terminal(120, 32);
        let initial_width = initial_layout
            .thought_box
            .expect("wide layout should include thought rail")
            .width;
        let divider = initial_layout
            .split_divider
            .expect("wide layout should expose a divider");
        let hitbox = initial_layout
            .split_hitbox
            .expect("wide layout should expose a divider hitbox");
        assert!(hitbox.contains(divider.x, divider.y));

        assert!(app.start_split_drag(initial_layout, divider.x));
        assert!(app.split_drag_active);
        assert!(app.drag_split(initial_layout, divider.x + 10));

        let dragged_layout = app.layout_for_terminal(120, 32);
        let dragged_width = dragged_layout
            .thought_box
            .expect("dragged layout should include thought rail")
            .width;
        assert!(dragged_width > initial_width);

        app.stop_split_drag();
        assert!(!app.split_drag_active);
    }

    #[test]
    fn refresh_keeps_latest_thought_per_session_in_timestamp_order() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![
            session_summary_with_thought(
                "sess-2",
                "beta",
                "/Users/b/repos/beta",
                "indexing repo",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-1",
                "alpha",
                "/Users/b/repos/alpha",
                "writing tests",
                "2026-03-08T14:00:06Z",
            ),
        ]));
        api.push_fetch_sessions(Ok(vec![
            session_summary_with_thought(
                "sess-2",
                "beta",
                "/Users/b/repos/beta",
                "indexing repo",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-1",
                "alpha",
                "/Users/b/repos/alpha",
                "patching sidebar",
                "2026-03-08T14:00:07Z",
            ),
        ]));
        let mut app = make_app(api);

        app.refresh(layout);
        app.refresh(layout);

        assert_eq!(
            app.thought_log
                .iter()
                .map(|entry| (entry.session_id.as_str(), entry.thought.as_str()))
                .collect::<Vec<_>>(),
            vec![("sess-2", "indexing repo"), ("sess-1", "patching sidebar"),]
        );
    }

    #[test]
    fn refresh_ignores_null_duplicate_and_stale_thoughts() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
            "sess-3",
            "gamma",
            "/Users/b/repos/gamma",
            "reading logs",
            "2026-03-08T14:00:05Z",
        )]));

        let mut duplicate = session_summary_with_thought(
            "sess-3",
            "gamma",
            "/Users/b/repos/gamma",
            "reading logs",
            "2026-03-08T14:00:05Z",
        );
        let mut stale = session_summary_with_thought(
            "sess-3",
            "gamma",
            "/Users/b/repos/gamma",
            "reading logs",
            "2026-03-08T14:00:04Z",
        );
        let mut cleared = session_summary("sess-3", "gamma", "/Users/b/repos/gamma");
        duplicate.last_activity_at = timestamp("2026-03-08T14:00:06Z");
        stale.last_activity_at = timestamp("2026-03-08T14:00:07Z");
        cleared.last_activity_at = timestamp("2026-03-08T14:00:08Z");

        api.push_fetch_sessions(Ok(vec![duplicate]));
        api.push_fetch_sessions(Ok(vec![stale]));
        api.push_fetch_sessions(Ok(vec![cleared]));

        let mut app = make_app(api);
        app.refresh(layout);
        app.refresh(layout);
        app.refresh(layout);
        app.refresh(layout);

        assert_eq!(app.thought_log.len(), 1);
        assert_eq!(app.thought_log[0].thought, "reading logs");
    }

    #[test]
    fn selection_changes_do_not_reset_global_thought_timeline() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        app.merge_sessions(
            vec![
                session_summary("sess-1", "alpha", "/Users/b/repos/alpha"),
                session_summary("sess-2", "beta", "/Users/b/repos/beta"),
            ],
            layout.overview_field,
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "alpha",
                "/Users/b/repos/alpha",
                "patching sidebar",
                "2026-03-08T14:00:07Z",
            )],
            layout.thought_entry_capacity(),
        );
        app.selected_id = Some("sess-1".to_string());
        let before = app.thought_log.clone();

        app.move_selection(1, layout.overview_field);

        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
        assert_eq!(app.thought_log, before);
    }

    #[test]
    fn thought_timeline_trims_to_visible_capacity() {
        let api = MockApi::new();
        let layout = test_layout(120, 24);
        let mut app = make_app(api);
        assert_eq!(layout.thought_entry_capacity(), 10);

        for idx in 0..15 {
            let second = idx + 1;
            let updated_at = format!("2026-03-08T14:00:{second:02}Z");
            let thought = format!("thought {idx}");
            let session_id = format!("sess-{idx}");
            let tmux_name = format!("alpha-{idx}");
            let session = session_summary_with_thought(
                &session_id,
                &tmux_name,
                "/Users/b/repos/alpha",
                &thought,
                &updated_at,
            );
            app.capture_thought_updates(&[session], layout.thought_entry_capacity());
        }

        assert_eq!(app.thought_log.len(), 10);
        assert_eq!(
            app.thought_log.first().map(|entry| entry.thought.as_str()),
            Some("thought 5")
        );
        assert_eq!(
            app.thought_log.last().map(|entry| entry.thought.as_str()),
            Some("thought 14")
        );
    }

    #[test]
    fn refresh_prunes_exited_sessions_from_thought_timeline_and_header_filter_chips() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        api.push_fetch_sessions(Ok(vec![
            session_summary_with_thought(
                "sess-1",
                "7",
                "/Users/b/repos/throngterm",
                "patching tui",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-2",
                "9",
                "/Users/b/repos/opensource/skills",
                "indexing docs",
                "2026-03-08T14:00:06Z",
            ),
        ]));
        api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
            "sess-2",
            "9",
            "/Users/b/repos/opensource/skills",
            "indexing docs",
            "2026-03-08T14:00:06Z",
        )]));
        let mut app = make_app(api);

        app.refresh(layout);
        let initial_header = build_header_filter_layout(&app, 120);
        assert!(initial_header
            .chips
            .iter()
            .any(|chip| chip.label == "1xthrongterm"));
        assert!(initial_header
            .chips
            .iter()
            .any(|chip| chip.label == "1xskills"));

        app.refresh(layout);

        assert_eq!(
            app.thought_log
                .iter()
                .map(|entry| entry.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sess-2"]
        );

        let header = build_header_filter_layout(&app, 120);
        assert_eq!(
            header
                .chips
                .iter()
                .map(|chip| chip.label.as_str())
                .collect::<Vec<_>>(),
            vec!["1xskills"]
        );
        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        assert_eq!(
            panel
                .rows
                .iter()
                .map(|row| row.line.as_str())
                .collect::<Vec<_>>(),
            vec!["9: indexing docs"]
        );
    }

    #[test]
    fn render_header_filter_strip_shows_repo_chips_and_thought_rows() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api);

        let throngterm_theme_id = "/tmp/throngterm".to_string();
        let skills_theme_id = "/tmp/skills".to_string();
        let throngterm_color = Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        };
        let skills_color = Color::Rgb {
            r: 79,
            g: 166,
            b: 106,
        };
        app.repo_themes
            .insert(throngterm_theme_id.clone(), repo_theme("#B89875"));
        app.repo_themes
            .insert(skills_theme_id.clone(), repo_theme("#4FA66A"));

        let mut first = session_summary_with_thought(
            "sess-1",
            "7",
            "/Users/b/repos/throngterm",
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        first.repo_theme_id = Some(throngterm_theme_id.clone());

        let mut second = session_summary_with_thought(
            "sess-2",
            "2",
            "/Users/b/repos/throngterm",
            "wiring filter state",
            "2026-03-08T14:00:06Z",
        );
        second.repo_theme_id = Some(throngterm_theme_id);

        let mut third = session_summary_with_thought(
            "sess-3",
            "9",
            "/Users/b/repos/opensource/skills",
            "indexing docs",
            "2026-03-08T14:00:07Z",
        );
        third.repo_theme_id = Some(skills_theme_id);

        app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        assert_eq!(
            panel
                .rows
                .iter()
                .map(|row| row.line.as_str())
                .collect::<Vec<_>>(),
            vec![
                "7: patching tui",
                "2: wiring filter state",
                "9: indexing docs",
            ]
        );

        let header = build_header_filter_layout(&app, 120);
        let throngterm_chip = header
            .chips
            .iter()
            .find(|chip| chip.label == "2xthrongterm")
            .expect("throngterm chip should exist");
        let skills_chip = header
            .chips
            .iter()
            .find(|chip| chip.label == "1xskills")
            .expect("skills chip should exist");
        assert_eq!(throngterm_chip.color, throngterm_color);
        assert_eq!(skills_chip.color, skills_color);

        let mut renderer = test_renderer(120, 32);
        render_header_filter_strip(&app, &mut renderer, 120);

        assert_eq!(
            cell_at(&renderer, throngterm_chip.rect.x, throngterm_chip.rect.y).fg,
            throngterm_color
        );
        assert_eq!(
            cell_at(&renderer, skills_chip.rect.x, skills_chip.rect.y).fg,
            skills_color
        );
        assert!(row_text(&renderer, 2).ends_with("1xskills  2xthrongterm"));
    }

    #[test]
    fn active_repo_header_chip_maps_to_code_open_action() {
        let api = MockApi::new();
        let mut app = make_app(api);
        app.repo_themes
            .insert("/tmp/throngterm".to_string(), repo_theme("#B89875"));
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "7",
                "/Users/b/repos/throngterm",
                "patching tui",
                "2026-03-08T14:00:05Z",
            )],
            test_layout(120, 32).thought_entry_capacity(),
        );
        app.set_thought_filter_cwd("/Users/b/repos/throngterm".to_string());

        let header = build_header_filter_layout(&app, 120);
        let active_chip = header
            .chips
            .iter()
            .find(|chip| chip.label == "code .")
            .expect("active repo chip should expose code dot")
            .clone();

        assert_eq!(
            header_filter_action_at(&app, 120, active_chip.rect.x, active_chip.rect.y),
            Some(ThoughtPanelAction::OpenRepoInEditor(
                "/Users/b/repos/throngterm".to_string()
            ))
        );
    }

    #[test]
    fn header_filter_strip_and_thought_rows_apply_and_clear_filters() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api);

        app.repo_themes
            .insert("/tmp/throngterm".to_string(), repo_theme("#B89875"));
        app.repo_themes
            .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

        let mut first = session_summary_with_thought(
            "sess-1",
            "7",
            "/Users/b/repos/throngterm",
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        first.repo_theme_id = Some("/tmp/throngterm".to_string());

        let mut second = session_summary_with_thought(
            "sess-2",
            "2",
            "/Users/b/repos/throngterm",
            "wiring filter state",
            "2026-03-08T14:00:06Z",
        );
        second.repo_theme_id = Some("/tmp/throngterm".to_string());

        let mut third = session_summary_with_thought(
            "sess-3",
            "9",
            "/Users/b/repos/opensource/skills",
            "indexing docs",
            "2026-03-08T14:00:07Z",
        );
        third.repo_theme_id = Some("/tmp/skills".to_string());

        app.merge_sessions(
            vec![first.clone(), second.clone(), third.clone()],
            layout.overview_field,
        );
        app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

        let initial_header = build_header_filter_layout(&app, 120);
        let chip = initial_header
            .chips
            .iter()
            .find(|chip| chip.label == "2xthrongterm")
            .expect("throngterm chip should exist")
            .clone();
        app.handle_header_filter_click(120, chip.rect.x, chip.rect.y);

        assert_eq!(
            app.thought_filter.cwd.as_deref(),
            Some("/Users/b/repos/throngterm")
        );
        assert_eq!(app.active_thought_filter_text(), "filter: pwd=throngterm");
        assert_eq!(
            app.visible_thought_entries(layout.thought_entry_capacity())
                .into_iter()
                .map(|entry| entry.tmux_name.as_str())
                .collect::<Vec<_>>(),
            vec!["7", "2"]
        );
        assert_eq!(
            visible_entity_ids(&app),
            vec!["sess-2".to_string(), "sess-1".to_string()]
        );

        let filtered_header = build_header_filter_layout(&app, 120);
        let active_chip = filtered_header
            .chips
            .iter()
            .find(|chip| chip.label == "code .")
            .expect("active repo chip should become code dot");
        let dimmed_chip = filtered_header
            .chips
            .iter()
            .find(|chip| chip.label == "1xskills")
            .expect("inactive repo chip should stay visible");
        assert_eq!(dimmed_chip.color, Color::DarkGrey);

        let mut renderer = test_renderer(120, 32);
        app.render(&mut renderer, layout);
        assert!(!row_text(&renderer, 1).contains("filter: pwd"));
        assert_eq!(
            cell_at(&renderer, active_chip.rect.x, active_chip.rect.y).fg,
            active_chip.color
        );
        assert_eq!(
            cell_at(&renderer, dimmed_chip.rect.x, dimmed_chip.rect.y).fg,
            Color::DarkGrey
        );
        assert!(row_text(&renderer, 2).contains("code ."));
        assert!(row_text(&renderer, 2).contains("1xskills"));
        assert!(row_text(&renderer, 2).contains("[clear filters]"));

        let filtered_panel =
            build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let row_index = filtered_panel
            .rows
            .iter()
            .position(|row| row.tmux_name == "2")
            .expect("session 2 row should exist");
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(filtered_panel.rows.len() as u16);
        let row_rect = filtered_panel.rows[row_index]
            .row_rect
            .expect("row should have a click target");
        app.selected_id = Some("sess-3".to_string());
        app.handle_thought_click(
            row_rect.x.saturating_add(4),
            row_start_y + row_index as u16,
            thought_content,
            layout.thought_entry_capacity(),
        );

        assert_eq!(
            app.thought_filter.cwd.as_deref(),
            Some("/Users/b/repos/throngterm")
        );
        assert_eq!(app.thought_filter.tmux_name.as_deref(), Some("2"));
        assert_eq!(
            app.active_thought_filter_text(),
            "filter: pwd=throngterm, num=2"
        );
        assert_eq!(
            app.visible_thought_entries(layout.thought_entry_capacity())
                .into_iter()
                .map(|entry| entry.tmux_name.as_str())
                .collect::<Vec<_>>(),
            vec!["2"]
        );
        assert_eq!(visible_entity_ids(&app), vec!["sess-2".to_string()]);
        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

        let cleared_header = build_header_filter_layout(&app, 120);
        let clear_rect = cleared_header
            .clear_filters_rect
            .expect("clear filters button should exist");
        app.handle_header_filter_click(120, clear_rect.x, clear_rect.y);

        assert_eq!(app.thought_filter, ThoughtFilter::default());
        assert_eq!(app.active_thought_filter_text(), "filter: none");
        assert_eq!(
            app.visible_thought_entries(layout.thought_entry_capacity())
                .into_iter()
                .map(|entry| entry.tmux_name.as_str())
                .collect::<Vec<_>>(),
            vec!["7", "2", "9"]
        );
        assert_eq!(
            visible_entity_ids(&app),
            vec![
                "sess-2".to_string(),
                "sess-1".to_string(),
                "sess-3".to_string(),
            ]
        );
    }

    #[test]
    fn clicking_thought_body_filters_to_that_session() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api);
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "7",
                "/Users/b/repos/throngterm",
                "patching tui",
                "2026-03-08T14:00:05Z",
            )],
            layout.thought_entry_capacity(),
        );

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);
        let line = panel.rows[0].line.clone();
        let body_x = thought_content
            .x
            .saturating_add(display_width("7").saturating_add(3));
        assert!(body_x < thought_content.x.saturating_add(display_width(&line)));

        app.handle_thought_click(
            body_x,
            row_start_y,
            thought_content,
            layout.thought_entry_capacity(),
        );

        assert_eq!(app.thought_filter.tmux_name.as_deref(), Some("7"));
        assert_eq!(app.active_thought_filter_text(), "filter: num=7");
    }

    #[test]
    fn wrapped_latest_thought_stays_bottom_aligned() {
        let api = MockApi::new();
        let mut app = make_app(api);
        let thought_content = Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 5,
        };

        app.capture_thought_updates(
            &[
                session_summary_with_thought(
                    "sess-1",
                    "7",
                    "/Users/b/repos/throngterm",
                    "older",
                    "2026-03-08T14:00:05Z",
                ),
                session_summary_with_thought(
                    "sess-2",
                    "9",
                    "/Users/b/repos/throngterm",
                    "latest thought stays at bottom",
                    "2026-03-08T14:00:06Z",
                ),
            ],
            4,
        );

        let panel = build_thought_panel(&app, thought_content, 4);

        assert_eq!(
            panel
                .rows
                .iter()
                .map(|row| row.line.as_str())
                .collect::<Vec<_>>(),
            vec!["9: latest", "thought", "stays at", "bottom"]
        );
        assert_eq!(
            panel.rows.last().map(|row| row.line.as_str()),
            Some("bottom")
        );
    }

    #[test]
    fn clicking_wrapped_thought_line_filters_to_that_session() {
        let api = MockApi::new();
        let mut app = make_app(api);
        let thought_content = Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 5,
        };
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-2",
                "9",
                "/Users/b/repos/throngterm",
                "latest thought stays at bottom",
                "2026-03-08T14:00:06Z",
            )],
            4,
        );

        let panel = build_thought_panel(&app, thought_content, 4);
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);

        app.handle_thought_click(1, row_start_y + 3, thought_content, 4);

        assert_eq!(app.thought_filter.tmux_name.as_deref(), Some("9"));
        assert_eq!(app.active_thought_filter_text(), "filter: num=9");
    }

    #[test]
    fn repo_theme_colors_override_state_colors_in_thought_history() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        let theme_id = "/tmp/buildooor".to_string();
        let theme_color = Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        };
        app.repo_themes.insert(
            theme_id.clone(),
            RepoTheme {
                body: "#B89875".to_string(),
                outline: "#3D2F24".to_string(),
                accent: "#1D1914".to_string(),
                shirt: "#AA9370".to_string(),
            },
        );

        let mut busy = session_summary_with_thought(
            "sess-1",
            "alpha",
            "/Users/b/repos/alpha",
            "indexing repo",
            "2026-03-08T14:00:05Z",
        );
        busy.state = SessionState::Busy;
        busy.repo_theme_id = Some(theme_id.clone());

        let mut attention = session_summary_with_thought(
            "sess-1",
            "alpha",
            "/Users/b/repos/alpha",
            "needs input",
            "2026-03-08T14:00:06Z",
        );
        attention.state = SessionState::Attention;
        attention.repo_theme_id = Some(theme_id);

        app.capture_thought_updates(&[busy], layout.thought_entry_capacity());
        app.capture_thought_updates(&[attention], layout.thought_entry_capacity());

        assert_eq!(
            app.thought_log
                .iter()
                .map(|entry| entry.color)
                .collect::<Vec<_>>(),
            vec![theme_color]
        );

        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut renderer = test_renderer(120, 32);
        render_thought_panel(
            &app,
            &mut renderer,
            thought_content,
            layout.thought_entry_capacity(),
        );

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);
        assert_eq!(panel.rows.len(), 1);
        assert_eq!(cell_at(&renderer, thought_content.x, row_start_y).ch, 'a');
        assert_eq!(
            cell_at(&renderer, thought_content.x, row_start_y).fg,
            theme_color
        );
    }

    #[test]
    fn thought_history_rows_follow_live_session_color() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api);

        let mut session = session_summary_with_thought(
            "sess-1",
            "alpha",
            "/Users/b/repos/throngterm",
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        session.state = SessionState::Busy;

        app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
        app.merge_sessions(vec![session.clone()], layout.overview_field);

        session.state = SessionState::Attention;
        session.last_activity_at = timestamp("2026-03-08T14:00:06Z");
        app.merge_sessions(vec![session], layout.overview_field);

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let header = build_header_filter_layout(&app, 120);
        let chip = header
            .chips
            .iter()
            .find(|chip| chip.label == "1xthrongterm")
            .expect("repo chip should exist");

        assert_eq!(panel.rows.len(), 1);
        assert_eq!(panel.rows[0].color, Color::Magenta);
        assert_eq!(chip.color, Color::Magenta);
    }

    #[test]
    fn render_entity_uses_repo_theme_body_color() {
        let field = test_layout(120, 32).overview_field;
        let mut session = session_summary("sess-1", "alpha", "/Users/b/repos/buildooor");
        session.state = SessionState::Busy;
        session.repo_theme_id = Some("/tmp/buildooor".to_string());
        let entity = SessionEntity::new(session, field);
        let mut repo_themes = HashMap::new();
        repo_themes.insert(
            "/tmp/buildooor".to_string(),
            RepoTheme {
                body: "#B89875".to_string(),
                outline: "#3D2F24".to_string(),
                accent: "#1D1914".to_string(),
                shirt: "#AA9370".to_string(),
            },
        );
        let rect = entity.screen_rect(field);
        let mut renderer = test_renderer(120, 32);

        render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

        assert_eq!(
            cell_at(&renderer, rect.x, rect.y).fg,
            Color::Rgb {
                r: 184,
                g: 152,
                b: 117,
            }
        );
    }

    #[test]
    fn selected_entity_preserves_repo_theme_body_color() {
        let field = test_layout(120, 32).overview_field;
        let mut session = session_summary("sess-1", "alpha", "/Users/b/repos/buildooor");
        session.state = SessionState::Busy;
        session.repo_theme_id = Some("/tmp/buildooor".to_string());
        let entity = SessionEntity::new(session, field);
        let mut repo_themes = HashMap::new();
        repo_themes.insert("/tmp/buildooor".to_string(), repo_theme("#B89875"));
        let rect = entity.screen_rect(field);
        let mut renderer = test_renderer(120, 32);

        render_entity(&mut renderer, &entity, rect, true, 0, &repo_themes);

        assert_eq!(
            cell_at(&renderer, rect.x, rect.y).fg,
            Color::Rgb {
                r: 184,
                g: 152,
                b: 117,
            }
        );
        assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
        assert_eq!(
            cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
            Color::White
        );
    }

    #[test]
    fn selected_entity_preserves_fallback_state_color() {
        let field = test_layout(120, 32).overview_field;
        let mut session = session_summary("sess-1", "alpha", "/Users/b/repos/throngterm");
        session.state = SessionState::Attention;
        session.rest_state = RestState::Active;
        let entity = SessionEntity::new(session, field);
        let rect = entity.screen_rect(field);
        let mut renderer = test_renderer(120, 32);

        render_entity(&mut renderer, &entity, rect, true, 0, &HashMap::new());

        assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, Color::Magenta);
        assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
        assert_eq!(
            cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
            Color::White
        );
    }

    #[test]
    fn spawned_selected_entity_matches_thought_color() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let field = layout.overview_field;
        let theme_id = "/tmp/throngterm".to_string();
        let theme_color = Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        };
        let mut spawned_session = session_summary("sess-42", "42", "/Users/b/repos/throngterm");
        spawned_session.repo_theme_id = Some(theme_id.clone());
        api.push_create_session(Ok(create_response_with_theme(
            spawned_session.clone(),
            repo_theme("#B89875"),
        )));
        let mut app = make_app(api);

        app.spawn_session("/Users/b/repos/throngterm", None, field);

        let mut thought_session = session_summary_with_thought(
            "sess-42",
            "42",
            "/Users/b/repos/throngterm",
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        thought_session.repo_theme_id = Some(theme_id);
        app.capture_thought_updates(&[thought_session.clone()], layout.thought_entry_capacity());
        app.merge_sessions(vec![thought_session], field);

        let entity = app
            .selected()
            .expect("spawned session should be selected")
            .clone();
        let rect = entity.screen_rect(field);
        let mut entity_renderer = test_renderer(120, 32);
        render_entity(
            &mut entity_renderer,
            &entity,
            rect,
            true,
            0,
            &app.repo_themes,
        );

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        assert_eq!(panel.rows.len(), 1);
        assert_eq!(panel.rows[0].color, theme_color);

        let mut thought_renderer = test_renderer(120, 32);
        render_thought_panel(
            &app,
            &mut thought_renderer,
            thought_content,
            layout.thought_entry_capacity(),
        );
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);

        assert_eq!(cell_at(&entity_renderer, rect.x, rect.y).fg, theme_color);
        assert_eq!(
            cell_at(&thought_renderer, thought_content.x, row_start_y).fg,
            theme_color
        );
    }

    #[test]
    fn sleeping_entity_pins_to_top_left_grid_slot() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![sleeping_session(
                "sess-sleep-1",
                "7",
                "/Users/b/repos/throngterm",
                "2026-03-08T12:00:00Z",
            )],
            field,
        );

        assert_eq!(
            entity_rect_for(&app, "sess-sleep-1", field),
            sleep_grid_rect(field, 0)
        );
    }

    #[test]
    fn attention_sleeping_entity_pins_to_top_left_grid_slot() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![attention_session(
                "sess-attn-sleep-1",
                "7",
                "/Users/b/repos/throngterm",
                RestState::Sleeping,
                "2026-03-08T12:00:00Z",
            )],
            field,
        );

        let entity = app
            .entities
            .iter()
            .find(|entity| entity.session.session_id == "sess-attn-sleep-1")
            .expect("entity should exist");
        assert!(entity.is_sleeping());
        assert_eq!(
            entity_rect_for(&app, "sess-attn-sleep-1", field),
            sleep_grid_rect(field, 0)
        );
    }

    #[test]
    fn attention_session_state_text_uses_rest_state() {
        let active = attention_session(
            "sess-attn-active",
            "7",
            "/Users/b/repos/throngterm",
            RestState::Active,
            "2026-03-08T12:40:00Z",
        );
        let drowsy = attention_session(
            "sess-attn-drowsy",
            "8",
            "/Users/b/repos/throngterm",
            RestState::Drowsy,
            "2026-03-08T12:20:00Z",
        );
        let sleeping = attention_session(
            "sess-attn-sleep",
            "9",
            "/Users/b/repos/throngterm",
            RestState::Sleeping,
            "2026-03-08T12:00:00Z",
        );

        assert_eq!(session_state_text(&active), "attention");
        assert_eq!(session_state_text(&drowsy), "drowsy");
        assert_eq!(session_state_text(&sleeping), "sleeping");
    }

    #[test]
    fn render_picker_uses_current_repo_theme_color() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("buildooor");
        fs::create_dir_all(&repo_root).expect("create repo");
        write_repo_theme_file(&repo_root, "#B89875");

        let mut picker = PickerState::new(
            2,
            2,
            dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
            true,
        );
        let mut repo_themes = HashMap::new();
        picker.sync_theme_colors(&mut repo_themes);

        let field = test_field();
        let layout = picker_layout(&picker, field);
        let mut renderer = test_renderer(100, 30);

        render_picker(&mut renderer, &picker, field);

        let expected = Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        };
        assert_eq!(
            cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
            expected
        );
        assert_eq!(
            cell_at(&renderer, layout.content.x, layout.content.y).fg,
            expected
        );
        assert_eq!(
            cell_at(
                &renderer,
                layout.spawn_here_button.x,
                layout.spawn_here_button.y
            )
            .fg,
            expected
        );
    }

    #[test]
    fn render_picker_uses_entry_repo_theme_color() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("throngterm");
        fs::create_dir_all(&repo_root).expect("create repo");
        write_repo_theme_file(&repo_root, "#4FA66A");

        let mut picker = PickerState::new(
            2,
            2,
            dir_response(
                temp.path().to_string_lossy().as_ref(),
                &[("throngterm", true)],
            ),
            true,
        );
        let mut repo_themes = HashMap::new();
        picker.sync_theme_colors(&mut repo_themes);

        let field = test_field();
        let layout = picker_layout(&picker, field);
        let mut renderer = test_renderer(100, 30);

        render_picker(&mut renderer, &picker, field);

        assert_eq!(
            cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
            Color::Rgb {
                r: 79,
                g: 166,
                b: 106,
            }
        );
    }

    #[test]
    fn sleeping_entities_fill_vertical_grid_by_sleepiness() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![
                sleeping_session(
                    "sess-new",
                    "8",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:20:00Z",
                ),
                sleeping_session(
                    "sess-mid",
                    "7",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:10:00Z",
                ),
                sleeping_session(
                    "sess-old",
                    "9",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:00:00Z",
                ),
            ],
            field,
        );

        assert_eq!(
            entity_rect_for(&app, "sess-old", field),
            sleep_grid_rect(field, 0)
        );
        assert_eq!(
            entity_rect_for(&app, "sess-mid", field),
            sleep_grid_rect(field, 1)
        );
        assert_eq!(
            entity_rect_for(&app, "sess-new", field),
            sleep_grid_rect(field, 2)
        );
    }

    #[test]
    fn sleeping_entities_use_tmux_name_tiebreaker() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![
                sleeping_session(
                    "sess-b",
                    "8",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:00:00Z",
                ),
                sleeping_session(
                    "sess-a",
                    "7",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:00:00Z",
                ),
            ],
            field,
        );

        assert_eq!(
            entity_rect_for(&app, "sess-a", field),
            sleep_grid_rect(field, 0)
        );
        assert_eq!(
            entity_rect_for(&app, "sess-b", field),
            sleep_grid_rect(field, 1)
        );
    }

    #[test]
    fn existing_entity_relocates_into_sleep_grid_when_it_falls_asleep() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        app.entities.push(entity_at(
            field,
            "sess-1",
            "dev",
            "/Users/b/repos/dev",
            30,
            8,
        ));

        app.merge_sessions(
            vec![sleeping_session(
                "sess-1",
                "dev",
                "/Users/b/repos/dev",
                "2026-03-08T12:00:00Z",
            )],
            field,
        );

        assert_eq!(
            entity_rect_for(&app, "sess-1", field),
            sleep_grid_rect(field, 0)
        );
    }

    #[test]
    fn sleeping_entities_stay_fixed_after_tick() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![
                sleeping_session(
                    "sess-a",
                    "7",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:00:00Z",
                ),
                sleeping_session(
                    "sess-b",
                    "8",
                    "/Users/b/repos/throngterm",
                    "2026-03-08T12:10:00Z",
                ),
            ],
            field,
        );
        for entity in &mut app.entities {
            entity.vx = 1.0;
            entity.vy = 1.0;
        }

        app.tick(field);

        assert_eq!(
            entity_rect_for(&app, "sess-a", field),
            sleep_grid_rect(field, 0)
        );
        assert_eq!(
            entity_rect_for(&app, "sess-b", field),
            sleep_grid_rect(field, 1)
        );
    }

    #[test]
    fn drowsy_sprite_frame_is_static() {
        assert_eq!(drowsy_frame(0), drowsy_frame(1));
        assert_eq!(SpriteKind::Drowsy.speed_scale(), 0.0);
    }

    #[test]
    fn drowsy_entities_stay_fixed_after_tick() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        let mut entity = entity_at(field, "sess-1", "dev", "/Users/b/repos/dev", 30, 8);
        entity.session.thought_state = ThoughtState::Holding;
        entity.session.rest_state = RestState::Drowsy;
        entity.vx = 1.0;
        entity.vy = 1.0;
        app.entities.push(entity);

        app.tick(field);

        assert_eq!(
            entity_rect_for(&app, "sess-1", field),
            Rect {
                x: 30,
                y: 8,
                width: ENTITY_WIDTH,
                height: ENTITY_HEIGHT,
            }
        );
    }

    #[test]
    fn drowsy_entities_remain_fixed_during_collisions() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        let mut drowsy = entity_at(field, "sess-drowsy", "7", "/Users/b/repos/dev", 30, 8);
        drowsy.session.thought_state = ThoughtState::Holding;
        drowsy.session.rest_state = RestState::Drowsy;
        drowsy.vx = 0.0;
        drowsy.vy = 0.0;

        let mut active = entity_at(field, "sess-active", "8", "/Users/b/repos/dev", 30, 8);
        active.session.thought_state = ThoughtState::Active;
        active.session.rest_state = RestState::Active;
        active.vx = 1.0;
        active.vy = 0.0;

        app.entities.push(drowsy);
        app.entities.push(active);

        app.tick(field);

        assert_eq!(
            entity_rect_for(&app, "sess-drowsy", field),
            Rect {
                x: 30,
                y: 8,
                width: ENTITY_WIDTH,
                height: ENTITY_HEIGHT,
            }
        );
        assert_ne!(
            entity_rect_for(&app, "sess-active", field),
            Rect {
                x: 30,
                y: 8,
                width: ENTITY_WIDTH,
                height: ENTITY_HEIGHT,
            }
        );
    }

    #[test]
    fn non_sleeping_entities_keep_their_normal_motion() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        let mut entity = entity_at(field, "sess-1", "dev", "/Users/b/repos/dev", 30, 8);
        entity.session.thought_state = ThoughtState::Active;
        entity.session.rest_state = RestState::Active;
        entity.vx = 1.0;
        entity.vy = 1.0;
        app.entities.push(entity);

        app.tick(field);

        assert_eq!(
            entity_rect_for(&app, "sess-1", field),
            Rect {
                x: 31,
                y: 9,
                width: ENTITY_WIDTH,
                height: ENTITY_HEIGHT,
            }
        );
    }

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

    #[test]
    fn empty_field_click_opens_picker_with_managed_order() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response(
            "/Users/b/repos",
            &[("opensource", true), ("throngterm", true)],
        )));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.entities.push(entity_at(
            field,
            "sess-1",
            "dev",
            "/Users/b/repos/dev",
            30,
            8,
        ));

        app.handle_field_click(10, 10, field);

        let picker = app.picker.as_ref().expect("picker should open");
        assert!(picker.managed_only);
        assert_eq!(picker.base_path, "/Users/b/repos");
        assert_eq!(
            picker
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["opensource", "throngterm"]
        );
        assert_eq!(api.list_calls(), vec![(None, true)]);
    }

    #[test]
    fn navigating_into_folder_opens_initial_request_composer() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("opensource", true)])));
        api.push_list_dirs(Ok(dir_response(
            "/Users/b/repos/opensource",
            &[("skills", false)],
        )));

        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);
        app.activate_picker_entry(0, field);
        app.activate_picker_entry(0, field);

        assert_eq!(
            api.list_calls(),
            vec![
                (None, true),
                (Some("/Users/b/repos/opensource".to_string()), true),
            ]
        );
        assert_eq!(
            api.create_calls(),
            Vec::<(String, SpawnTool, Option<String>)>::new()
        );
        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some("/Users/b/repos/opensource/skills")
        );
        assert!(app.picker.is_some());
    }

    #[test]
    fn spawn_here_opens_initial_request_for_current_path() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response("/Users/b/repos/opensource", &[("skills", true)]),
            true,
        ));

        app.spawn_session_from_picker(field);

        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some("/Users/b/repos/opensource")
        );
    }

    #[test]
    fn toggling_to_all_reloads_same_path_without_reordering() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("opensource", true)])));
        api.push_list_dirs(Ok(dir_response(
            "/Users/b/repos",
            &[("Alpha", true), ("beta", true), ("zzz-old", true)],
        )));
        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);
        app.picker_set_managed_only(false);

        let picker = app.picker.as_ref().expect("picker should stay open");
        assert!(!picker.managed_only);
        assert_eq!(
            picker
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "beta", "zzz-old"]
        );
        assert_eq!(
            api.list_calls(),
            vec![(None, true), (Some("/Users/b/repos".to_string()), false),]
        );
    }

    #[test]
    fn dir_list_failure_blocks_spawn_and_shows_error() {
        let api = MockApi::new();
        api.push_list_dirs(Err("Permission denied".to_string()));
        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);

        assert!(app.picker.is_none());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("Permission denied")
        );
        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
    }

    #[test]
    fn submitting_initial_request_creates_hidden_session_without_native_open() {
        let api = MockApi::new();
        api.push_create_session(Ok(create_response(
            "sess-55",
            "55",
            "/Users/b/repos/throngterm",
        )));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response("/Users/b/repos", &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: "/Users/b/repos/throngterm".to_string(),
            value: "add hidden spawn flow".to_string(),
        });

        app.submit_initial_request(field);

        assert_eq!(
            api.create_calls(),
            vec![(
                "/Users/b/repos/throngterm".to_string(),
                SpawnTool::Codex,
                Some("add hidden spawn flow".to_string()),
            )]
        );
        assert!(api.open_calls().is_empty());
        assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
        assert!(app.picker.is_none());
        assert!(app.initial_request.is_none());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("created 55")
        );
        assert!(app
            .entities
            .iter()
            .any(|entity| entity.session.session_id == "sess-55"));
    }

    #[test]
    fn session_create_failure_does_not_attempt_native_open() {
        let api = MockApi::new();
        api.push_create_session(Err("tmux failed to start".to_string()));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response("/Users/b/repos", &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: "/Users/b/repos/throngterm".to_string(),
            value: "fix tmux startup".to_string(),
        });

        app.submit_initial_request(field);

        assert_eq!(
            api.create_calls(),
            vec![(
                "/Users/b/repos/throngterm".to_string(),
                SpawnTool::Codex,
                Some("fix tmux startup".to_string()),
            )]
        );
        assert!(api.open_calls().is_empty());
        assert!(app.entities.is_empty());
        assert_eq!(
            app.initial_request
                .as_ref()
                .map(|state| state.value.as_str()),
            Some("fix tmux startup")
        );
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("tmux failed to start")
        );
    }

    #[test]
    fn blank_initial_request_is_rejected_locally() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api.clone());
        app.initial_request = Some(InitialRequestState {
            cwd: "/Users/b/repos/throngterm".to_string(),
            value: "   ".to_string(),
        });

        app.submit_initial_request(field);

        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("enter an initial request")
        );
    }

    #[test]
    fn esc_cancels_initial_request_without_creating_session() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response("/Users/b/repos", &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: "/Users/b/repos/throngterm".to_string(),
            value: "investigate snapshot restore".to_string(),
        });

        app.handle_initial_request_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), field);

        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert!(app.initial_request.is_none());
        assert!(app.picker.is_some());
    }

    #[test]
    fn clicking_existing_thronglet_still_opens_it_directly() {
        let api = MockApi::new();
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-7".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.entities.push(entity_at(
            field,
            "sess-7",
            "dev",
            "/Users/b/repos/dev",
            30,
            8,
        ));
        app.selected_id = Some("sess-7".to_string());

        app.handle_field_click(30, 8, field);

        assert!(api.list_calls().is_empty());
        assert!(api.create_calls().is_empty());
        assert_eq!(api.open_calls(), vec!["sess-7".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("focused dev")
        );
    }

    #[test]
    fn filtered_out_thronglets_are_not_click_targets() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("throngterm", true)])));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.entities.push(entity_at(
            field,
            "sess-1",
            "2",
            "/Users/b/repos/throngterm",
            12,
            6,
        ));
        app.entities.push(entity_at(
            field,
            "sess-3",
            "9",
            "/Users/b/repos/opensource/skills",
            30,
            8,
        ));
        app.selected_id = Some("sess-3".to_string());

        app.set_thought_filter_cwd("/Users/b/repos/throngterm".to_string());
        app.handle_field_click(30, 8, field);

        assert_eq!(visible_entity_ids(&app), vec!["sess-1".to_string()]);
        assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
        assert!(api.open_calls().is_empty());
        assert!(app.picker.is_some());
    }

    #[test]
    fn refresh_clears_selection_when_filters_hide_all_sessions() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![session_summary(
            "sess-3",
            "9",
            "/Users/b/repos/opensource/skills",
        )]));
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![
                session_summary("sess-1", "7", "/Users/b/repos/throngterm"),
                session_summary("sess-2", "2", "/Users/b/repos/throngterm"),
            ],
            layout.overview_field,
        );
        app.selected_id = Some("sess-1".to_string());
        app.set_thought_filter_cwd("/Users/b/repos/throngterm".to_string());

        app.refresh(layout);

        assert!(app.visible_entities().is_empty());
        assert!(app.selected_id.is_none());

        app.open_selected();

        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("no session selected")
        );
    }
}
