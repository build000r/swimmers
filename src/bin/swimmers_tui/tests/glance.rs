use super::*;
use std::collections::{HashMap, HashSet};

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

    let mut attention_sleeping = session_summary("glance-4b", "4b", TEST_REPO_SWIMMERS);
    attention_sleeping.state = SessionState::Attention;
    attention_sleeping.rest_state = RestState::Sleeping;
    assert!(matches!(
        SpriteKind::from_session(&attention_sleeping),
        SpriteKind::Attention
    ));
    assert_eq!(session_state_text(&attention_sleeping), "attention");

    let mut busy = session_summary("glance-5", "5", TEST_REPO_SWIMMERS);
    busy.state = SessionState::Busy;
    busy.rest_state = RestState::Active;
    assert!(matches!(SpriteKind::from_session(&busy), SpriteKind::Busy));
    let label = session_state_text(&busy);
    assert_eq!(label, "busy");
    labels.insert(label);

    let mut busy_sleeping = session_summary("glance-6", "6", TEST_REPO_SWIMMERS);
    busy_sleeping.state = SessionState::Busy;
    busy_sleeping.thought_state = ThoughtState::Sleeping;
    busy_sleeping.rest_state = RestState::Sleeping;
    assert!(matches!(
        SpriteKind::from_session(&busy_sleeping),
        SpriteKind::Busy
    ));
    let label = session_state_text(&busy_sleeping);
    assert_eq!(label, "busy");
    labels.insert(label);

    let mut error = session_summary("glance-7", "7", TEST_REPO_SWIMMERS);
    error.state = SessionState::Error;
    error.rest_state = RestState::Sleeping;
    assert!(matches!(
        SpriteKind::from_session(&error),
        SpriteKind::Error
    ));
    let label = session_state_text(&error);
    assert_eq!(label, "error");
    labels.insert(label);

    let mut exited = session_summary("glance-8", "8", TEST_REPO_SWIMMERS);
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

#[test]
fn low_evidence_state_is_annotated_in_glance_label() {
    let mut busy = session_summary("glance-low", "low", TEST_REPO_SWIMMERS);
    busy.state = SessionState::Busy;
    busy.rest_state = RestState::Active;
    busy.state_evidence = StateEvidence::unobserved("summary_cache_degraded");

    assert_eq!(session_state_text(&busy), "busy?");
}

#[test]
fn observed_medium_evidence_keeps_glance_label_clean() {
    let mut busy = session_summary("glance-medium", "medium", TEST_REPO_SWIMMERS);
    busy.state = SessionState::Busy;
    busy.rest_state = RestState::Active;
    busy.state_evidence = StateEvidence::new("local_input");

    assert_eq!(session_state_text(&busy), "busy");
}

// ---------------------------------------------------------------------------
// Golden visual snapshot: render_balls_theme cord + body + drop, per
// (state, rest_state, StateEvidence) tuple. The earlier label-only Glance Test
// did not catch the silent-fallback bug where an unobserved cached state
// rendered an identical-looking ball to a fresh one. These assertions lock in
// the warning encoding (`'` cord + `( ? )` body for unverified) and prove every
// canonical verified state still renders distinctly.
// ---------------------------------------------------------------------------

fn render_single_ball(session: SessionSummary) -> Vec<String> {
    // Tall enough that even DeepSleep / Exited drops fit (base drop is 12 +
    // age-bonus + body height). 18 rows leaves room for cord and floor row.
    let field = Rect {
        x: 0,
        y: 0,
        width: 24,
        height: 18,
    };
    let mut renderer = test_renderer(field.width, field.height);
    let entity = SessionEntity::new(session, field);
    let entities: Vec<&SessionEntity> = vec![&entity];
    let repo_themes: HashMap<String, RepoTheme> = HashMap::new();
    render_balls_theme(&mut renderer, field, &entities, None, &repo_themes, 0);

    (0..field.height).map(|y| row_text(&renderer, y)).collect()
}

fn frame_to_string(frame: &[String]) -> String {
    frame.join("\n")
}

#[derive(Clone, Copy)]
struct StateCase {
    name: &'static str,
    state: SessionState,
    rest_state: RestState,
    expected_kind: SpriteKind,
    /// The per-kind cord character drawn when StateEvidence is high+observed.
    /// Must stay in sync with render_balls_theme_ball.
    verified_cord: char,
    /// A glyph from the kind-specific body that must appear in the verified
    /// rendering and must NOT appear in the ghost rendering. `' '` means
    /// "no kind-unique marker" (e.g. Active is a smooth `(   )`).
    verified_body_marker: char,
}

const VERIFIED_CASES: &[StateCase] = &[
    StateCase {
        name: "active",
        state: SessionState::Idle,
        rest_state: RestState::Active,
        expected_kind: SpriteKind::Active,
        verified_cord: '|',
        verified_body_marker: ' ',
    },
    StateCase {
        name: "drowsy",
        state: SessionState::Idle,
        rest_state: RestState::Drowsy,
        expected_kind: SpriteKind::Drowsy,
        verified_cord: '|',
        verified_body_marker: ' ',
    },
    StateCase {
        name: "sleeping",
        state: SessionState::Idle,
        rest_state: RestState::Sleeping,
        expected_kind: SpriteKind::Sleeping,
        verified_cord: '|',
        verified_body_marker: 'z',
    },
    StateCase {
        name: "deep_sleep",
        state: SessionState::Idle,
        rest_state: RestState::DeepSleep,
        expected_kind: SpriteKind::DeepSleep,
        verified_cord: '|',
        verified_body_marker: 'z',
    },
    StateCase {
        name: "busy",
        state: SessionState::Busy,
        rest_state: RestState::Active,
        expected_kind: SpriteKind::Busy,
        verified_cord: ':',
        verified_body_marker: '*',
    },
    StateCase {
        name: "attention",
        state: SessionState::Attention,
        rest_state: RestState::Active,
        expected_kind: SpriteKind::Attention,
        verified_cord: '!',
        verified_body_marker: '!',
    },
    StateCase {
        name: "error",
        state: SessionState::Error,
        rest_state: RestState::Active,
        expected_kind: SpriteKind::Error,
        verified_cord: 'x',
        verified_body_marker: 'x',
    },
    StateCase {
        name: "exited",
        state: SessionState::Exited,
        rest_state: RestState::DeepSleep,
        expected_kind: SpriteKind::Exited,
        verified_cord: 'x',
        verified_body_marker: 'x',
    },
];

fn case_session(case: StateCase, evidence: StateEvidence) -> SessionSummary {
    let id = format!("ball-{}", case.name);
    let mut session = session_summary(&id, case.name, TEST_REPO_SWIMMERS);
    session.state = case.state;
    session.rest_state = case.rest_state;
    session.state_evidence = evidence;
    assert_eq!(
        SpriteKind::from_session(&session),
        case.expected_kind,
        "case {} should map to {:?} sprite kind",
        case.name,
        case.expected_kind,
    );
    session
}

#[test]
fn glance_balls_theme_verified_states_render_distinct_cord_and_body() {
    let mut frames: HashMap<&'static str, String> = HashMap::new();
    for case in VERIFIED_CASES {
        let session = case_session(*case, StateEvidence::new("osc133_prompt"));
        let frame = render_single_ball(session);
        let blob = frame_to_string(&frame);

        // Verified rendering must NOT carry the ghost overlay.
        assert!(
            !blob.contains("( ? )") && !blob.contains(".?."),
            "case {}: verified ball must not draw ghost overlay\n{}",
            case.name,
            blob,
        );
        // Per-kind cord char must appear at least once below the water line.
        let cord_count = blob.chars().filter(|c| *c == case.verified_cord).count();
        assert!(
            cord_count >= 1,
            "case {}: expected verified cord char {:?} to appear, got\n{}",
            case.name,
            case.verified_cord,
            blob,
        );
        // Per-kind body marker (e.g. `*` for busy) must appear when defined.
        if case.verified_body_marker != ' ' {
            assert!(
                blob.contains(case.verified_body_marker),
                "case {}: expected body marker {:?} in rendering\n{}",
                case.name,
                case.verified_body_marker,
                blob,
            );
        }
        frames.insert(case.name, blob);
    }

    // All eight verified frames must be pairwise distinct so the operator can
    // tell each state apart at a glance even with no color (the same property
    // the Product Test relies on for label distinctness).
    let unique: HashSet<&String> = frames.values().collect();
    assert_eq!(
        unique.len(),
        VERIFIED_CASES.len(),
        "verified ball frames collapsed; pairwise rendering must stay distinct",
    );
}

#[test]
fn glance_balls_theme_unverified_state_renders_ghost_overlay() {
    // Pick an honestly-busy session whose state cache went stale (the exact
    // Rank-1 root-cause path: supervisor.rs collect_live_summaries fallback).
    let case = VERIFIED_CASES
        .iter()
        .find(|case| case.name == "busy")
        .copied()
        .expect("busy verified case");
    let session = case_session(case, StateEvidence::unobserved("summary_cache_overloaded"));
    let frame = render_single_ball(session);
    let blob = frame_to_string(&frame);

    assert!(
        blob.contains("( ? )"),
        "unverified ball must draw `( ? )` body to flag uncertainty\n{}",
        blob,
    );
    assert!(
        blob.contains(".?."),
        "unverified ball must draw `.?.` top so the ghost mark is visible above the body\n{}",
        blob,
    );
    assert!(
        blob.contains('\''),
        "unverified ball must draw a sparse `'` cord, not the per-kind solid cord\n{}",
        blob,
    );
    // The verified-busy cord (`:`) appears inside the water-body row already.
    // What we want to prove is that the *cord descending to the ball* is no
    // longer the per-kind char. That cord lives in the column under the ball
    // anchor — find a row where the ghost cord is the ONLY non-space char and
    // confirm it is `'` not `:`.
    let mut saw_ghost_cord_row = false;
    for line in &frame {
        let trimmed = line.trim();
        if trimmed.len() == 1 && trimmed.starts_with('\'') {
            saw_ghost_cord_row = true;
            break;
        }
    }
    assert!(
        saw_ghost_cord_row,
        "expected at least one cord-only row to be `'`, got frame:\n{}",
        blob,
    );
}

#[test]
fn glance_balls_theme_observed_medium_confidence_stays_verified_visual() {
    // TUI sessions commonly settle through output-silence / local-input
    // heuristics. Those are observed medium-confidence detector events, not
    // stale cache fallbacks, so they should not turn the whole aquarium grey.
    let case = VERIFIED_CASES
        .iter()
        .find(|case| case.name == "active")
        .copied()
        .expect("active verified case");
    let session = case_session(case, StateEvidence::new("local_input"));
    assert_eq!(
        session_state_text(&session),
        "active",
        "label predicate baseline",
    );
    let frame = render_single_ball(session);
    let blob = frame_to_string(&frame);
    assert!(
        !blob.contains("( ? )") && !blob.contains(".?."),
        "observed medium-confidence ball must not draw ghost overlay\n{}",
        blob,
    );
}

#[test]
fn glance_balls_theme_unverified_ghost_distinct_from_verified_active() {
    // The actual operator question: when I see a ball, can I tell unverified
    // from verified at a glance? Compare the busy-stale ghost frame with the
    // verified-busy frame and require them to differ.
    let case = VERIFIED_CASES
        .iter()
        .find(|case| case.name == "busy")
        .copied()
        .expect("busy verified case");
    let verified = render_single_ball(case_session(case, StateEvidence::new("osc133_prompt")));
    let ghost = render_single_ball(case_session(
        case,
        StateEvidence::unobserved("summary_cache_overloaded"),
    ));
    assert_ne!(
        frame_to_string(&verified),
        frame_to_string(&ghost),
        "verified and ghost busy renderings must be visually distinguishable; \
         this assertion is the regression guard for the silent-fallback bug",
    );
}
