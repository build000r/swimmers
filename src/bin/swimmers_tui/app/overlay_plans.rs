use super::*;

/// Load the list of overlay-configured domain plans from disk, mapped into
/// the TUI's `PlanPanelEntry` shape.
pub(crate) fn load_overlay_plan_entries() -> Vec<PlanPanelEntry> {
    let Some(overlay) = swimmers::session::overlay::default_overlay() else {
        return Vec::new();
    };
    overlay
        .list_all_plans()
        .into_iter()
        .map(|entry| PlanPanelEntry {
            slug: entry.slug,
            client_label: entry.client_label,
            kind: entry.kind.to_string(),
            schema_path: entry.schema_path.to_string_lossy().into_owned(),
        })
        .collect()
}
