use super::*;

impl<C: TuiApi> App<C> {
    pub(crate) fn resolve_collisions(&mut self, field: Rect) {
        let entity_count = self.entities.len();
        let throttle_collisions = entity_count > COLLISION_THROTTLE_ENTITY_THRESHOLD;

        for idx in 0..entity_count {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            if throttle_collisions && !right.is_empty() {
                let budget = COLLISION_THROTTLE_PAIR_BUDGET.min(right.len());
                let start = (self.tick as usize).wrapping_add(idx.saturating_mul(13)) % right.len();
                for step in 0..budget {
                    let b_index = (start + step) % right.len();
                    let b = &mut right[b_index];
                    Self::resolve_collision_pair(a, b, field);
                }
            } else {
                for b in right {
                    Self::resolve_collision_pair(a, b, field);
                }
            }
        }
    }

    fn resolve_collision_pair(a: &mut SessionEntity, b: &mut SessionEntity, field: Rect) {
        let a_rect = a.screen_rect(field);
        let b_rect = b.screen_rect(field);
        if !intersects(a_rect, b_rect) {
            return;
        }

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
