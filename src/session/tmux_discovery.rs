use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiscoveryCandidate {
    pub(super) tmux_name: String,
    pub(super) reuse_session_id: Option<String>,
}

pub(super) fn plan_tmux_discovery_candidates(
    listed_tmux_names: &[String],
    tracked_tmux_names: &HashSet<String>,
    stale_session_ids_by_tmux: &HashMap<String, String>,
) -> (Vec<DiscoveryCandidate>, u64) {
    let mut planner = TmuxDiscoveryPlanner::new(tracked_tmux_names, stale_session_ids_by_tmux);

    for tmux_name in listed_tmux_names {
        planner.consider(tmux_name);
    }

    planner.finish()
}

pub(super) fn parse_tmux_session_names(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

struct TmuxDiscoveryPlanner<'a> {
    tracked_tmux_names: &'a HashSet<String>,
    stale_session_ids_by_tmux: &'a HashMap<String, String>,
    seen_tmux_names: HashSet<String>,
    highest_numeric: u64,
    candidates: Vec<DiscoveryCandidate>,
}

impl<'a> TmuxDiscoveryPlanner<'a> {
    fn new(
        tracked_tmux_names: &'a HashSet<String>,
        stale_session_ids_by_tmux: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            tracked_tmux_names,
            stale_session_ids_by_tmux,
            seen_tmux_names: HashSet::new(),
            highest_numeric: 0,
            candidates: Vec::new(),
        }
    }

    fn consider(&mut self, tmux_name: &str) {
        if tmux_name.is_empty() {
            return;
        }

        self.bump_numeric_counter(tmux_name);

        if self.should_skip(tmux_name) {
            return;
        }

        self.candidates.push(DiscoveryCandidate {
            tmux_name: tmux_name.to_string(),
            reuse_session_id: self.stale_session_ids_by_tmux.get(tmux_name).cloned(),
        });
    }

    fn bump_numeric_counter(&mut self, tmux_name: &str) {
        if let Ok(n) = tmux_name.parse::<u64>() {
            self.highest_numeric = self.highest_numeric.max(n.saturating_add(1));
        }
    }

    fn should_skip(&mut self, tmux_name: &str) -> bool {
        !self.seen_tmux_names.insert(tmux_name.to_string())
            || self.tracked_tmux_names.contains(tmux_name)
    }

    fn finish(self) -> (Vec<DiscoveryCandidate>, u64) {
        (self.candidates, self.highest_numeric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn plan_tmux_discovery_skips_tracked_and_dedupes_names() {
        let listed = vec![
            "main".to_string(),
            "main".to_string(),
            "codex-123".to_string(),
        ];
        let tracked = HashSet::from_iter(["main".to_string()]);
        let stale_by_tmux = HashMap::new();

        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &stale_by_tmux);

        assert_eq!(highest_numeric, 0);
        assert_eq!(
            candidates,
            vec![DiscoveryCandidate {
                tmux_name: "codex-123".to_string(),
                reuse_session_id: None,
            }]
        );
    }

    #[test]
    fn plan_tmux_discovery_reuses_stale_id_and_bumps_numeric_counter() {
        let listed = vec![
            "7".to_string(),
            "7".to_string(),
            "codex-20260302-162713".to_string(),
        ];
        let tracked = HashSet::new();
        let stale_by_tmux =
            HashMap::from_iter([("codex-20260302-162713".to_string(), "sess_12".to_string())]);

        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &stale_by_tmux);

        assert_eq!(highest_numeric, 8);
        assert_eq!(
            candidates,
            vec![
                DiscoveryCandidate {
                    tmux_name: "7".to_string(),
                    reuse_session_id: None,
                },
                DiscoveryCandidate {
                    tmux_name: "codex-20260302-162713".to_string(),
                    reuse_session_id: Some("sess_12".to_string()),
                },
            ]
        );
    }

    #[test]
    fn plan_tmux_discovery_skips_empty_names() {
        let listed = vec!["".to_string(), "  ".to_string(), "".to_string()];
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &HashSet::new(), &HashMap::new());
        assert_eq!(highest_numeric, 0);
        assert_eq!(
            candidates,
            vec![DiscoveryCandidate {
                tmux_name: "  ".to_string(),
                reuse_session_id: None,
            }]
        );
    }

    #[test]
    fn parse_tmux_session_names_preserves_exact_names() {
        let names = parse_tmux_session_names(b"alpha\n  padded  \n\tindented\n\nbeta\n");

        assert_eq!(
            names,
            vec![
                "alpha".to_string(),
                "  padded  ".to_string(),
                "\tindented".to_string(),
                "beta".to_string(),
            ]
        );
    }

    #[test]
    fn plan_tmux_discovery_all_tracked_returns_empty_candidates() {
        let listed = vec!["alpha".to_string(), "beta".to_string()];
        let tracked = HashSet::from_iter(["alpha".to_string(), "beta".to_string()]);
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &HashMap::new());
        assert_eq!(highest_numeric, 0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn plan_tmux_discovery_empty_list_returns_empty() {
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&[], &HashSet::new(), &HashMap::new());
        assert_eq!(highest_numeric, 0);
        assert!(candidates.is_empty());
    }
}
