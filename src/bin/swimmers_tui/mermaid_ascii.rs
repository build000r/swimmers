use super::layout::Rect;
use resvg::tiny_skia::Pixmap;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MermaidAsciiSample {
    grid: [[bool; 2]; 4],
    row_counts: [u8; 4],
    col_counts: [u8; 2],
    total: u8,
}

fn mermaid_ascii_sample(pixmap: &Pixmap, cell_x: u16, cell_y: u16) -> MermaidAsciiSample {
    let base_x = u32::from(cell_x) * 2;
    let base_y = u32::from(cell_y) * 4;
    let mut grid = [[false; 2]; 4];
    let mut row_counts = [0u8; 4];
    let mut col_counts = [0u8; 2];
    let mut total = 0u8;

    for sub_y in 0..4 {
        for sub_x in 0..2 {
            let dark = pixel_is_dark(pixmap, base_x + sub_x as u32, base_y + sub_y as u32);
            grid[sub_y][sub_x] = dark;
            if dark {
                row_counts[sub_y] += 1;
                col_counts[sub_x] += 1;
                total += 1;
            }
        }
    }

    MermaidAsciiSample {
        grid,
        row_counts,
        col_counts,
        total,
    }
}

fn mermaid_ascii_has_horizontal(sample: &MermaidAsciiSample) -> bool {
    sample.row_counts.into_iter().any(|count| count == 2)
}

fn mermaid_ascii_has_vertical(sample: &MermaidAsciiSample) -> bool {
    sample.col_counts[0] >= 3 && sample.col_counts[1] >= 3
}

fn mermaid_ascii_has_right_arrow(sample: &MermaidAsciiSample, horizontal: bool) -> bool {
    sample.col_counts[1] >= 3
        && sample.col_counts[0] <= 1
        && horizontal
        && (sample.grid[1][1] || sample.grid[2][1])
}

fn mermaid_ascii_has_left_arrow(sample: &MermaidAsciiSample, horizontal: bool) -> bool {
    sample.col_counts[0] >= 3
        && sample.col_counts[1] <= 1
        && horizontal
        && (sample.grid[1][0] || sample.grid[2][0])
}

fn mermaid_ascii_has_diagonal(sample: &MermaidAsciiSample) -> bool {
    ((sample.grid[0][0] || sample.grid[1][0]) && (sample.grid[2][1] || sample.grid[3][1]))
        || ((sample.grid[0][1] || sample.grid[1][1]) && (sample.grid[2][0] || sample.grid[3][0]))
}

fn mermaid_ascii_char(sample: &MermaidAsciiSample) -> char {
    let horizontal = mermaid_ascii_has_horizontal(sample);
    [
        (sample.total == 0, ' '),
        (mermaid_ascii_has_right_arrow(sample, horizontal), '>'),
        (mermaid_ascii_has_left_arrow(sample, horizontal), '<'),
        (mermaid_ascii_has_vertical(sample), '|'),
        (horizontal, '_'),
        (mermaid_ascii_has_diagonal(sample), '\\'),
        (sample.col_counts[0] >= 2 || sample.col_counts[1] >= 2, '|'),
        (sample.total >= 2, '_'),
    ]
    .into_iter()
    .find_map(|(matched, ch)| matched.then_some(ch))
    .unwrap_or(' ')
}

fn mermaid_ascii_cell(pixmap: &Pixmap, cell_x: u16, cell_y: u16) -> char {
    mermaid_ascii_char(&mermaid_ascii_sample(pixmap, cell_x, cell_y))
}

pub(crate) fn pixmap_to_ascii_lines(pixmap: &Pixmap, content_rect: Rect) -> Vec<String> {
    let mut lines = Vec::new();
    for cell_y in 0..content_rect.height {
        let mut line = String::with_capacity(content_rect.width as usize);
        for cell_x in 0..content_rect.width {
            line.push(mermaid_ascii_cell(pixmap, cell_x, cell_y));
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(rows: [&str; 4]) -> MermaidAsciiSample {
        let mut grid = [[false; 2]; 4];
        let mut row_counts = [0u8; 4];
        let mut col_counts = [0u8; 2];
        let mut total = 0u8;

        for (sub_y, row) in rows.into_iter().enumerate() {
            for (sub_x, ch) in row.chars().enumerate() {
                let dark = ch == '#';
                grid[sub_y][sub_x] = dark;
                if dark {
                    row_counts[sub_y] += 1;
                    col_counts[sub_x] += 1;
                    total += 1;
                }
            }
        }

        MermaidAsciiSample {
            grid,
            row_counts,
            col_counts,
            total,
        }
    }

    #[test]
    fn mermaid_ascii_char_preserves_priority_order() {
        assert_eq!(mermaid_ascii_char(&sample(["..", "..", "..", ".."])), ' ');
        assert_eq!(mermaid_ascii_char(&sample([".#", "##", ".#", ".#"])), '>');
        assert_eq!(mermaid_ascii_char(&sample(["#.", "##", "#.", "#."])), '<');
        assert_eq!(mermaid_ascii_char(&sample(["##", "##", "##", ".."])), '|');
        assert_eq!(mermaid_ascii_char(&sample(["##", "..", "..", ".."])), '_');
        assert_eq!(mermaid_ascii_char(&sample(["#.", "..", ".#", ".."])), '\\');
        assert_eq!(mermaid_ascii_char(&sample(["#.", "#.", "..", ".."])), '|');
        assert_eq!(mermaid_ascii_char(&sample(["#.", ".#", "..", ".."])), '_');
        assert_eq!(mermaid_ascii_char(&sample(["#.", "..", "..", ".."])), ' ');
    }

    #[test]
    fn mermaid_ascii_cell_samples_same_two_by_four_pixel_block() {
        let mut pixmap = Pixmap::new(6, 8).expect("pixmap");
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(255, 255, 255, 255));
        for (x, y) in [(2, 4), (3, 5), (2, 6)] {
            let idx = ((y * pixmap.width() + x) * 4) as usize;
            pixmap.data_mut()[idx..idx + 4].copy_from_slice(&[0, 0, 0, 255]);
        }

        assert_eq!(mermaid_ascii_cell(&pixmap, 1, 1), '\\');
        assert_eq!(mermaid_ascii_cell(&pixmap, 0, 1), ' ');
        assert_eq!(mermaid_ascii_cell(&pixmap, 1, 0), ' ');
    }
}
