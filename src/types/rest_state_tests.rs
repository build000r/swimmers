use super::*;

fn base() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn tc1_freshly_active_idle_stays_active() {
    let now = base();
    let last = now - Duration::seconds(5);
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, last, now),
        RestState::Active
    );
}

#[test]
fn tc2_two_minutes_silent_becomes_drowsy() {
    let now = base();
    let last = now - Duration::minutes(2);
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, last, now),
        RestState::Drowsy
    );
}

#[test]
fn tc3_long_idle_stays_drowsy_without_thought_daemon() {
    let now = base();
    let last = now - Duration::minutes(10);
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, last, now),
        RestState::Drowsy
    );
}

#[test]
fn tc4_hours_silent_stays_drowsy_without_thought_daemon() {
    let now = base();
    let last = now - Duration::hours(2);
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, last, now),
        RestState::Drowsy
    );
}

#[test]
fn tc5_busy_session_ignores_idle_duration() {
    let now = base();
    let last = now - Duration::hours(1);
    assert_eq!(
        rest_state_from_idle(SessionState::Busy, last, now),
        RestState::Active
    );
}

#[test]
fn tc5b_attention_session_ignores_idle_duration() {
    let now = base();
    let last = now - Duration::minutes(10);
    assert_eq!(
        rest_state_from_idle(SessionState::Attention, last, now),
        RestState::Active,
        "attention-flagged sessions must keep animating until dismissed"
    );
}

#[test]
fn tc7_exited_stays_deep_sleep() {
    let now = base();
    let last = now - Duration::seconds(1);
    assert_eq!(
        rest_state_from_idle(SessionState::Exited, last, now),
        RestState::DeepSleep
    );
}

#[test]
fn tc11_future_last_activity_resolves_to_active() {
    let now = base();
    let last = now + Duration::minutes(1);
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, last, now),
        RestState::Active,
        "clock skew must not panic or sleep the session"
    );
}

#[test]
fn threshold_boundaries() {
    let now = base();
    // Exactly at drowsy threshold → Drowsy
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, now - REST_STATE_DROWSY_AFTER, now),
        RestState::Drowsy
    );
    // Long-idle fallback remains Drowsy; sleeping requires transcript state.
    assert_eq!(
        rest_state_from_idle(SessionState::Idle, now - Duration::hours(3), now),
        RestState::Drowsy
    );
}

#[test]
fn fallback_rest_state_unchanged() {
    // TC-10: fallback_rest_state must keep its existing behavior for
    // preserved call sites (stale-session path, test fixtures).
    assert_eq!(
        fallback_rest_state(SessionState::Exited, ThoughtState::Holding),
        RestState::DeepSleep
    );
    assert_eq!(
        fallback_rest_state(SessionState::Idle, ThoughtState::Holding),
        RestState::Drowsy
    );
    assert_eq!(
        fallback_rest_state(SessionState::Busy, ThoughtState::Holding),
        RestState::Active
    );
}
