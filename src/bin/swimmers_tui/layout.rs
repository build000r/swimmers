use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl WorkspaceLayout {
    #[allow(dead_code)]
    pub(crate) fn for_terminal(width: u16, height: u16) -> Self {
        Self::for_terminal_with_ratio(width, height, THOUGHT_RAIL_DEFAULT_RATIO)
    }

    pub(crate) fn for_terminal_with_ratio(width: u16, height: u16, thought_ratio: f32) -> Self {
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

    pub(crate) fn thought_entry_capacity(self) -> usize {
        self.thought_content
            .map(|content| content.height.saturating_sub(THOUGHT_RAIL_HEADER_ROWS) as usize)
            .unwrap_or(0)
    }

    pub(crate) fn thought_ratio_for_divider_x(self, x: u16) -> Option<f32> {
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
