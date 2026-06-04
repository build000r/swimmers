use super::*;

impl<C: TuiApi> App<C> {
    pub(crate) fn render(&mut self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.clear();

        if renderer.width() < MIN_WIDTH || renderer.height() < MIN_HEIGHT {
            render_too_small(renderer);
            return;
        }

        self.render_shell(renderer, layout);

        if matches!(self.fish_bowl_mode, FishBowlMode::Aquarium) {
            self.render_aquarium(renderer, layout.overview_field);
        } else if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
            render_mermaid_viewer(renderer, layout.overview_field, viewer);
        }

        self.render_overlays(renderer, layout);
        render_footer(self, renderer, layout.footer_start_y);
    }

    fn render_shell(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.draw_box(
            frame_rect(renderer.width(), renderer.height()),
            Color::DarkGrey,
        );
        renderer.draw_text(2, 1, "swimmers tui", Color::Cyan);
        self.render_theme_toggle(renderer);
        self.render_header_right(renderer);
        render_header_filter_strip(self, renderer, renderer.width());
        renderer.draw_box(layout.workspace_box, Color::DarkGrey);
        self.render_thought_split(renderer, layout);
    }

    fn render_theme_toggle(&self, renderer: &mut Renderer) {
        let toggle_rect = self.sprite_theme_rect(renderer.width());
        let mut toggle_x = toggle_rect.x;
        for theme in SpriteTheme::override_options() {
            let label = format!("[{}]", SpriteTheme::override_label(theme));
            let color = if self.sprite_theme_override == theme {
                Color::Cyan
            } else {
                Color::DarkGrey
            };
            renderer.draw_text(toggle_x, toggle_rect.y, &label, color);
            toggle_x = toggle_x.saturating_add(display_width(&label) + 1);
        }
    }

    fn render_header_right(&self, renderer: &mut Renderer) {
        let max_right_width = renderer.width().saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let right_x = renderer
            .width()
            .saturating_sub(display_width(&right_text))
            .saturating_sub(2);
        renderer.draw_text(right_x, 1, &right_text, Color::DarkGrey);
    }

    fn render_thought_split(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        let (Some(thought_box), Some(thought_content)) =
            (layout.thought_box, layout.thought_content)
        else {
            return;
        };

        renderer.draw_box(thought_box, Color::DarkGrey);
        renderer.draw_box(layout.overview_box, Color::DarkGrey);
        if let Some(split_divider) = layout.split_divider {
            let divider_color = if self.split_drag_active {
                Color::Cyan
            } else {
                Color::DarkGrey
            };
            renderer.draw_vline(
                split_divider.x,
                split_divider.y,
                split_divider.height,
                ':',
                divider_color,
            );
        }
        render_thought_panel(
            self,
            renderer,
            thought_content,
            layout.thought_entry_capacity(),
        );
    }

    fn render_aquarium(&self, renderer: &mut Renderer, field: Rect) {
        let skill_panel = build_skill_panel(self, field);
        let tank_field = skill_panel.tank_field;
        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.render_empty_aquarium_message(renderer, tank_field);
        }

        if self.uses_balls_scene(&visible_entities) {
            render_balls_theme(
                renderer,
                tank_field,
                &visible_entities,
                self.selected_id.as_deref(),
                &self.repo_themes,
                self.tick,
            );
        } else {
            self.render_fish_scene(renderer, tank_field, visible_entities);
        }

        render_skill_panel(self, renderer, field);
        self.render_api_stale_banner(renderer, tank_field);
    }

    fn render_empty_aquarium_message(&self, renderer: &mut Renderer, field: Rect) {
        let (text, color) = empty_aquarium_message(
            !self.entities.is_empty(),
            self.thought_filter.is_active(),
            self.tmux_dependency_unavailable(),
        );
        let (x, y) = centered_empty_aquarium_position(field, text);
        renderer.draw_text(x, y, text, color);
    }

    fn tmux_dependency_unavailable(&self) -> bool {
        self.backend_health
            .as_ref()
            .and_then(|h| h.dependencies.as_ref())
            .map(|deps| deps.tmux_discovery.status == "unavailable")
            .unwrap_or(false)
    }

    fn render_fish_scene(
        &self,
        renderer: &mut Renderer,
        field: Rect,
        visible_entities: Vec<&SessionEntity>,
    ) {
        render_aquarium_background(renderer, field, self.tick);

        for entity in visible_entities {
            let rect = entity.screen_rect(field);
            let selected = self
                .selected_id
                .as_ref()
                .map(|selected| *selected == entity.session.session_id)
                .unwrap_or(false);
            render_entity_with_theme(
                renderer,
                entity,
                rect,
                selected,
                self.tick,
                &self.repo_themes,
                self.effective_sprite_theme_for_session(&entity.session),
            );
        }
    }

    fn render_api_stale_banner(&self, renderer: &mut Renderer, field: Rect) {
        let max_width = field.width as usize;
        let mut y = field.y;
        if let Some(banner) = self.api_refresh_health.banner_text() {
            renderer.draw_text(field.x, y, &truncate_label(banner, max_width), Color::Red);
            return;
        }
        if let Some(banner) = self
            .backend_health
            .as_ref()
            .and_then(backend_health_warning_text)
        {
            renderer.draw_text(
                field.x,
                y,
                &truncate_label(&banner, max_width),
                Color::Yellow,
            );
            y = y.saturating_add(1);
        }
        if let Some(dep_line) = self
            .backend_health
            .as_ref()
            .and_then(|h| h.dependencies.as_ref())
            .and_then(dependency_degradation_line)
        {
            if y < field.bottom() {
                renderer.draw_text(
                    field.x,
                    y,
                    &truncate_label(&dep_line, max_width),
                    Color::Yellow,
                );
            }
        }
    }

    fn render_overlays(&self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        if let Some(picker) = &self.picker {
            render_picker(renderer, picker, layout.overview_field);
        }
        if let Some(initial_request) = &self.initial_request {
            let group_context = self
                .group_input_targets
                .as_ref()
                .map(|targets| (targets.label.as_str(), targets.session_ids.len()));
            render_initial_request(
                renderer,
                initial_request,
                &self.voice_state,
                layout.overview_field,
                group_context,
            );
        }
        if let Some(editor) = &self.thought_config_editor {
            render_thought_config_editor(renderer, editor, layout.overview_field);
        }
        if self.show_help {
            render_help_overlay(renderer, layout.overview_field);
        }
    }
}

pub(super) fn empty_aquarium_message(
    has_entities: bool,
    thought_filter_active: bool,
    tmux_unavailable: bool,
) -> (&'static str, Color) {
    match (has_entities, thought_filter_active, tmux_unavailable) {
        (true, true, _) => ("no swimmers match filters", Color::DarkGrey),
        (_, _, true) => (
            "tmux unavailable - run swimmers config doctor",
            Color::Yellow,
        ),
        _ => (
            "no tmux sessions found - press r after starting one",
            Color::DarkGrey,
        ),
    }
}

pub(super) fn centered_empty_aquarium_position(field: Rect, text: &str) -> (u16, u16) {
    let x = field
        .x
        .saturating_add(field.width.saturating_sub(text.len() as u16) / 2);
    let y = field.y + field.height / 2;
    (x, y)
}
