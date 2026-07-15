use crate::session::tmux_discovery::{
    parse_tmux_session_names, plan_tmux_discovery_candidates, DiscoveryCandidate,
};
use crate::types::{
    SessionStatePayload, SUMMARY_CAUSE_STARTUP_MISSING_TMUX, SUMMARY_CAUSE_TMUX_RECONCILE_MISSING,
};
use tracing::error;

use super::*;

const TMUX_LIST_SESSIONS_TIMEOUT: Duration = Duration::from_secs(2);

struct ListedTmuxSessions {
    reliable: bool,
    names: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TmuxListSessionsOutcome {
    Listed(Vec<String>),
    NoSessions,
    TmuxError(String),
    CommandError(String),
}

struct MissingTrackedSessionSummary {
    session_id: String,
    previous_state: SessionState,
    summary: SessionSummary,
}

struct AdoptSessionPlan {
    session_id: String,
    tmux_target: TmuxTarget,
    stale_seed: Option<SessionSummary>,
    reused_session_id: bool,
    last_activity_override: Option<chrono::DateTime<Utc>>,
    batch: Option<SessionBatchMembership>,
}

impl SessionSupervisor {
    async fn list_tmux_session_names(
        &self,
        reason: &'static str,
        tmux_target: &TmuxTarget,
    ) -> ListedTmuxSessions {
        let list_started = Instant::now();
        info!(
            reason,
            phase = "tmux_list_sessions",
            tmux_target = %tmux_target.display_label(),
            "running tmux list-sessions"
        );
        let output = run_bounded_tmux_command_for_target(
            self.config.tmux_bin.as_str(),
            tmux_target,
            &["list-sessions", "-F", "#{session_name}"],
            TMUX_LIST_SESSIONS_TIMEOUT,
            "list-sessions",
        )
        .await;
        let outcome = match output {
            Ok(output) => classify_tmux_list_sessions_output(
                output.status.success(),
                &output.stdout,
                &output.stderr,
            ),
            Err(error) => classify_tmux_list_sessions_command_error(error),
        };

        self.build_listed_tmux_sessions(reason, list_started.elapsed(), outcome)
    }

    fn build_listed_tmux_sessions(
        &self,
        reason: &'static str,
        elapsed: Duration,
        outcome: TmuxListSessionsOutcome,
    ) -> ListedTmuxSessions {
        match outcome {
            TmuxListSessionsOutcome::Listed(names) => {
                log_tmux_list_success(reason, elapsed, names.len());
                self.record_tmux_discovery_success(reason, names.len());
                ListedTmuxSessions {
                    reliable: true,
                    names,
                }
            }
            TmuxListSessionsOutcome::NoSessions => {
                info!(
                    reason,
                    phase = "tmux_list_sessions",
                    elapsed_ms = elapsed.as_millis() as u64,
                    "no existing tmux sessions found"
                );
                self.record_tmux_discovery_success(reason, 0);
                ListedTmuxSessions {
                    reliable: true,
                    names: Vec::new(),
                }
            }
            TmuxListSessionsOutcome::TmuxError(stderr) => {
                warn!(
                    reason,
                    phase = "tmux_list_sessions",
                    elapsed_ms = elapsed.as_millis() as u64,
                    "tmux list-sessions returned error: {}",
                    stderr
                );
                self.record_tmux_discovery_failure(reason, stderr.trim().to_string());
                ListedTmuxSessions {
                    reliable: false,
                    names: Vec::new(),
                }
            }
            TmuxListSessionsOutcome::CommandError(error) => {
                warn!(
                    reason,
                    phase = "tmux_list_sessions",
                    elapsed_ms = elapsed.as_millis() as u64,
                    "tmux list-sessions failed: {}",
                    error
                );
                self.record_tmux_discovery_failure(reason, error);
                ListedTmuxSessions {
                    reliable: false,
                    names: Vec::new(),
                }
            }
        }
    }

    async fn tracked_tmux_names(&self, tmux_target: &TmuxTarget) -> HashSet<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|handle| &handle.tmux_target == tmux_target)
            .map(|handle| handle.tmux_name.clone())
            .collect()
    }

    async fn active_session_id_for_tmux(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .find(|handle| handle.tmux_name == tmux_name && handle.tmux_target == *tmux_target)
            .map(|handle| handle.session_id.clone())
    }

    async fn stale_summary_for_id(&self, session_id: &str) -> Option<SessionSummary> {
        let stale = self.stale_sessions.read().await;
        stale
            .iter()
            .find(|summary| summary.session_id == session_id)
            .cloned()
    }

    async fn stale_summaries_for_tmux(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) -> Vec<SessionSummary> {
        let stale = self.stale_sessions.read().await;
        stale
            .iter()
            .filter(|summary| summary.tmux_name == tmux_name && summary.tmux_target == *tmux_target)
            .cloned()
            .collect()
    }

    async fn stale_session_ids_by_tmux(&self, tmux_target: &TmuxTarget) -> HashMap<String, String> {
        let stale = self.stale_sessions.read().await;
        let mut by_tmux = HashMap::new();
        for summary in stale.iter() {
            if summary.tmux_target != *tmux_target {
                continue;
            }
            by_tmux
                .entry(summary.tmux_name.clone())
                .or_insert_with(|| summary.session_id.clone());
        }
        by_tmux
    }

    async fn attach_discovered_sessions(
        self: &Arc<Self>,
        reason: &'static str,
        listed_tmux_names: &[String],
        tmux_target: &TmuxTarget,
    ) -> u64 {
        let tracked_tmux_names = self.tracked_tmux_names(tmux_target).await;
        let stale_session_ids_by_tmux = self.stale_session_ids_by_tmux(tmux_target).await;
        let (candidates, highest_numeric) = plan_tmux_discovery_candidates(
            listed_tmux_names,
            &tracked_tmux_names,
            &stale_session_ids_by_tmux,
        );

        for candidate in candidates {
            self.attach_discovery_candidate(candidate, reason, tmux_target.clone())
                .await;
        }

        highest_numeric
    }

    async fn attach_discovery_candidate(
        self: &Arc<Self>,
        candidate: DiscoveryCandidate,
        reason: &'static str,
        tmux_target: TmuxTarget,
    ) {
        let tmux_name = candidate.tmux_name;
        let session_id = match candidate.reuse_session_id {
            Some(id) => {
                self.bump_id_counter_from_session_id(&id);
                id
            }
            None => self.allocate_unique_session_id().await,
        };
        info!(
            session_id = %session_id,
            tmux_name = %tmux_name,
            tmux_target = %tmux_target.display_label(),
            "discovered existing tmux session"
        );

        // Carry the persisted `last_activity_at` forward so long-silent
        // sessions resume in the correct fallback rest state (e.g. discovered
        // at startup after an overnight idle should wake up already drowsy,
        // not reset to Active before transcript sync has a chance to mark it
        // as waiting on the user).
        let persisted = self.persisted_session(&session_id).await;
        let last_activity_override = persisted.as_ref().map(|ps| ps.last_activity_at);
        let batch = persisted.and_then(|ps| ps.batch);

        match crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            tmux_target.clone(),
            true,
            None,
            None,
            None,
            self.config.clone(),
            last_activity_override,
            batch,
        ) {
            Ok(handle) => {
                if !self
                    .insert_discovered_handle(session_id.clone(), tmux_name.clone(), handle)
                    .await
                {
                    return;
                }
                self.emit_discovered_created_event(session_id, tmux_name, tmux_target, reason);
            }
            Err(error) => {
                error!(tmux_name = %tmux_name, "failed to attach to tmux session: {}", error);
            }
        }
    }

    async fn insert_discovered_handle(
        &self,
        session_id: String,
        tmux_name: String,
        handle: ActorHandle,
    ) -> bool {
        let mut sessions = self.sessions.write().await;
        if sessions.values().any(|existing| {
            existing.tmux_name == tmux_name && existing.tmux_target == handle.tmux_target
        }) {
            debug!(
                tmux_name = %tmux_name,
                tmux_target = %handle.tmux_target.display_label(),
                "skipping duplicate discovered tmux session"
            );
            drop(sessions);
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
            return false;
        }
        sessions.insert(session_id, handle);
        true
    }

    fn emit_discovered_created_event(
        &self,
        session_id: String,
        tmux_name: String,
        tmux_target: TmuxTarget,
        reason: &'static str,
    ) {
        let mut summary = self.build_placeholder_summary(&session_id, &tmux_name);
        summary.tmux_target = tmux_target;
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id,
            summary,
            reason: reason.into(),
            repo_theme: None,
        });
    }

    async fn reconcile_stale_sessions_after_discovery(
        &self,
        discovery_reliable: bool,
        listed_tmux_names: &[String],
        tmux_target: &TmuxTarget,
    ) {
        if !discovery_reliable {
            warn!("skipping stale reconciliation due unreliable tmux discovery");
            return;
        }

        let discovered_tmux_names: HashSet<String> = listed_tmux_names.iter().cloned().collect();
        let unresolved_stale = {
            let mut stale = self.stale_sessions.write().await;
            let mut unresolved = Vec::new();
            stale.retain(|summary| {
                if summary.tmux_target != *tmux_target {
                    return true;
                }
                if discovered_tmux_names.contains(&summary.tmux_name) {
                    return false;
                }
                unresolved.push(summary.clone());
                false
            });
            unresolved
        };

        if !unresolved_stale.is_empty() {
            debug!(
                remaining_stale = unresolved_stale.len(),
                "stale sessions after discovery"
            );
        }

        for summary in unresolved_stale {
            let previous_state = summary.state;
            self.emit_missing_tmux_events(
                summary,
                previous_state,
                SUMMARY_CAUSE_STARTUP_MISSING_TMUX,
            );
        }
    }

    async fn summary_for_missing_tracked_handle(
        &self,
        handle: &ActorHandle,
        cached: Option<&SessionSummary>,
    ) -> SessionSummary {
        let (tx, rx) = oneshot::channel();
        if handle
            .cmd_tx
            .send(SessionCommand::GetSummary(tx))
            .await
            .is_ok()
        {
            if let Ok(Ok(summary)) = tokio::time::timeout(PROCESS_EXIT_SUMMARY_TIMEOUT, rx).await {
                return summary;
            }
        }

        cached.cloned().unwrap_or_else(|| {
            self.build_placeholder_summary(&handle.session_id, &handle.tmux_name)
        })
    }

    pub(super) fn mark_missing_tmux_summary(summary: SessionSummary) -> SessionSummary {
        summary.into_missing_tmux_stale(SUMMARY_CAUSE_TMUX_RECONCILE_MISSING)
    }

    async fn missing_tracked_handles(
        &self,
        listed_tmux_names: &HashSet<String>,
        tmux_target: &TmuxTarget,
    ) -> Vec<ActorHandle> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|handle| {
                handle.tmux_target == *tmux_target && !listed_tmux_names.contains(&handle.tmux_name)
            })
            .cloned()
            .collect()
    }

    async fn stale_summaries_for_missing_tracked_handles(
        &self,
        missing_handles: &[ActorHandle],
    ) -> Vec<MissingTrackedSessionSummary> {
        let cached_summaries = self.summary_cache.read().await.clone();
        let mut stale_summaries = Vec::with_capacity(missing_handles.len());
        for handle in missing_handles {
            let summary = self
                .summary_for_missing_tracked_handle(
                    handle,
                    cached_summaries.get(&handle.session_id),
                )
                .await;
            stale_summaries.push(MissingTrackedSessionSummary {
                session_id: handle.session_id.clone(),
                previous_state: summary.state,
                summary: Self::mark_missing_tmux_summary(summary),
            });
        }
        stale_summaries
    }

    async fn remove_still_missing_tracked_handles(
        &self,
        missing_handles: &[ActorHandle],
        listed_tmux_names: &HashSet<String>,
        tmux_target: &TmuxTarget,
    ) -> Vec<ActorHandle> {
        let mut sessions = self.sessions.write().await;
        let mut removed = Vec::with_capacity(missing_handles.len());
        for handle in missing_handles {
            let still_missing = sessions
                .get(&handle.session_id)
                .map(|current| {
                    current.tmux_target == *tmux_target
                        && !listed_tmux_names.contains(&current.tmux_name)
                })
                .unwrap_or(false);
            if still_missing {
                if let Some(handle) = sessions.remove(&handle.session_id) {
                    removed.push(handle);
                }
            }
        }
        crate::metrics::set_active_sessions(sessions.len());
        removed
    }

    async fn forget_removed_tracked_summary_cache(&self, removed_ids: &HashSet<String>) {
        let mut cache = self.summary_cache.write().await;
        for session_id in removed_ids {
            cache.remove(session_id);
        }
    }

    async fn retain_removed_tracked_stale_summaries(
        &self,
        stale_summaries: &[MissingTrackedSessionSummary],
        removed_ids: &HashSet<String>,
    ) {
        let mut stale = self.stale_sessions.write().await;
        for stale_summary in stale_summaries {
            if !removed_ids.contains(&stale_summary.session_id) {
                continue;
            }
            stale.retain(|existing| {
                existing.session_id != stale_summary.summary.session_id
                    && !(existing.tmux_name == stale_summary.summary.tmux_name
                        && existing.tmux_target == stale_summary.summary.tmux_target)
            });
            stale.push(stale_summary.summary.clone());
        }
    }

    async fn shutdown_removed_tracked_handles(&self, removed_handles: Vec<ActorHandle>) {
        for handle in removed_handles {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
        }
    }

    fn emit_removed_tracked_missing_events(
        &self,
        stale_summaries: Vec<MissingTrackedSessionSummary>,
        removed_ids: &HashSet<String>,
    ) {
        for stale_summary in stale_summaries {
            if removed_ids.contains(&stale_summary.session_id) {
                self.emit_missing_tmux_events(
                    stale_summary.summary,
                    stale_summary.previous_state,
                    SUMMARY_CAUSE_TMUX_RECONCILE_MISSING,
                );
            }
        }
    }

    async fn reconcile_tracked_sessions_after_discovery(
        &self,
        discovery_reliable: bool,
        listed_tmux_names: &[String],
        tmux_target: &TmuxTarget,
    ) {
        if !discovery_reliable {
            return;
        }

        let listed_tmux_names = listed_tmux_names.iter().cloned().collect::<HashSet<_>>();
        let missing_handles = self
            .missing_tracked_handles(&listed_tmux_names, tmux_target)
            .await;
        if missing_handles.is_empty() {
            return;
        }

        let stale_summaries = self
            .stale_summaries_for_missing_tracked_handles(&missing_handles)
            .await;
        let removed_handles = self
            .remove_still_missing_tracked_handles(&missing_handles, &listed_tmux_names, tmux_target)
            .await;
        if removed_handles.is_empty() {
            return;
        }

        let removed_ids = removed_handles
            .iter()
            .map(|handle| handle.session_id.clone())
            .collect::<HashSet<_>>();
        self.forget_removed_tracked_summary_cache(&removed_ids)
            .await;
        self.retain_removed_tracked_stale_summaries(&stale_summaries, &removed_ids)
            .await;

        self.shutdown_removed_tracked_handles(removed_handles).await;
        self.emit_removed_tracked_missing_events(stale_summaries, &removed_ids);

        self.persist_registry().await;
    }

    fn emit_missing_tmux_events(
        &self,
        summary: SessionSummary,
        previous_state: SessionState,
        reason: &'static str,
    ) {
        let payload = SessionStatePayload {
            state: SessionState::Exited,
            previous_state,
            current_command: summary.current_command.clone(),
            state_evidence: crate::types::StateEvidence::new(reason),
            transport_health: TransportHealth::Disconnected,
            exit_reason: Some(reason.to_string()),
            at: Utc::now(),
        };
        let event = ControlEvent {
            event: "session_state".to_string(),
            session_id: summary.session_id.clone(),
            payload: serde_json::to_value(&payload).unwrap_or_default(),
        };
        let _ = self.thought_tx.send(event);

        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: summary.session_id,
            reason: reason.to_string(),
            delete_mode: crate::config::SessionDeleteMode::DetachBridge,
            tmux_session_alive: false,
        });
    }

    async fn finish_tmux_discovery(&self, discovery_reliable: bool, highest_numeric: u64) {
        self.next_name_counter
            .fetch_max(highest_numeric, Ordering::SeqCst);

        let sessions = self.sessions.read().await;
        crate::metrics::set_active_sessions(sessions.len());
        info!(count = sessions.len(), "tmux session discovery complete");

        if discovery_reliable {
            self.persist_registry().await;
        }
    }

    async fn resolve_adopt_session_identity(
        self: &Arc<Self>,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
        requested_session_id: Option<String>,
    ) -> Result<(String, Option<SessionSummary>), TmuxAdoptError> {
        match requested_session_id {
            Some(session_id) => {
                self.resolve_requested_adopt_session_identity(tmux_name, tmux_target, session_id)
                    .await
            }
            None => {
                self.resolve_unrequested_adopt_session_identity(tmux_name, tmux_target)
                    .await
            }
        }
    }

    async fn resolve_requested_adopt_session_identity(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
        session_id: String,
    ) -> Result<(String, Option<SessionSummary>), TmuxAdoptError> {
        if self.sessions.read().await.contains_key(&session_id) {
            return Err(TmuxAdoptError::AlreadyTracked {
                tmux_name: tmux_name.to_string(),
                session_id,
            });
        }

        let stale = self
            .stale_summary_for_id(&session_id)
            .await
            .ok_or_else(|| TmuxAdoptError::StaleSessionNotFound {
                session_id: session_id.clone(),
            })?;
        if stale.tmux_name != tmux_name || stale.tmux_target != *tmux_target {
            return Err(TmuxAdoptError::StaleSessionConflict {
                session_id,
                stale_tmux_name: stale.tmux_name,
                requested_tmux_name: tmux_name.to_string(),
            });
        }

        Ok((stale.session_id.clone(), Some(stale)))
    }

    async fn resolve_unrequested_adopt_session_identity(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) -> Result<(String, Option<SessionSummary>), TmuxAdoptError> {
        let stale_matches = self.stale_summaries_for_tmux(tmux_name, tmux_target).await;
        match stale_matches.len() {
            0 => Ok((self.allocate_unique_session_id().await, None)),
            1 => {
                let stale = stale_matches.into_iter().next().expect("one stale match");
                Ok((stale.session_id.clone(), Some(stale)))
            }
            count => Err(TmuxAdoptError::AmbiguousTarget {
                tmux_name: tmux_name.to_string(),
                matches: count,
            }),
        }
    }

    async fn reject_already_tracked_adopt_target(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) -> Result<(), TmuxAdoptError> {
        if let Some(active_id) = self
            .active_session_id_for_tmux(tmux_name, tmux_target)
            .await
        {
            return Err(TmuxAdoptError::AlreadyTracked {
                tmux_name: tmux_name.to_string(),
                session_id: active_id,
            });
        }

        Ok(())
    }

    async fn verify_adopt_target_exists(
        &self,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) -> Result<(), TmuxAdoptError> {
        let listed = self
            .list_tmux_session_names("manual_tmux_adopt", tmux_target)
            .await;
        if !listed.reliable {
            return Err(TmuxAdoptError::DiscoveryUnavailable);
        }

        Self::reject_missing_or_ambiguous_adopt_target(tmux_name, &listed.names)
    }

    fn reject_missing_or_ambiguous_adopt_target(
        tmux_name: &str,
        listed_tmux_names: &[String],
    ) -> Result<(), TmuxAdoptError> {
        match listed_tmux_names
            .iter()
            .filter(|name| *name == tmux_name)
            .count()
        {
            0 => Err(TmuxAdoptError::TargetNotFound {
                tmux_name: tmux_name.to_string(),
            }),
            1 => Ok(()),
            matches => Err(TmuxAdoptError::AmbiguousTarget {
                tmux_name: tmux_name.to_string(),
                matches,
            }),
        }
    }

    async fn prepare_adopt_session_plan(
        self: &Arc<Self>,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
        requested_session_id: Option<String>,
    ) -> Result<AdoptSessionPlan, TmuxAdoptError> {
        let (session_id, stale_seed) = self
            .resolve_adopt_session_identity(tmux_name, tmux_target, requested_session_id)
            .await?;
        let reused_session_id = stale_seed.is_some();
        if reused_session_id {
            self.bump_id_counter_from_session_id(&session_id);
        }

        // Read the persisted record once, and only when a field actually falls
        // back to it, rather than re-loading the registry per field.
        let persisted = if stale_seed
            .as_ref()
            .is_none_or(|summary| summary.batch.is_none())
        {
            self.persisted_session(&session_id).await
        } else {
            None
        };
        let last_activity_override = match stale_seed.as_ref() {
            Some(summary) => Some(summary.last_activity_at),
            None => persisted.as_ref().map(|ps| ps.last_activity_at),
        };
        let batch = match stale_seed
            .as_ref()
            .and_then(|summary| summary.batch.clone())
        {
            Some(batch) => Some(batch),
            None => persisted.and_then(|ps| ps.batch),
        };

        Ok(AdoptSessionPlan {
            session_id,
            tmux_target: tmux_target.clone(),
            stale_seed,
            reused_session_id,
            last_activity_override,
            batch,
        })
    }

    fn spawn_adopted_tmux_actor(
        &self,
        tmux_name: &str,
        plan: &AdoptSessionPlan,
    ) -> Result<ActorHandle, TmuxAdoptError> {
        crate::session::actor::SessionActor::spawn(
            plan.session_id.clone(),
            tmux_name.to_string(),
            plan.tmux_target.clone(),
            true,
            None,
            None,
            None,
            self.config.clone(),
            plan.last_activity_override,
            plan.batch.clone(),
        )
        .map_err(|error| TmuxAdoptError::SpawnFailed {
            tmux_name: tmux_name.to_string(),
            message: error.to_string(),
        })
    }

    fn build_adopted_summary(
        &self,
        session_id: &str,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
        stale_seed: Option<SessionSummary>,
        reason: &'static str,
    ) -> (SessionSummary, Option<RepoTheme>) {
        let mut summary = stale_seed
            .unwrap_or_else(|| self.build_placeholder_summary(session_id, tmux_name))
            .revive_from_stale(session_id, tmux_name, reason);
        summary.tmux_target = tmux_target.clone();
        let repo_theme = self.resolve_repo_theme_for_summary(&mut summary);
        (summary, repo_theme)
    }

    async fn insert_adopted_handle(
        &self,
        tmux_name: &str,
        session_id: &str,
        handle: ActorHandle,
    ) -> Result<(), TmuxAdoptError> {
        let tmux_target = handle.tmux_target.clone();
        if self
            .insert_discovered_handle(session_id.to_string(), tmux_name.to_string(), handle)
            .await
        {
            return Ok(());
        }

        let active_id = self
            .active_session_id_for_tmux(tmux_name, &tmux_target)
            .await
            .unwrap_or_else(|| "<unknown>".to_string());
        Err(TmuxAdoptError::AlreadyTracked {
            tmux_name: tmux_name.to_string(),
            session_id: active_id,
        })
    }

    async fn finish_adopted_tmux_session(
        &self,
        tmux_name: &str,
        plan: AdoptSessionPlan,
        handle: ActorHandle,
    ) -> Result<AdoptedTmuxSession, TmuxAdoptError> {
        self.insert_adopted_handle(tmux_name, &plan.session_id, handle)
            .await?;

        let reason = if plan.reused_session_id {
            "manual_tmux_reattach"
        } else {
            "manual_tmux_adopt"
        };
        let (summary, repo_theme) = self.build_adopted_summary(
            &plan.session_id,
            tmux_name,
            &plan.tmux_target,
            plan.stale_seed,
            reason,
        );

        self.retain_non_adopted_stale_summaries(&plan.session_id, tmux_name, &plan.tmux_target)
            .await;
        self.cache_adopted_summary(&plan.session_id, summary.clone())
            .await;
        self.update_active_session_metric().await;
        self.emit_adopted_created_event(
            plan.session_id,
            summary.clone(),
            reason,
            repo_theme.clone(),
        );
        self.persist_registry().await;

        Ok(AdoptedTmuxSession {
            session: summary,
            repo_theme,
            reused_session_id: plan.reused_session_id,
        })
    }

    async fn retain_non_adopted_stale_summaries(
        &self,
        session_id: &str,
        tmux_name: &str,
        tmux_target: &TmuxTarget,
    ) {
        let mut stale = self.stale_sessions.write().await;
        stale.retain(|existing| {
            existing.session_id != session_id
                && !(existing.tmux_name == tmux_name && existing.tmux_target == *tmux_target)
        });
    }

    async fn cache_adopted_summary(&self, session_id: &str, summary: SessionSummary) {
        let mut cache = self.summary_cache.write().await;
        cache.insert(session_id.to_string(), summary);
    }

    async fn update_active_session_metric(&self) {
        let sessions = self.sessions.read().await;
        crate::metrics::set_active_sessions(sessions.len());
    }

    fn emit_adopted_created_event(
        &self,
        session_id: String,
        summary: SessionSummary,
        reason: &'static str,
        repo_theme: Option<RepoTheme>,
    ) {
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id,
            summary,
            reason: reason.to_string(),
            repo_theme,
        });
    }

    /// Discover existing tmux sessions and create actors for each one.
    /// Called once at server startup.
    pub async fn discover_tmux_sessions(self: &Arc<Self>) -> anyhow::Result<()> {
        self.discover_tmux_sessions_with_reason("startup_discovery")
            .await
    }

    /// Discover existing tmux sessions, attaching only sessions not already
    /// tracked by an in-memory actor, and emit Created events with `reason`.
    pub async fn discover_tmux_sessions_with_reason(
        self: &Arc<Self>,
        reason: &'static str,
    ) -> anyhow::Result<()> {
        let _discovery_guard = self.discovery_lock.lock().await;
        let tmux_target = self.config.tmux_target.clone();
        let listed = self.list_tmux_session_names(reason, &tmux_target).await;
        let highest_numeric = self
            .attach_discovered_sessions(reason, &listed.names, &tmux_target)
            .await;
        self.reconcile_stale_sessions_after_discovery(listed.reliable, &listed.names, &tmux_target)
            .await;
        self.reconcile_tracked_sessions_after_discovery(
            listed.reliable,
            &listed.names,
            &tmux_target,
        )
        .await;
        self.finish_tmux_discovery(listed.reliable, highest_numeric)
            .await;

        Ok(())
    }

    /// Explicitly adopt a tmux session that already exists outside swimmers,
    /// optionally reusing a stale swimmers session id when that binding is
    /// unambiguous.
    pub async fn adopt_tmux_session(
        self: &Arc<Self>,
        tmux_name: String,
        tmux_target: Option<TmuxTarget>,
        session_id: Option<String>,
    ) -> Result<AdoptedTmuxSession, TmuxAdoptError> {
        if tmux_name.is_empty() {
            return Err(TmuxAdoptError::EmptyTmuxName);
        }
        let tmux_target = tmux_target.unwrap_or_else(|| self.config.tmux_target.clone());
        tmux_target
            .validate()
            .map_err(|error| TmuxAdoptError::InvalidTarget {
                message: error.to_string(),
            })?;

        let _discovery_guard = self.discovery_lock.lock().await;
        self.reject_already_tracked_adopt_target(&tmux_name, &tmux_target)
            .await?;
        self.verify_adopt_target_exists(&tmux_name, &tmux_target)
            .await?;
        let plan = self
            .prepare_adopt_session_plan(&tmux_name, &tmux_target, session_id)
            .await?;
        let handle = self.spawn_adopted_tmux_actor(&tmux_name, &plan)?;
        self.finish_adopted_tmux_session(&tmux_name, plan, handle)
            .await
    }
}

fn tmux_list_reports_no_sessions(stderr: &str) -> bool {
    stderr.contains("no server running") || stderr.contains("no sessions")
}

pub(super) fn classify_tmux_list_sessions_output(
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> TmuxListSessionsOutcome {
    if success {
        return TmuxListSessionsOutcome::Listed(parse_tmux_session_names(stdout));
    }

    let stderr = String::from_utf8_lossy(stderr);
    if tmux_list_reports_no_sessions(&stderr) {
        TmuxListSessionsOutcome::NoSessions
    } else {
        TmuxListSessionsOutcome::TmuxError(stderr.into_owned())
    }
}

pub(super) fn classify_tmux_list_sessions_command_error(
    error: impl fmt::Display,
) -> TmuxListSessionsOutcome {
    TmuxListSessionsOutcome::CommandError(error.to_string())
}

fn log_tmux_list_success(reason: &'static str, elapsed: Duration, listed_sessions: usize) {
    let elapsed_ms = elapsed.as_millis() as u64;
    if elapsed >= Duration::from_secs(2) {
        warn!(
            reason,
            phase = "tmux_list_sessions",
            elapsed_ms,
            listed_sessions,
            "tmux list-sessions completed slowly"
        );
    } else {
        info!(
            reason,
            phase = "tmux_list_sessions",
            elapsed_ms,
            listed_sessions,
            "tmux list-sessions completed"
        );
    }
}
