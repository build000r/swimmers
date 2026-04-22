use super::*;
use std::collections::HashSet;

// docs/VISION.md "Product Test" depends on these state->label mappings being
// glance-distinct so users can identify status in seconds without reading text.
#[test]
fn glance_product_test_state_to_sprite_label_mapping_is_canonical_and_distinct() {
    let mut labels: HashSet<&'static str> = HashSet::new();

    let mut idle_active = session_summary("glance-1", "1", TEST_REPO_SWIMMERS);
    idle_active.state = SessionState::Idle;
    idle_active.rest_state = RestState::Active;
    assert!(matches!(
        SpriteKind::from_session(&idle_active),
        SpriteKind::Active
    ));
    let label = session_state_text(&idle_active);
    assert_eq!(label, "active");
    labels.insert(label);

    let mut idle_drowsy = session_summary("glance-2", "2", TEST_REPO_SWIMMERS);
    idle_drowsy.state = SessionState::Idle;
    idle_drowsy.rest_state = RestState::Drowsy;
    assert!(matches!(
        SpriteKind::from_session(&idle_drowsy),
        SpriteKind::Drowsy
    ));
    let label = session_state_text(&idle_drowsy);
    assert_eq!(label, "drowsy");
    labels.insert(label);

    let mut idle_sleeping = session_summary("glance-3", "3", TEST_REPO_SWIMMERS);
    idle_sleeping.state = SessionState::Idle;
    idle_sleeping.rest_state = RestState::Sleeping;
    assert!(matches!(
        SpriteKind::from_session(&idle_sleeping),
        SpriteKind::Sleeping
    ));
    let label = session_state_text(&idle_sleeping);
    assert_eq!(label, "sleeping");
    labels.insert(label);

    let mut attention_active = session_summary("glance-4", "4", TEST_REPO_SWIMMERS);
    attention_active.state = SessionState::Attention;
    attention_active.rest_state = RestState::Active;
    assert!(matches!(
        SpriteKind::from_session(&attention_active),
        SpriteKind::Attention
    ));
    let label = session_state_text(&attention_active);
    assert_eq!(label, "attention");
    labels.insert(label);

    let mut busy = session_summary("glance-5", "5", TEST_REPO_SWIMMERS);
    busy.state = SessionState::Busy;
    busy.rest_state = RestState::DeepSleep;
    assert!(matches!(SpriteKind::from_session(&busy), SpriteKind::Busy));
    let label = session_state_text(&busy);
    assert_eq!(label, "busy");
    labels.insert(label);

    let mut error = session_summary("glance-6", "6", TEST_REPO_SWIMMERS);
    error.state = SessionState::Error;
    error.rest_state = RestState::Sleeping;
    assert!(matches!(
        SpriteKind::from_session(&error),
        SpriteKind::Error
    ));
    let label = session_state_text(&error);
    assert_eq!(label, "error");
    labels.insert(label);

    let mut exited = session_summary("glance-7", "7", TEST_REPO_SWIMMERS);
    exited.state = SessionState::Exited;
    exited.rest_state = RestState::Drowsy;
    assert!(matches!(
        SpriteKind::from_session(&exited),
        SpriteKind::Exited
    ));
    let label = session_state_text(&exited);
    assert_eq!(label, "exited");
    labels.insert(label);

    assert_eq!(
        labels.len(),
        7,
        "canonical glance labels must stay distinct; collapsing labels breaks Product Test readability"
    );
}
