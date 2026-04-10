use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteTheme {
    Fish,
    Balls,
    Jelly,
}

impl Default for SpriteTheme {
    fn default() -> Self {
        Self::Balls
    }
}

impl SpriteTheme {
    pub(crate) const fn override_options() -> [Option<Self>; 4] {
        [None, Some(Self::Fish), Some(Self::Balls), Some(Self::Jelly)]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Balls => "balls",
            Self::Jelly => "jelly",
        }
    }

    pub(crate) fn override_label(theme: Option<Self>) -> &'static str {
        theme.map(Self::label).unwrap_or("auto")
    }

    pub(crate) fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fish" => Some(Self::Fish),
            "balls" => Some(Self::Balls),
            "jelly" => Some(Self::Jelly),
            _ => None,
        }
    }

    pub(crate) fn from_repo_theme(theme: &RepoTheme) -> Option<Self> {
        theme.sprite.as_deref().and_then(Self::from_name)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SpriteKind {
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
    pub(crate) fn from_session(session: &SessionSummary) -> Self {
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

    pub(crate) fn speed_scale(self) -> f32 {
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

    pub(crate) fn bob_amplitude(self) -> f32 {
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

    pub(crate) fn frame_with_theme(self, tick: u64, theme: SpriteTheme) -> [&'static str; 4] {
        match theme {
            SpriteTheme::Fish => match self {
                Self::Active => active_frame(tick),
                Self::Busy => busy_frame(tick),
                Self::Drowsy => drowsy_frame(tick),
                Self::Sleeping => sleeping_frame(tick),
                Self::DeepSleep => deep_sleep_frame(tick),
                Self::Attention => attention_frame(tick),
                Self::Error => error_frame(tick),
                Self::Exited => exited_frame(tick),
            },
            SpriteTheme::Balls | SpriteTheme::Jelly => match self {
                Self::Active => ball_active_frame(tick),
                Self::Busy => ball_busy_frame(tick),
                Self::Drowsy => ball_drowsy_frame(tick),
                Self::Sleeping => ball_sleeping_frame(tick),
                Self::DeepSleep => ball_deep_sleep_frame(tick),
                Self::Attention => ball_attention_frame(tick),
                Self::Error => ball_error_frame(tick),
                Self::Exited => ball_exited_frame(tick),
            },
        }
    }
}

pub(crate) fn active_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn busy_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn drowsy_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn sleeping_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn attention_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn error_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn deep_sleep_frame(tick: u64) -> [&'static str; 4] {
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

pub(crate) fn exited_frame(tick: u64) -> [&'static str; 4] {
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

// Ball/sack sprites.  All frames are exactly 12 columns wide and 4 rows tall
// so they occupy the same entity slot as the fish.  The shape progresses from
// perky and round in `active` to dramatically saggy in `deep_sleep`: the top
// pinches in while the bottom bulges out, as if the sack is drooping under
// its own weight.

pub(crate) fn ball_active_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  .-~~~-.   ",
            " ( o   o )  ",
            "  '.___.'   ",
            "            ",
        ]
    } else {
        [
            "  .-~~~-.   ",
            " ( O   O )  ",
            "  '.___.'   ",
            "    ' '     ",
        ]
    }
}

pub(crate) fn ball_busy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  .~*~*~.   ",
            " ( O * O )  ",
            "  \\_____/   ",
            "   v   v    ",
        ]
    } else {
        [
            "  .~*~*~.   ",
            " ( * O * )  ",
            "  \\_____/   ",
            "    v v     ",
        ]
    }
}

pub(crate) fn ball_drowsy_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  .-----.   ",
            " ( -   - )  ",
            " (  ___  )  ",
            "  '-----'   ",
        ]
    } else {
        [
            "  .-----.   ",
            " ( _   _ )  ",
            " (  ___  )  ",
            "  '-----'   ",
        ]
    }
}

pub(crate) fn ball_sleeping_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "   .---.    ",
            "  ( z z )   ",
            " ( _____ )  ",
            "  '-----'   ",
        ]
    } else {
        [
            "   .---.    ",
            "  ( z Z )   ",
            " ( _____ )  ",
            "  '-----'   ",
        ]
    }
}

pub(crate) fn ball_deep_sleep_frame(tick: u64) -> [&'static str; 4] {
    // Reeeeally saggy: pinched top, dramatically bulging bottom, drooping low.
    if tick % 8 < 4 {
        [
            "    ,-,     ",
            "   ( - )    ",
            "  ( ___ )   ",
            " (_______)  ",
        ]
    } else {
        [
            "    ,-,     ",
            "   ( _ )    ",
            "  ( ___ )   ",
            " (_______)  ",
        ]
    }
}

pub(crate) fn ball_attention_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  .-!!!-.   ",
            " ( O ! O )  ",
            "  '.___.'   ",
            "     !      ",
        ]
    } else {
        [
            "  .-!!!-.   ",
            " ( ! O ! )  ",
            "  '.___.'   ",
            "    ! !     ",
        ]
    }
}

pub(crate) fn ball_error_frame(tick: u64) -> [&'static str; 4] {
    if tick % 8 < 4 {
        [
            "  .xx-xx.   ",
            " ( X _ X )  ",
            "  '._X_.'   ",
            "    \\ /     ",
        ]
    } else {
        [
            "  .xx-xx.   ",
            " ( x _ x )  ",
            "  '._x_.'   ",
            "    / \\     ",
        ]
    }
}

pub(crate) fn ball_exited_frame(tick: u64) -> [&'static str; 4] {
    let _ = tick;
    [
        "            ",
        "    _____   ",
        "   (_____)  ",
        "    '---'   ",
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestAnchor {
    FreeSwim,
    Bottom,
    Top,
}

#[derive(Clone)]
pub(crate) struct SessionEntity {
    pub(crate) session: SessionSummary,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) vx: f32,
    pub(crate) vy: f32,
    pub(crate) swim_anchor_x: f32,
    pub(crate) swim_anchor_y: f32,
    pub(crate) swim_center_y: f32,
    pub(crate) bob_phase: f32,
}

impl SessionEntity {
    pub(crate) fn new(session: SessionSummary, field: Rect) -> Self {
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

    pub(crate) fn sprite_kind(&self) -> SpriteKind {
        SpriteKind::from_session(&self.session)
    }

    pub(crate) fn rest_anchor(&self) -> RestAnchor {
        match self.sprite_kind() {
            SpriteKind::Sleeping => RestAnchor::Bottom,
            SpriteKind::DeepSleep | SpriteKind::Exited => RestAnchor::Top,
            _ => RestAnchor::FreeSwim,
        }
    }

    pub(crate) fn is_stationary(&self) -> bool {
        !matches!(self.rest_anchor(), RestAnchor::FreeSwim)
    }

    pub(crate) fn set_relative_position(&mut self, x: u16, y: u16) {
        self.x = x as f32;
        self.y = y as f32;
        self.swim_anchor_x = self.x;
        self.swim_anchor_y = self.y;
        self.swim_center_y = self.y;
    }

    pub(crate) fn tick(&mut self, field: Rect, tick: u64) {
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

    pub(crate) fn screen_rect(&self, field: Rect) -> Rect {
        Rect {
            x: field.x + self.x.max(0.0).round() as u16,
            y: field.y + self.y.max(0.0).round() as u16,
            width: ENTITY_WIDTH,
            height: ENTITY_HEIGHT,
        }
    }
}

pub(crate) fn stable_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn rest_grid_columns(field: Rect) -> usize {
    usize::from((field.width / ENTITY_WIDTH).max(1))
}

pub(crate) fn rest_grid_rows(field: Rect) -> usize {
    usize::from((field.height / ENTITY_HEIGHT).max(1))
}

pub(crate) fn bottom_rest_origin(field: Rect, slot: usize) -> (u16, u16) {
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

pub(crate) fn top_rest_origin(field: Rect, slot: usize) -> (u16, u16) {
    let columns = rest_grid_columns(field);
    let max_rows = rest_grid_rows(field).saturating_sub(1);
    let row = (slot / columns).min(max_rows);
    let column = slot % columns;
    (column as u16 * ENTITY_WIDTH, row as u16 * ENTITY_HEIGHT)
}

pub(crate) fn compare_sleepiness(left: &SessionSummary, right: &SessionSummary) -> Ordering {
    left.last_activity_at
        .cmp(&right.last_activity_at)
        .then_with(|| left.tmux_name.cmp(&right.tmux_name))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

pub(crate) fn separate_from_fixed_entity(entity: &mut SessionEntity, obstacle: Rect, field: Rect) {
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

pub(crate) fn swim_speed(hash: u64) -> f32 {
    let segment = (hash & 0xff) as f32 / 255.0;
    0.18 + segment * 0.22
}

pub(crate) fn vertical_drift(hash: u64) -> f32 {
    let segment = ((hash >> 8) & 0xff) as f32 / 255.0;
    let speed = 0.03 + segment * 0.05;
    if hash & 2 == 0 {
        speed
    } else {
        -speed
    }
}

pub(crate) fn bob_phase(hash: u64) -> f32 {
    ((hash >> 16) & 0xff) as f32 / 255.0 * TAU
}

pub(crate) fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.right() && a.right() > b.x && a.y < b.bottom() && a.bottom() > b.y
}
