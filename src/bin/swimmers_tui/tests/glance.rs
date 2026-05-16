use super::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

const GLANCE_STATE_COVERAGE_FIXTURE: &str =
    include_str!("../../../../tests/fixtures/glance_state_coverage_10.json");

#[derive(Debug, Deserialize)]
struct GlanceFixtureManifest {
    version: u64,
    profile_id: String,
    sessions: Vec<GlanceFixtureSession>,
}

#[derive(Debug, Deserialize)]
struct GlanceFixtureSession {
    id: String,
    tmux_name: String,
    role: String,
    state: SessionState,
    rest_state: RestState,
    thought_state: ThoughtState,
    state_evidence: GlanceFixtureEvidence,
    transport_health: TransportHealth,
    current_command: Option<String>,
    expected_sprite: String,
    expected_label: String,
    requires_name_for_glance: bool,
}

#[derive(Debug, Deserialize)]
struct GlanceFixtureEvidence {
    cause: String,
    observed: bool,
}

fn sprite_kind_name(kind: SpriteKind) -> &'static str {
    match kind {
        SpriteKind::Active => "active",
        SpriteKind::Busy => "busy",
        SpriteKind::Drowsy => "drowsy",
        SpriteKind::Sleeping => "sleeping",
        SpriteKind::DeepSleep => "deep_sleep",
        SpriteKind::Attention => "attention",
        SpriteKind::Error => "error",
        SpriteKind::Exited => "exited",
    }
}

fn session_from_glance_fixture(fixture: &GlanceFixtureSession, tmux_name: &str) -> SessionSummary {
    let mut session = session_summary(&fixture.id, tmux_name, TEST_REPO_SWIMMERS);
    session.state = fixture.state;
    session.rest_state = fixture.rest_state;
    session.thought_state = fixture.thought_state;
    session.state_evidence = StateEvidence::with_observed_at(
        &fixture.state_evidence.cause,
        fixture.state_evidence.observed.then(Utc::now),
    );
    session.transport_health = fixture.transport_health;
    session.current_command = fixture.current_command.clone();
    session
}

fn load_glance_fixture_manifest() -> GlanceFixtureManifest {
    serde_json::from_str(GLANCE_STATE_COVERAGE_FIXTURE).expect("glance fixture manifest")
}

fn render_glance_fixture_frame(
    manifest: &GlanceFixtureManifest,
) -> (String, Vec<serde_json::Value>, u128) {
    let started = std::time::Instant::now();
    let field = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 32,
    };
    let mut renderer = test_renderer(field.width, field.height);
    let sessions = manifest
        .sessions
        .iter()
        .map(|fixture| session_from_glance_fixture(fixture, &fixture.tmux_name))
        .collect::<Vec<_>>();
    let entities = sessions
        .into_iter()
        .map(|session| SessionEntity::new(session, field))
        .collect::<Vec<_>>();
    let entity_refs = entities.iter().collect::<Vec<_>>();
    let repo_themes: HashMap<String, RepoTheme> = HashMap::new();
    render_balls_theme(&mut renderer, field, &entity_refs, None, &repo_themes, 0);
    let frame = (0..field.height)
        .map(|y| row_text(&renderer, y))
        .collect::<Vec<_>>()
        .join("\n");
    let elapsed_ms = started.elapsed().as_millis();
    let observations = manifest
        .sessions
        .iter()
        .zip(entities.iter())
        .map(|(fixture, entity)| {
            serde_json::json!({
                "id": fixture.id,
                "role": fixture.role,
                "state": fixture.state,
                "rest_state": fixture.rest_state,
                "transport_health": fixture.transport_health,
                "current_command": fixture.current_command,
                "expected_sprite": fixture.expected_sprite,
                "actual_sprite": sprite_kind_name(entity.sprite_kind()),
                "expected_label": fixture.expected_label,
                "actual_label": session_state_text(&entity.session),
                "tmux_name_used_for_state": false
            })
        })
        .collect::<Vec<_>>();

    (frame, observations, elapsed_ms)
}

fn write_glance_artifacts(
    manifest: &GlanceFixtureManifest,
    frame: &str,
    observations: &[serde_json::Value],
    elapsed_ms: u128,
) {
    let Ok(artifact_dir) = std::env::var("SWIMMERS_GLANCE_ARTIFACT_DIR") else {
        return;
    };
    let artifact_dir = std::path::PathBuf::from(artifact_dir);
    std::fs::create_dir_all(&artifact_dir).expect("create glance artifact dir");

    let raw_manifest: serde_json::Value =
        serde_json::from_str(GLANCE_STATE_COVERAGE_FIXTURE).expect("raw manifest json");
    std::fs::write(
        artifact_dir.join("sessions.json"),
        serde_json::to_vec_pretty(&raw_manifest["sessions"]).expect("serialize sessions artifact"),
    )
    .expect("write sessions artifact");
    std::fs::write(artifact_dir.join("tui-frame.txt"), frame).expect("write TUI frame artifact");
    std::fs::write(
        artifact_dir.join("state-observations.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "profile_id": manifest.profile_id,
            "session_count": manifest.sessions.len(),
            "first_frame_elapsed_ms": elapsed_ms,
            "observations": observations,
        }))
        .expect("serialize state observations"),
    )
    .expect("write state observations artifact");
}

fn write_native_handoff_artifact(payload: serde_json::Value) {
    let Ok(artifact_dir) = std::env::var("SWIMMERS_GLANCE_ARTIFACT_DIR") else {
        return;
    };
    let artifact_dir = std::path::PathBuf::from(artifact_dir);
    std::fs::create_dir_all(&artifact_dir).expect("create glance artifact dir");
    std::fs::write(
        artifact_dir.join("native-open.json"),
        serde_json::to_vec_pretty(&payload).expect("serialize native handoff artifact"),
    )
    .expect("write native handoff artifact");
}

fn fixture_id_for_role(manifest: &GlanceFixtureManifest, role: &str) -> String {
    manifest
        .sessions
        .iter()
        .find(|fixture| fixture.role == role)
        .map(|fixture| fixture.id.clone())
        .unwrap_or_else(|| panic!("missing fixture role {role}"))
}

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
fn fixture_manifest_matches_source_state_predicates() {
    let manifest = load_glance_fixture_manifest();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.profile_id, "glance_state_coverage_10");
    assert_eq!(
        manifest.sessions.len(),
        10,
        "Vision Glance fixture must stay at exactly 10 sessions",
    );

    let mut ids = HashSet::new();
    let mut roles = HashSet::new();
    let mut busy_running = 0usize;
    let mut idle_states = 0usize;

    for (index, fixture) in manifest.sessions.iter().enumerate() {
        assert!(ids.insert(fixture.id.as_str()), "duplicate fixture id");
        roles.insert(fixture.role.as_str());
        assert!(
            !fixture.requires_name_for_glance,
            "{} must not require reading names for state detection",
            fixture.id,
        );

        let session = session_from_glance_fixture(fixture, &fixture.tmux_name);
        assert_eq!(
            sprite_kind_name(SpriteKind::from_session(&session)),
            fixture.expected_sprite,
            "{} sprite should come from SessionSummary fields",
            fixture.id,
        );
        assert_eq!(
            session_state_text(&session),
            fixture.expected_label,
            "{} label should come from source predicates",
            fixture.id,
        );

        let renamed = session_from_glance_fixture(fixture, &format!("renamed-{index}"));
        assert_eq!(
            SpriteKind::from_session(&renamed),
            SpriteKind::from_session(&session),
            "{} sprite changed after tmux name rewrite",
            fixture.id,
        );
        assert_eq!(
            session_state_text(&renamed),
            session_state_text(&session),
            "{} label changed after tmux name rewrite",
            fixture.id,
        );

        if fixture.state == SessionState::Busy && fixture.current_command.is_some() {
            busy_running += 1;
        }
        if fixture.state == SessionState::Idle {
            idle_states += 1;
        }
    }

    for required_role in [
        "ai_agent_compiling",
        "running_tests",
        "idle",
        "awaiting_user",
        "errored",
        "exited",
        "stale_degraded",
    ] {
        assert!(
            roles.contains(required_role),
            "fixture profile missing required role {required_role}",
        );
    }
    assert!(
        busy_running >= 4,
        "fixture must include mixed running sessions, got {busy_running}",
    );
    assert_eq!(idle_states, 2, "fixture must include two idle states");
}

#[test]
fn fixture_manifest_renders_first_frame_artifacts() {
    let manifest = load_glance_fixture_manifest();
    let (frame, observations, elapsed_ms) = render_glance_fixture_frame(&manifest);

    assert!(
        elapsed_ms <= 2_000,
        "fixture first frame took {elapsed_ms}ms, above the 2s Glance target",
    );
    assert!(
        frame.contains('*'),
        "busy fixture state must be visible in first frame\n{frame}",
    );
    assert!(
        frame.contains('!'),
        "attention fixture state must be visible in first frame\n{frame}",
    );
    assert!(
        frame.contains('x'),
        "error/exited fixture states must be visible in first frame\n{frame}",
    );
    assert!(
        frame.contains('z'),
        "sleeping fixture state must be visible in first frame\n{frame}",
    );
    assert!(
        frame.contains("( ? )") && frame.contains(".?."),
        "stale/degraded fixture state must render the ghost overlay\n{frame}",
    );

    for observation in &observations {
        assert_eq!(
            observation["expected_sprite"], observation["actual_sprite"],
            "sprite mismatch in glance observation: {observation}",
        );
        assert_eq!(
            observation["expected_label"], observation["actual_label"],
            "label mismatch in glance observation: {observation}",
        );
        assert_eq!(observation["tmux_name_used_for_state"], false);
    }

    write_glance_artifacts(&manifest, &frame, &observations, elapsed_ms);
}

#[test]
fn fixture_manifest_native_handoff_targets_errored_and_attention_once() {
    let manifest = load_glance_fixture_manifest();
    let errored_id = fixture_id_for_role(&manifest, "errored");
    let attention_id = fixture_id_for_role(&manifest, "awaiting_user");
    let sessions = manifest
        .sessions
        .iter()
        .map(|fixture| session_from_glance_fixture(fixture, &fixture.tmux_name))
        .collect::<Vec<_>>();

    let api = MockApi::new();
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: errored_id.clone(),
        status: "focused".to_string(),
        pane_id: Some("%glance-error".to_string()),
    }));
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: attention_id.clone(),
        status: "focused".to_string(),
        pane_id: Some("%glance-attention".to_string()),
    }));

    let field = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 32,
    };
    let mut app = make_app(api.clone());
    app.merge_sessions(sessions, field);
    poll_until_selection_publication(&mut app);
    let initial_publish_calls = api.publish_calls();

    app.selected_id = Some(errored_id.clone());
    app.open_selected();
    assert!(app.pending_interaction.is_some());
    app.open_selected();
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("wait for the current action to finish"),
        "duplicate open while pending should not churn focus",
    );
    poll_until_interaction(&mut app);
    poll_until_selection_publication(&mut app);
    assert_eq!(api.open_calls(), vec![errored_id.clone()]);

    app.selected_id = Some(attention_id.clone());
    app.open_selected();
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    poll_until_selection_publication(&mut app);

    assert_eq!(
        api.open_calls(),
        vec![errored_id.clone(), attention_id.clone()],
        "native handoff should target selected errored and attention fixtures exactly once",
    );
    let expected_target_publications = [Some(errored_id.clone()), Some(attention_id.clone())];
    assert!(
        api.publish_calls().ends_with(&expected_target_publications),
        "selection publication should track the exact handoff targets",
    );

    write_native_handoff_artifact(serde_json::json!({
        "profile_id": manifest.profile_id,
        "mode": "simulated_native_handoff",
        "requires_real_iterm_or_ghostty": false,
        "duplicate_pending_open_suppressed": true,
        "targets": [
            {
                "role": "errored",
                "session_id": errored_id,
                "status": "focused",
                "pane_id": "%glance-error"
            },
            {
                "role": "awaiting_user",
                "session_id": attention_id,
                "status": "focused",
                "pane_id": "%glance-attention"
            }
        ],
        "open_calls": api.open_calls(),
        "initial_publish_calls": initial_publish_calls,
        "publish_calls": api.publish_calls()
    }));
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
