fn visible_entity_ids(app: &App<MockApi>) -> Vec<String> {
    app.visible_entities()
        .into_iter()
        .map(|entity| entity.session.session_id.clone())
        .collect()
}

fn session_summary(session_id: &str, tmux_name: &str, cwd: &str) -> SessionSummary {
    SessionSummary {
        session_id: session_id.to_string(),
        tmux_name: tmux_name.to_string(),
        state: SessionState::Idle,
        current_command: None,
        state_evidence: StateEvidence::new("osc133_prompt"),
        cwd: cwd.to_string(),
        tool: Some("Codex".to_string()),
        token_count: 0,
        context_limit: 192_000,
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: RestState::Drowsy,
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
    }
}

fn healthy_backend_health() -> BackendHealthResponse {
    BackendHealthResponse {
        status: "healthy".to_string(),
        thought_bridge: BackendThoughtBridgeHealth {
            status: "healthy".to_string(),
            ..BackendThoughtBridgeHealth::default()
        },
        persistence: BackendPersistenceHealth {
            available: true,
            ok: true,
            last_successful_operation: Some("save_sessions".to_string()),
            ..BackendPersistenceHealth::default()
        },
        dependencies: Some(BackendDependencyLedger {
            tmux_discovery: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            tmux_capture: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            native_scripts: BackendDependencySnapshot {
                status: "healthy".to_string(),
                last_error: None,
            },
            remote_targets: BackendDependencySnapshot {
                status: "not_configured".to_string(),
                last_error: None,
            },
        }),
    }
}

fn with_batch(
    mut session: SessionSummary,
    id: &str,
    label: &str,
    index: usize,
    total: usize,
) -> SessionSummary {
    session.batch = Some(SessionBatchMembership {
        id: id.to_string(),
        label: label.to_string(),
        index,
        total,
        created_at: Utc::now(),
        prompt_excerpt: Some(label.to_string()),
    });
    session
}

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("valid timestamp")
        .with_timezone(&Utc)
}

fn session_summary_with_thought(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    thought: &str,
    updated_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought = Some(thought.to_string());
    session.thought_state = ThoughtState::Active;
    session.rest_state = RestState::Active;
    session.thought_updated_at = Some(timestamp(updated_at));
    session
}

fn sleeping_session_with_thought(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    thought: &str,
    updated_at: &str,
) -> SessionSummary {
    let mut session = session_summary_with_thought(session_id, tmux_name, cwd, thought, updated_at);
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::Sleeping;
    session
}

fn mermaid_artifact(
    session_id: &str,
    path: &str,
    updated_at: &str,
    source: &str,
) -> MermaidArtifactResponse {
    let slice_name = swimmers::session::artifacts::extract_mmd_slice_name(path).map(str::to_owned);
    MermaidArtifactResponse {
        session_id: session_id.to_string(),
        available: true,
        path: Some(path.to_string()),
        updated_at: Some(timestamp(updated_at)),
        source: Some(source.to_string()),
        error: None,
        slice_name,
        plan_files: None,
    }
}

fn session_skill(name: &str, source_bucket: &str, source: &str, path: &str) -> SessionSkillSummary {
    SessionSkillSummary {
        name: name.to_string(),
        description: Some(format!("{name} skill")),
        state: Some("ok".to_string()),
        availability: Some("installed".to_string()),
        layer: Some("project:codex".to_string()),
        source_bucket: Some(source_bucket.to_string()),
        source: Some(source.to_string()),
        path: Some(path.to_string()),
    }
}

fn session_skill_response(
    session_id: &str,
    cwd: &str,
    skills: Vec<SessionSkillSummary>,
) -> SessionSkillListResponse {
    SessionSkillListResponse {
        session_id: session_id.to_string(),
        source: "sbp".to_string(),
        cwd: cwd.to_string(),
        available: true,
        query: None,
        skills,
        issues: Vec::new(),
        message: None,
    }
}

fn sleeping_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::Sleeping;
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn deep_sleep_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::DeepSleep;
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn attention_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    rest_state: RestState,
    last_activity_at: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.state = SessionState::Attention;
    session.rest_state = rest_state;
    session.thought_state = match rest_state {
        RestState::Sleeping | RestState::DeepSleep => ThoughtState::Sleeping,
        RestState::Active | RestState::Drowsy => ThoughtState::Holding,
    };
    session.last_activity_at = timestamp(last_activity_at);
    session
}

fn repo_theme(body: &str) -> RepoTheme {
    RepoTheme {
        body: body.to_string(),
        outline: "#222222".to_string(),
        accent: "#111111".to_string(),
        shirt: "#333333".to_string(),
        sprite: None,
    }
}

fn repo_theme_with_sprite(body: &str, sprite: Option<&str>) -> RepoTheme {
    let mut theme = repo_theme(body);
    theme.sprite = sprite.map(str::to_string);
    theme
}

fn session_summary_with_theme_id(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    theme_id: &str,
) -> SessionSummary {
    let mut session = session_summary(session_id, tmux_name, cwd);
    session.repo_theme_id = Some(theme_id.to_string());
    session
}

fn write_repo_theme_colors(root: &Path, body: &str) {
    write_repo_theme_colors_with_sprite(root, body, None);
}

fn write_repo_theme_colors_with_sprite(root: &Path, body: &str, sprite: Option<&str>) {
    let theme_dir = root.join(".swimmers");
    fs::create_dir_all(&theme_dir).expect("create theme dir");
    let mut content = serde_json::json!({
        "palette": {
            "body": body,
            "outline": "#222222",
            "accent": "#111111",
            "shirt": "#333333",
        }
    });
    if let Some(sprite) = sprite {
        content["sprite"] = serde_json::Value::String(sprite.to_string());
    }
    fs::write(
        theme_dir.join("colors.json"),
        serde_json::to_string(&content).expect("serialize theme"),
    )
    .expect("write theme colors");
}

fn dir_response(path: &str, names: &[(&str, bool)]) -> DirListResponse {
    DirListResponse {
        path: path.to_string(),
        entries: names
            .iter()
            .map(|(name, has_children)| DirEntry {
                name: (*name).to_string(),
                has_children: *has_children,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: None,
                has_restart: None,
                open_url: None,
            })
            .collect(),
        overlay_label: None,
        groups: Vec::new(),
        launch_targets: Vec::new(),
        default_launch_target: None,
    }
}

fn dir_response_with_launch_targets(path: &str, names: &[(&str, bool)]) -> DirListResponse {
    let mut response = dir_response(path, names);
    response.launch_targets = vec![
        LaunchTargetSummary::local(),
        LaunchTargetSummary {
            id: "jeremy-skillbox".to_string(),
            label: "Jeremy Skillbox".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: Some("http://100.105.106.104:3210".to_string()),
            auth_token_env: Some("SWIMMERS_JEREMY_AUTH_TOKEN".to_string()),
            path_mappings: Vec::new(),
        },
    ];
    response.default_launch_target = Some("local".to_string());
    response
}

fn dir_response_with_groups(
    path: &str,
    names: &[(&str, bool)],
    groups: &[&str],
) -> DirListResponse {
    let mut response = dir_response(path, names);
    response.groups = groups.iter().map(|group| (*group).to_string()).collect();
    response
}

fn repo_dir_entry(
    name: &str,
    has_children: bool,
    repo_dirty: Option<bool>,
    repo_action: Option<RepoActionStatus>,
) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        has_children,
        is_running: None,
        repo_dirty,
        repo_action,
        group: None,
        groups: Vec::new(),
        full_path: None,
        has_restart: None,
        open_url: None,
    }
}

fn repo_search_response(paths: &[&str]) -> DirRepoSearchResponse {
    DirRepoSearchResponse {
        roots: vec![
            "/Users/tester/repos".to_string(),
            "/Users/tester/hard".to_string(),
        ],
        entries: paths
            .iter()
            .map(|path| {
                let name = Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| (*path).to_string());
                DirEntry {
                    name: format!("{name}  {path}"),
                    has_children: false,
                    is_running: None,
                    repo_dirty: None,
                    repo_action: None,
                    group: None,
                    groups: Vec::new(),
                    full_path: Some((*path).to_string()),
                    has_restart: None,
                    open_url: None,
                }
            })
            .collect(),
    }
}

fn write_repo_theme_file(path: &std::path::Path, body: &str) {
    write_repo_theme_file_in(path, ".swimmers", body);
}

fn write_repo_theme_file_in(path: &std::path::Path, theme_dir: &str, body: &str) {
    let swimmers_dir = path.join(theme_dir);
    fs::create_dir_all(&swimmers_dir).expect("create theme dir");
    let contents = format!(
        concat!(
            "{{\n",
            "  \"palette\": {{\n",
            "    \"body\": \"{}\",\n",
            "    \"outline\": \"#3D2F24\",\n",
            "    \"accent\": \"#1D1914\",\n",
            "    \"shirt\": \"#AA9370\"\n",
            "  }}\n",
            "}}\n"
        ),
        body,
    );
    fs::write(swimmers_dir.join("colors.json"), contents).expect("write colors.json");
}

fn color_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb { r, g, b } => (r, g, b),
        other => panic!("expected rgb color, got {other:?}"),
    }
}

fn assert_dark_terminal_readable(color: Color) {
    assert!(
        contrast_ratio(color_rgb(color), DARK_TERMINAL_BG_RGB) >= MIN_DARK_TERMINAL_CONTRAST,
        "expected {color:?} to satisfy the dark-terminal contrast threshold"
    );
}

fn create_response(session_id: &str, tmux_name: &str, cwd: &str) -> CreateSessionResponse {
    CreateSessionResponse {
        session: session_summary(session_id, tmux_name, cwd),
        repo_theme: None,
    }
}

fn create_response_with_theme(
    session: SessionSummary,
    repo_theme: RepoTheme,
) -> CreateSessionResponse {
    CreateSessionResponse {
        session,
        repo_theme: Some(repo_theme),
    }
}

fn create_batch_response(items: &[(&str, &str, &str)]) -> CreateSessionsBatchResponse {
    CreateSessionsBatchResponse {
        results: items
            .iter()
            .enumerate()
            .map(
                |(index, (session_id, tmux_name, cwd))| CreateSessionsBatchResult {
                    index,
                    cwd: (*cwd).to_string(),
                    ok: true,
                    session: Some(session_summary(session_id, tmux_name, cwd)),
                    repo_theme: None,
                    error: None,
                },
            )
            .collect(),
    }
}

fn entity_at(
    field: Rect,
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    x: u16,
    y: u16,
) -> SessionEntity {
    let mut entity = SessionEntity::new(session_summary(session_id, tmux_name, cwd), field);
    entity.x = x.saturating_sub(field.x) as f32;
    entity.y = y.saturating_sub(field.y) as f32;
    entity.swim_anchor_x = entity.x;
    entity.swim_anchor_y = entity.y;
    entity.swim_center_y = entity.y;
    entity
}

fn entity_rect_for(app: &App<MockApi>, session_id: &str, field: Rect) -> Rect {
    app.entities
        .iter()
        .find(|entity| entity.session.session_id == session_id)
        .expect("entity should exist")
        .screen_rect(field)
}

fn sleep_grid_rect(field: Rect, slot: usize) -> Rect {
    let (x, y) = bottom_rest_origin(field, slot);
    Rect {
        x: field.x + x,
        y: field.y + y,
        width: ENTITY_WIDTH,
        height: ENTITY_HEIGHT,
    }
}

fn deep_sleep_grid_rect(field: Rect, slot: usize) -> Rect {
    let (x, y) = top_rest_origin(field, slot);
    Rect {
        x: field.x + x,
        y: field.y + y,
        width: ENTITY_WIDTH,
        height: ENTITY_HEIGHT,
    }
}
