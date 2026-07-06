use super::*;

impl<C: TuiApi> App<C> {
    pub(crate) fn resolve_collisions(&mut self, field: Rect) {
        let entity_count = self.entities.len();
        let tick = self.tick;

        for idx in 0..entity_count {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            for b_index in selected_pair_indices(entity_count, right.len(), idx, tick) {
                let b = &mut right[b_index];
                Self::resolve_collision_pair(a, b, field);
            }
        }
    }

    fn resolve_collision_pair(a: &mut SessionEntity, b: &mut SessionEntity, field: Rect) {
        let a_rect = a.screen_rect(field);
        let b_rect = b.screen_rect(field);
        if intersects(a_rect, b_rect) {
            resolve_intersecting_pair(a, b, a_rect, b_rect, field);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollisionPairSelection {
    All,
    Throttled { start: usize, budget: usize },
}

impl CollisionPairSelection {
    fn len(self, right_len: usize) -> usize {
        match self {
            Self::All => right_len,
            Self::Throttled { budget, .. } => budget,
        }
    }

    fn index_for_step(self, step: usize, right_len: usize) -> usize {
        match self {
            Self::All => step,
            Self::Throttled { start, .. } => (start + step) % right_len,
        }
    }
}

fn selected_pair_indices(
    entity_count: usize,
    right_len: usize,
    entity_index: usize,
    tick: u64,
) -> impl Iterator<Item = usize> {
    let selection = collision_pair_selection(entity_count, right_len, entity_index, tick);
    (0..selection.len(right_len)).map(move |step| selection.index_for_step(step, right_len))
}

fn collision_pair_selection(
    entity_count: usize,
    right_len: usize,
    entity_index: usize,
    tick: u64,
) -> CollisionPairSelection {
    if throttle_collision_pairs(entity_count, right_len) {
        throttled_pair_selection(entity_index, right_len, tick)
    } else {
        CollisionPairSelection::All
    }
}

fn throttle_collision_pairs(entity_count: usize, right_len: usize) -> bool {
    entity_count > COLLISION_THROTTLE_ENTITY_THRESHOLD && right_len > 0
}

fn throttled_pair_selection(
    entity_index: usize,
    right_len: usize,
    tick: u64,
) -> CollisionPairSelection {
    CollisionPairSelection::Throttled {
        start: collision_rotation_start(entity_index, right_len, tick),
        budget: COLLISION_THROTTLE_PAIR_BUDGET.min(right_len),
    }
}

fn collision_rotation_start(entity_index: usize, right_len: usize, tick: u64) -> usize {
    (tick as usize).wrapping_add(entity_index.saturating_mul(13)) % right_len
}

fn resolve_intersecting_pair(
    a: &mut SessionEntity,
    b: &mut SessionEntity,
    a_rect: Rect,
    b_rect: Rect,
    field: Rect,
) {
    match (a.is_stationary(), b.is_stationary()) {
        (true, true) => {}
        (true, false) => separate_from_fixed_entity(b, a_rect, field),
        (false, true) => separate_from_fixed_entity(a, b_rect, field),
        (false, false) => separate_moving_pair(a, b, field),
    }
}

fn separate_moving_pair(a: &mut SessionEntity, b: &mut SessionEntity, field: Rect) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use swimmers::types::{StateEvidence, ThoughtSource, ThoughtState};

    fn pair_indices(
        entity_count: usize,
        right_len: usize,
        entity_index: usize,
        tick: u64,
    ) -> Vec<usize> {
        selected_pair_indices(entity_count, right_len, entity_index, tick).collect()
    }

    #[test]
    fn under_threshold_collision_selection_visits_all_pairs_in_order() {
        assert_eq!(
            pair_indices(COLLISION_THROTTLE_ENTITY_THRESHOLD, 4, 2, 99),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn over_threshold_collision_selection_applies_budget_and_tick_rotation() {
        let indices = pair_indices(
            COLLISION_THROTTLE_ENTITY_THRESHOLD + 1,
            COLLISION_THROTTLE_PAIR_BUDGET + 4,
            1,
            25,
        );

        assert_eq!(indices.len(), COLLISION_THROTTLE_PAIR_BUDGET);
        assert_eq!(
            indices,
            vec![
                10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 0, 1, 2, 3,
                4, 5,
            ]
        );
    }

    #[test]
    fn moving_entity_separates_from_fixed_collision_partner() {
        let field = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 30,
        };
        let mut moving = entity_at(field, "moving", RestState::Active, 11.0, 5.0);
        let mut fixed = entity_at(field, "fixed", RestState::Sleeping, 12.0, 5.0);
        let fixed_before = fixed.clone();

        App::<ApiClient>::resolve_collision_pair(&mut moving, &mut fixed, field);

        assert_moving_was_separated(&moving);
        assert_entity_motion_eq(&fixed, &fixed_before);

        let mut fixed = entity_at(field, "fixed-first", RestState::Sleeping, 12.0, 5.0);
        let mut moving = entity_at(field, "moving-second", RestState::Active, 11.0, 5.0);
        let fixed_before = fixed.clone();

        App::<ApiClient>::resolve_collision_pair(&mut fixed, &mut moving, field);

        assert_moving_was_separated(&moving);
        assert_entity_motion_eq(&fixed, &fixed_before);
    }

    fn entity_at(
        field: Rect,
        session_id: &str,
        rest_state: RestState,
        x: f32,
        y: f32,
    ) -> SessionEntity {
        let mut entity = SessionEntity::new(test_session(session_id, rest_state), field);
        entity.x = x;
        entity.y = y;
        entity.vx = 0.25;
        entity.vy = -0.5;
        entity.swim_anchor_x = x;
        entity.swim_anchor_y = y;
        entity.swim_center_y = y;
        entity
    }

    fn test_session(session_id: &str, rest_state: RestState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: session_id.to_string(),
            tmux_target: swimmers::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            current_command: None,
            state_evidence: StateEvidence::new("test"),
            cwd: "/tmp".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
            environment: Default::default(),
        }
    }

    fn assert_entity_motion_eq(actual: &SessionEntity, expected: &SessionEntity) {
        assert_eq!(actual.x, expected.x);
        assert_eq!(actual.y, expected.y);
        assert_eq!(actual.vx, expected.vx);
        assert_eq!(actual.vy, expected.vy);
        assert_eq!(actual.swim_anchor_x, expected.swim_anchor_x);
        assert_eq!(actual.swim_anchor_y, expected.swim_anchor_y);
        assert_eq!(actual.swim_center_y, expected.swim_center_y);
    }

    fn assert_moving_was_separated(entity: &SessionEntity) {
        assert_eq!(entity.x, 0.0);
        assert_eq!(entity.y, 10.0);
        assert_eq!(entity.vx, -0.25);
        assert_eq!(entity.vy, 0.5);
        assert_eq!(entity.swim_anchor_x, entity.x);
        assert_eq!(entity.swim_anchor_y, entity.y);
        assert_eq!(entity.swim_center_y, entity.y);
    }
}
