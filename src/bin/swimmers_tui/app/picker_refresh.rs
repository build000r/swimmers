use super::*;

fn picker_refresh_is_due(
    picker_open: bool,
    interaction_idle: bool,
    composer_closed: bool,
    last_picker_refresh: Option<Instant>,
) -> bool {
    picker_open
        && interaction_idle
        && composer_closed
        && last_picker_refresh
            .map(|last| last.elapsed() >= REFRESH_INTERVAL)
            .unwrap_or(true)
}

impl<C: TuiApi> App<C> {
    pub(crate) fn should_refresh_picker(&self) -> bool {
        picker_refresh_is_due(
            self.picker.is_some(),
            self.pending_interaction.is_none(),
            self.initial_request.is_none(),
            self.last_picker_refresh,
        )
    }

    pub(crate) fn maybe_refresh_picker(&mut self) {
        if !self.should_refresh_picker() {
            return;
        }

        let Some((path, managed_only, group)) = self.picker.as_ref().map(|picker| {
            (
                picker.current_path.clone(),
                picker.managed_only,
                picker.current_group.clone(),
            )
        }) else {
            return;
        };

        self.picker_reload_with_options(Some(path), managed_only, group, false, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_refresh_is_due_for_open_idle_picker_without_previous_refresh() {
        assert!(picker_refresh_is_due(true, true, true, None));
    }

    #[test]
    fn picker_refresh_is_due_after_interval() {
        assert!(picker_refresh_is_due(
            true,
            true,
            true,
            Some(Instant::now() - REFRESH_INTERVAL - Duration::from_millis(1))
        ));
    }

    #[test]
    fn picker_refresh_waits_for_interval() {
        assert!(!picker_refresh_is_due(
            true,
            true,
            true,
            Some(Instant::now())
        ));
    }

    #[test]
    fn picker_refresh_waits_for_open_picker_and_idle_ui() {
        assert!(!picker_refresh_is_due(false, true, true, None));
        assert!(!picker_refresh_is_due(true, false, true, None));
        assert!(!picker_refresh_is_due(true, true, false, None));
    }
}
