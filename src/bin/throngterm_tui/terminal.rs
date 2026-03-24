use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) ch: char,
    pub(crate) fg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Reset,
        }
    }
}

pub(crate) struct Renderer {
    pub(crate) stdout: BufWriter<Stdout>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) buffer: Vec<Cell>,
    pub(crate) last_buffer: Vec<Cell>,
    pub(crate) terminal_state: TerminalState,
}

#[derive(Default)]
pub(crate) struct TerminalState {
    pub(crate) raw_mode_enabled: bool,
    pub(crate) terminal_ui_active: bool,
}

pub(crate) fn enter_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        cursor::Hide,
        Clear(ClearType::All)
    )
}

pub(crate) fn leave_terminal_ui(writer: &mut impl Write) -> io::Result<()> {
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
    pub(crate) fn init_with<W, EnableRawMode, EnterUi>(
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

    pub(crate) fn cleanup_with<W, LeaveUi, DisableRawMode>(
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
    pub(crate) fn new() -> io::Result<Self> {
        if !io::stdout().is_terminal() {
            return Err(io::Error::other("stdout is not a tty"));
        }

        let (width, height) = crossterm_terminal::size()?;
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

    pub(crate) fn init(&mut self) -> io::Result<()> {
        self.terminal_state.init_with(
            &mut self.stdout,
            crossterm_terminal::enable_raw_mode,
            enter_terminal_ui,
        )
    }

    pub(crate) fn cleanup(&mut self) -> io::Result<()> {
        self.terminal_state.cleanup_with(
            &mut self.stdout,
            leave_terminal_ui,
            crossterm_terminal::disable_raw_mode,
        )
    }

    pub(crate) fn manual_resize(&mut self, width: u16, height: u16) -> io::Result<()> {
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

    pub(crate) fn width(&self) -> u16 {
        self.width
    }

    pub(crate) fn height(&self) -> u16 {
        self.height
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.fill(Cell::default());
    }

    pub(crate) fn fill_rect(&mut self, rect: Rect, ch: char, fg: Color) {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                self.draw_char(x, y, ch, fg);
            }
        }
    }

    pub(crate) fn draw_char(&mut self, x: u16, y: u16, ch: char, fg: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = (y as usize) * (self.width as usize) + (x as usize);
        if let Some(cell) = self.buffer.get_mut(idx) {
            *cell = Cell { ch, fg };
        }
    }

    pub(crate) fn draw_text(&mut self, x: u16, y: u16, text: &str, fg: Color) {
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

    pub(crate) fn draw_hline(&mut self, x: u16, y: u16, width: u16, ch: char, fg: Color) {
        for dx in 0..width {
            self.draw_char(x + dx, y, ch, fg);
        }
    }

    pub(crate) fn draw_vline(&mut self, x: u16, y: u16, height: u16, ch: char, fg: Color) {
        for dy in 0..height {
            self.draw_char(x, y + dy, ch, fg);
        }
    }

    pub(crate) fn draw_box(&mut self, rect: Rect, fg: Color) {
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

    pub(crate) fn flush(&mut self) -> io::Result<()> {
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
