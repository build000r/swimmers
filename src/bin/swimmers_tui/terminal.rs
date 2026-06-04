use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlushOp {
    MoveTo(u16, u16),
    SetForegroundColor(Color),
    Print(char),
    ResetColor,
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

fn run_teardown_step<Teardown>(is_active: &mut bool, teardown: Teardown) -> Option<io::Error>
where
    Teardown: FnOnce() -> io::Result<()>,
{
    if !*is_active {
        return None;
    }

    let error = teardown().err();
    *is_active = false;
    error
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
        let leave_error = run_teardown_step(&mut self.terminal_ui_active, || leave_ui(writer));
        let raw_mode_error = run_teardown_step(&mut self.raw_mode_enabled, disable_raw_mode);

        leave_error.or(raw_mode_error).map_or(Ok(()), Err)
    }
}

fn changed_cell_indices<'a>(
    buffer: &'a [Cell],
    last_buffer: &'a [Cell],
) -> impl Iterator<Item = usize> + 'a {
    buffer
        .iter()
        .zip(last_buffer)
        .enumerate()
        .filter_map(|(idx, (cell, prev))| (*cell != *prev).then_some(idx))
}

fn cell_position(width: u16, idx: usize) -> (u16, u16) {
    let width = width as usize;
    ((idx % width) as u16, (idx / width) as u16)
}

fn append_cell_flush_ops(
    ops: &mut Vec<FlushOp>,
    current_color: &mut Color,
    last_pos: &mut Option<(u16, u16)>,
    x: u16,
    y: u16,
    cell: Cell,
) {
    if *last_pos != Some((x, y)) {
        ops.push(FlushOp::MoveTo(x, y));
    }

    if cell.fg != *current_color {
        ops.push(FlushOp::SetForegroundColor(cell.fg));
        *current_color = cell.fg;
    }

    ops.push(FlushOp::Print(cell.ch));
    *last_pos = Some((x.saturating_add(1), y));
}

fn plan_flush_ops(width: u16, height: u16, buffer: &[Cell], last_buffer: &[Cell]) -> Vec<FlushOp> {
    let visible_len = (width as usize) * (height as usize);
    let mut ops = Vec::new();
    let mut current_color = Color::Reset;
    let mut last_pos = None;

    for idx in changed_cell_indices(&buffer[..visible_len], &last_buffer[..visible_len]) {
        let (x, y) = cell_position(width, idx);
        append_cell_flush_ops(
            &mut ops,
            &mut current_color,
            &mut last_pos,
            x,
            y,
            buffer[idx],
        );
    }

    if current_color != Color::Reset {
        ops.push(FlushOp::ResetColor);
    }

    ops
}

fn queue_flush_op(writer: &mut impl Write, op: FlushOp) -> io::Result<()> {
    match op {
        FlushOp::MoveTo(x, y) => queue!(writer, cursor::MoveTo(x, y)),
        FlushOp::SetForegroundColor(color) => queue!(writer, SetForegroundColor(color)),
        FlushOp::Print(ch) => queue!(writer, Print(ch)),
        FlushOp::ResetColor => queue!(writer, ResetColor),
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
        for op in plan_flush_ops(self.width, self.height, &self.buffer, &self.last_buffer) {
            queue_flush_op(&mut self.stdout, op)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as TestCell;
    use std::sync::{Arc, Mutex};

    #[test]
    fn cleanup_clears_flags_and_returns_first_error_when_both_steps_fail() {
        let mut terminal_state = TerminalState {
            raw_mode_enabled: true,
            terminal_ui_active: true,
        };
        let mut output = Vec::new();
        let events = Arc::new(Mutex::new(Vec::new()));

        let error = terminal_state
            .cleanup_with(
                &mut output,
                {
                    let events = Arc::clone(&events);
                    move |_writer| {
                        events.lock().unwrap().push("leave_terminal_ui");
                        Err(io::Error::other("leave failed"))
                    }
                },
                {
                    let events = Arc::clone(&events);
                    move || {
                        events.lock().unwrap().push("disable_raw_mode");
                        Err(io::Error::other("disable failed"))
                    }
                },
            )
            .expect_err("cleanup should return the first teardown error");

        assert_eq!(error.to_string(), "leave failed");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["leave_terminal_ui", "disable_raw_mode"]
        );
        assert!(!terminal_state.terminal_ui_active);
        assert!(!terminal_state.raw_mode_enabled);
    }

    #[test]
    fn cleanup_clears_ui_flag_when_leave_fails_without_raw_mode() {
        let mut terminal_state = TerminalState {
            raw_mode_enabled: false,
            terminal_ui_active: true,
        };
        let mut output = Vec::new();
        let disable_calls = TestCell::new(0usize);

        let error = terminal_state
            .cleanup_with(
                &mut output,
                |_writer| Err(io::Error::other("leave failed")),
                || {
                    disable_calls.set(disable_calls.get() + 1);
                    Ok(())
                },
            )
            .expect_err("cleanup should return the leave error");

        assert_eq!(error.to_string(), "leave failed");
        assert_eq!(disable_calls.get(), 0);
        assert!(!terminal_state.terminal_ui_active);
        assert!(!terminal_state.raw_mode_enabled);
    }

    #[test]
    fn cleanup_clears_raw_flag_when_disable_fails_without_active_ui() {
        let mut terminal_state = TerminalState {
            raw_mode_enabled: true,
            terminal_ui_active: false,
        };
        let mut output = Vec::new();
        let leave_calls = TestCell::new(0usize);

        let error = terminal_state
            .cleanup_with(
                &mut output,
                |_writer| {
                    leave_calls.set(leave_calls.get() + 1);
                    Ok(())
                },
                || Err(io::Error::other("disable failed")),
            )
            .expect_err("cleanup should return the raw mode teardown error");

        assert_eq!(error.to_string(), "disable failed");
        assert_eq!(leave_calls.get(), 0);
        assert!(!terminal_state.terminal_ui_active);
        assert!(!terminal_state.raw_mode_enabled);
    }

    #[test]
    fn flush_plan_emits_only_changed_cells_and_minimizes_color_changes() {
        let last_buffer = vec![Cell::default(); 4];
        let buffer = vec![
            Cell {
                ch: 'a',
                fg: Color::Red,
            },
            Cell {
                ch: 'b',
                fg: Color::Red,
            },
            Cell::default(),
            Cell {
                ch: 'c',
                fg: Color::Blue,
            },
        ];

        assert_eq!(
            plan_flush_ops(4, 1, &buffer, &last_buffer),
            vec![
                FlushOp::MoveTo(0, 0),
                FlushOp::SetForegroundColor(Color::Red),
                FlushOp::Print('a'),
                FlushOp::Print('b'),
                FlushOp::MoveTo(3, 0),
                FlushOp::SetForegroundColor(Color::Blue),
                FlushOp::Print('c'),
                FlushOp::ResetColor,
            ]
        );
    }

    #[test]
    fn flush_plan_omits_reset_when_changed_cells_end_in_reset_color() {
        let last_buffer = vec![
            Cell::default(),
            Cell {
                ch: 'x',
                fg: Color::Green,
            },
        ];
        let buffer = vec![
            Cell {
                ch: 'x',
                fg: Color::Green,
            },
            Cell::default(),
        ];

        assert_eq!(
            plan_flush_ops(2, 1, &buffer, &last_buffer),
            vec![
                FlushOp::MoveTo(0, 0),
                FlushOp::SetForegroundColor(Color::Green),
                FlushOp::Print('x'),
                FlushOp::SetForegroundColor(Color::Reset),
                FlushOp::Print(' '),
            ]
        );
    }

    #[test]
    fn flush_plan_is_empty_when_no_cells_changed() {
        let buffer = vec![Cell::default(); 6];

        assert!(plan_flush_ops(3, 2, &buffer, &buffer).is_empty());
    }
}
