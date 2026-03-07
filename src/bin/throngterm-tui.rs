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
use futures::future::BoxFuture;
use reqwest::Client;
use tokio::runtime::Runtime;

use throngterm::config::{AuthMode, Config};
use throngterm::types::{
    CreateSessionRequest, CreateSessionResponse, DirEntry, DirListResponse, ErrorResponse,
    NativeDesktopOpenRequest, NativeDesktopOpenResponse, NativeDesktopStatusResponse,
    SessionListResponse, SessionState, SessionSummary, SpawnTool, ThoughtState,
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
            managed_only,
            selection: PickerSelection::SpawnHere,
            scroll: 0,
        }
    }

    fn apply_response(&mut self, response: DirListResponse) {
        self.current_path = response.path;
        self.entries = response.entries;
        self.selection = PickerSelection::SpawnHere;
        self.scroll = 0;
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

struct App<C: TuiApi> {
    runtime: Runtime,
    client: C,
    entities: Vec<SessionEntity>,
    selected_id: Option<String>,
    native_status: Option<NativeDesktopStatusResponse>,
    picker: Option<PickerState>,
    message: Option<(String, Instant)>,
    last_refresh: Option<Instant>,
    tick: u64,
}

impl<C: TuiApi> App<C> {
    fn new(runtime: Runtime, client: C) -> Self {
        Self {
            runtime,
            client,
            entities: Vec::new(),
            selected_id: None,
            native_status: None,
            picker: None,
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

    fn close_picker(&mut self) {
        self.picker = None;
    }

    fn open_picker(&mut self, x: u16, y: u16) {
        match self.runtime.block_on(self.client.list_dirs(None, true)) {
            Ok(response) => {
                self.picker = Some(PickerState::new(x, y, response, true));
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

    fn picker_activate_selection(&mut self, field: Rect) {
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
            PickerSelection::SpawnHere => self.spawn_session(&current_path, field),
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
                    self.spawn_session(&path, field);
                }
            }
        }
    }

    fn spawn_session(&mut self, cwd: &str, field: Rect) {
        match self
            .runtime
            .block_on(self.client.create_session(cwd, SpawnTool::Codex))
        {
            Ok(response) => {
                let session = response.session;
                let session_id = session.session_id.clone();
                let tmux_name = session.tmux_name.clone();
                self.upsert_session(session, field);
                self.selected_id = Some(session_id.clone());
                self.close_picker();
                self.open_session_for_label(&session_id, &tmux_name);
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
        let Some(selected_id) = self.selected_id.clone() else {
            self.set_message("no session selected");
            return;
        };

        let label = selected_label(self.selected().map(|entity| &entity.session.tmux_name));
        self.open_session_for_label(&selected_id, &label);
    }

    fn handle_field_click(&mut self, x: u16, y: u16, field: Rect) {
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
            .entities
            .iter()
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

    fn spawn_session_from_picker(&mut self, field: Rect) {
        let Some(path) = self
            .picker
            .as_ref()
            .map(|picker| picker.current_path.clone())
        else {
            return;
        };
        self.spawn_session(&path, field);
    }

    fn activate_picker_entry(&mut self, index: usize, field: Rect) {
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
            self.spawn_session(&path, field);
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

        if let Some(picker) = &self.picker {
            render_picker(renderer, picker, field);
        }

        render_footer(self, renderer, field_box.bottom() + 1);
    }
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

fn render_picker(renderer: &mut Renderer, picker: &PickerState, field: Rect) {
    let layout = picker_layout(picker, field);
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, Color::White);

    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        "spawn codex",
        Color::Cyan,
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
    renderer.draw_text(path_x, layout.content.y + 1, &path_label, Color::White);

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
        Color::White
    } else {
        Color::Yellow
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
        let color = if picker.selection == PickerSelection::Entry(index) {
            Color::White
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

    let help = if app.picker.is_some() {
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
                            app.move_selection(-1, current_field);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.move_selection(-1, current_field);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.move_selection(1, current_field);
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char('o') => {
                        if app.picker.is_some() {
                            app.picker_activate_selection(current_field);
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
                            let field = field_box(renderer.width(), renderer.height()).inset(1);
                            app.refresh(field);
                        }
                    }
                    _ => {}
                },
                Event::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
                {
                    let field = field_box(renderer.width(), renderer.height()).inset(1);
                    if field.contains(mouse.column, mouse.row) {
                        app.handle_field_click(mouse.column, mouse.row, field);
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
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use throngterm::types::{ThoughtSource, TransportHealth};

    #[derive(Default)]
    struct MockApiState {
        fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
        native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
        open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
        list_dirs_results: VecDeque<Result<DirListResponse, String>>,
        create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
        open_calls: Vec<String>,
        list_calls: Vec<(Option<String>, bool)>,
        create_calls: Vec<(String, SpawnTool)>,
    }

    #[derive(Clone, Default)]
    struct MockApi {
        state: Arc<Mutex<MockApiState>>,
    }

    impl MockApi {
        fn new() -> Self {
            Self::default()
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

        fn create_calls(&self) -> Vec<(String, SpawnTool)> {
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
        ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
            let state = self.state.clone();
            let cwd = cwd.to_string();
            Box::pin(async move {
                let mut state = state.lock().unwrap();
                state.create_calls.push((cwd, spawn_tool));
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

    fn make_app(api: MockApi) -> App<MockApi> {
        App::new(test_runtime(), api)
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
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
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

    fn create_response(session_id: &str, tmux_name: &str, cwd: &str) -> CreateSessionResponse {
        CreateSessionResponse {
            session: session_summary(session_id, tmux_name, cwd),
            sprite_pack: None,
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
    fn navigating_into_folder_creates_and_opens_codex_session() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("opensource", true)])));
        api.push_list_dirs(Ok(dir_response(
            "/Users/b/repos/opensource",
            &[("skills", false)],
        )));
        api.push_create_session(Ok(create_response(
            "sess-42",
            "42",
            "/Users/b/repos/opensource/skills",
        )));
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-42".to_string(),
            status: "opened".to_string(),
            pane_id: None,
        }));

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
            vec![(
                "/Users/b/repos/opensource/skills".to_string(),
                SpawnTool::Codex,
            )]
        );
        assert_eq!(api.open_calls(), vec!["sess-42".to_string()]);
        assert_eq!(app.selected_id.as_deref(), Some("sess-42"));
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("opened 42")
        );
        assert!(app
            .entities
            .iter()
            .any(|entity| entity.session.session_id == "sess-42"));
    }

    #[test]
    fn spawn_here_uses_current_path_with_codex() {
        let api = MockApi::new();
        api.push_create_session(Ok(create_response(
            "sess-55",
            "55",
            "/Users/b/repos/opensource",
        )));
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-55".to_string(),
            status: "opened".to_string(),
            pane_id: None,
        }));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response("/Users/b/repos/opensource", &[("skills", true)]),
            true,
        ));

        app.spawn_session_from_picker(field);

        assert_eq!(
            api.create_calls(),
            vec![("/Users/b/repos/opensource".to_string(), SpawnTool::Codex)]
        );
        assert_eq!(api.open_calls(), vec!["sess-55".to_string()]);
        assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
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
    fn session_create_failure_does_not_attempt_native_open() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("throngterm", false)])));
        api.push_create_session(Err("tmux failed to start".to_string()));
        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);
        app.activate_picker_entry(0, field);

        assert_eq!(
            api.create_calls(),
            vec![("/Users/b/repos/throngterm".to_string(), SpawnTool::Codex)]
        );
        assert!(api.open_calls().is_empty());
        assert!(app.entities.is_empty());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("tmux failed to start")
        );
    }

    #[test]
    fn native_open_failure_preserves_created_session() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response("/Users/b/repos", &[("throngterm", false)])));
        api.push_create_session(Ok(create_response(
            "sess-77",
            "77",
            "/Users/b/repos/throngterm",
        )));
        api.push_open_session(Err("native open unavailable".to_string()));
        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);
        app.activate_picker_entry(0, field);

        assert_eq!(api.open_calls(), vec!["sess-77".to_string()]);
        assert!(app
            .entities
            .iter()
            .any(|entity| entity.session.session_id == "sess-77"));
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("native open unavailable")
        );
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
}
