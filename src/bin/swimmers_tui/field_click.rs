use super::*;

enum FieldClickPlan {
    Noop,
    ClosePicker,
    PickerAction(PickerAction),
    OpenSkillAtlas(SkillPanelAction),
    OpenSession { session_id: String, label: String },
    OpenPicker { x: u16, y: u16 },
}

impl FieldClickPlan {
    fn apply<C: TuiApi>(self, app: &mut App<C>, field: Rect) {
        match self {
            FieldClickPlan::Noop => {}
            FieldClickPlan::ClosePicker => app.close_picker(),
            FieldClickPlan::PickerAction(action) => app.handle_picker_action(action, field),
            FieldClickPlan::OpenSkillAtlas(action) => app.open_skill_atlas_viewer(action),
            FieldClickPlan::OpenSession { session_id, label } => {
                app.select_and_open_session(session_id, label);
            }
            FieldClickPlan::OpenPicker { x, y } => app.open_picker(x, y),
        }
    }
}

pub(super) fn handle_field_click<C: TuiApi>(app: &mut App<C>, x: u16, y: u16, field: Rect) {
    plan_field_click(app, x, y, field).apply(app, field);
}

fn plan_field_click<C: TuiApi>(app: &App<C>, x: u16, y: u16, field: Rect) -> FieldClickPlan {
    if app.initial_request.is_some() {
        return FieldClickPlan::Noop;
    }

    if let Some(plan) = picker_click_plan(app.picker.as_ref(), x, y, field) {
        return plan;
    }

    if let Some(action) = skill_panel_action_at(app, field, x, y) {
        return FieldClickPlan::OpenSkillAtlas(action);
    }

    tank_click_plan(app, x, y, field)
}

fn picker_click_plan(
    picker: Option<&PickerState>,
    x: u16,
    y: u16,
    field: Rect,
) -> Option<FieldClickPlan> {
    let picker = picker?;
    let layout = picker_layout(picker, field);
    if !layout.frame.contains(x, y) {
        return Some(FieldClickPlan::ClosePicker);
    }

    Some(
        picker_action_at(picker, &layout, x, y)
            .map(FieldClickPlan::PickerAction)
            .unwrap_or(FieldClickPlan::Noop),
    )
}

fn tank_click_plan<C: TuiApi>(app: &App<C>, x: u16, y: u16, field: Rect) -> FieldClickPlan {
    let tank_field = build_skill_panel(app, field).tank_field;
    if !tank_field.contains(x, y) {
        return FieldClickPlan::Noop;
    }

    clicked_session_plan(app, tank_field, x, y).unwrap_or(FieldClickPlan::OpenPicker { x, y })
}

fn clicked_session_plan<C: TuiApi>(
    app: &App<C>,
    tank_field: Rect,
    x: u16,
    y: u16,
) -> Option<FieldClickPlan> {
    let visible_entities = app.visible_entities();
    let hit = if app.uses_balls_scene(&visible_entities) {
        balls_theme_hit_test(&visible_entities, tank_field, x, y)
    } else {
        visible_entities
            .iter()
            .copied()
            .find(|entity| entity.screen_rect(tank_field).contains(x, y))
    }?;

    Some(FieldClickPlan::OpenSession {
        session_id: hit.session.session_id.clone(),
        label: selected_label(Some(&hit.session.tmux_name)),
    })
}
