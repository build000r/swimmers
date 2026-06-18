use super::*;
use swimmers::session_labels::{session_canonical_cwd_key, session_cwd_label};

impl<C: TuiApi> App<C> {
    pub(crate) fn trim_thought_log(&mut self, capacity: usize) {
        if capacity == 0 || self.thought_log.len() <= capacity {
            return;
        }

        let drop_count = self.thought_log.len() - capacity;
        self.thought_log.drain(0..drop_count);
    }

    pub(crate) fn upsert_thought_log_entries(
        &mut self,
        entries: impl IntoIterator<Item = ThoughtLogEntry>,
        capacity: usize,
    ) {
        for entry in entries {
            if let Some(index) = self
                .thought_log
                .iter()
                .position(|existing| existing.session_id == entry.session_id)
            {
                self.thought_log.remove(index);
            }
            self.thought_log.push(entry);
        }
        self.thought_log.sort_by(compare_thought_log_entries);
        self.trim_thought_log(capacity);
    }

    #[allow(dead_code)]
    pub(crate) fn visible_thought_entries(&self, capacity: usize) -> Vec<&ThoughtLogEntry> {
        if capacity == 0 {
            return Vec::new();
        }

        let filtered = self
            .thought_log
            .iter()
            .filter(|entry| self.thought_filter.matches(entry))
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(capacity);
        filtered[start..].to_vec()
    }

    pub(crate) fn thought_entry_display_color(&self, entry: &ThoughtLogEntry) -> Color {
        self.entities
            .iter()
            .find(|entity| entity.session.session_id == entry.session_id)
            .map(|entity| session_display_color(&entity.session, &self.repo_themes))
            .unwrap_or(entry.color)
    }

    pub(crate) fn header_repo_summaries(&self) -> Vec<ThoughtRepoSummary> {
        let mut grouped = HashMap::<String, ThoughtRepoSummary>::new();
        for (index, entity) in self.entities.iter().enumerate() {
            let session = &entity.session;
            let cwd = session_canonical_cwd_key(session);
            let Some(label) = path_tail_label(&cwd) else {
                continue;
            };
            let color = session_display_color(session, &self.repo_themes);

            let summary = grouped
                .entry(cwd.clone())
                .or_insert_with(|| ThoughtRepoSummary {
                    cwd: cwd.clone(),
                    label,
                    count: 0,
                    color,
                    last_seen: index,
                });
            summary.count += 1;
            summary.color = color;
            summary.last_seen = index;
        }

        let mut summaries = grouped.into_values().collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_seen
                .cmp(&left.last_seen)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.cwd.cmp(&right.cwd))
        });
        summaries
    }

    #[allow(dead_code)]
    pub(crate) fn active_thought_filter_text(&self) -> String {
        active_thought_filter_text(&self.thought_filter)
    }

    pub(crate) fn set_thought_filter_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = Some(cwd);
        self.thought_filter.fleet = None;
        self.thought_filter.excluded_cwds.clear();
        self.thought_filter.filter_out_mode = false;
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn set_thought_filter_fleet(&mut self, fleet: ThoughtFleetFilter) {
        self.thought_filter.cwd = None;
        self.thought_filter.fleet = Some(fleet);
        self.thought_filter.excluded_cwds.clear();
        self.thought_filter.filter_out_mode = false;
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_mode(&mut self) {
        self.thought_filter.filter_out_mode = !self.thought_filter.filter_out_mode;
        if self.thought_filter.filter_out_mode {
            self.thought_filter.cwd = None;
        } else {
            self.thought_filter.excluded_cwds.clear();
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = None;
        self.thought_filter.filter_out_mode = true;
        if !self.thought_filter.excluded_cwds.insert(cwd.clone()) {
            self.thought_filter.excluded_cwds.remove(&cwd);
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_group_by(&mut self) {
        self.thought_group_by = self.thought_group_by.toggled();
        self.set_message(format!(
            "thought rail grouped by {}",
            self.thought_group_by.label()
        ));
    }

    pub(crate) fn toggle_thought_show_all(&mut self) {
        self.thought_show_all = !self.thought_show_all;
        let mode = if self.thought_show_all {
            "showing all agents"
        } else {
            "showing asleep agents"
        };
        self.set_message(format!("clawgs rail {mode}"));
    }

    pub(crate) fn clear_thought_filters(&mut self) {
        self.thought_filter.clear();
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn reconcile_thought_log_sessions(&mut self, sessions: &[SessionSummary]) {
        let session_by_id = sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();

        self.thought_log
            .retain(|entry| session_by_id.contains_key(entry.session_id.as_str()));
        self.last_logged_thoughts
            .retain(|session_id, _| session_by_id.contains_key(session_id.as_str()));

        for entry in &mut self.thought_log {
            let Some(session) = session_by_id.get(entry.session_id.as_str()) else {
                continue;
            };
            entry.tmux_name = session.tmux_name.clone();
            entry.cwd = session_canonical_cwd_key(session);
            entry.pwd_label = session_cwd_label(session);
            entry.target_key = thought_filter_target_key(session);
            entry.target_label = session_target_label(session);
            entry.state_key = thought_filter_state_key(session.state).to_string();
            entry.readiness_key = thought_filter_readiness_key(session).to_string();
            entry.transport_key =
                thought_filter_transport_key(session.transport_health).to_string();
            entry.batch = session.batch.clone();
            entry.state = session.state;
            entry.current_command = session.current_command.clone();
            entry.tool = session.tool.clone();
            entry.rest_state = session.rest_state;
            entry.color = session_display_color(session, &self.repo_themes);
            entry.is_stale = session.is_stale;
            entry.transport_health = session.transport_health;
            entry.commit_candidate = session.commit_candidate;
        }

        self.thought_log.sort_by(compare_thought_log_entries);
    }

    pub(crate) fn capture_thought_updates(
        &mut self,
        sessions: &[SessionSummary],
        thought_capacity: usize,
    ) {
        let mut pending = Vec::new();
        for session in sessions {
            let Some(thought) = normalize_thought_text(session.thought.as_deref()) else {
                continue;
            };

            let incoming = ThoughtFingerprint {
                thought: thought.clone(),
                updated_at: session.thought_updated_at,
            };
            if !should_append_thought(
                self.last_logged_thoughts.get(&session.session_id),
                &incoming,
            ) {
                continue;
            }

            self.last_logged_thoughts
                .insert(session.session_id.clone(), incoming);
            pending.push(ThoughtLogEntry::from_session(
                session,
                thought,
                &self.repo_themes,
            ));
        }

        pending.sort_by(compare_thought_log_entries);

        if !pending.is_empty() {
            self.upsert_thought_log_entries(pending, thought_capacity);
        }
    }
}
