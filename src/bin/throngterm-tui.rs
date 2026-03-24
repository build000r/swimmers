use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error as StdError;
use std::f32::consts::TAU;
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, IsTerminal, Stdout, Write};
use std::process::Command as ProcessCommand;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::future::BoxFuture;
use mermaid_rs_renderer::{
    compute_layout, parse_mermaid, render_svg, DiagramKind, Layout as MermaidLayout, RenderOptions,
};
use reqwest::Client;
use resvg::tiny_skia::{Pixmap, Transform};
use tokio::runtime::Runtime;
use usvg::Tree;

use throngterm::config::{AuthMode, Config};
use throngterm::repo_theme::{discover_repo_theme, existing_repo_theme};
use throngterm::types::{
    CreateSessionRequest, CreateSessionResponse, DirEntry, DirListResponse, ErrorResponse,
    MermaidArtifactResponse, NativeDesktopOpenRequest, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, PublishSelectionRequest, RepoTheme, RestState,
    SessionListResponse, SessionState, SessionSummary, SpawnTool,
};

const MIN_WIDTH: u16 = 70;
const MIN_HEIGHT: u16 = 20;
const FRAME_DURATION: Duration = Duration::from_millis(100);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const MESSAGE_TTL: Duration = Duration::from_secs(5);
const API_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const API_REQUEST_TIMEOUT: Duration = Duration::from_millis(750);
const API_NATIVE_OPEN_TIMEOUT: Duration = Duration::from_secs(3);
const SPRITE_HEIGHT: u16 = 4;
const LABEL_HEIGHT: u16 = 1;
const ENTITY_WIDTH: u16 = 12;
const ENTITY_HEIGHT: u16 = SPRITE_HEIGHT + LABEL_HEIGHT;
const SWIM_BOB_RATE: f32 = 0.08;
const SWIM_VERTICAL_DRIFT: f32 = 0.06;
const SWIM_DRIFT_LIMIT: f32 = 1.0;
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
const THOUGHT_MERMAID_LABEL: &str = "[mmd]";
const MERMAID_BACK_LABEL: &str = "[back to bowl]";
const MERMAID_VIEW_MIN_WIDTH: u16 = 16;
const MERMAID_VIEW_MIN_HEIGHT: u16 = 8;
const MERMAID_ZOOM_STEP: f32 = 1.2;
const MERMAID_MIN_ZOOM: f32 = 0.5;
const MERMAID_MAX_ZOOM: f32 = 8.0;

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
    terminal_state: TerminalState,
}

#[derive(Default)]
struct TerminalState {
    raw_mode_enabled: bool,
    terminal_ui_active: bool,
}

fn enter_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        cursor::Hide,
        Clear(ClearType::All)
    )
}

fn leave_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show,
        ResetColor
    )
}

impl TerminalState {
    fn init_with<W, EnableRawMode, EnterUi>(
        &mut self,
        writer: &mut W,
        enable_raw_mode: EnableRawMode,
        enter_ui: EnterUi,
    ) -> io::Result<()>
    where
        W: Write,
        EnableRawMode: FnOnce() -> io::Result<()>,
        EnterUi: FnOnce(&mut W) -> io::Result<()>,
    {
        enable_raw_mode()?;
        self.raw_mode_enabled = true;
        // Mark the UI as needing teardown before issuing enter sequences so a
        // mid-init failure still triggers the full restore path on cleanup.
        self.terminal_ui_active = true;
        enter_ui(writer)
    }

    fn cleanup_with<W, LeaveUi, DisableRawMode>(
        &mut self,
        writer: &mut W,
        leave_ui: LeaveUi,
        disable_raw_mode: DisableRawMode,
    ) -> io::Result<()>
    where
        W: Write,
        LeaveUi: FnOnce(&mut W) -> io::Result<()>,
        DisableRawMode: FnOnce() -> io::Result<()>,
    {
        let mut first_error = None;

        if self.terminal_ui_active {
            if let Err(err) = leave_ui(writer) {
                first_error = Some(err);
            }
            self.terminal_ui_active = false;
        }

        if self.raw_mode_enabled {
            if let Err(err) = disable_raw_mode() {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            self.raw_mode_enabled = false;
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        Ok(())
    }
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
            terminal_state: TerminalState::default(),
        })
    }

    fn init(&mut self) -> io::Result<()> {
        self.terminal_state.init_with(
            &mut self.stdout,
            terminal::enable_raw_mode,
            enter_terminal_ui,
        )
    }

    fn cleanup(&mut self) -> io::Result<()> {
        self.terminal_state.cleanup_with(
            &mut self.stdout,
            leave_terminal_ui,
            terminal::disable_raw_mode,
        )
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
    DeepSleep,
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
                RestState::Sleeping => Self::Sleeping,
                RestState::DeepSleep => Self::DeepSleep,
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
            Self::DeepSleep => Color::DarkBlue,
            Self::Attention => Color::Magenta,
            Self::Error => Color::Red,
            Self::Exited => Color::DarkGrey,
        }
    }

    fn speed_scale(self) -> f32 {
        match self {
            Self::Active => 1.0,
            Self::Busy => 1.3,
            Self::Drowsy => 0.5,
            Self::Sleeping => 0.0,
            Self::DeepSleep => 0.0,
            Self::Attention => 1.15,
            Self::Error => 0.8,
            Self::Exited => 0.0,
        }
    }

    fn bob_amplitude(self) -> f32 {
        match self {
            Self::Active => 1.2,
            Self::Busy => 1.45,
            Self::Drowsy => 0.75,
            Self::Sleeping => 0.0,
            Self::DeepSleep => 0.0,
            Self::Attention => 1.3,
            Self::Error => 1.6,
            Self::Exited => 0.0,
        }
    }

    fn frame(self, tick: u64) -> [&'static str; 4] {
        match self {
            Self::Active => active_frame(tick),
            Self::Busy => busy_frame(tick),
            Self::Drowsy => drowsy_frame(tick),
            Self::Sleeping => sleeping_frame(tick),
            Self::DeepSleep => deep_sleep_frame(tick),
            Self::Attention => attention_frame(tick),
            Self::Error => error_frame(tick),
            Self::Exited => exited_frame(tick),
        }
    }
}

fn active_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "   o   .    ",
            "><o)))'>    ",
            "  /_/_      ",
            "      .     ",
        ]
    } else {
        [
            "      o     ",
            "><o)))'>    ",
            "   \\_\\      ",
            "   .    o   ",
        ]
    }
}

fn busy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  o O  .    ",
            "><O)))'>    ",
            "  /_/_      ",
            "    O   o   ",
        ]
    } else {
        [
            "   O   o    ",
            "><O)))'>    ",
            "   \\_\\      ",
            "  .   O     ",
        ]
    }
}

fn drowsy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "    .       ",
            "><-)))'>    ",
            "  /_/_      ",
            "      .     ",
        ]
    } else {
        [
            "      .     ",
            "><-)))'>    ",
            "   \\_\\      ",
            "    .       ",
        ]
    }
}

fn sleeping_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            " z z        ",
            "            ",
            "  ><-)))'>  ",
            "    \\_\\     ",
        ]
    } else {
        [
            "  z Z       ",
            "            ",
            "  ><-)))'>  ",
            "   /_/_     ",
        ]
    }
}

fn attention_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  !   o     ",
            "><!)))'>    ",
            "  /_/_      ",
            "     .      ",
        ]
    } else {
        [
            "    o   !   ",
            "><!)))'>    ",
            "   \\_\\      ",
            "   .        ",
        ]
    }
}

fn error_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            " .   x      ",
            "><x)))'>    ",
            "  /_/_      ",
            "    . o     ",
        ]
    } else {
        [
            "   x   .    ",
            "><x)))'>    ",
            "   \\_\\      ",
            "   o        ",
        ]
    }
}

fn deep_sleep_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "   /_/_  Z  ",
            "  ><-)))'>  ",
            "            ",
            "            ",
        ]
    } else {
        [
            "    \\_\\ z   ",
            "  ><-)))'>  ",
            "            ",
            "            ",
        ]
    }
}

fn exited_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "   /_/_ xxx",
            "  ><x)))'>  ",
            "            ",
            "            ",
        ]
    } else {
        [
            "    \\_\\ xxx",
            "  ><x)))'>  ",
            "            ",
            "            ",
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestAnchor {
    FreeSwim,
    Bottom,
    Top,
}

#[derive(Clone)]
struct SessionEntity {
    session: SessionSummary,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    swim_anchor_x: f32,
    swim_anchor_y: f32,
    swim_center_y: f32,
    bob_phase: f32,
}

impl SessionEntity {
    fn new(session: SessionSummary, field: Rect) -> Self {
        let hash = stable_hash(&session.session_id);
        let max_x = field.width.saturating_sub(ENTITY_WIDTH).max(1);
        let max_y = field.height.saturating_sub(ENTITY_HEIGHT).max(1);
        let x = (hash % (max_x as u64)) as f32;
        let y = ((hash / 13) % (max_y as u64)) as f32;
        let vx = swim_speed(hash);
        let vy = vertical_drift(hash);

        Self {
            session,
            x,
            y,
            vx,
            vy,
            swim_anchor_x: x,
            swim_anchor_y: y,
            swim_center_y: y,
            bob_phase: bob_phase(hash),
        }
    }

    fn sprite_kind(&self) -> SpriteKind {
        SpriteKind::from_session(&self.session)
    }

    fn rest_anchor(&self) -> RestAnchor {
        match self.sprite_kind() {
            SpriteKind::Sleeping => RestAnchor::Bottom,
            SpriteKind::DeepSleep | SpriteKind::Exited => RestAnchor::Top,
            _ => RestAnchor::FreeSwim,
        }
    }

    fn is_stationary(&self) -> bool {
        !matches!(self.rest_anchor(), RestAnchor::FreeSwim)
    }

    fn set_relative_position(&mut self, x: u16, y: u16) {
        self.x = x as f32;
        self.y = y as f32;
        self.swim_anchor_x = self.x;
        self.swim_anchor_y = self.y;
        self.swim_center_y = self.y;
    }

    fn tick(&mut self, field: Rect, tick: u64) {
        let sprite = self.sprite_kind();
        let speed = sprite.speed_scale();
        if speed == 0.0 || field.width <= ENTITY_WIDTH || field.height <= ENTITY_HEIGHT {
            return;
        }

        let max_y = field.height.saturating_sub(ENTITY_HEIGHT) as f32;

        self.x = self
            .swim_anchor_x
            .clamp(0.0, field.width.saturating_sub(ENTITY_WIDTH) as f32);

        let min_center = (self.swim_anchor_y - SWIM_DRIFT_LIMIT).max(0.0);
        let max_center = (self.swim_anchor_y + SWIM_DRIFT_LIMIT).min(max_y);
        self.swim_center_y += self.vy * speed * SWIM_VERTICAL_DRIFT;
        if self.swim_center_y <= min_center {
            self.swim_center_y = min_center;
            self.vy = self.vy.abs();
        } else if self.swim_center_y >= max_center {
            self.swim_center_y = max_center;
            self.vy = -self.vy.abs();
        }

        let bob = ((tick as f32 * SWIM_BOB_RATE) + self.bob_phase).sin() * sprite.bob_amplitude();
        self.y = (self.swim_center_y + bob).clamp(0.0, max_y);
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
            .connect_timeout(API_CONNECT_TIMEOUT)
            .timeout(API_REQUEST_TIMEOUT)
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

    fn transport_error(&self, action: &str, err: reqwest::Error) -> String {
        friendly_transport_error(&self.base_url, action, &err)
    }

    fn startup_access_error(&self, path: &str, status: reqwest::StatusCode) -> String {
        match status {
            reqwest::StatusCode::UNAUTHORIZED => format!(
                "backend at {} requires valid auth for {}. Set AUTH_MODE=token and AUTH_TOKEN to match the target API.",
                self.base_url, path
            ),
            reqwest::StatusCode::FORBIDDEN => format!(
                "backend at {} denied startup access to {}. Use a token with the required session scope for this TUI instance.",
                self.base_url, path
            ),
            _ => format!(
                "backend at {} rejected startup access to {} ({status})",
                self.base_url, path
            ),
        }
    }

    async fn ensure_startup_access(
        &self,
        response: reqwest::Response,
        path: &str,
    ) -> Result<(), String> {
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(self.startup_access_error(path, status))
            }
            _ => Err(read_error(response).await),
        }
    }

    async fn preflight_session_refresh_access(&self) -> Result<(), String> {
        let url = format!("{}/v1/sessions", self.base_url);
        let response = self
            .with_auth(self.http.get(url))
            .send()
            .await
            .map_err(|err| self.transport_error("refresh sessions", err))?;

        self.ensure_startup_access(response, "/v1/sessions").await
    }

    async fn preflight_selection_sync_access(&self) -> Result<(), String> {
        let url = format!("{}/v1/selection", self.base_url);
        let response = self
            .with_auth(self.http.put(url))
            .json(&PublishSelectionRequest { session_id: None })
            .send()
            .await
            .map_err(|err| self.transport_error("clear the published selection", err))?;

        self.ensure_startup_access(response, "/v1/selection").await
    }

    async fn preflight_startup_access(&self) -> Result<(), String> {
        self.preflight_session_refresh_access().await?;
        self.preflight_selection_sync_access().await?;
        Ok(())
    }
}

fn root_error_message(err: &(dyn StdError + 'static)) -> String {
    let mut current = Some(err);
    let mut last = err.to_string();

    while let Some(next) = current.and_then(StdError::source) {
        let next_text = next.to_string();
        if !next_text.is_empty() {
            last = next_text;
        }
        current = Some(next);
    }

    last
}

fn friendly_transport_error(base_url: &str, action: &str, err: &reqwest::Error) -> String {
    let detail = root_error_message(err);
    let summary = if err.is_timeout() {
        format!("timed out while trying to {action}")
    } else {
        format!("could not {action}")
    };

    format!(
        "backend unavailable at {base_url}: {summary} ({detail}). Start `throngterm` or set THRONGTERM_TUI_URL."
    )
}

trait TuiApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>>;
    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>>;
    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>>;
    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>>;
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
                .map_err(|err| self.transport_error("refresh sessions", err))?;

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

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            let url = format!(
                "{}/v1/sessions/{}/mermaid-artifact",
                self.base_url, session_id
            );
            let response = self
                .with_auth(self.http.get(url))
                .send()
                .await
                .map_err(|err| self.transport_error("fetch mermaid artifact", err))?;

            if response.status().is_success() {
                return response
                    .json::<MermaidArtifactResponse>()
                    .await
                    .map_err(|err| format!("failed to parse mermaid artifact: {err}"));
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
                .map_err(|err| self.transport_error("check native desktop status", err))?;

            if response.status().is_success() {
                return response
                    .json::<NativeDesktopStatusResponse>()
                    .await
                    .map_err(|err| format!("failed to parse native status: {err}"));
            }

            Err(read_error(response).await)
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        let session_id = session_id.map(|value| value.to_string());
        Box::pin(async move {
            let url = format!("{}/v1/selection", self.base_url);
            let response = self
                .with_auth(self.http.put(url))
                .json(&PublishSelectionRequest { session_id })
                .send()
                .await
                .map_err(|err| self.transport_error("publish the selected session", err))?;

            if response.status().is_success() {
                return Ok(());
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
                .timeout(API_NATIVE_OPEN_TIMEOUT)
                .json(&NativeDesktopOpenRequest { session_id })
                .send()
                .await
                .map_err(|err| self.transport_error("open the selected session", err))?;

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
                .map_err(|err| self.transport_error("list directories", err))?;

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
                .map_err(|err| self.transport_error("create a session", err))?;

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

#[derive(Clone, Debug)]
enum FishBowlMode {
    Aquarium,
    Mermaid(MermaidViewerState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MermaidSourceCacheKey {
    source_hash: u64,
    sample_width: u32,
    sample_height: u32,
}

#[derive(Clone, Debug)]
struct MermaidPreparedRender {
    key: MermaidSourceCacheKey,
    tree: Tree,
    layout: MermaidLayout,
    semantic_lines: Vec<MermaidSemanticLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MermaidTextAnchor {
    Start,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MermaidDetailLevel {
    L1,
    L2,
    L3,
}

impl MermaidDetailLevel {
    fn label(self) -> &'static str {
        match self {
            MermaidDetailLevel::L1 => "L1",
            MermaidDetailLevel::L2 => "L2",
            MermaidDetailLevel::L3 => "L3",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MermaidSemanticKind {
    SubgraphTitle,
    NodeTitle,
    EdgeLabel,
    ClassMember,
    ErAttributeName,
    ErAttributeType,
}

impl MermaidSemanticKind {
    fn min_detail_level(self) -> MermaidDetailLevel {
        match self {
            MermaidSemanticKind::SubgraphTitle | MermaidSemanticKind::NodeTitle => {
                MermaidDetailLevel::L1
            }
            MermaidSemanticKind::EdgeLabel
            | MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName => MermaidDetailLevel::L2,
            MermaidSemanticKind::ErAttributeType => MermaidDetailLevel::L3,
        }
    }

    fn priority(self) -> u8 {
        match self {
            MermaidSemanticKind::SubgraphTitle => 0,
            MermaidSemanticKind::NodeTitle => 1,
            MermaidSemanticKind::ErAttributeName => 2,
            MermaidSemanticKind::ClassMember => 3,
            MermaidSemanticKind::EdgeLabel => 4,
            MermaidSemanticKind::ErAttributeType => 5,
        }
    }

    fn is_visible_for_owner(self, owner_cols: f32, owner_rows: f32) -> bool {
        match self {
            MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::EdgeLabel => true,
            MermaidSemanticKind::ClassMember => owner_cols >= 10.0 && owner_rows >= 2.5,
            MermaidSemanticKind::ErAttributeName => owner_cols >= 8.0 && owner_rows >= 2.5,
            MermaidSemanticKind::ErAttributeType => owner_cols >= 12.0 && owner_rows >= 3.0,
        }
    }

    fn row_nudge_budget(self) -> i32 {
        match self {
            MermaidSemanticKind::ClassMember
            | MermaidSemanticKind::ErAttributeName
            | MermaidSemanticKind::ErAttributeType => 2,
            MermaidSemanticKind::SubgraphTitle
            | MermaidSemanticKind::NodeTitle
            | MermaidSemanticKind::EdgeLabel => 0,
        }
    }
}

#[derive(Clone, Debug)]
struct MermaidSemanticLine {
    text: String,
    diagram_x: f32,
    diagram_y: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_width: f32,
    owner_height: f32,
}

#[derive(Clone, Debug)]
struct MermaidProjectedLine {
    x: u16,
    y: u16,
    text: String,
}

#[derive(Clone, Debug)]
struct MermaidViewerState {
    session_id: String,
    tmux_name: String,
    path: Option<String>,
    source: Option<String>,
    artifact_error: Option<String>,
    render_error: Option<String>,
    unsupported_reason: Option<String>,
    zoom: f32,
    center_x: f32,
    center_y: f32,
    diagram_width: f32,
    diagram_height: f32,
    back_rect: Option<Rect>,
    content_rect: Option<Rect>,
    cached_rect: Option<Rect>,
    cached_zoom: f32,
    cached_center_x: f32,
    cached_center_y: f32,
    cached_lines: Vec<String>,
    cached_semantic_lines: Vec<MermaidProjectedLine>,
    prepared_render: Option<MermaidPreparedRender>,
    source_prepare_count: u64,
    viewport_render_count: u64,
}

impl MermaidViewerState {
    fn display_path(&self) -> &str {
        self.path.as_deref().unwrap_or("unknown.mmd")
    }

    fn openable_path(&self) -> Option<&str> {
        self.path.as_deref().filter(|path| !path.trim().is_empty())
    }

    fn invalidate_viewport_cache(&mut self) {
        self.cached_rect = None;
    }

    fn invalidate_source_cache(&mut self) {
        self.prepared_render = None;
        self.cached_lines.clear();
        self.cached_semantic_lines.clear();
        self.render_error = None;
        self.invalidate_viewport_cache();
    }
}

trait ArtifactOpener: Send + Sync {
    fn open(&self, path: &str) -> io::Result<()>;
}

#[derive(Default)]
struct SystemArtifactOpener;

impl ArtifactOpener for SystemArtifactOpener {
    fn open(&self, path: &str) -> io::Result<()> {
        if cfg!(target_os = "macos") {
            ProcessCommand::new("open").arg(path).spawn().map(|_| ())
        } else if cfg!(target_os = "windows") {
            ProcessCommand::new("cmd")
                .args(["/C", "start", "", path])
                .spawn()
                .map(|_| ())
        } else {
            ProcessCommand::new("xdg-open")
                .arg(path)
                .spawn()
                .map(|_| ())
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MermaidDragState {
    start_column: u16,
    start_row: u16,
    start_center_x: f32,
    start_center_y: f32,
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
    artifact_opener: Arc<dyn ArtifactOpener>,
    entities: Vec<SessionEntity>,
    thought_log: Vec<ThoughtLogEntry>,
    thought_filter: ThoughtFilter,
    last_logged_thoughts: HashMap<String, ThoughtFingerprint>,
    mermaid_artifacts: HashMap<String, MermaidArtifactResponse>,
    repo_themes: HashMap<String, RepoTheme>,
    selected_id: Option<String>,
    published_selected_id: Option<String>,
    native_status: Option<NativeDesktopStatusResponse>,
    picker: Option<PickerState>,
    initial_request: Option<InitialRequestState>,
    fish_bowl_mode: FishBowlMode,
    mermaid_drag: Option<MermaidDragState>,
    message: Option<(String, Instant)>,
    last_refresh: Option<Instant>,
    thought_panel_ratio: f32,
    split_drag_active: bool,
    tick: u64,
}

impl<C: TuiApi> App<C> {
    fn new(runtime: Runtime, client: C) -> Self {
        Self::with_artifact_opener(runtime, client, Arc::new(SystemArtifactOpener))
    }

    fn with_artifact_opener(
        runtime: Runtime,
        client: C,
        artifact_opener: Arc<dyn ArtifactOpener>,
    ) -> Self {
        Self {
            runtime,
            client,
            artifact_opener,
            entities: Vec::new(),
            thought_log: Vec::new(),
            thought_filter: ThoughtFilter::default(),
            last_logged_thoughts: HashMap::new(),
            mermaid_artifacts: HashMap::new(),
            repo_themes: HashMap::new(),
            selected_id: None,
            published_selected_id: None,
            native_status: None,
            picker: None,
            initial_request: None,
            fish_bowl_mode: FishBowlMode::Aquarium,
            mermaid_drag: None,
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
        let message = message.into();
        if self
            .message
            .as_ref()
            .map(|(existing, _)| existing == &message)
            .unwrap_or(false)
        {
            return;
        }
        self.message = Some((message, Instant::now()));
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

    fn publish_selection(&mut self, session_id: Option<String>, force: bool) {
        if !force && session_id == self.published_selected_id {
            return;
        }

        match self
            .runtime
            .block_on(self.client.publish_selection(session_id.as_deref()))
        {
            Ok(()) => {
                self.published_selected_id = session_id;
            }
            Err(err) => self.set_message(err),
        }
    }

    fn sync_selection_publication(&mut self) {
        self.publish_selection(self.selected_id.clone(), false);
    }

    fn clear_published_selection(&mut self) {
        self.publish_selection(None, true);
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
        self.sync_selection_publication();
    }

    fn clear_thought_filters(&mut self) {
        self.thought_filter.clear();
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    fn refresh_mermaid_artifacts(&mut self, sessions: &[SessionSummary]) {
        let mut next = HashMap::new();
        for session in sessions {
            match self
                .runtime
                .block_on(self.client.fetch_mermaid_artifact(&session.session_id))
            {
                Ok(artifact) if artifact.available => {
                    next.insert(session.session_id.clone(), artifact);
                }
                Ok(_) => {}
                Err(err) => self.set_message(err),
            }
        }
        self.mermaid_artifacts = next;

        if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
            if let Some(artifact) = self.mermaid_artifacts.get(&viewer.session_id) {
                let path_changed = viewer.path != artifact.path;
                let source_changed = viewer.source != artifact.source;
                let error_changed = viewer.artifact_error != artifact.error;
                viewer.path = artifact.path.clone();
                viewer.source = artifact.source.clone();
                viewer.artifact_error = artifact.error.clone();
                viewer.render_error = None;
                if source_changed || error_changed {
                    viewer.invalidate_source_cache();
                } else if path_changed {
                    viewer.invalidate_viewport_cache();
                }
            }
        }
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
        self.refresh_with_feedback(layout, false);
    }

    fn manual_refresh(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback(layout, true);
    }

    fn refresh_with_feedback(&mut self, layout: WorkspaceLayout, show_success_message: bool) {
        match self.runtime.block_on(self.client.fetch_sessions()) {
            Ok(sessions) => {
                self.sync_repo_themes(&sessions);
                self.refresh_mermaid_artifacts(&sessions);
                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);
                if show_success_message {
                    let count = self.entities.len();
                    self.set_message(format!("refreshed {count} session{}", pluralize(count)));
                }
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
        self.layout_resting_entities(field);
        self.reconcile_selection();
        self.sync_selection_publication();
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
        self.layout_resting_entities(field);
        for entity in &mut self.entities {
            entity.tick(field, self.tick);
        }
        self.resolve_collisions(field);
    }

    fn layout_resting_entities(&mut self, field: Rect) {
        let mut bottom_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Bottom).then_some(index)
            })
            .collect::<Vec<_>>();
        let mut top_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Top).then_some(index)
            })
            .collect::<Vec<_>>();

        bottom_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });
        top_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });

        for (slot, entity_index) in bottom_resting.into_iter().enumerate() {
            let (x, y) = bottom_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
        }
        for (slot, entity_index) in top_resting.into_iter().enumerate() {
            let (x, y) = top_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
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
                            a.swim_anchor_x = a.x;
                            b.swim_anchor_x = b.x;
                            a.swim_anchor_y = a.y;
                            b.swim_anchor_y = b.y;
                            a.swim_center_y = a.y;
                            b.swim_center_y = b.y;
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
            self.sync_selection_publication();
            return;
        }

        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.selected_id = None;
            self.sync_selection_publication();
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
        self.sync_selection_publication();
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

    fn handle_paste(&mut self, text: &str) {
        if let Some(initial_request) = &mut self.initial_request {
            initial_request.value.push_str(text);
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
                self.sync_selection_publication();
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

        self.select_and_open_session(selected_id, label);
    }

    fn select_and_open_session(&mut self, session_id: String, label: String) {
        self.selected_id = Some(session_id.clone());
        self.sync_selection_publication();
        self.open_session_for_label(&session_id, &label);
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
            ThoughtPanelAction::OpenSession { session_id, label } => {
                self.select_and_open_session(session_id, label);
            }
            ThoughtPanelAction::OpenMermaid(session_id) => self.open_mermaid_viewer(session_id),
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

    fn open_mermaid_artifact(&mut self) {
        let Some(path) = (match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.openable_path().map(str::to_string),
            FishBowlMode::Aquarium => None,
        }) else {
            self.set_message("Mermaid artifact path unavailable");
            return;
        };

        let path_label = path_tail_label(&path).unwrap_or_else(|| path.clone());
        match self.artifact_opener.open(&path) {
            Ok(_) => self.set_message(format!("open Mermaid artifact -> {path_label}")),
            Err(err) => self.set_message(format!("failed to open Mermaid artifact: {err}")),
        }
    }

    fn open_mermaid_viewer(&mut self, session_id: String) {
        let Some(session) = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .map(|entity| entity.session.clone())
        else {
            self.set_message("missing session for Mermaid viewer");
            return;
        };

        let Some(artifact) = self.mermaid_artifacts.get(&session.session_id).cloned() else {
            self.set_message("no Mermaid artifact found");
            return;
        };

        let unsupported_reason = detect_mermaid_backend_support();
        self.fish_bowl_mode = FishBowlMode::Mermaid(MermaidViewerState {
            session_id: session.session_id.clone(),
            tmux_name: session.tmux_name.clone(),
            path: artifact.path,
            source: artifact.source,
            artifact_error: artifact.error,
            render_error: None,
            unsupported_reason,
            zoom: 1.0,
            center_x: 0.0,
            center_y: 0.0,
            diagram_width: 0.0,
            diagram_height: 0.0,
            back_rect: None,
            content_rect: None,
            cached_rect: None,
            cached_zoom: 1.0,
            cached_center_x: 0.0,
            cached_center_y: 0.0,
            cached_lines: Vec::new(),
            cached_semantic_lines: Vec::new(),
            prepared_render: None,
            source_prepare_count: 0,
            viewport_render_count: 0,
        });
    }

    fn close_mermaid_viewer(&mut self) {
        self.fish_bowl_mode = FishBowlMode::Aquarium;
        self.mermaid_drag = None;
    }

    fn mermaid_viewer_mut(&mut self) -> Option<&mut MermaidViewerState> {
        match &mut self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => Some(viewer),
            FishBowlMode::Aquarium => None,
        }
    }

    fn pan_mermaid_viewer(&mut self, dx: f32, dy: f32) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.center_x += dx;
        viewer.center_y += dy;
        viewer.invalidate_viewport_cache();
    }

    fn zoom_mermaid_viewer(
        &mut self,
        factor: f32,
        pointer: Option<(u16, u16)>,
        content_rect: Rect,
    ) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        let old_zoom = viewer.zoom.max(MERMAID_MIN_ZOOM);
        let new_zoom = (old_zoom * factor).clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM);
        if (new_zoom - old_zoom).abs() < f32::EPSILON {
            return;
        }

        if let Some((column, row)) = pointer {
            let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
            let base_scale = mermaid_fit_scale(
                viewer.diagram_width,
                viewer.diagram_height,
                sample_width as f32,
                sample_height as f32,
            );
            let old_scale = base_scale * old_zoom;
            let new_scale = base_scale * new_zoom;
            if old_scale > 0.0 && new_scale > 0.0 {
                let anchor_x = (column.saturating_sub(content_rect.x) as f32) * 2.0;
                let anchor_y = (row.saturating_sub(content_rect.y) as f32) * 4.0;
                let dx = anchor_x - sample_width as f32 / 2.0;
                let dy = anchor_y - sample_height as f32 / 2.0;
                let diagram_x = viewer.center_x + dx / old_scale;
                let diagram_y = viewer.center_y + dy / old_scale;
                viewer.center_x = diagram_x - dx / new_scale;
                viewer.center_y = diagram_y - dy / new_scale;
            }
        }

        viewer.zoom = new_zoom;
        viewer.invalidate_viewport_cache();
    }

    fn reset_mermaid_viewer_fit(&mut self) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.zoom = 1.0;
        viewer.center_x = 0.0;
        viewer.center_y = 0.0;
        viewer.invalidate_viewport_cache();
    }

    fn handle_mermaid_mouse_down(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let back_rect = viewer.back_rect.unwrap_or(Rect {
            x: field.x,
            y: field.y,
            width: display_width(MERMAID_BACK_LABEL),
            height: 1,
        });
        if back_rect.contains(mouse.column, mouse.row) {
            self.close_mermaid_viewer();
            return true;
        }

        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if content_rect.contains(mouse.column, mouse.row) {
            self.mermaid_drag = Some(MermaidDragState {
                start_column: mouse.column,
                start_row: mouse.row,
                start_center_x: viewer.center_x,
                start_center_y: viewer.center_y,
            });
            return true;
        }

        false
    }

    fn handle_mermaid_mouse_drag(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(drag) = self.mermaid_drag else {
            return false;
        };
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
        let scale = mermaid_fit_scale(
            viewer.diagram_width,
            viewer.diagram_height,
            sample_width as f32,
            sample_height as f32,
        ) * viewer.zoom.max(MERMAID_MIN_ZOOM);
        if scale <= 0.0 {
            return false;
        }
        let dx = (mouse.column as i32 - drag.start_column as i32) as f32 * 2.0;
        let dy = (mouse.row as i32 - drag.start_row as i32) as f32 * 4.0;
        viewer.center_x = drag.start_center_x - dx / scale;
        viewer.center_y = drag.start_center_y - dy / scale;
        viewer.invalidate_viewport_cache();
        true
    }

    fn handle_mermaid_mouse_up(&mut self) -> bool {
        let active = self.mermaid_drag.is_some();
        self.mermaid_drag = None;
        active
    }

    fn handle_mermaid_scroll(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
        factor: f32,
    ) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if !content_rect.contains(mouse.column, mouse.row) {
            return false;
        }
        self.zoom_mermaid_viewer(factor, Some((mouse.column, mouse.row)), content_rect);
        true
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
            .map(|entity| {
                (
                    entity.session.session_id.clone(),
                    selected_label(Some(&entity.session.tmux_name)),
                )
            });

        if let Some((session_id, label)) = hit {
            self.select_and_open_session(session_id, label);
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

    fn render(&mut self, renderer: &mut Renderer, layout: WorkspaceLayout) {
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

        match &mut self.fish_bowl_mode {
            FishBowlMode::Aquarium => {
                render_aquarium_background(renderer, layout.overview_field, self.tick);

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
            }
            FishBowlMode::Mermaid(viewer) => {
                render_mermaid_viewer(renderer, layout.overview_field, viewer);
            }
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
    OpenSession { session_id: String, label: String },
    OpenMermaid(String),
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
    text_rect: Option<Rect>,
    mermaid_rect: Option<Rect>,
    session_id: String,
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

#[derive(Clone, Debug)]
struct ThoughtPanelEntryView {
    session_id: String,
    tmux_name: String,
    updated_at: Option<DateTime<Utc>>,
    color: Color,
    text: String,
    has_mermaid: bool,
}

const DARK_TERMINAL_BG_RGB: (u8, u8, u8) = (0x11, 0x11, 0x11);
const MIN_DARK_TERMINAL_CONTRAST: f64 = 4.5;
const DARK_TERMINAL_COLOR_SEARCH_STEPS: usize = 12;

fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let trimmed = value.trim();
    if trimmed.len() != 7 || !trimmed.starts_with('#') {
        return None;
    }

    let r = u8::from_str_radix(&trimmed[1..3], 16).ok()?;
    let g = u8::from_str_radix(&trimmed[3..5], 16).ok()?;
    let b = u8::from_str_radix(&trimmed[5..7], 16).ok()?;
    Some((r, g, b))
}

fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb { r, g, b }
}

fn linearize_srgb_channel(channel: u8) -> f64 {
    let value = channel as f64 / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    0.2126 * linearize_srgb_channel(r)
        + 0.7152 * linearize_srgb_channel(g)
        + 0.0722 * linearize_srgb_channel(b)
}

fn contrast_ratio(foreground: (u8, u8, u8), background: (u8, u8, u8)) -> f64 {
    let fg = relative_luminance(foreground);
    let bg = relative_luminance(background);
    let (lighter, darker) = if fg >= bg { (fg, bg) } else { (bg, fg) };
    (lighter + 0.05) / (darker + 0.05)
}

fn mix_towards_white((r, g, b): (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
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
fn adjust_for_dark_terminal(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
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

fn repo_theme_display_color(value: &str) -> Option<Color> {
    let rgb = parse_hex_rgb(value)?;
    Some(rgb_color(adjust_for_dark_terminal(rgb)))
}

fn session_theme_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Option<Color> {
    let theme_id = session.repo_theme_id.as_ref()?;
    let theme = repo_themes.get(theme_id)?;
    repo_theme_display_color(&theme.body)
}

fn session_display_color(
    session: &SessionSummary,
    repo_themes: &HashMap<String, RepoTheme>,
) -> Color {
    session_theme_color(session, repo_themes)
        .unwrap_or_else(|| SpriteKind::from_session(session).color())
}

fn compare_thought_panel_entries(
    left: &ThoughtPanelEntryView,
    right: &ThoughtPanelEntryView,
) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.tmux_name.cmp(&right.tmux_name))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

fn build_thought_panel_entries<C: TuiApi>(app: &App<C>) -> Vec<ThoughtPanelEntryView> {
    let mut entries = Vec::new();
    let mut thought_sessions = HashSet::new();

    for entry in app
        .thought_log
        .iter()
        .filter(|entry| app.thought_filter.matches(entry))
    {
        thought_sessions.insert(entry.session_id.clone());
        entries.push(ThoughtPanelEntryView {
            session_id: entry.session_id.clone(),
            tmux_name: entry.tmux_name.clone(),
            updated_at: entry.updated_at,
            color: app.thought_entry_display_color(entry),
            text: format!("{}: {}", entry.tmux_name, entry.thought.replace('\n', " ")),
            has_mermaid: app
                .mermaid_artifacts
                .get(&entry.session_id)
                .map(|artifact| artifact.available)
                .unwrap_or(false),
        });
    }

    for entity in app.visible_entities() {
        if thought_sessions.contains(&entity.session.session_id) {
            continue;
        }
        let Some(artifact) = app.mermaid_artifacts.get(&entity.session.session_id) else {
            continue;
        };
        entries.push(ThoughtPanelEntryView {
            session_id: entity.session.session_id.clone(),
            tmux_name: entity.session.tmux_name.clone(),
            updated_at: artifact.updated_at,
            color: session_display_color(&entity.session, &app.repo_themes),
            text: format!("{}: mermaid diagram ready", entity.session.tmux_name),
            has_mermaid: true,
        });
    }

    entries.sort_by(compare_thought_panel_entries);
    entries
}

fn build_rows_for_panel_entry(
    entry: &ThoughtPanelEntryView,
    thought_content: Rect,
) -> Vec<ThoughtRowLayout> {
    let button_width = display_width(THOUGHT_MERMAID_LABEL);
    let reserved = if entry.has_mermaid {
        button_width.saturating_add(1)
    } else {
        0
    };
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
            ThoughtRowLayout {
                text_rect: (visible_line_width > 0).then_some(Rect {
                    x: text_x,
                    y: 0,
                    width: visible_line_width,
                    height: 1,
                }),
                mermaid_rect: (index == 0 && entry.has_mermaid).then_some(Rect {
                    x: thought_content.x,
                    y: 0,
                    width: button_width,
                    height: 1,
                }),
                session_id: entry.session_id.clone(),
                tmux_name: entry.tmux_name.clone(),
                line,
                color: entry.color,
            }
        })
        .collect()
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
        let y = start_y + offset as u16;
        if let Some(rect) = row.mermaid_rect {
            renderer.draw_text(rect.x, y, THOUGHT_MERMAID_LABEL, Color::Cyan);
        }
        if let Some(rect) = row.text_rect {
            renderer.draw_text(rect.x, y, &row.line, row.color);
        }
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
        let text_rect = row.text_rect.map(|rect| Rect {
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
        if mermaid_rect
            .map(|rect| rect.contains(x, y))
            .unwrap_or(false)
        {
            return Some(ThoughtPanelAction::OpenMermaid(row.session_id));
        }
        if text_rect.map(|rect| rect.contains(x, y)).unwrap_or(false) {
            return Some(ThoughtPanelAction::OpenSession {
                session_id: row.session_id,
                label: row.tmux_name,
            });
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
    let color = repo_theme_display_color(&theme.body)?;
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
        "resize the terminal and reopen the TUI",
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

fn rest_grid_columns(field: Rect) -> usize {
    usize::from((field.width / ENTITY_WIDTH).max(1))
}

fn rest_grid_rows(field: Rect) -> usize {
    usize::from((field.height / ENTITY_HEIGHT).max(1))
}

fn bottom_rest_origin(field: Rect, slot: usize) -> (u16, u16) {
    let columns = rest_grid_columns(field);
    let max_rows = rest_grid_rows(field).saturating_sub(1);
    let row = (slot / columns).min(max_rows);
    let column = slot % columns;
    (
        column as u16 * ENTITY_WIDTH,
        field
            .height
            .saturating_sub(ENTITY_HEIGHT * (row as u16 + 1)),
    )
}

fn top_rest_origin(field: Rect, slot: usize) -> (u16, u16) {
    let columns = rest_grid_columns(field);
    let max_rows = rest_grid_rows(field).saturating_sub(1);
    let row = (slot / columns).min(max_rows);
    let column = slot % columns;
    (column as u16 * ENTITY_WIDTH, row as u16 * ENTITY_HEIGHT)
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
    entity.swim_anchor_x = entity.x;
    entity.swim_anchor_y = entity.y;
    entity.swim_center_y = entity.y;
}

fn swim_speed(hash: u64) -> f32 {
    let segment = (hash & 0xff) as f32 / 255.0;
    0.18 + segment * 0.22
}

fn vertical_drift(hash: u64) -> f32 {
    let segment = ((hash >> 8) & 0xff) as f32 / 255.0;
    let speed = 0.03 + segment * 0.05;
    if hash & 2 == 0 {
        speed
    } else {
        -speed
    }
}

fn bob_phase(hash: u64) -> f32 {
    ((hash >> 16) & 0xff) as f32 / 255.0 * TAU
}

fn detect_mermaid_backend_support() -> Option<String> {
    let term = env::var("TERM").unwrap_or_default();
    if term.is_empty() || term == "dumb" {
        return Some("inline Mermaid rendering is unsupported for TERM=dumb".to_string());
    }
    None
}

fn mermaid_content_rect(field: Rect) -> Rect {
    if field.height <= 1 {
        return Rect {
            x: field.x,
            y: field.y,
            width: field.width,
            height: 0,
        };
    }
    Rect {
        x: field.x,
        y: field.y + 1,
        width: field.width,
        height: field.height - 1,
    }
}

fn mermaid_sample_dimensions(content_rect: Rect) -> (u32, u32) {
    (
        u32::from(content_rect.width.max(1)) * 2,
        u32::from(content_rect.height.max(1)) * 4,
    )
}

fn mermaid_fit_scale(
    diagram_width: f32,
    diagram_height: f32,
    sample_width: f32,
    sample_height: f32,
) -> f32 {
    if diagram_width <= 0.0 || diagram_height <= 0.0 || sample_width <= 0.0 || sample_height <= 0.0
    {
        return 1.0;
    }
    (sample_width / diagram_width)
        .min(sample_height / diagram_height)
        .max(0.000_1)
}

fn clamp_mermaid_center(center: f32, visible: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    if visible >= total {
        return total / 2.0;
    }
    center.clamp(visible / 2.0, total - visible / 2.0)
}

fn mermaid_pan_step(viewer: &MermaidViewerState, content_rect: Rect) -> (f32, f32) {
    if viewer.diagram_width <= 0.0 || viewer.diagram_height <= 0.0 {
        return (40.0, 24.0);
    }
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.max(MERMAID_MIN_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;
    (visible_width / 6.0, visible_height / 6.0)
}

fn mermaid_source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn mermaid_source_cache_key(source: &str, content_rect: Rect) -> MermaidSourceCacheKey {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    MermaidSourceCacheKey {
        source_hash: mermaid_source_hash(source),
        sample_width,
        sample_height,
    }
}

fn mermaid_render_options(content_rect: Rect) -> RenderOptions {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    RenderOptions::default()
        .with_preferred_aspect_ratio_parts(sample_width as f32, sample_height as f32)
}

fn mermaid_fontdb() -> Arc<usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTDB
        .get_or_init(|| {
            let mut fontdb = usvg::fontdb::Database::new();
            fontdb.load_system_fonts();
            Arc::new(fontdb)
        })
        .clone()
}

fn mermaid_usvg_options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: mermaid_fontdb(),
        ..usvg::Options::default()
    }
}

fn mermaid_kind_supports_semantic_overlay(kind: DiagramKind) -> bool {
    matches!(
        kind,
        DiagramKind::Flowchart
            | DiagramKind::Class
            | DiagramKind::State
            | DiagramKind::Er
            | DiagramKind::Requirement
            | DiagramKind::Packet
    )
}

fn mermaid_is_divider_line(line: &str) -> bool {
    line.trim() == "---"
}

fn push_mermaid_indexed_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    lines: &[(usize, &str)],
    diagram_x: f32,
    start_y: f32,
    line_height: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_width: f32,
    owner_height: f32,
) {
    for (idx, line) in lines {
        let text = line.trim();
        if text.is_empty() || mermaid_is_divider_line(text) {
            continue;
        }
        target.push(MermaidSemanticLine {
            text: text.to_string(),
            diagram_x,
            diagram_y: start_y + *idx as f32 * line_height,
            anchor,
            kind,
            owner_width,
            owner_height,
        });
    }
}

fn push_mermaid_text_block_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    label: &mermaid_rs_renderer::layout::TextBlock,
    diagram_x: f32,
    center_y: f32,
    font_size: f32,
    line_height: f32,
    anchor: MermaidTextAnchor,
    kind: MermaidSemanticKind,
    owner_width: f32,
    owner_height: f32,
) {
    if label.lines.is_empty() {
        return;
    }
    let total_height = label.lines.len() as f32 * line_height;
    let start_y = center_y - total_height / 2.0 + font_size;
    let indexed_lines: Vec<(usize, &str)> = label
        .lines
        .iter()
        .enumerate()
        .map(|(idx, line)| (idx, line.as_str()))
        .collect();
    push_mermaid_indexed_semantic_lines(
        target,
        &indexed_lines,
        diagram_x,
        start_y,
        line_height,
        anchor,
        kind,
        owner_width,
        owner_height,
    );
}

fn extend_mermaid_class_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    theme_font_size: f32,
    class_line_height: f32,
    node_padding_x: f32,
) {
    let total_height = node.label.lines.len() as f32 * class_line_height;
    let start_y = node.y + node.height / 2.0 - total_height / 2.0 + theme_font_size;
    let center_x = node.x + node.width / 2.0;
    let left_x = node.x + node_padding_x.max(10.0);
    let Some(divider_idx) = node
        .label
        .lines
        .iter()
        .position(|line| mermaid_is_divider_line(line))
    else {
        let indexed_lines: Vec<(usize, &str)> = node
            .label
            .lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (idx, line.as_str()))
            .collect();
        push_mermaid_indexed_semantic_lines(
            target,
            &indexed_lines,
            center_x,
            start_y,
            class_line_height,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeTitle,
            node.width,
            node.height,
        );
        return;
    };

    let title_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .take(divider_idx)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty()).then_some((idx, line.as_str()))
        })
        .collect();
    let member_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .skip(divider_idx + 1)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty() && !mermaid_is_divider_line(text)).then_some((idx, line.as_str()))
        })
        .collect();

    push_mermaid_indexed_semantic_lines(
        target,
        &title_lines,
        center_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeTitle,
        node.width,
        node.height,
    );
    push_mermaid_indexed_semantic_lines(
        target,
        &member_lines,
        left_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Start,
        MermaidSemanticKind::ClassMember,
        node.width,
        node.height,
    );
}

fn extend_mermaid_er_semantic_lines(
    target: &mut Vec<MermaidSemanticLine>,
    node: &mermaid_rs_renderer::NodeLayout,
    theme_font_size: f32,
    class_line_height: f32,
    node_padding_x: f32,
) {
    let Some(divider_idx) = node
        .label
        .lines
        .iter()
        .position(|line| mermaid_is_divider_line(line))
    else {
        return;
    };

    let total_height = node.label.lines.len() as f32 * class_line_height;
    let start_y = node.y + node.height / 2.0 - total_height / 2.0 + theme_font_size;
    let center_x = node.x + node.width / 2.0;
    let left_x = node.x + node_padding_x.max(10.0);

    let title_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .take(divider_idx)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty()).then_some((idx, line.as_str()))
        })
        .collect();
    push_mermaid_indexed_semantic_lines(
        target,
        &title_lines,
        center_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Center,
        MermaidSemanticKind::NodeTitle,
        node.width,
        node.height,
    );

    let attr_lines: Vec<(usize, &str)> = node
        .label
        .lines
        .iter()
        .enumerate()
        .skip(divider_idx + 1)
        .filter_map(|(idx, line)| {
            let text = line.trim();
            (!text.is_empty() && !mermaid_is_divider_line(text)).then_some((idx, line.as_str()))
        })
        .collect();
    if attr_lines.is_empty() {
        return;
    }

    let mut parsed_attrs = Vec::new();
    let mut max_type_chars = 0usize;
    let mut use_columns = true;
    for (idx, line) in &attr_lines {
        let trimmed = line.trim();
        let mut parts = trimmed.split_whitespace();
        let Some(data_type) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            use_columns = false;
            break;
        };
        max_type_chars = max_type_chars.max(data_type.chars().count());
        parsed_attrs.push((*idx, data_type.to_string(), name.to_string()));
    }

    let gap_chars = 2usize;
    let name_x = left_x + ((max_type_chars + gap_chars) as f32 * theme_font_size * 0.6);
    let content_width = (node.width - node_padding_x.max(10.0) * 2.0).max(0.0);
    if use_columns && parsed_attrs.len() == attr_lines.len() && name_x < node.x + content_width {
        for (idx, data_type, name) in parsed_attrs {
            let diagram_y = start_y + idx as f32 * class_line_height;
            target.push(MermaidSemanticLine {
                text: data_type,
                diagram_x: left_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeType,
                owner_width: node.width,
                owner_height: node.height,
            });
            target.push(MermaidSemanticLine {
                text: name,
                diagram_x: name_x,
                diagram_y,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::ErAttributeName,
                owner_width: node.width,
                owner_height: node.height,
            });
        }
        return;
    }

    push_mermaid_indexed_semantic_lines(
        target,
        &attr_lines,
        left_x,
        start_y,
        class_line_height,
        MermaidTextAnchor::Start,
        MermaidSemanticKind::ErAttributeName,
        node.width,
        node.height,
    );
}

fn build_mermaid_semantic_lines(
    layout: &MermaidLayout,
    options: &RenderOptions,
) -> Vec<MermaidSemanticLine> {
    if !mermaid_kind_supports_semantic_overlay(layout.kind) {
        return Vec::new();
    }

    let theme_font_size = options.theme.font_size;
    let base_line_height = theme_font_size * options.layout.label_line_height;
    let class_line_height = theme_font_size * options.layout.class_label_line_height();
    let state_font_size = if layout.kind == DiagramKind::State {
        theme_font_size * 0.85
    } else {
        theme_font_size
    };
    let state_line_height = state_font_size * options.layout.label_line_height;
    let mut semantic_lines = Vec::new();

    for subgraph in &layout.subgraphs {
        if subgraph.label.trim().is_empty() {
            continue;
        }
        if layout.kind == DiagramKind::State {
            let header_height =
                (subgraph.label_block.height + theme_font_size * 0.75).max(theme_font_size * 1.4);
            let label_x =
                subgraph.x + (theme_font_size * 0.6).max(subgraph.label_block.height * 0.35);
            let label_y = subgraph.y + header_height / 2.0;
            push_mermaid_text_block_semantic_lines(
                &mut semantic_lines,
                &subgraph.label_block,
                label_x,
                label_y,
                state_font_size,
                state_line_height,
                MermaidTextAnchor::Start,
                MermaidSemanticKind::SubgraphTitle,
                subgraph.width,
                subgraph.height,
            );
        } else {
            let label_x = subgraph.x + subgraph.width / 2.0;
            let label_y = subgraph.y + 12.0 + subgraph.label_block.height / 2.0;
            push_mermaid_text_block_semantic_lines(
                &mut semantic_lines,
                &subgraph.label_block,
                label_x,
                label_y,
                theme_font_size,
                base_line_height,
                MermaidTextAnchor::Center,
                MermaidSemanticKind::SubgraphTitle,
                subgraph.width,
                subgraph.height,
            );
        }
    }

    for edge in &layout.edges {
        if let Some(label) = edge.label.as_ref() {
            if let Some((label_x, label_y)) = edge.label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    0.0,
                    0.0,
                );
            }
        }
        if let Some(label) = edge.start_label.as_ref() {
            if let Some((label_x, label_y)) = edge.start_label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    0.0,
                    0.0,
                );
            }
        }
        if let Some(label) = edge.end_label.as_ref() {
            if let Some((label_x, label_y)) = edge.end_label_anchor {
                let (font_size, line_height) = if layout.kind == DiagramKind::State {
                    (state_font_size, state_line_height)
                } else {
                    (theme_font_size, base_line_height)
                };
                push_mermaid_text_block_semantic_lines(
                    &mut semantic_lines,
                    label,
                    label_x,
                    label_y,
                    font_size,
                    line_height,
                    MermaidTextAnchor::Center,
                    MermaidSemanticKind::EdgeLabel,
                    0.0,
                    0.0,
                );
            }
        }
    }

    for node in layout.nodes.values() {
        if node.hidden || node.anchor_subgraph.is_some() {
            continue;
        }
        let hide_label = node.label.lines.iter().all(|line| line.trim().is_empty())
            || node.id.starts_with("__start_")
            || node.id.starts_with("__end_");
        if hide_label {
            continue;
        }

        if layout.kind == DiagramKind::Er
            && node
                .label
                .lines
                .iter()
                .any(|line| mermaid_is_divider_line(line))
        {
            extend_mermaid_er_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
            );
            continue;
        }

        if node
            .label
            .lines
            .iter()
            .any(|line| mermaid_is_divider_line(line))
        {
            extend_mermaid_class_semantic_lines(
                &mut semantic_lines,
                node,
                theme_font_size,
                class_line_height,
                options.layout.node_padding_x,
            );
            continue;
        }

        let center_x = node.x + node.width / 2.0;
        let center_y = node.y + node.height / 2.0;
        let (font_size, line_height) = if layout.kind == DiagramKind::State {
            (state_font_size, state_line_height)
        } else {
            (theme_font_size, base_line_height)
        };
        push_mermaid_text_block_semantic_lines(
            &mut semantic_lines,
            &node.label,
            center_x,
            center_y,
            font_size,
            line_height,
            MermaidTextAnchor::Center,
            MermaidSemanticKind::NodeTitle,
            node.width,
            node.height,
        );
    }

    semantic_lines
}

#[derive(Clone, Copy, Debug)]
struct MermaidViewportTransform {
    scale: f32,
    tx: f32,
    ty: f32,
}

fn mermaid_detail_level_for_view(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> MermaidDetailLevel {
    let _ = content_rect;
    let effective_zoom = viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM);

    if effective_zoom >= 2.4 {
        MermaidDetailLevel::L3
    } else if effective_zoom >= 1.4 {
        MermaidDetailLevel::L2
    } else {
        MermaidDetailLevel::L1
    }
}

fn mermaid_viewport_transform(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(u32, u32, MermaidViewportTransform), String> {
    ensure_mermaid_prepared_render(viewer, content_rect)?;
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    if viewer.center_x <= 0.0 && viewer.center_y <= 0.0 {
        viewer.center_x = viewer.diagram_width / 2.0;
        viewer.center_y = viewer.diagram_height / 2.0;
    }

    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;
    viewer.center_x = clamp_mermaid_center(viewer.center_x, visible_width, viewer.diagram_width);
    viewer.center_y = clamp_mermaid_center(viewer.center_y, visible_height, viewer.diagram_height);

    Ok((
        sample_width,
        sample_height,
        MermaidViewportTransform {
            scale,
            tx: sample_width as f32 / 2.0 - viewer.center_x * scale,
            ty: sample_height as f32 / 2.0 - viewer.center_y * scale,
        },
    ))
}

fn clip_mermaid_overlay_text(text: &str, _skip: usize, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    text.chars().take(max_chars).collect()
}

fn project_mermaid_semantic_lines(
    lines: &[MermaidSemanticLine],
    transform: MermaidViewportTransform,
    content_rect: Rect,
    detail_level: MermaidDetailLevel,
) -> Vec<MermaidProjectedLine> {
    #[derive(Clone)]
    struct MermaidProjectedCandidate {
        priority: u8,
        kind: MermaidSemanticKind,
        x: u16,
        y: u16,
        text: String,
    }

    let mut candidates = Vec::new();
    let left = content_rect.x as i32;
    let right = content_rect.right() as i32;
    let top = content_rect.y as i32;
    let bottom = content_rect.bottom() as i32;

    for line in lines {
        if line.kind.min_detail_level() > detail_level {
            continue;
        }

        let owner_cols = (line.owner_width * transform.scale / 2.0).max(0.0);
        let owner_rows = (line.owner_height * transform.scale / 4.0).max(0.0);
        if !line.kind.is_visible_for_owner(owner_cols, owner_rows) {
            continue;
        }

        let projected_x = line.diagram_x * transform.scale + transform.tx;
        let projected_y = line.diagram_y * transform.scale + transform.ty;
        let mut screen_y = top + (projected_y / 4.0).floor() as i32;
        if screen_y < top || screen_y >= bottom {
            if line.kind.row_nudge_budget() > 0 {
                screen_y = screen_y.clamp(top, bottom.saturating_sub(1));
            } else {
                continue;
            }
        }

        let anchor_x = left + (projected_x / 2.0).floor() as i32;
        let text_width = display_width(&line.text) as i32;
        if text_width <= 0 {
            continue;
        }
        let mut screen_x = match line.anchor {
            MermaidTextAnchor::Start => anchor_x,
            MermaidTextAnchor::Center => anchor_x - text_width / 2,
        };
        if screen_x >= right || screen_x + text_width <= left {
            continue;
        }

        let skipped_chars = if screen_x < left {
            (left - screen_x) as usize
        } else {
            0
        };
        if screen_x < left {
            screen_x = left;
        }
        let max_chars = right.saturating_sub(screen_x) as usize;
        let clipped = clip_mermaid_overlay_text(&line.text, skipped_chars, max_chars);
        if clipped.is_empty() {
            continue;
        }

        candidates.push(MermaidProjectedCandidate {
            priority: line.kind.priority(),
            kind: line.kind,
            x: screen_x as u16,
            y: screen_y as u16,
            text: clipped,
        });
    }

    candidates.sort_by_key(|line| (line.priority, line.y, line.x));

    let mut occupied_rows: HashMap<u16, Vec<(u16, u16)>> = HashMap::new();
    let mut projected = Vec::new();
    for candidate in candidates {
        let start = candidate.x;
        let end = candidate
            .x
            .saturating_add(display_width(&candidate.text).max(1));
        let budget = candidate.kind.row_nudge_budget();
        let mut target_y = None;
        let mut row_candidates = vec![candidate.y as i32];
        for offset in 1..=budget {
            row_candidates.push(candidate.y as i32 + offset);
            row_candidates.push(candidate.y as i32 - offset);
        }

        for row in row_candidates {
            if row < top || row >= bottom {
                continue;
            }
            let row = row as u16;
            let overlaps = occupied_rows
                .get(&row)
                .map(|ranges| {
                    ranges
                        .iter()
                        .any(|(left, right)| start < *right && end > *left)
                })
                .unwrap_or(false);
            if !overlaps {
                target_y = Some(row);
                break;
            }
        }

        let Some(target_y) = target_y else {
            continue;
        };

        occupied_rows
            .entry(target_y)
            .or_default()
            .push((start, end));
        projected.push(MermaidProjectedLine {
            x: candidate.x,
            y: target_y,
            text: candidate.text,
        });
    }

    projected.sort_by_key(|line| (line.y, line.x));
    projected
}

fn braille_bit(sub_x: u32, sub_y: u32) -> u32 {
    match (sub_x, sub_y) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0,
    }
}

fn pixel_is_dark(pixmap: &Pixmap, x: u32, y: u32) -> bool {
    let width = pixmap.width();
    let height = pixmap.height();
    if x >= width || y >= height {
        return false;
    }
    let idx = ((y * width + x) * 4) as usize;
    let data = pixmap.data();
    if idx + 3 >= data.len() {
        return false;
    }
    let b = data[idx] as f32;
    let g = data[idx + 1] as f32;
    let r = data[idx + 2] as f32;
    let a = data[idx + 3] as f32 / 255.0;
    if a <= 0.1 {
        return false;
    }
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    luminance < 230.0
}

fn pixmap_to_braille_lines(pixmap: &Pixmap, content_rect: Rect) -> Vec<String> {
    let mut lines = Vec::new();
    for cell_y in 0..content_rect.height {
        let mut line = String::with_capacity(content_rect.width as usize);
        for cell_x in 0..content_rect.width {
            let mut bits = 0u32;
            let base_x = u32::from(cell_x) * 2;
            let base_y = u32::from(cell_y) * 4;
            for sub_y in 0..4 {
                for sub_x in 0..2 {
                    if pixel_is_dark(pixmap, base_x + sub_x, base_y + sub_y) {
                        bits |= braille_bit(sub_x, sub_y);
                    }
                }
            }
            if bits == 0 {
                line.push(' ');
            } else {
                line.push(char::from_u32(0x2800 + bits).unwrap_or(' '));
            }
        }
        lines.push(line);
    }
    lines
}

fn ensure_mermaid_prepared_render(
    viewer: &mut MermaidViewerState,
    content_rect: Rect,
) -> Result<(), String> {
    let source = viewer
        .source
        .as_deref()
        .ok_or_else(|| "Mermaid source unavailable".to_string())?;
    let key = mermaid_source_cache_key(source, content_rect);
    let prepared = if let Some(prepared) = viewer
        .prepared_render
        .as_ref()
        .filter(|prepared| prepared.key == key)
    {
        prepared.clone()
    } else {
        let options = mermaid_render_options(content_rect);
        let parsed = parse_mermaid(source).map_err(|err| err.to_string())?;
        let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
        let semantic_lines = build_mermaid_semantic_lines(&layout, &options);
        let svg = render_svg(&layout, &options.theme, &options.layout);
        let tree = Tree::from_str(&svg, &mermaid_usvg_options())
            .map_err(|err| format!("failed to parse rendered SVG: {err}"))?;
        let prepared = MermaidPreparedRender {
            key,
            tree,
            layout,
            semantic_lines,
        };
        viewer.prepared_render = Some(prepared.clone());
        viewer.source_prepare_count = viewer.source_prepare_count.saturating_add(1);
        prepared
    };
    viewer.diagram_width = prepared.layout.width.max(1.0);
    viewer.diagram_height = prepared.layout.height.max(1.0);
    Ok(())
}

fn render_mermaid_lines(viewer: &mut MermaidViewerState, content_rect: Rect) -> Result<(), String> {
    let (sample_width, sample_height, transform) =
        mermaid_viewport_transform(viewer, content_rect)?;
    let detail_level = mermaid_detail_level_for_view(viewer, content_rect);

    let mut pixmap = Pixmap::new(sample_width, sample_height)
        .ok_or_else(|| "failed to allocate Mermaid viewport".to_string())?;
    pixmap.fill(resvg::tiny_skia::Color::from_rgba8(255, 255, 255, 255));

    let mut pixmap_mut = pixmap.as_mut();
    let Some(prepared) = viewer.prepared_render.as_ref() else {
        return Err("Mermaid source unavailable".to_string());
    };
    resvg::render(
        &prepared.tree,
        Transform::from_row(
            transform.scale,
            0.0,
            0.0,
            transform.scale,
            transform.tx,
            transform.ty,
        ),
        &mut pixmap_mut,
    );

    viewer.cached_lines = pixmap_to_braille_lines(&pixmap, content_rect);
    viewer.cached_semantic_lines = project_mermaid_semantic_lines(
        &prepared.semantic_lines,
        transform,
        content_rect,
        detail_level,
    );
    viewer.cached_rect = Some(content_rect);
    viewer.cached_zoom = viewer.zoom;
    viewer.cached_center_x = viewer.center_x;
    viewer.cached_center_y = viewer.center_y;
    viewer.viewport_render_count = viewer.viewport_render_count.saturating_add(1);
    Ok(())
}

fn render_wrapped_lines(renderer: &mut Renderer, rect: Rect, text: &str, color: Color) {
    let mut y = rect.y;
    for line in wrap_text(text, rect.width as usize) {
        if y >= rect.bottom() {
            break;
        }
        renderer.draw_text(
            rect.x,
            y,
            &truncate_label(&line, rect.width as usize),
            color,
        );
        y += 1;
    }
}

fn render_mermaid_viewer(renderer: &mut Renderer, field: Rect, viewer: &mut MermaidViewerState) {
    renderer.fill_rect(field, ' ', Color::Reset);
    viewer.back_rect = Some(Rect {
        x: field.x,
        y: field.y,
        width: display_width(MERMAID_BACK_LABEL),
        height: 1,
    });
    renderer.draw_text(field.x, field.y, MERMAID_BACK_LABEL, Color::Cyan);

    let content_rect = mermaid_content_rect(field);
    viewer.content_rect = Some(content_rect);
    let detail_level = mermaid_detail_level_for_view(viewer, content_rect);
    let status_x = field
        .x
        .saturating_add(display_width(MERMAID_BACK_LABEL) + 1);
    let status_width = field.right().saturating_sub(status_x) as usize;
    let detail_label = format!("detail {}", detail_level.label());
    let fixed_width = usize::from(display_width(&viewer.tmux_name))
        + usize::from(display_width(" | "))
        + usize::from(display_width(&detail_label))
        + usize::from(display_width(" | "))
        + usize::from(display_width(" | zoom 100% | "))
        + usize::from(display_width(" | o open"));
    let status = format!(
        "{} | {} | {} | zoom {:>3.0}% | o open",
        viewer.tmux_name,
        detail_label,
        shorten_path(
            viewer.display_path(),
            status_width.saturating_sub(fixed_width)
        ),
        viewer.zoom * 100.0,
    );
    renderer.draw_text(
        status_x,
        field.y,
        &truncate_label(&status, status_width),
        Color::DarkGrey,
    );

    if content_rect.width < MERMAID_VIEW_MIN_WIDTH || content_rect.height < MERMAID_VIEW_MIN_HEIGHT
    {
        render_wrapped_lines(
            renderer,
            content_rect,
            "Mermaid view too small",
            Color::DarkGrey,
        );
        return;
    }

    if let Some(reason) = viewer.unsupported_reason.as_deref() {
        render_wrapped_lines(renderer, content_rect, reason, Color::DarkGrey);
        return;
    }
    if let Some(error) = viewer.artifact_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
        return;
    }

    let needs_rerender = viewer.cached_rect != Some(content_rect)
        || viewer.prepared_render.is_none()
        || (viewer.cached_zoom - viewer.zoom).abs() > f32::EPSILON
        || (viewer.cached_center_x - viewer.center_x).abs() > f32::EPSILON
        || (viewer.cached_center_y - viewer.center_y).abs() > f32::EPSILON;
    if needs_rerender {
        viewer.render_error = render_mermaid_lines(viewer, content_rect).err();
    }

    if let Some(error) = viewer.render_error.as_deref() {
        render_wrapped_lines(renderer, content_rect, error, Color::Red);
        return;
    }

    for (offset, line) in viewer.cached_lines.iter().enumerate() {
        let y = content_rect.y + offset as u16;
        if y >= content_rect.bottom() {
            break;
        }
        renderer.draw_text(content_rect.x, y, line, Color::White);
    }
    for line in &viewer.cached_semantic_lines {
        renderer.draw_text(line.x, line.y, &line.text, Color::White);
    }
}

fn render_aquarium_background(renderer: &mut Renderer, field: Rect, tick: u64) {
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

fn initialize_tui_app() -> Result<(App<ApiClient>, Renderer), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    let runtime = Runtime::new()?;
    let client = ApiClient::from_env().map_err(io::Error::other)?;
    runtime
        .block_on(client.preflight_startup_access())
        .map_err(io::Error::other)?;
    let mut renderer = Renderer::new()?;
    renderer.init()?;

    let mut app = App::new(runtime, client);
    let initial_layout = app.layout_for_terminal(renderer.width(), renderer.height());
    app.refresh(initial_layout);

    Ok((app, renderer))
}

fn prepare_frame<C: TuiApi>(app: &mut App<C>, renderer: &mut Renderer) -> WorkspaceLayout {
    let layout = app.layout_for_terminal(renderer.width(), renderer.height());
    if layout.split_divider.is_none() {
        app.stop_split_drag();
    }
    app.trim_thought_log(layout.thought_entry_capacity());
    if app.should_refresh() {
        app.refresh(layout);
    }
    app.tick(layout.overview_field);
    app.render(renderer, layout);
    layout
}

fn handle_key_event<C: TuiApi>(app: &mut App<C>, layout: WorkspaceLayout, key: KeyEvent) -> bool {
    if app.initial_request.is_some() {
        app.handle_initial_request_key(key, layout.overview_field);
        return true;
    }

    if let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode {
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(layout.overview_field));
        let (step_x, step_y) = mermaid_pan_step(viewer, content_rect);
        return match key.code {
            KeyCode::Char('q') => false,
            KeyCode::Esc => {
                app.close_mermaid_viewer();
                true
            }
            KeyCode::Left | KeyCode::Char('h') => {
                app.pan_mermaid_viewer(-step_x, 0.0);
                true
            }
            KeyCode::Right | KeyCode::Char('l') => {
                app.pan_mermaid_viewer(step_x, 0.0);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.pan_mermaid_viewer(0.0, -step_y);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.pan_mermaid_viewer(0.0, step_y);
                true
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                app.zoom_mermaid_viewer(MERMAID_ZOOM_STEP, None, content_rect);
                true
            }
            KeyCode::Char('-') => {
                app.zoom_mermaid_viewer(1.0 / MERMAID_ZOOM_STEP, None, content_rect);
                true
            }
            KeyCode::Char('o') => {
                app.open_mermaid_artifact();
                true
            }
            KeyCode::Char('0') => {
                app.reset_mermaid_viewer_fit();
                true
            }
            _ => true,
        };
    }

    match key.code {
        KeyCode::Char('q') => false,
        KeyCode::Esc => {
            if app.picker.is_some() {
                app.close_picker();
                true
            } else {
                false
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
            if app.picker.is_some() {
                app.picker_up();
            } else {
                app.move_selection(-1, layout.overview_field);
            }
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1, layout.overview_field);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1, layout.overview_field);
            true
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char('o') => {
            if app.picker.is_some() {
                app.picker_activate_selection(layout.overview_field);
            } else {
                app.open_selected();
            }
            true
        }
        KeyCode::Char('e') => {
            app.picker_set_managed_only(true);
            true
        }
        KeyCode::Char('a') => {
            app.picker_set_managed_only(false);
            true
        }
        KeyCode::Char('r') => {
            if let Some((path, managed_only)) = app
                .picker
                .as_ref()
                .map(|picker| (picker.current_path.clone(), picker.managed_only))
            {
                app.picker_reload(Some(path), managed_only);
            } else {
                app.manual_refresh(layout);
            }
            true
        }
        _ => true,
    }
}

fn handle_mouse_down<C: TuiApi>(
    app: &mut App<C>,
    renderer: &Renderer,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) {
    if app.initial_request.is_some() {
        return;
    }
    if handle_split_or_header_click(app, renderer.width(), layout, mouse) {
        return;
    }
    if app.handle_mermaid_mouse_down(layout.overview_field, mouse) {
        return;
    }
    handle_workspace_click(app, layout, mouse);
}

fn handle_split_or_header_click<C: TuiApi>(
    app: &mut App<C>,
    width: u16,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) -> bool {
    if layout
        .split_hitbox
        .map(|hitbox| hitbox.contains(mouse.column, mouse.row))
        .unwrap_or(false)
    {
        app.start_split_drag(layout, mouse.column);
        return true;
    }
    if header_filter_action_at(app, width, mouse.column, mouse.row).is_some() {
        app.handle_header_filter_click(width, mouse.column, mouse.row);
        return true;
    }
    false
}

fn handle_workspace_click<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    mouse: crossterm::event::MouseEvent,
) {
    if let Some(thought_box) = layout.thought_box {
        if thought_box.contains(mouse.column, mouse.row) {
            if let Some(thought_content) = layout.thought_content {
                app.handle_thought_click(
                    mouse.column,
                    mouse.row,
                    thought_content,
                    layout.thought_entry_capacity(),
                );
            }
            return;
        }
    }
    if layout.overview_field.contains(mouse.column, mouse.row) {
        app.handle_field_click(mouse.column, mouse.row, layout.overview_field);
    }
}

fn handle_tui_event<C: TuiApi>(
    app: &mut App<C>,
    renderer: &mut Renderer,
    layout: WorkspaceLayout,
    event: Event,
) -> io::Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            Ok(handle_key_event(app, layout, key))
        }
        Event::Paste(text) => {
            app.handle_paste(&text);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) => {
            handle_mouse_down(app, renderer, layout, mouse);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) => {
            if app.drag_split(layout, mouse.column) {
                return Ok(true);
            }
            if app.handle_mermaid_mouse_drag(layout.overview_field, mouse) {
                return Ok(true);
            }
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) => {
            app.stop_split_drag();
            app.handle_mermaid_mouse_up();
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::ScrollUp) => {
            let _ = app.handle_mermaid_scroll(layout.overview_field, mouse, MERMAID_ZOOM_STEP);
            Ok(true)
        }
        Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::ScrollDown) => {
            let _ =
                app.handle_mermaid_scroll(layout.overview_field, mouse, 1.0 / MERMAID_ZOOM_STEP);
            Ok(true)
        }
        Event::Resize(width, height) => {
            app.stop_split_drag();
            renderer.manual_resize(width, height)?;
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, mut renderer) = initialize_tui_app()?;

    loop {
        let layout = prepare_frame(&mut app, &mut renderer);
        renderer.flush()?;

        if event::poll(FRAME_DURATION)?
            && !handle_tui_event(&mut app, &mut renderer, layout, event::read()?)?
        {
            break;
        }
    }

    app.clear_published_selection();
    renderer.cleanup()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as TestCell;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use tempfile::tempdir;
    use throngterm::types::{ThoughtSource, ThoughtState, TransportHealth};

    const EXPECTED_TERMINAL_ENTRY: &str = concat!(
        "\u{1b}[?1049h",
        "\u{1b}[?1000h",
        "\u{1b}[?1002h",
        "\u{1b}[?1003h",
        "\u{1b}[?1015h",
        "\u{1b}[?1006h",
        "\u{1b}[?2004h",
        "\u{1b}[?25l",
        "\u{1b}[2J",
    );

    const EXPECTED_TERMINAL_TEARDOWN: &str = concat!(
        "\u{1b}[?2004l",
        "\u{1b}[?1006l",
        "\u{1b}[?1015l",
        "\u{1b}[?1003l",
        "\u{1b}[?1002l",
        "\u{1b}[?1000l",
        "\u{1b}[?1049l",
        "\u{1b}[?25h",
        "\u{1b}[0m",
    );

    #[derive(Default)]
    struct MockApiState {
        fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
        mermaid_artifact_results: VecDeque<Result<MermaidArtifactResponse, String>>,
        native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
        publish_selection_results: VecDeque<Result<(), String>>,
        open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
        list_dirs_results: VecDeque<Result<DirListResponse, String>>,
        create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
        publish_calls: Vec<Option<String>>,
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

        fn push_mermaid_artifact(&self, result: Result<MermaidArtifactResponse, String>) {
            self.state
                .lock()
                .unwrap()
                .mermaid_artifact_results
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

        fn publish_calls(&self) -> Vec<Option<String>> {
            self.state.lock().unwrap().publish_calls.clone()
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

        fn fetch_mermaid_artifact(
            &self,
            session_id: &str,
        ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
            let state = self.state.clone();
            let session_id = session_id.to_string();
            Box::pin(async move {
                state
                    .lock()
                    .unwrap()
                    .mermaid_artifact_results
                    .pop_front()
                    .unwrap_or_else(|| {
                        Ok(MermaidArtifactResponse {
                            session_id,
                            available: false,
                            path: None,
                            updated_at: None,
                            source: None,
                            error: None,
                        })
                    })
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

        fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
            let state = self.state.clone();
            let session_id = session_id.map(|value| value.to_string());
            Box::pin(async move {
                let mut state = state.lock().unwrap();
                state.publish_calls.push(session_id);
                state
                    .publish_selection_results
                    .pop_front()
                    .unwrap_or(Ok(()))
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

    const TEST_REPOS_ROOT: &str = "/tmp/repos";
    const TEST_REPO_ALPHA: &str = "/tmp/repos/alpha";
    const TEST_REPO_BETA: &str = "/tmp/repos/beta";
    const TEST_REPO_BUILDOOOR: &str = "/tmp/repos/buildooor";
    const TEST_REPO_DEV: &str = "/tmp/repos/dev";
    const TEST_REPO_GAMMA: &str = "/tmp/repos/gamma";
    const TEST_REPO_OPENSOURCE: &str = "/tmp/repos/opensource";
    const TEST_REPO_SKILLS: &str = "/tmp/repos/opensource/skills";
    const TEST_REPO_THRONGTERM: &str = "/tmp/repos/throngterm";

    #[derive(Default)]
    struct MockArtifactOpenerState {
        calls: Vec<String>,
        error: Option<String>,
    }

    #[derive(Clone, Default)]
    struct MockArtifactOpener {
        state: Arc<Mutex<MockArtifactOpenerState>>,
    }

    impl MockArtifactOpener {
        fn calls(&self) -> Vec<String> {
            self.state.lock().unwrap().calls.clone()
        }

        fn fail_with(&self, message: &str) {
            self.state.lock().unwrap().error = Some(message.to_string());
        }
    }

    impl ArtifactOpener for MockArtifactOpener {
        fn open(&self, path: &str) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(path.to_string());
            if let Some(message) = state.error.clone() {
                return Err(io::Error::other(message));
            }
            Ok(())
        }
    }

    fn make_app(api: MockApi) -> App<MockApi> {
        App::new(test_runtime(), api)
    }

    fn make_app_with_artifact_opener(
        api: MockApi,
        artifact_opener: Arc<dyn ArtifactOpener>,
    ) -> App<MockApi> {
        App::with_artifact_opener(test_runtime(), api, artifact_opener)
    }

    fn test_api_client(base_url: String, auth_token: Option<&str>) -> ApiClient {
        ApiClient {
            http: Client::builder()
                .connect_timeout(Duration::from_millis(50))
                .timeout(Duration::from_millis(100))
                .build()
                .expect("http client"),
            base_url,
            auth_token: auth_token.map(str::to_string),
        }
    }

    async fn spawn_guarded_startup_server(
        expected_token: &str,
        selection_status: axum::http::StatusCode,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::http::{HeaderMap, StatusCode};
        use axum::routing::{get, put};
        use axum::Router;

        let expected_sessions_auth = format!("Bearer {expected_token}");
        let expected_selection_auth = expected_sessions_auth.clone();

        let app = Router::new()
            .route(
                "/v1/sessions",
                get(move |headers: HeaderMap| {
                    let expected_auth = expected_sessions_auth.clone();
                    async move {
                        if headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            == Some(expected_auth.as_str())
                        {
                            StatusCode::OK
                        } else {
                            StatusCode::UNAUTHORIZED
                        }
                    }
                }),
            )
            .route(
                "/v1/selection",
                put(move |headers: HeaderMap| {
                    let expected_auth = expected_selection_auth.clone();
                    async move {
                        if headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            == Some(expected_auth.as_str())
                        {
                            selection_status
                        } else {
                            StatusCode::UNAUTHORIZED
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test api");
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn api_client_transport_errors_are_actionable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client = test_api_client(format!("http://127.0.0.1:{port}"), None);

        let error = client
            .fetch_sessions()
            .await
            .expect_err("closed localhost port should fail");
        assert!(error.contains("backend unavailable at"));
        assert!(error.contains("Start `throngterm` or set THRONGTERM_TUI_URL."));
        assert!(!error.contains("error sending request for url"));
    }

    async fn spawn_delayed_api_server(
        sessions_delay: Option<Duration>,
        native_open_delay: Option<Duration>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::routing::{get, post};
        use axum::{Json, Router};

        let app = Router::new()
            .route(
                "/v1/sessions",
                get(move || async move {
                    if let Some(delay) = sessions_delay {
                        tokio::time::sleep(delay).await;
                    }
                    Json(SessionListResponse {
                        sessions: vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
                        version: 1,
                        repo_themes: HashMap::new(),
                    })
                }),
            )
            .route(
                "/v1/native/open",
                post(move || async move {
                    if let Some(delay) = native_open_delay {
                        tokio::time::sleep(delay).await;
                    }
                    Json(NativeDesktopOpenResponse {
                        session_id: "sess-1".to_string(),
                        status: "focused".to_string(),
                        pane_id: Some("pane-1".to_string()),
                    })
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test api");
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn api_client_open_session_allows_slower_native_open_responses() {
        let (base_url, handle) =
            spawn_delayed_api_server(None, Some(Duration::from_millis(150))).await;
        let client = test_api_client(base_url, None);

        let response = client
            .open_session("sess-1")
            .await
            .expect("native open should outlive the default polling timeout");

        handle.abort();
        assert_eq!(response.session_id, "sess-1");
        assert_eq!(response.status, "focused");
        assert_eq!(response.pane_id.as_deref(), Some("pane-1"));
    }

    #[tokio::test]
    async fn api_client_fetch_sessions_keeps_short_timeout_for_refresh() {
        let (base_url, handle) =
            spawn_delayed_api_server(Some(Duration::from_millis(150)), None).await;
        let client = test_api_client(base_url.clone(), None);

        let error = client
            .fetch_sessions()
            .await
            .expect_err("refresh should keep the short polling timeout");

        handle.abort();
        assert!(error.contains(&base_url));
        assert!(error.contains("timed out while trying to refresh sessions"));
    }

    #[tokio::test]
    async fn startup_preflight_accepts_matching_bearer_token() {
        let (base_url, handle) =
            spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
        let client = test_api_client(base_url, Some("testtoken"));

        let result = client.preflight_startup_access().await;

        handle.abort();
        assert!(
            result.is_ok(),
            "matching token should pass startup preflight"
        );
    }

    #[tokio::test]
    async fn startup_preflight_requires_matching_auth_for_sessions() {
        let (base_url, handle) =
            spawn_guarded_startup_server("testtoken", axum::http::StatusCode::OK).await;
        let client = test_api_client(base_url.clone(), None);

        let error = client
            .preflight_startup_access()
            .await
            .expect_err("missing auth should fail startup preflight");

        handle.abort();
        assert!(error.contains(&base_url));
        assert!(error.contains("/v1/sessions"));
        assert!(error.contains("AUTH_MODE=token"));
        assert!(error.contains("AUTH_TOKEN"));
    }

    #[tokio::test]
    async fn startup_preflight_requires_selection_scope() {
        let (base_url, handle) =
            spawn_guarded_startup_server("testtoken", axum::http::StatusCode::FORBIDDEN).await;
        let client = test_api_client(base_url.clone(), Some("testtoken"));

        let error = client
            .preflight_startup_access()
            .await
            .expect_err("selection auth failure should fail startup preflight");

        handle.abort();
        assert!(error.contains(&base_url));
        assert!(error.contains("/v1/selection"));
        assert!(error.contains("required session scope"));
    }

    #[test]
    fn set_message_deduplicates_repeated_errors() {
        let api = MockApi::new();
        let mut app = make_app(api);
        app.set_message("backend unavailable");
        let first = app.message.as_ref().expect("message").1;

        std::thread::sleep(Duration::from_millis(5));
        app.set_message("backend unavailable");

        let second = app.message.as_ref().expect("message").1;
        assert_eq!(first, second);
    }

    #[test]
    fn auto_refresh_keeps_existing_footer_message() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![session_summary(
            "sess-7",
            "7",
            TEST_REPO_THRONGTERM,
        )]));
        let mut app = make_app(api);
        app.set_message("sticky status");

        app.refresh(layout);

        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("sticky status")
        );
    }

    #[test]
    fn manual_refresh_reports_session_count() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![
            session_summary("sess-7", "7", TEST_REPO_THRONGTERM),
            session_summary("sess-8", "8", TEST_REPO_OPENSOURCE),
        ]));
        let mut app = make_app(api);

        app.manual_refresh(layout);

        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("refreshed 2 sessions")
        );
    }

    fn test_renderer(width: u16, height: u16) -> Renderer {
        let buffer_size = (width as usize) * (height as usize);
        Renderer {
            stdout: BufWriter::new(io::stdout()),
            width,
            height,
            buffer: vec![Cell::default(); buffer_size],
            last_buffer: vec![Cell::default(); buffer_size],
            terminal_state: TerminalState::default(),
        }
    }

    #[test]
    fn enter_terminal_ui_enables_bracketed_paste_with_mouse_capture() {
        let mut output = Vec::new();

        enter_terminal_ui(&mut output).expect("enter terminal UI should write ANSI codes");

        assert_eq!(
            String::from_utf8(output).expect("terminal startup output should be valid utf-8"),
            EXPECTED_TERMINAL_ENTRY
        );
    }

    #[test]
    fn leave_terminal_ui_disables_bracketed_paste_before_leaving_alt_screen() {
        let mut output = Vec::new();

        leave_terminal_ui(&mut output).expect("leave terminal UI should write ANSI codes");

        assert_eq!(
            String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
            EXPECTED_TERMINAL_TEARDOWN
        );
    }

    #[test]
    fn cleanup_is_noop_when_renderer_is_inactive() {
        let mut renderer = test_renderer(80, 24);

        renderer.cleanup().expect("inactive cleanup should succeed");

        assert!(!renderer.terminal_state.raw_mode_enabled);
        assert!(!renderer.terminal_state.terminal_ui_active);
    }

    #[test]
    fn cleanup_after_runtime_error_restores_terminal_in_reverse_order() {
        let mut terminal_state = TerminalState::default();
        let mut output = Vec::new();
        let events = Arc::new(Mutex::new(Vec::new()));

        terminal_state
            .init_with(
                &mut output,
                {
                    let events = Arc::clone(&events);
                    move || {
                        events.lock().unwrap().push("enable_raw_mode");
                        Ok(())
                    }
                },
                {
                    let events = Arc::clone(&events);
                    move |_writer| {
                        events.lock().unwrap().push("enter_terminal_ui");
                        Ok(())
                    }
                },
            )
            .expect("terminal init should succeed");

        terminal_state
            .cleanup_with(
                &mut output,
                {
                    let events = Arc::clone(&events);
                    move |writer| {
                        events.lock().unwrap().push("leave_terminal_ui");
                        leave_terminal_ui(writer)
                    }
                },
                {
                    let events = Arc::clone(&events);
                    move || {
                        events.lock().unwrap().push("disable_raw_mode");
                        Ok(())
                    }
                },
            )
            .expect("cleanup should succeed after a runtime error");

        assert_eq!(
            String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
            EXPECTED_TERMINAL_TEARDOWN
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [
                "enable_raw_mode",
                "enter_terminal_ui",
                "leave_terminal_ui",
                "disable_raw_mode",
            ]
        );
    }

    #[test]
    fn failed_init_still_runs_full_cleanup_once() {
        let mut terminal_state = TerminalState::default();
        let mut output = Vec::new();
        let leave_calls = TestCell::new(0usize);
        let disable_calls = TestCell::new(0usize);

        let err = terminal_state
            .init_with(
                &mut output,
                || Ok(()),
                |_writer| Err(io::Error::other("forced init failure")),
            )
            .expect_err("init should surface the forced failure");
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "forced init failure");

        terminal_state
            .cleanup_with(
                &mut output,
                |writer| {
                    leave_calls.set(leave_calls.get() + 1);
                    leave_terminal_ui(writer)
                },
                || {
                    disable_calls.set(disable_calls.get() + 1);
                    Ok(())
                },
            )
            .expect("cleanup should restore the terminal after init failure");

        terminal_state
            .cleanup_with(
                &mut output,
                |writer| {
                    leave_calls.set(leave_calls.get() + 1);
                    leave_terminal_ui(writer)
                },
                || {
                    disable_calls.set(disable_calls.get() + 1);
                    Ok(())
                },
            )
            .expect("second cleanup should be a no-op");

        assert_eq!(
            String::from_utf8(output).expect("terminal teardown output should be valid utf-8"),
            EXPECTED_TERMINAL_TEARDOWN
        );
        assert_eq!(leave_calls.get(), 1);
        assert_eq!(disable_calls.get(), 1);
        assert!(!terminal_state.raw_mode_enabled);
        assert!(!terminal_state.terminal_ui_active);
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

    fn find_text_position(renderer: &Renderer, needle: &str) -> Option<(u16, u16)> {
        for y in 0..renderer.height {
            let row = row_text(renderer, y);
            if let Some(byte_index) = row.find(needle) {
                let char_index = row[..byte_index].chars().count() as u16;
                return Some((char_index, y));
            }
        }
        None
    }

    fn open_mermaid_test_viewer(
        source: &str,
        width: u16,
        height: u16,
    ) -> (App<MockApi>, Renderer, WorkspaceLayout) {
        let api = MockApi::new();
        let layout = test_layout(width, height);
        let mut app = make_app(api);
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                source,
            ),
        );
        app.open_mermaid_viewer("sess-1".to_string());
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.unsupported_reason = None;
        (app, test_renderer(width, height), layout)
    }

    fn find_cached_semantic_line(viewer: &MermaidViewerState, needle: &str) -> Option<(u16, u16)> {
        viewer
            .cached_semantic_lines
            .iter()
            .find(|line| line.text == needle)
            .map(|line| (line.x, line.y))
    }

    fn cached_semantic_texts(viewer: &MermaidViewerState) -> Vec<String> {
        viewer
            .cached_semantic_lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
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

    fn mermaid_artifact(
        session_id: &str,
        path: &str,
        updated_at: &str,
        source: &str,
    ) -> MermaidArtifactResponse {
        MermaidArtifactResponse {
            session_id: session_id.to_string(),
            available: true,
            path: Some(path.to_string()),
            updated_at: Some(timestamp(updated_at)),
            source: Some(source.to_string()),
            error: None,
        }
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

    fn deep_sleep_session(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        last_activity_at: &str,
    ) -> SessionSummary {
        let mut session = session_summary(session_id, tmux_name, cwd);
        session.thought_state = ThoughtState::Sleeping;
        session.rest_state = RestState::DeepSleep;
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

    fn color_rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb { r, g, b } => (r, g, b),
            other => panic!("expected rgb color, got {other:?}"),
        }
    }

    fn assert_dark_terminal_readable(color: Color) {
        assert!(
            contrast_ratio(color_rgb(color), DARK_TERMINAL_BG_RGB) >= MIN_DARK_TERMINAL_CONTRAST,
            "expected {color:?} to satisfy the dark-terminal contrast threshold"
        );
    }

    fn create_response(session_id: &str, tmux_name: &str, cwd: &str) -> CreateSessionResponse {
        CreateSessionResponse {
            session: session_summary(session_id, tmux_name, cwd),
            repo_theme: None,
        }
    }

    fn create_response_with_theme(
        session: SessionSummary,
        repo_theme: RepoTheme,
    ) -> CreateSessionResponse {
        CreateSessionResponse {
            session,
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
        entity.swim_anchor_x = entity.x;
        entity.swim_anchor_y = entity.y;
        entity.swim_center_y = entity.y;
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
        let (x, y) = bottom_rest_origin(field, slot);
        Rect {
            x: field.x + x,
            y: field.y + y,
            width: ENTITY_WIDTH,
            height: ENTITY_HEIGHT,
        }
    }

    fn deep_sleep_grid_rect(field: Rect, slot: usize) -> Rect {
        let (x, y) = top_rest_origin(field, slot);
        Rect {
            x: field.x + x,
            y: field.y + y,
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
                TEST_REPO_BETA,
                "indexing repo",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-1",
                "alpha",
                TEST_REPO_ALPHA,
                "writing tests",
                "2026-03-08T14:00:06Z",
            ),
        ]));
        api.push_fetch_sessions(Ok(vec![
            session_summary_with_thought(
                "sess-2",
                "beta",
                TEST_REPO_BETA,
                "indexing repo",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-1",
                "alpha",
                TEST_REPO_ALPHA,
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
            TEST_REPO_GAMMA,
            "reading logs",
            "2026-03-08T14:00:05Z",
        )]));

        let mut duplicate = session_summary_with_thought(
            "sess-3",
            "gamma",
            TEST_REPO_GAMMA,
            "reading logs",
            "2026-03-08T14:00:05Z",
        );
        let mut stale = session_summary_with_thought(
            "sess-3",
            "gamma",
            TEST_REPO_GAMMA,
            "reading logs",
            "2026-03-08T14:00:04Z",
        );
        let mut cleared = session_summary("sess-3", "gamma", TEST_REPO_GAMMA);
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
                session_summary("sess-1", "alpha", TEST_REPO_ALPHA),
                session_summary("sess-2", "beta", TEST_REPO_BETA),
            ],
            layout.overview_field,
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "alpha",
                TEST_REPO_ALPHA,
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
                TEST_REPO_ALPHA,
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
                TEST_REPO_THRONGTERM,
                "patching tui",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-2",
                "9",
                TEST_REPO_SKILLS,
                "indexing docs",
                "2026-03-08T14:00:06Z",
            ),
        ]));
        api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_SKILLS,
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
            TEST_REPO_THRONGTERM,
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        first.repo_theme_id = Some(throngterm_theme_id.clone());

        let mut second = session_summary_with_thought(
            "sess-2",
            "2",
            TEST_REPO_THRONGTERM,
            "wiring filter state",
            "2026-03-08T14:00:06Z",
        );
        second.repo_theme_id = Some(throngterm_theme_id);

        let mut third = session_summary_with_thought(
            "sess-3",
            "9",
            TEST_REPO_SKILLS,
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
                TEST_REPO_THRONGTERM,
                "patching tui",
                "2026-03-08T14:00:05Z",
            )],
            test_layout(120, 32).thought_entry_capacity(),
        );
        app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());

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
                TEST_REPO_THRONGTERM.to_string()
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
        let mut app = make_app(api.clone());

        app.repo_themes
            .insert("/tmp/throngterm".to_string(), repo_theme("#B89875"));
        app.repo_themes
            .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

        let mut first = session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
            "patching tui",
            "2026-03-08T14:00:05Z",
        );
        first.repo_theme_id = Some("/tmp/throngterm".to_string());

        let mut second = session_summary_with_thought(
            "sess-2",
            "2",
            TEST_REPO_THRONGTERM,
            "wiring filter state",
            "2026-03-08T14:00:06Z",
        );
        second.repo_theme_id = Some("/tmp/throngterm".to_string());

        let mut third = session_summary_with_thought(
            "sess-3",
            "9",
            TEST_REPO_SKILLS,
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
            Some(TEST_REPO_THRONGTERM)
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
            .text_rect
            .expect("row should have a click target");
        app.selected_id = Some("sess-3".to_string());
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-2".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        app.handle_thought_click(
            row_rect.x.saturating_add(4),
            row_start_y + row_index as u16,
            thought_content,
            layout.thought_entry_capacity(),
        );

        assert_eq!(
            app.thought_filter.cwd.as_deref(),
            Some(TEST_REPO_THRONGTERM)
        );
        assert_eq!(app.thought_filter.tmux_name, None);
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
        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
        assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("focused 2")
        );

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
    fn clicking_thought_body_opens_that_session() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "7",
                TEST_REPO_THRONGTERM,
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

        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-1".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        app.handle_thought_click(
            body_x,
            row_start_y,
            thought_content,
            layout.thought_entry_capacity(),
        );

        assert_eq!(app.thought_filter.tmux_name, None);
        assert_eq!(app.active_thought_filter_text(), "filter: none");
        assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
        assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("focused 7")
        );
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
                    TEST_REPO_THRONGTERM,
                    "older",
                    "2026-03-08T14:00:05Z",
                ),
                session_summary_with_thought(
                    "sess-2",
                    "9",
                    TEST_REPO_THRONGTERM,
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
    fn clicking_wrapped_thought_line_opens_that_session() {
        let api = MockApi::new();
        let mut app = make_app(api.clone());
        let thought_content = Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 5,
        };
        app.merge_sessions(
            vec![session_summary("sess-2", "9", TEST_REPO_THRONGTERM)],
            test_field(),
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-2",
                "9",
                TEST_REPO_THRONGTERM,
                "latest thought stays at bottom",
                "2026-03-08T14:00:06Z",
            )],
            4,
        );

        let panel = build_thought_panel(&app, thought_content, 4);
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);

        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-2".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        app.handle_thought_click(1, row_start_y + 3, thought_content, 4);

        assert_eq!(app.thought_filter.tmux_name, None);
        assert_eq!(app.active_thought_filter_text(), "filter: none");
        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
        assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("focused 9")
        );
    }

    #[test]
    fn clicking_thought_row_surfaces_native_open_errors() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "7",
                TEST_REPO_THRONGTERM,
                "patching tui",
                "2026-03-08T14:00:05Z",
            )],
            layout.thought_entry_capacity(),
        );

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let row_start_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);
        let body_x = thought_content
            .x
            .saturating_add(display_width("7").saturating_add(3));

        api.push_open_session(Err("native open unavailable".to_string()));
        app.handle_thought_click(
            body_x,
            row_start_y,
            thought_content,
            layout.thought_entry_capacity(),
        );

        assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
        assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("native open unavailable")
        );
        assert_eq!(app.active_thought_filter_text(), "filter: none");
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
            TEST_REPO_ALPHA,
            "indexing repo",
            "2026-03-08T14:00:05Z",
        );
        busy.state = SessionState::Busy;
        busy.repo_theme_id = Some(theme_id.clone());

        let mut attention = session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
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
    fn low_contrast_repo_theme_color_is_adjusted_in_thought_history_and_header() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api);
        let theme_id = "/tmp/skills".to_string();
        let raw_color = rgb_color((0x39, 0x30, 0xB5));
        let expected = repo_theme_display_color("#3930B5").expect("display color");
        app.repo_themes
            .insert(theme_id.clone(), repo_theme("#3930B5"));

        let mut session = session_summary_with_thought(
            "sess-1",
            "9",
            TEST_REPO_SKILLS,
            "indexing docs",
            "2026-03-08T14:00:07Z",
        );
        session.state = SessionState::Busy;
        session.repo_theme_id = Some(theme_id);

        app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
        app.merge_sessions(vec![session], layout.overview_field);

        assert_ne!(expected, raw_color);
        assert_dark_terminal_readable(expected);

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        assert_eq!(panel.rows.len(), 1);
        assert_eq!(panel.rows[0].color, expected);

        let header = build_header_filter_layout(&app, 120);
        let chip = header
            .chips
            .iter()
            .find(|chip| chip.label == "1xskills")
            .expect("skills chip should exist");
        assert_eq!(chip.color, expected);

        let mut renderer = test_renderer(120, 32);
        render_thought_panel(
            &app,
            &mut renderer,
            thought_content,
            layout.thought_entry_capacity(),
        );
        assert_eq!(
            cell_at(
                &renderer,
                thought_content.x,
                thought_content.bottom().saturating_sub(1)
            )
            .fg,
            expected
        );

        render_header_filter_strip(&app, &mut renderer, 120);
        assert_eq!(cell_at(&renderer, chip.rect.x, chip.rect.y).fg, expected);
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
            TEST_REPO_THRONGTERM,
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
        let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
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
    fn render_entity_adjusts_low_contrast_repo_theme_color() {
        let field = test_layout(120, 32).overview_field;
        let mut session = session_summary("sess-1", "alpha", TEST_REPO_SKILLS);
        session.state = SessionState::Busy;
        session.repo_theme_id = Some("/tmp/skills".to_string());
        let entity = SessionEntity::new(session, field);
        let mut repo_themes = HashMap::new();
        repo_themes.insert("/tmp/skills".to_string(), repo_theme("#3930B5"));
        let rect = entity.screen_rect(field);
        let mut renderer = test_renderer(120, 32);
        let expected = session_display_color(&entity.session, &repo_themes);

        render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

        assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
        assert_dark_terminal_readable(expected);
        assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, expected);
    }

    #[test]
    fn selected_entity_preserves_repo_theme_body_color() {
        let field = test_layout(120, 32).overview_field;
        let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
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
        let mut session = session_summary("sess-1", "alpha", TEST_REPO_THRONGTERM);
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
        let mut spawned_session = session_summary("sess-42", "42", TEST_REPO_THRONGTERM);
        spawned_session.repo_theme_id = Some(theme_id.clone());
        api.push_create_session(Ok(create_response_with_theme(
            spawned_session.clone(),
            repo_theme("#B89875"),
        )));
        let mut app = make_app(api);

        app.spawn_session(TEST_REPO_THRONGTERM, None, field);

        let mut thought_session = session_summary_with_thought(
            "sess-42",
            "42",
            TEST_REPO_THRONGTERM,
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
    fn sleeping_entity_pins_to_bottom_left_grid_slot() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![sleeping_session(
                "sess-sleep-1",
                "7",
                TEST_REPO_THRONGTERM,
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
    fn attention_sleeping_entity_pins_to_bottom_left_grid_slot() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![attention_session(
                "sess-attn-sleep-1",
                "7",
                TEST_REPO_THRONGTERM,
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
        assert_eq!(entity.rest_anchor(), RestAnchor::Bottom);
        assert_eq!(
            entity_rect_for(&app, "sess-attn-sleep-1", field),
            sleep_grid_rect(field, 0)
        );
    }

    #[test]
    fn deep_sleep_entity_floats_to_top_left_grid_slot() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![deep_sleep_session(
                "sess-deep-1",
                "7",
                TEST_REPO_THRONGTERM,
                "2026-03-08T12:00:00Z",
            )],
            field,
        );

        let entity = app
            .entities
            .iter()
            .find(|entity| entity.session.session_id == "sess-deep-1")
            .expect("entity should exist");
        assert_eq!(entity.rest_anchor(), RestAnchor::Top);
        assert_eq!(
            entity_rect_for(&app, "sess-deep-1", field),
            deep_sleep_grid_rect(field, 0)
        );
    }

    #[test]
    fn attention_session_state_text_uses_rest_state() {
        let active = attention_session(
            "sess-attn-active",
            "7",
            TEST_REPO_THRONGTERM,
            RestState::Active,
            "2026-03-08T12:40:00Z",
        );
        let drowsy = attention_session(
            "sess-attn-drowsy",
            "8",
            TEST_REPO_THRONGTERM,
            RestState::Drowsy,
            "2026-03-08T12:20:00Z",
        );
        let sleeping = attention_session(
            "sess-attn-sleep",
            "9",
            TEST_REPO_THRONGTERM,
            RestState::Sleeping,
            "2026-03-08T12:00:00Z",
        );
        let deep_sleep = attention_session(
            "sess-attn-deep",
            "10",
            TEST_REPO_THRONGTERM,
            RestState::DeepSleep,
            "2026-03-08T11:00:00Z",
        );

        assert_eq!(session_state_text(&active), "attention");
        assert_eq!(session_state_text(&drowsy), "drowsy");
        assert_eq!(session_state_text(&sleeping), "sleeping");
        assert_eq!(session_state_text(&deep_sleep), "deep sleep");
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
    fn picker_theme_color_for_path_keeps_stored_theme_body_while_adjusting_display_color() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("skills");
        fs::create_dir_all(repo_root.join("src")).expect("create repo");
        write_repo_theme_file(&repo_root, "#3930B5");
        let colors_path = repo_root.join(".throngterm").join("colors.json");
        let original = fs::read_to_string(&colors_path).expect("read colors.json");
        let theme_id = repo_root.to_string_lossy().into_owned();
        let mut repo_themes = HashMap::new();

        let color = picker_theme_color_for_path(theme_id.as_str(), &mut repo_themes)
            .expect("theme color should resolve");

        assert_ne!(color, rgb_color((0x39, 0x30, 0xB5)));
        assert_dark_terminal_readable(color);
        assert_eq!(
            repo_themes
                .get(theme_id.as_str())
                .expect("theme should be cached")
                .body,
            "#3930B5"
        );
        assert_eq!(
            fs::read_to_string(colors_path).expect("reread colors.json"),
            original
        );
    }

    #[test]
    fn render_picker_adjusts_low_contrast_repo_theme_color() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path().join("skills");
        fs::create_dir_all(repo_root.join("src")).expect("create repo");
        write_repo_theme_file(&repo_root, "#3930B5");

        let mut picker = PickerState::new(
            2,
            2,
            dir_response(repo_root.to_string_lossy().as_ref(), &[("src", true)]),
            true,
        );
        let mut repo_themes = HashMap::new();
        picker.sync_theme_colors(&mut repo_themes);

        let expected = picker.current_theme_color.expect("current theme color");
        let field = test_field();
        let layout = picker_layout(&picker, field);
        let mut renderer = test_renderer(100, 30);

        render_picker(&mut renderer, &picker, field);

        assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
        assert_dark_terminal_readable(expected);
        assert_eq!(picker.entry_theme_colors, vec![Some(expected)]);
        assert_eq!(
            cell_at(&renderer, layout.frame.x, layout.frame.y).fg,
            expected
        );
        assert_eq!(
            cell_at(&renderer, layout.content.x, layout.content.y + 1).fg,
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
        assert_eq!(
            cell_at(&renderer, layout.content.x, layout.first_entry_y).fg,
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
    fn sleeping_entities_fill_bottom_row_by_sleepiness() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![
                sleeping_session(
                    "sess-new",
                    "8",
                    TEST_REPO_THRONGTERM,
                    "2026-03-08T12:20:00Z",
                ),
                sleeping_session(
                    "sess-mid",
                    "7",
                    TEST_REPO_THRONGTERM,
                    "2026-03-08T12:10:00Z",
                ),
                sleeping_session(
                    "sess-old",
                    "9",
                    TEST_REPO_THRONGTERM,
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
                sleeping_session("sess-b", "8", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
                sleeping_session("sess-a", "7", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
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
        app.entities
            .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

        app.merge_sessions(
            vec![sleeping_session(
                "sess-1",
                "dev",
                TEST_REPO_DEV,
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
                sleeping_session("sess-a", "7", TEST_REPO_THRONGTERM, "2026-03-08T12:00:00Z"),
                sleeping_session("sess-b", "8", TEST_REPO_THRONGTERM, "2026-03-08T12:10:00Z"),
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
    fn drowsy_sprite_uses_fish_motion_profile() {
        assert_eq!(SpriteKind::Drowsy.speed_scale(), 0.5);
        assert!(drowsy_frame(0)[1].contains("><-"));
    }

    #[test]
    fn drowsy_entities_bob_in_place_after_tick() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
        entity.session.thought_state = ThoughtState::Holding;
        entity.session.rest_state = RestState::Drowsy;
        entity.bob_phase = 0.0;
        entity.vx = 1.0;
        entity.vy = 0.0;
        app.entities.push(entity);

        for _ in 0..16 {
            app.tick(field);
        }

        let rect = entity_rect_for(&app, "sess-1", field);
        assert_eq!(rect.x, 30);
        assert_ne!(rect.y, 8);
        assert!((rect.y as i32 - 8).abs() <= 3);
    }

    #[test]
    fn deep_sleep_entities_stay_fixed_after_tick() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);

        app.merge_sessions(
            vec![
                deep_sleep_session(
                    "sess-deep-a",
                    "7",
                    TEST_REPO_THRONGTERM,
                    "2026-03-08T12:00:00Z",
                ),
                deep_sleep_session(
                    "sess-deep-b",
                    "8",
                    TEST_REPO_THRONGTERM,
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
            entity_rect_for(&app, "sess-deep-a", field),
            deep_sleep_grid_rect(field, 0)
        );
        assert_eq!(
            entity_rect_for(&app, "sess-deep-b", field),
            deep_sleep_grid_rect(field, 1)
        );
    }

    #[test]
    fn active_entities_swim_in_place_with_bob() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
        entity.session.thought_state = ThoughtState::Active;
        entity.session.rest_state = RestState::Active;
        entity.bob_phase = 0.0;
        entity.vx = 1.0;
        entity.vy = 0.0;
        app.entities.push(entity);

        for _ in 0..16 {
            app.tick(field);
        }

        let moved = app
            .entities
            .iter()
            .find(|entity| entity.session.session_id == "sess-1")
            .expect("entity should exist");
        assert_eq!(moved.screen_rect(field).x, 30);
        assert_ne!(moved.screen_rect(field).y, 8);
        assert!((moved.screen_rect(field).y as i32 - 8).abs() <= 3);
    }

    #[test]
    fn busy_entities_hold_horizontal_position() {
        let api = MockApi::new();
        let field = test_field();
        let mut app = make_app(api);
        let mut entity = entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8);
        entity.session.state = SessionState::Busy;
        entity.bob_phase = 0.0;
        entity.vx = 1.0;
        entity.vy = 0.0;
        app.entities.push(entity);

        for _ in 0..16 {
            app.tick(field);
        }

        let rect = entity_rect_for(&app, "sess-1", field);
        assert_eq!(rect.x, 30);
        assert_ne!(rect.y, 8);
        assert!((rect.y as i32 - 8).abs() <= 3);
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
            TEST_REPOS_ROOT,
            &[("opensource", true), ("throngterm", true)],
        )));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.entities
            .push(entity_at(field, "sess-1", "dev", TEST_REPO_DEV, 30, 8));

        app.handle_field_click(10, 10, field);

        let picker = app.picker.as_ref().expect("picker should open");
        assert!(picker.managed_only);
        assert_eq!(picker.base_path, TEST_REPOS_ROOT);
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
        api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
        api.push_list_dirs(Ok(dir_response(TEST_REPO_OPENSOURCE, &[("skills", false)])));

        let field = test_field();
        let mut app = make_app(api.clone());

        app.handle_field_click(10, 10, field);
        app.activate_picker_entry(0, field);
        app.activate_picker_entry(0, field);

        assert_eq!(
            api.list_calls(),
            vec![(None, true), (Some(TEST_REPO_OPENSOURCE.to_string()), true),]
        );
        assert_eq!(
            api.create_calls(),
            Vec::<(String, SpawnTool, Option<String>)>::new()
        );
        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some(TEST_REPO_SKILLS)
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
            dir_response(TEST_REPO_OPENSOURCE, &[("skills", true)]),
            true,
        ));

        app.spawn_session_from_picker(field);

        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some(TEST_REPO_OPENSOURCE)
        );
    }

    #[test]
    fn toggling_to_all_reloads_same_path_without_reordering() {
        let api = MockApi::new();
        api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("opensource", true)])));
        api.push_list_dirs(Ok(dir_response(
            TEST_REPOS_ROOT,
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
            vec![(None, true), (Some(TEST_REPOS_ROOT.to_string()), false),]
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
        api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            10,
            10,
            dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: "add hidden spawn flow".to_string(),
        });

        app.submit_initial_request(field);

        assert_eq!(
            api.create_calls(),
            vec![(
                TEST_REPO_THRONGTERM.to_string(),
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
    fn pasting_initial_request_buffers_multiline_without_submitting() {
        let api = MockApi::new();
        let mut app = make_app(api.clone());
        let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: String::new(),
        });

        app.handle_paste(pasted);

        assert_eq!(
            app.initial_request
                .as_ref()
                .map(|state| state.value.as_str()),
            Some(pasted)
        );
        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
    }

    #[test]
    fn pressing_enter_after_pasting_initial_request_submits_once() {
        let api = MockApi::new();
        api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
        let field = test_field();
        let mut app = make_app(api.clone());
        let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: String::new(),
        });

        app.handle_paste(pasted);
        app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

        assert_eq!(
            api.create_calls(),
            vec![(
                TEST_REPO_THRONGTERM.to_string(),
                SpawnTool::Codex,
                Some(pasted.to_string()),
            )]
        );
        assert!(api.open_calls().is_empty());
        assert!(app.initial_request.is_none());
        assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
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
            dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: "fix tmux startup".to_string(),
        });

        app.submit_initial_request(field);

        assert_eq!(
            api.create_calls(),
            vec![(
                TEST_REPO_THRONGTERM.to_string(),
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
            cwd: TEST_REPO_THRONGTERM.to_string(),
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
    fn typing_initial_request_and_pressing_enter_still_creates_hidden_session() {
        let api = MockApi::new();
        api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_THRONGTERM)));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: String::new(),
        });

        for ch in "add hidden spawn flow".chars() {
            app.handle_initial_request_key(
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                field,
            );
        }
        app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);

        assert_eq!(
            api.create_calls(),
            vec![(
                TEST_REPO_THRONGTERM.to_string(),
                SpawnTool::Codex,
                Some("add hidden spawn flow".to_string()),
            )]
        );
        assert!(api.open_calls().is_empty());
        assert!(app.initial_request.is_none());
        assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("created 55")
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
            dir_response(TEST_REPOS_ROOT, &[("throngterm", false)]),
            true,
        ));
        app.initial_request = Some(InitialRequestState {
            cwd: TEST_REPO_THRONGTERM.to_string(),
            value: "investigate snapshot restore".to_string(),
        });

        app.handle_initial_request_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), field);

        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert!(app.initial_request.is_none());
        assert!(app.picker.is_some());
    }

    #[test]
    fn paste_outside_initial_request_is_ignored() {
        let api = MockApi::new();
        let mut app = make_app(api.clone());
        app.selected_id = Some("sess-7".to_string());

        app.handle_paste("q\n### TC-7\n- Then: shell spill");

        assert_eq!(app.selected_id.as_deref(), Some("sess-7"));
        assert!(api.create_calls().is_empty());
        assert!(api.open_calls().is_empty());
        assert!(app.initial_request.is_none());
        assert!(app.picker.is_none());
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
        app.entities
            .push(entity_at(field, "sess-7", "dev", TEST_REPO_DEV, 30, 8));
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
        api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("throngterm", true)])));
        let field = test_field();
        let mut app = make_app(api.clone());
        app.entities
            .push(entity_at(field, "sess-1", "2", TEST_REPO_THRONGTERM, 12, 6));
        app.entities
            .push(entity_at(field, "sess-3", "9", TEST_REPO_SKILLS, 30, 8));
        app.selected_id = Some("sess-3".to_string());

        app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());
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
        api.push_fetch_sessions(Ok(vec![session_summary("sess-3", "9", TEST_REPO_SKILLS)]));
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![
                session_summary("sess-1", "7", TEST_REPO_THRONGTERM),
                session_summary("sess-2", "2", TEST_REPO_THRONGTERM),
            ],
            layout.overview_field,
        );
        app.selected_id = Some("sess-1".to_string());
        app.set_thought_filter_cwd(TEST_REPO_THRONGTERM.to_string());

        app.refresh(layout);

        assert!(app.visible_entities().is_empty());
        assert!(app.selected_id.is_none());
        assert_eq!(
            api.publish_calls(),
            vec![Some("sess-2".to_string()), Some("sess-1".to_string()), None,]
        );

        app.open_selected();

        assert!(api.open_calls().is_empty());
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("no session selected")
        );
    }

    #[test]
    fn refresh_publishes_selected_session_for_external_dispatch() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        api.push_fetch_sessions(Ok(vec![session_summary(
            "sess-throngterm",
            "7",
            TEST_REPO_THRONGTERM,
        )]));
        let mut app = make_app(api.clone());

        app.refresh(layout);

        assert_eq!(app.selected_id.as_deref(), Some("sess-throngterm"));
        assert_eq!(
            api.publish_calls(),
            vec![Some("sess-throngterm".to_string())]
        );
    }

    #[test]
    fn picker_action_at_resolves_controls_and_entries() {
        let mut picker = PickerState::new(
            4,
            4,
            dir_response("/tmp", &[("alpha", true), ("beta", false)]),
            true,
        );
        picker.apply_response(dir_response("/tmp/nested", &[("child", false)]));
        let layout = picker_layout(&picker, test_field());

        assert!(matches!(
            picker_action_at(
                &picker,
                layout,
                layout.close_button.x,
                layout.close_button.y
            ),
            Some(PickerAction::Close)
        ));
        assert!(matches!(
            picker_action_at(&picker, layout, layout.env_button.x, layout.env_button.y),
            Some(PickerAction::ToggleManaged(true))
        ));
        assert!(matches!(
            picker_action_at(&picker, layout, layout.all_button.x, layout.all_button.y),
            Some(PickerAction::ToggleManaged(false))
        ));
        assert!(matches!(
            picker_action_at(
                &picker,
                layout,
                layout.spawn_here_button.x,
                layout.spawn_here_button.y
            ),
            Some(PickerAction::ActivateCurrentPath)
        ));
        assert!(matches!(
            picker_action_at(&picker, layout, layout.content.x, layout.first_entry_y),
            Some(PickerAction::ActivateEntry(0))
        ));
        assert!(matches!(
            picker_action_at(
                &picker,
                layout,
                layout.content.right(),
                layout.first_entry_y
            ),
            None
        ));
        assert!(matches!(
            layout
                .back_button
                .and_then(|button| picker_action_at(&picker, layout, button.x, button.y)),
            Some(PickerAction::Up)
        ));
    }

    #[test]
    fn renderer_flush_copies_drawn_cells_into_last_buffer() {
        let mut renderer = test_renderer(4, 2);
        renderer.draw_char(0, 0, 'A', Color::Green);
        renderer.draw_char(1, 0, 'B', Color::Yellow);

        renderer.flush().expect("flush should succeed");

        assert!(renderer
            .buffer
            .iter()
            .zip(renderer.last_buffer.iter())
            .all(|(current, previous)| current == previous));
    }

    #[test]
    fn move_selection_updates_picker_and_visible_session_selection() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![
                session_summary("sess-1", "1", TEST_REPO_ALPHA),
                session_summary("sess-2", "2", TEST_REPO_BETA),
            ],
            layout.overview_field,
        );

        app.move_selection(1, layout.overview_field);
        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

        let mut picker = PickerState::new(
            3,
            3,
            dir_response("/tmp", &[("alpha", false), ("beta", false)]),
            true,
        );
        picker.selection = PickerSelection::SpawnHere;
        app.picker = Some(picker);

        app.move_selection(1, layout.overview_field);

        assert!(matches!(
            app.picker.as_ref().map(|picker| picker.selection),
            Some(PickerSelection::Entry(0))
        ));
    }

    #[test]
    fn handle_key_event_covers_initial_request_picker_and_quit_paths() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![
                session_summary("sess-1", "1", TEST_REPO_ALPHA),
                session_summary("sess-2", "2", TEST_REPO_BETA),
            ],
            layout.overview_field,
        );

        app.open_initial_request("/tmp/project".to_string());
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        ));
        assert_eq!(
            app.initial_request
                .as_ref()
                .map(|state| state.value.as_str()),
            Some("x")
        );

        app.close_initial_request();
        app.picker = Some(PickerState::new(
            3,
            3,
            dir_response("/tmp", &[("alpha", false)]),
            true,
        ));
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
        assert!(app.picker.is_none());

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        ));
        assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

        assert!(!handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ));
    }

    #[test]
    fn picker_activate_selection_opens_initial_request_and_reloads_children() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api.clone());
        app.picker = Some(PickerState::new(
            2,
            2,
            dir_response("/tmp", &[("child", true), ("leaf", false)]),
            true,
        ));

        app.picker_activate_selection(layout.overview_field);
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some("/tmp")
        );

        app.close_initial_request();
        if let Some(picker) = &mut app.picker {
            picker.selection = PickerSelection::Entry(0);
        }
        api.push_list_dirs(Ok(dir_response("/tmp/child", &[("nested", false)])));
        app.picker_activate_selection(layout.overview_field);
        assert_eq!(
            api.list_calls(),
            vec![(Some("/tmp/child".to_string()), true)]
        );

        if let Some(picker) = &mut app.picker {
            picker.apply_response(dir_response("/tmp", &[("leaf", false)]));
            picker.selection = PickerSelection::Entry(0);
        }
        app.picker_activate_selection(layout.overview_field);
        assert_eq!(
            app.initial_request.as_ref().map(|state| state.cwd.as_str()),
            Some("/tmp/leaf")
        );
    }

    #[test]
    fn handle_workspace_click_routes_thought_and_overview_interactions() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        let mut app = make_app(api.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.capture_thought_updates(
            &[session_summary_with_thought(
                "sess-1",
                "7",
                TEST_REPO_THRONGTERM,
                "patching tui",
                "2026-03-08T14:00:05Z",
            )],
            layout.thought_entry_capacity(),
        );

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        let row_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);
        let body_x = thought_content
            .x
            .saturating_add(display_width("7").saturating_add(3));
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-1".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        handle_workspace_click(
            &mut app,
            layout,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: body_x,
                row: row_y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.thought_filter.tmux_name, None);
        assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
        assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
        assert_eq!(
            app.message.as_ref().map(|(message, _)| message.as_str()),
            Some("focused 7")
        );

        let entity_rect = entity_rect_for(&app, "sess-1", layout.overview_field);
        api.push_open_session(Ok(NativeDesktopOpenResponse {
            session_id: "sess-1".to_string(),
            status: "focused".to_string(),
            pane_id: None,
        }));
        handle_workspace_click(
            &mut app,
            layout,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: entity_rect.x,
                row: entity_rect.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
        assert_eq!(
            api.open_calls(),
            vec!["sess-1".to_string(), "sess-1".to_string()]
        );
    }

    #[test]
    fn refresh_builds_synthetic_mermaid_row_and_preserves_text_click_behavior() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let thought_content = layout
            .thought_content
            .expect("wide layout enables thought rail");
        api.push_fetch_sessions(Ok(vec![session_summary(
            "sess-1",
            "7",
            TEST_REPO_THRONGTERM,
        )]));
        api.push_mermaid_artifact(Ok(mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        )));
        let mut app = make_app(api);

        app.refresh(layout);

        let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
        assert_eq!(panel.rows.len(), 1);
        assert_eq!(panel.rows[0].line, "7: mermaid diagram ready");
        let mermaid_rect = panel.rows[0].mermaid_rect.expect("mermaid button");
        let text_rect = panel.rows[0].text_rect.expect("synthetic row text");
        let row_y = thought_content
            .bottom()
            .saturating_sub(panel.rows.len() as u16);

        assert_eq!(
            thought_panel_action_at(
                &app,
                thought_content,
                layout.thought_entry_capacity(),
                mermaid_rect.x,
                row_y,
            ),
            Some(ThoughtPanelAction::OpenMermaid("sess-1".to_string()))
        );
        assert_eq!(
            thought_panel_action_at(
                &app,
                thought_content,
                layout.thought_entry_capacity(),
                text_rect.x,
                row_y,
            ),
            Some(ThoughtPanelAction::OpenSession {
                session_id: "sess-1".to_string(),
                label: "7".to_string(),
            })
        );
    }

    #[test]
    fn mermaid_viewer_renders_inline_unsupported_state_and_back_button_restores_aquarium() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        let mut renderer = test_renderer(120, 32);
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.unsupported_reason = Some("unsupported terminal backend".to_string());

        app.render(&mut renderer, layout);

        let message_row = mermaid_content_rect(layout.overview_field).y;
        assert!(row_text(&renderer, message_row).contains("unsupported terminal backend"));

        let back_rect = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.back_rect.expect("back rect"),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(app.handle_mermaid_mouse_down(
            layout.overview_field,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: back_rect.x,
                row: back_rect.y,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
    }

    #[test]
    fn mermaid_keyboard_controls_pan_zoom_reset_and_escape() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        let content_rect = mermaid_content_rect(layout.overview_field);
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.content_rect = Some(content_rect);
        viewer.diagram_width = 1000.0;
        viewer.diagram_height = 800.0;
        viewer.center_x = 500.0;
        viewer.center_y = 400.0;
        viewer.unsupported_reason = None;

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
        let (zoom_after_plus, center_after_plus) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (viewer.zoom, (viewer.center_x, viewer.center_y)),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(zoom_after_plus > 1.0);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        ));
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        ));
        let (center_after_pan_x, center_after_pan_y) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(center_after_pan_x > center_after_plus.0);
        assert!(center_after_pan_y > center_after_plus.1);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
        ));
        let (zoom_after_reset, center_after_reset_x, center_after_reset_y) =
            match &app.fish_bowl_mode {
                FishBowlMode::Mermaid(viewer) => (viewer.zoom, viewer.center_x, viewer.center_y),
                FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
            };
        assert_eq!(zoom_after_reset, 1.0);
        assert_eq!(center_after_reset_x, 0.0);
        assert_eq!(center_after_reset_y, 0.0);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
        assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
    }

    #[test]
    fn mermaid_mouse_drag_and_scroll_update_viewport() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        let content_rect = mermaid_content_rect(layout.overview_field);
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.content_rect = Some(content_rect);
        viewer.diagram_width = 1000.0;
        viewer.diagram_height = 800.0;
        viewer.center_x = 500.0;
        viewer.center_y = 400.0;
        viewer.unsupported_reason = None;

        let start_column = content_rect.x + 4;
        let start_row = content_rect.y + 2;
        assert!(app.handle_mermaid_mouse_down(
            layout.overview_field,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: start_column,
                row: start_row,
                modifiers: KeyModifiers::NONE,
            },
        ));
        assert!(app.handle_mermaid_mouse_drag(
            layout.overview_field,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: start_column + 5,
                row: start_row + 2,
                modifiers: KeyModifiers::NONE,
            },
        ));
        let (center_after_drag_x, center_after_drag_y) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_ne!(center_after_drag_x, 500.0);
        assert_ne!(center_after_drag_y, 400.0);
        assert!(app.handle_mermaid_mouse_up());

        let zoom_before_scroll = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.zoom,
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(app.handle_mermaid_scroll(
            layout.overview_field,
            crossterm::event::MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: start_column,
                row: start_row,
                modifiers: KeyModifiers::NONE,
            },
            MERMAID_ZOOM_STEP,
        ));
        let zoom_after_scroll = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.zoom,
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(zoom_after_scroll > zoom_before_scroll);
    }

    #[test]
    fn mermaid_render_reuses_prepared_source_state_across_zoom_and_pan() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        let mut renderer = test_renderer(120, 32);
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.unsupported_reason = None;

        app.render(&mut renderer, layout);
        let (prepare_after_first, viewport_after_first, first_lines_empty) =
            match &app.fish_bowl_mode {
                FishBowlMode::Mermaid(viewer) => (
                    viewer.source_prepare_count,
                    viewer.viewport_render_count,
                    viewer.cached_lines.is_empty(),
                ),
                FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
            };
        assert_eq!(prepare_after_first, 1);
        assert_eq!(viewport_after_first, 1);
        assert!(!first_lines_empty);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
        app.render(&mut renderer, layout);
        let (prepare_after_zoom, viewport_after_zoom) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                (viewer.source_prepare_count, viewer.viewport_render_count)
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_eq!(prepare_after_zoom, 1);
        assert_eq!(viewport_after_zoom, 2);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        ));
        app.render(&mut renderer, layout);
        let (prepare_after_pan, viewport_after_pan) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                (viewer.source_prepare_count, viewer.viewport_render_count)
            }
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_eq!(prepare_after_pan, 1);
        assert_eq!(viewport_after_pan, 3);
    }

    #[test]
    fn mermaid_refresh_invalidates_prepared_source_state_when_artifact_changes() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api.clone());
        let mut renderer = test_renderer(120, 32);
        let sessions = vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)];
        app.merge_sessions(sessions.clone(), layout.overview_field);
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow-a.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.unsupported_reason = None;

        app.render(&mut renderer, layout);
        let prepare_after_first = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.source_prepare_count,
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_eq!(prepare_after_first, 1);

        api.push_mermaid_artifact(Ok(mermaid_artifact(
            "sess-1",
            "/tmp/repos/throngterm/flow-b.mmd",
            "2026-03-23T10:06:00Z",
            "graph TD\nA-->C\n",
        )));
        app.refresh_mermaid_artifacts(&sessions);
        app.render(&mut renderer, layout);

        let (prepare_after_refresh, refreshed_path) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                viewer.source_prepare_count,
                viewer.path.as_deref().map(str::to_string),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_eq!(prepare_after_refresh, 2);
        assert_eq!(
            refreshed_path.as_deref(),
            Some("/tmp/repos/throngterm/flow-b.mmd")
        );
    }

    #[test]
    fn mermaid_graph_node_labels_render_as_terminal_text() {
        let (mut app, mut renderer, layout) =
            open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

        app.render(&mut renderer, layout);

        let alpha = find_text_position(&renderer, "Alpha Node").expect("Alpha Node overlay");
        let beta = find_text_position(&renderer, "Beta Node").expect("Beta Node overlay");
        assert_eq!(cell_at(&renderer, alpha.0, alpha.1).ch, 'A');
        assert_eq!(cell_at(&renderer, beta.0, beta.1).ch, 'B');
        assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
    }

    #[test]
    fn mermaid_er_overview_hides_attribute_detail_until_zoomed() {
        let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        app.render(&mut renderer, layout);

        let semantic_texts = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(semantic_texts.contains(&"USER".to_string()));
        assert!(semantic_texts.contains(&"ORDER".to_string()));
        assert!(!semantic_texts.contains(&"email".to_string()));
        assert!(!semantic_texts.contains(&"user_id".to_string()));
        assert!(!semantic_texts.contains(&"uuid".to_string()));
        assert!(!semantic_texts.contains(&"places".to_string()));
        assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
    }

    #[test]
    fn mermaid_flowchart_overview_hides_edge_labels_until_zoomed() {
        let source =
            "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        app.render(&mut renderer, layout);

        assert!(find_text_position(&renderer, "Group One").is_some());
        assert!(find_text_position(&renderer, "Producer").is_some());
        assert!(find_text_position(&renderer, "Consumer").is_some());
        assert!(find_text_position(&renderer, "ships data").is_none());
        assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
    }

    #[test]
    fn mermaid_zoom_reveals_edge_labels_at_detail_l2() {
        let source =
            "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        ));
        app.render(&mut renderer, layout);

        let semantic_texts = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(
            find_text_position(&renderer, "ships data").is_some(),
            "status row: {}; semantic_texts: {:?}",
            row_text(&renderer, layout.overview_field.y),
            semantic_texts
        );
        assert!(
            row_text(&renderer, layout.overview_field.y).contains("detail L2"),
            "status row: {}",
            row_text(&renderer, layout.overview_field.y)
        );
    }

    #[test]
    fn mermaid_er_zoom_reveals_attribute_names_before_types() {
        let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        for _ in 0..3 {
            assert!(handle_key_event(
                &mut app,
                layout,
                KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            ));
        }
        app.render(&mut renderer, layout);

        let semantic_texts = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(semantic_texts.contains(&"USER".to_string()));
        assert!(
            semantic_texts.contains(&"id".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            semantic_texts.contains(&"email".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            !semantic_texts.contains(&"uuid".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            !semantic_texts.contains(&"string".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            row_text(&renderer, layout.overview_field.y).contains("detail L2"),
            "status row: {}",
            row_text(&renderer, layout.overview_field.y)
        );
    }

    #[test]
    fn mermaid_er_zoom_reveals_attribute_types_at_detail_l3() {
        let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        for _ in 0..5 {
            assert!(handle_key_event(
                &mut app,
                layout,
                KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            ));
        }
        app.render(&mut renderer, layout);

        let semantic_texts = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(
            semantic_texts.contains(&"uuid".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            semantic_texts.contains(&"string".to_string()),
            "{semantic_texts:?}"
        );
        assert!(
            row_text(&renderer, layout.overview_field.y).contains("detail L3"),
            "status row: {}",
            row_text(&renderer, layout.overview_field.y)
        );
    }

    #[test]
    fn mermaid_reset_fit_hides_subordinate_detail() {
        let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        for _ in 0..5 {
            assert!(handle_key_event(
                &mut app,
                layout,
                KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
            ));
        }
        app.render(&mut renderer, layout);
        assert!(
            row_text(&renderer, layout.overview_field.y).contains("detail L3"),
            "status row: {}",
            row_text(&renderer, layout.overview_field.y)
        );

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
        ));
        app.render(&mut renderer, layout);

        let semantic_texts = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert!(semantic_texts.contains(&"USER".to_string()));
        assert!(!semantic_texts.contains(&"id".to_string()));
        assert!(!semantic_texts.contains(&"email".to_string()));
        assert!(!semantic_texts.contains(&"uuid".to_string()));
        assert!(!semantic_texts.contains(&"string".to_string()));
        assert!(row_text(&renderer, layout.overview_field.y).contains("detail L1"));
    }

    #[test]
    fn mermaid_too_small_view_keeps_existing_guard() {
        let (mut app, mut renderer, _layout) =
            open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
        let small_field = Rect {
            x: 0,
            y: 0,
            width: 15,
            height: 7,
        };
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        render_mermaid_viewer(&mut renderer, small_field, viewer);

        assert!(find_text_position(&renderer, "Mermaid view").is_some());
        assert!(find_text_position(&renderer, "too small").is_some());
        let semantic_count = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => viewer.cached_semantic_lines.len(),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_eq!(semantic_count, 0);
    }

    #[test]
    fn mermaid_semantic_labels_track_zoom_and_pan() {
        let (mut app, mut renderer, layout) =
            open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
        let content_rect = mermaid_content_rect(layout.overview_field);

        app.render(&mut renderer, layout);
        let (alpha_before, beta_before) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node before"),
                find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node before"),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

        app.zoom_mermaid_viewer(MERMAID_ZOOM_STEP, None, content_rect);
        app.pan_mermaid_viewer(24.0, 18.0);
        app.render(&mut renderer, layout);

        let (alpha_after, beta_after, prepare_count) = match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node after"),
                find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node after"),
                viewer.source_prepare_count,
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
        assert_ne!(alpha_after, alpha_before);
        assert_ne!(beta_after, beta_before);
        assert_eq!(prepare_count, 1);
    }

    #[test]
    fn mermaid_sequence_diagram_falls_back_to_braille_only() {
        let (mut app, mut renderer, layout) =
            open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

        app.render(&mut renderer, layout);

        let (render_error, cached_lines_empty, cached_semantic_lines_empty) =
            match &app.fish_bowl_mode {
                FishBowlMode::Mermaid(viewer) => (
                    viewer.render_error.clone(),
                    viewer.cached_lines.is_empty(),
                    viewer.cached_semantic_lines.is_empty(),
                ),
                FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
            };
        assert_eq!(render_error, None);
        assert!(!cached_lines_empty);
        assert!(cached_semantic_lines_empty);
        assert!(find_text_position(&renderer, "hello").is_none());
    }

    #[test]
    fn mermaid_semantic_labels_clip_to_viewport_bounds() {
        let content_rect = Rect {
            x: 42,
            y: 10,
            width: 20,
            height: 5,
        };
        let projected = project_mermaid_semantic_lines(
            &[MermaidSemanticLine {
                text: "Alpha Node".to_string(),
                diagram_x: 0.0,
                diagram_y: 4.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_width: 20.0,
                owner_height: 8.0,
            }],
            MermaidViewportTransform {
                scale: 1.0,
                tx: -4.0,
                ty: 0.0,
            },
            content_rect,
            MermaidDetailLevel::L1,
        );

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].x, content_rect.x);
        assert_eq!(projected[0].y, content_rect.y + 1);
        assert_eq!(projected[0].text, "Alpha Node");
    }

    #[test]
    fn mermaid_resize_reprojects_semantic_labels() {
        let source =
            "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
        let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

        app.render(&mut renderer, layout);
        let group_before = find_text_position(&renderer, "Group One").expect("Group One before");
        let producer_before = find_text_position(&renderer, "Producer").expect("Producer before");

        let resized_layout = test_layout(160, 48);
        let mut resized_renderer = test_renderer(160, 48);
        app.render(&mut resized_renderer, resized_layout);

        let group_after =
            find_text_position(&resized_renderer, "Group One").expect("Group One after");
        let producer_after =
            find_text_position(&resized_renderer, "Producer").expect("Producer after");
        assert_ne!(group_after, group_before);
        assert_ne!(producer_after, producer_before);
    }

    #[test]
    fn mermaid_open_shortcut_uses_artifact_path_and_stays_in_viewer() {
        let api = MockApi::new();
        let opener = Arc::new(MockArtifactOpener::default());
        let layout = test_layout(120, 32);
        let mut app = make_app_with_artifact_opener(api, opener.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        ));

        assert_eq!(
            opener.calls(),
            vec!["/tmp/repos/throngterm/flow.mmd".to_string()]
        );
        assert!(matches!(app.fish_bowl_mode, FishBowlMode::Mermaid(_)));
        assert_eq!(
            app.visible_message(),
            Some("open Mermaid artifact -> flow.mmd")
        );
    }

    #[test]
    fn mermaid_open_shortcut_reports_failures_and_missing_paths() {
        let api = MockApi::new();
        let opener = Arc::new(MockArtifactOpener::default());
        opener.fail_with("boom");
        let layout = test_layout(120, 32);
        let mut app = make_app_with_artifact_opener(api, opener.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );

        app.open_mermaid_viewer("sess-1".to_string());
        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        ));
        assert_eq!(
            app.visible_message(),
            Some("failed to open Mermaid artifact: boom")
        );
        assert_eq!(
            opener.calls(),
            vec!["/tmp/repos/throngterm/flow.mmd".to_string()]
        );

        let opener = Arc::new(MockArtifactOpener::default());
        let mut app = make_app_with_artifact_opener(MockApi::new(), opener.clone());
        app.merge_sessions(
            vec![session_summary("sess-1", "7", TEST_REPO_THRONGTERM)],
            layout.overview_field,
        );
        app.mermaid_artifacts.insert(
            "sess-1".to_string(),
            mermaid_artifact(
                "sess-1",
                "/tmp/repos/throngterm/flow.mmd",
                "2026-03-23T10:05:00Z",
                "graph TD\nA-->B\n",
            ),
        );
        app.open_mermaid_viewer("sess-1".to_string());
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!("expected Mermaid viewer mode");
        };
        viewer.path = None;

        assert!(handle_key_event(
            &mut app,
            layout,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        ));
        assert_eq!(opener.calls(), Vec::<String>::new());
        assert_eq!(
            app.visible_message(),
            Some("Mermaid artifact path unavailable")
        );
    }

    #[test]
    fn handle_tui_event_covers_key_paste_mouse_and_resize_paths() {
        let api = MockApi::new();
        let layout = test_layout(120, 32);
        let mut app = make_app(api);
        let mut renderer = test_renderer(120, 32);
        app.open_initial_request("/tmp/project".to_string());

        assert!(handle_tui_event(
            &mut app,
            &mut renderer,
            layout,
            Event::Paste("hello".to_string()),
        )
        .expect("paste event should succeed"));
        assert_eq!(
            app.initial_request
                .as_ref()
                .map(|state| state.value.as_str()),
            Some("hello")
        );

        app.close_initial_request();
        assert!(!handle_tui_event(
            &mut app,
            &mut renderer,
            layout,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        )
        .expect("quit key should succeed"));

        assert!(handle_tui_event(
            &mut app,
            &mut renderer,
            layout,
            Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 10,
                row: 10,
                modifiers: KeyModifiers::NONE,
            }),
        )
        .expect("mouse up should succeed"));

        assert!(
            handle_tui_event(&mut app, &mut renderer, layout, Event::Resize(90, 20),)
                .expect("resize should succeed")
        );
        assert_eq!(renderer.width(), 90);
        assert_eq!(renderer.height(), 20);
    }
}
