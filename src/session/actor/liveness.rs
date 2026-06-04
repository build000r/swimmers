use std::collections::{HashSet, VecDeque};

use super::{PaneLiveness, ProcessEntry, ProcessTreeIndex};

pub(super) fn compute_pane_liveness(pane_pid: u32, entries: Vec<ProcessEntry>) -> PaneLiveness {
    ProcessTreeIndex::from_entries(entries).pane_liveness(pane_pid)
}

impl ProcessTreeIndex {
    fn pane_liveness(&self, pane_pid: u32) -> PaneLiveness {
        let (has_children, descendant_cpu) = self.descendant_liveness(pane_pid);

        PaneLiveness {
            has_children,
            descendant_cpu,
            process_snapshot_fresh: true,
        }
    }

    fn descendant_liveness(&self, root_pid: u32) -> (bool, f32) {
        let mut has_children = false;
        let mut descendant_cpu = 0.0;
        let mut queue = self.descendant_queue(root_pid);
        let mut visited = HashSet::from([root_pid]);

        while let Some(pid) = queue.pop_front() {
            if !visited.insert(pid) {
                continue;
            }

            has_children = true;
            descendant_cpu += self.by_pid.get(&pid).map_or(0.0, |entry| entry.pcpu);

            if let Some(child_pids) = self.children.get(&pid) {
                queue.extend(child_pids.iter().copied());
            }
        }

        (has_children, descendant_cpu)
    }

    fn descendant_queue(&self, root_pid: u32) -> VecDeque<u32> {
        self.children
            .get(&root_pid)
            .map(|child_pids| child_pids.iter().copied().collect())
            .unwrap_or_default()
    }
}
