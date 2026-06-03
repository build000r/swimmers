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

fn find_blank_position(renderer: &Renderer, rect: Rect) -> Option<(u16, u16)> {
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if cell_at(renderer, x, y).ch == ' ' {
                return Some((x, y));
            }
        }
    }
    None
}
