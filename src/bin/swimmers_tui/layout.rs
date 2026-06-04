use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Rect {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl Rect {
    pub(crate) fn right(self) -> u16 {
        self.x + self.width
    }

    pub(crate) fn bottom(self) -> u16 {
        self.y + self.height
    }

    pub(crate) fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub(crate) fn inset(self, amount: u16) -> Self {
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
pub(crate) struct WorkspaceLayout {
    pub(crate) workspace_box: Rect,
    pub(crate) overview_box: Rect,
    pub(crate) overview_field: Rect,
    pub(crate) thought_box: Option<Rect>,
    pub(crate) thought_content: Option<Rect>,
    pub(crate) split_divider: Option<Rect>,
    pub(crate) split_hitbox: Option<Rect>,
    pub(crate) footer_start_y: u16,
}

#[derive(Clone, Copy)]
struct WorkspaceFrame {
    workspace_box: Rect,
    inner: Rect,
    footer_start_y: u16,
}

#[derive(Clone, Copy)]
struct ThoughtRailSplit {
    thought_width: u16,
    overview_width: u16,
}

impl WorkspaceFrame {
    fn for_terminal(width: u16, height: u16) -> Self {
        let workspace_box = field_box(width, height);
        Self {
            workspace_box,
            footer_start_y: workspace_box.bottom() + 1,
            inner: workspace_box.inset(1),
        }
    }

    fn without_thought_panel(self) -> WorkspaceLayout {
        WorkspaceLayout {
            workspace_box: self.workspace_box,
            overview_box: self.workspace_box,
            overview_field: self.inner,
            thought_box: None,
            thought_content: None,
            split_divider: None,
            split_hitbox: None,
            footer_start_y: self.footer_start_y,
        }
    }

    fn with_thought_split(self, split: ThoughtRailSplit) -> WorkspaceLayout {
        let thought_box = Rect {
            x: self.inner.x,
            y: self.inner.y,
            width: split.thought_width,
            height: self.inner.height,
        };
        let overview_box = Rect {
            x: thought_box.right() + THOUGHT_RAIL_GAP,
            y: self.inner.y,
            width: split.overview_width,
            height: self.inner.height,
        };
        let split_divider = Rect {
            x: thought_box.right(),
            y: self.inner.y,
            width: THOUGHT_RAIL_GAP,
            height: self.inner.height,
        };
        let split_hitbox = Rect {
            x: split_divider.x.saturating_sub(1),
            y: self.inner.y,
            width: THOUGHT_RAIL_DRAG_HITBOX_WIDTH,
            height: self.inner.height,
        };

        WorkspaceLayout {
            workspace_box: self.workspace_box,
            overview_box,
            overview_field: overview_box.inset(1),
            thought_box: Some(thought_box),
            thought_content: Some(thought_box.inset(1)),
            split_divider: Some(split_divider),
            split_hitbox: Some(split_hitbox),
            footer_start_y: self.footer_start_y,
        }
    }
}

impl ThoughtRailSplit {
    fn for_inner_width(inner_width: u16, thought_ratio: f32) -> Option<Self> {
        let min_overview_width = min_overview_width();
        let ideal_thought_width =
            ((inner_width as f32) * sanitized_thought_ratio(thought_ratio)).floor() as u16;
        let ideal_thought_width = ideal_thought_width.max(THOUGHT_RAIL_MIN_PANEL_WIDTH);
        let max_thought_width = inner_width.saturating_sub(THOUGHT_RAIL_GAP + min_overview_width);
        let thought_width = ideal_thought_width.min(max_thought_width);
        let overview_width = inner_width.saturating_sub(thought_width + THOUGHT_RAIL_GAP);

        (overview_width >= min_overview_width).then_some(Self {
            thought_width,
            overview_width,
        })
    }
}

fn min_overview_width() -> u16 {
    ENTITY_WIDTH + 4
}

fn can_split_workspace(width: u16, inner: Rect) -> bool {
    width >= THOUGHT_RAIL_MIN_WIDTH
        && inner.height >= 3
        && inner.width >= THOUGHT_RAIL_MIN_PANEL_WIDTH + THOUGHT_RAIL_GAP + min_overview_width()
}

fn sanitized_thought_ratio(thought_ratio: f32) -> f32 {
    if thought_ratio.is_finite() {
        thought_ratio.clamp(0.0, 1.0)
    } else {
        THOUGHT_RAIL_DEFAULT_RATIO
    }
}

impl WorkspaceLayout {
    #[allow(dead_code)]
    pub(crate) fn for_terminal(width: u16, height: u16) -> Self {
        Self::for_terminal_with_ratio(width, height, THOUGHT_RAIL_DEFAULT_RATIO)
    }

    pub(crate) fn for_terminal_without_thought_panel(width: u16, height: u16) -> Self {
        WorkspaceFrame::for_terminal(width, height).without_thought_panel()
    }

    pub(crate) fn for_terminal_with_ratio(width: u16, height: u16, thought_ratio: f32) -> Self {
        let frame = WorkspaceFrame::for_terminal(width, height);
        let Some(split) = can_split_workspace(width, frame.inner)
            .then(|| ThoughtRailSplit::for_inner_width(frame.inner.width, thought_ratio))
            .flatten()
        else {
            return frame.without_thought_panel();
        };

        frame.with_thought_split(split)
    }

    pub(crate) fn thought_entry_capacity(self) -> usize {
        self.thought_content
            .map(|content| content.height.saturating_sub(THOUGHT_RAIL_HEADER_ROWS) as usize)
            .unwrap_or(0)
    }

    pub(crate) fn thought_ratio_for_divider_x(self, x: u16) -> Option<f32> {
        let thought_box = self.thought_box?;
        let inner = self.workspace_box.inset(1);
        let min_overview_width = min_overview_width();
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

pub(crate) fn frame_rect(width: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width,
        height,
    }
}

pub(crate) fn field_box(width: u16, height: u16) -> Rect {
    let footer_height = 6;
    Rect {
        x: 1,
        y: 3,
        width: width.saturating_sub(2),
        height: height.saturating_sub(footer_height + 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_without_thought_panel(layout: WorkspaceLayout, width: u16, height: u16) {
        let expected = WorkspaceLayout::for_terminal_without_thought_panel(width, height);

        assert_eq!(layout.workspace_box, expected.workspace_box);
        assert_eq!(layout.overview_box, expected.overview_box);
        assert_eq!(layout.overview_field, expected.overview_field);
        assert_eq!(layout.thought_box, None);
        assert_eq!(layout.thought_content, None);
        assert_eq!(layout.split_divider, None);
        assert_eq!(layout.split_hitbox, None);
        assert_eq!(layout.footer_start_y, expected.footer_start_y);
    }

    #[test]
    fn for_terminal_with_ratio_uses_single_panel_on_tiny_terminals() {
        assert_without_thought_panel(WorkspaceLayout::for_terminal_with_ratio(0, 0, 0.5), 0, 0);
        assert_without_thought_panel(WorkspaceLayout::for_terminal_with_ratio(20, 8, 0.5), 20, 8);
    }

    #[test]
    fn for_terminal_with_ratio_respects_width_and_height_split_thresholds() {
        assert_without_thought_panel(
            WorkspaceLayout::for_terminal_with_ratio(99, 20, 0.5),
            99,
            20,
        );
        assert_without_thought_panel(
            WorkspaceLayout::for_terminal_with_ratio(120, 13, 0.5),
            120,
            13,
        );

        assert!(WorkspaceLayout::for_terminal_with_ratio(100, 20, 0.5)
            .thought_box
            .is_some());
        assert!(WorkspaceLayout::for_terminal_with_ratio(120, 14, 0.5)
            .thought_box
            .is_some());
    }

    #[test]
    fn for_terminal_with_ratio_clamps_extreme_ratios_to_panel_bounds() {
        let min_layout = WorkspaceLayout::for_terminal_with_ratio(120, 32, -1.0);
        let max_layout = WorkspaceLayout::for_terminal_with_ratio(120, 32, 2.0);

        assert_eq!(
            min_layout
                .thought_box
                .expect("wide terminal should split at the minimum ratio")
                .width,
            THOUGHT_RAIL_MIN_PANEL_WIDTH
        );
        assert_eq!(
            min_layout
                .overview_box
                .width
                .saturating_add(THOUGHT_RAIL_GAP + THOUGHT_RAIL_MIN_PANEL_WIDTH),
            min_layout.workspace_box.inset(1).width
        );

        assert_eq!(max_layout.overview_box.width, min_overview_width());
        assert_eq!(
            max_layout
                .thought_box
                .expect("wide terminal should split at the maximum ratio")
                .width,
            max_layout
                .workspace_box
                .inset(1)
                .width
                .saturating_sub(THOUGHT_RAIL_GAP + min_overview_width())
        );
    }

    #[test]
    fn for_terminal_with_ratio_treats_non_finite_ratios_as_default() {
        let default_layout =
            WorkspaceLayout::for_terminal_with_ratio(120, 32, THOUGHT_RAIL_DEFAULT_RATIO);

        for ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let layout = WorkspaceLayout::for_terminal_with_ratio(120, 32, ratio);
            assert_eq!(layout.thought_box, default_layout.thought_box);
            assert_eq!(layout.thought_content, default_layout.thought_content);
            assert_eq!(layout.overview_box, default_layout.overview_box);
            assert_eq!(layout.overview_field, default_layout.overview_field);
            assert_eq!(layout.split_divider, default_layout.split_divider);
            assert_eq!(layout.split_hitbox, default_layout.split_hitbox);
        }
    }
}
