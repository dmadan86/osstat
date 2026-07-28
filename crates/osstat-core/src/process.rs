//! Processes: the flat records a platform reports, the tree built from them,
//! and the diff sent between ticks.
//!
//! Everything in this module is a pure function over plain data. That is
//! deliberate — it is the most intricate logic in the read path, and keeping it
//! free of `sysinfo` means orphans, cycles and PID reuse can be tested
//! exhaustively on a CI runner with no interesting processes on it at all.
//!
//! Three invariants are worth stating up front, because each of them is a bug
//! that would be easy to ship and hard to notice:
//!
//! 1. **A PID is not an identity.** Operating systems reuse them. Identity is
//!    [`ProcessKey`] — the PID *and* the start time.
//! 2. **Collapsing must never hide load.** A collapsed parent shows the sum of
//!    itself and every descendant, so the number on screen is the same whether
//!    a subtree is open or shut.
//! 3. **The parent table is untrusted input.** A snapshot can name a parent that
//!    has already exited, or — between two racing reads — describe a cycle.
//!    Neither may hang or drop a process.

// The public types re-export from the crate root as `ProcessRecord`, `ProcessTree`
// and so on; naming them `Record` and `Tree` to satisfy the lint would make every
// call site read worse than the lint is worth.
#![allow(clippy::module_name_repetitions)]

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Bytes in a mebibyte — the precision memory is compared at when diffing.
const MEBIBYTE: u64 = 1024 * 1024;

/// Bytes in a kibibyte — the precision IO rates are compared at when diffing.
const KIBIBYTE: u64 = 1024;

/// What a process is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum ProcessStatus {
    /// On a CPU or ready to be.
    Running,
    /// Waiting on something.
    Sleeping,
    /// Idle.
    Idle,
    /// Stopped by a signal or a debugger.
    Stopped,
    /// Exited but not yet reaped.
    Zombie,
    /// The platform reported something osstat does not model.
    Unknown,
}

/// The identity of a process.
///
/// A PID alone is not enough. Operating systems reuse PIDs, and a tick that
/// treats a recycled PID as the same process will happily show the new process
/// inheriting the old one's children and history. The start time is what makes
/// this a real identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProcessKey {
    /// Process identifier.
    pub pid: u32,
    /// Seconds since the Unix epoch at which the process started.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub started_at: u64,
}

/// One process, as a platform reports it — flat, with no hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProcessRecord {
    /// Process identifier.
    pub pid: u32,
    /// Parent's identifier, absent for a root.
    pub parent_pid: Option<u32>,
    /// Executable name as shown in the tree.
    pub name: String,
    /// Full path to the executable, where it could be read.
    pub exe: Option<String>,
    /// Owning user, where it could be resolved.
    pub user: Option<String>,
    /// CPU utilisation as a percentage. May exceed 100 on multi-core machines.
    pub cpu: f32,
    /// Resident memory in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub memory: u64,
    /// Bytes read per second during the last tick.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub disk_read_rate: u64,
    /// Bytes written per second during the last tick.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub disk_write_rate: u64,
    /// Seconds since the Unix epoch at which the process started.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub started_at: u64,
    /// What the process is currently doing.
    pub status: ProcessStatus,
}

impl ProcessRecord {
    /// This process's identity, which is more than its PID.
    #[must_use]
    pub const fn key(&self) -> ProcessKey {
        ProcessKey {
            pid: self.pid,
            started_at: self.started_at,
        }
    }
}

/// A process's own cost plus that of everything beneath it.
///
/// This is what a collapsed row displays. Without it, collapsing a busy subtree
/// would make its load vanish from the screen — the tree would be lying by
/// omission exactly when a user is hunting for what is eating their machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Rollup {
    /// Summed CPU percentage.
    ///
    /// Accumulated as `f64` even though the platform reports `f32`, so that
    /// summing a thousand processes does not drift.
    pub cpu: f64,
    /// Summed resident memory in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub memory: u64,
    /// Summed read rate in bytes per second.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub disk_read_rate: u64,
    /// Summed write rate in bytes per second.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub disk_write_rate: u64,
    /// How many processes are counted, including the one this describes.
    pub processes: u32,
}

impl Rollup {
    /// A roll-up covering a single process and nothing else.
    #[must_use]
    fn of(record: &ProcessRecord) -> Self {
        Self {
            cpu: f64::from(record.cpu),
            memory: record.memory,
            disk_read_rate: record.disk_read_rate,
            disk_write_rate: record.disk_write_rate,
            processes: 1,
        }
    }

    /// Folds another roll-up into this one.
    fn absorb(&mut self, other: Self) {
        self.cpu += other.cpu;
        self.memory = self.memory.saturating_add(other.memory);
        self.disk_read_rate = self.disk_read_rate.saturating_add(other.disk_read_rate);
        self.disk_write_rate = self.disk_write_rate.saturating_add(other.disk_write_rate);
        self.processes = self.processes.saturating_add(other.processes);
    }
}

/// A process and its descendants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProcessNode {
    /// This process.
    pub record: ProcessRecord,
    /// Children, ordered by PID so the tree is stable between ticks.
    pub children: Vec<ProcessNode>,
    /// This process plus everything beneath it.
    pub rollup: Rollup,
}

/// Every process on the machine, arranged by parentage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProcessTree {
    /// Processes with no live parent, ordered by PID.
    pub roots: Vec<ProcessNode>,
    /// Every process on the machine, summed.
    pub total: Rollup,
}

impl ProcessTree {
    /// Arranges flat records into a hierarchy, with roll-ups computed.
    ///
    /// The input is untrusted: it may name parents that have already exited,
    /// contain a process that claims to be its own parent, or — because a
    /// snapshot is not atomic — describe a cycle. Each case is handled by
    /// promoting a process to a root rather than by dropping it, so the count of
    /// processes in the tree always equals the count that went in.
    #[must_use]
    pub fn build(records: Vec<ProcessRecord>) -> Self {
        let records = dedupe_by_pid(records);
        if records.is_empty() {
            return Self::default();
        }

        let parents = resolve_parents(&records);
        let children = invert(&parents, records.len());

        let mut built: Vec<Option<ProcessNode>> = vec![None; records.len()];
        let mut records: Vec<Option<ProcessRecord>> = records.into_iter().map(Some).collect();

        for index in post_order(&parents, &children) {
            let Some(record) = records[index].take() else {
                continue;
            };

            let mut rollup = Rollup::of(&record);
            let mut kids = Vec::with_capacity(children[index].len());
            for &child in &children[index] {
                if let Some(node) = built[child].take() {
                    rollup.absorb(node.rollup);
                    kids.push(node);
                }
            }

            built[index] = Some(ProcessNode {
                record,
                children: kids,
                rollup,
            });
        }

        let mut total = Rollup::default();
        let mut roots = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            if parent.is_none()
                && let Some(node) = built[index].take()
            {
                total.absorb(node.rollup);
                roots.push(node);
            }
        }

        Self { roots, total }
    }

    /// How many processes the tree holds.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.total.processes
    }

    /// Whether the tree holds no processes at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total.processes == 0
    }
}

/// Keeps one record per PID, ordered by PID.
///
/// A single snapshot should never contain a duplicate PID, but "should never"
/// is not a guarantee when the source is three different operating systems.
/// Sorting also makes every downstream step deterministic, which is what lets
/// cycle-breaking be reproducible rather than dependent on hash order.
fn dedupe_by_pid(mut records: Vec<ProcessRecord>) -> Vec<ProcessRecord> {
    records.sort_by_key(|record| record.pid);
    records.dedup_by_key(|record| record.pid);
    records
}

/// Maps each record to its parent's index, breaking anything that is not a tree.
///
/// A parent reference survives only if it points at a different process that is
/// present in this snapshot. Everything else — an exited parent, a
/// self-parenting root such as PID 0, a cycle — resolves to `None`, which makes
/// that process a root.
fn resolve_parents(records: &[ProcessRecord]) -> Vec<Option<usize>> {
    let index_of: HashMap<u32, usize> = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.pid, index))
        .collect();

    let mut parents: Vec<Option<usize>> = records
        .iter()
        .map(|record| {
            let parent_pid = record.parent_pid?;
            if parent_pid == record.pid {
                return None;
            }
            index_of.get(&parent_pid).copied()
        })
        .collect();

    break_cycles(&mut parents);
    parents
}

/// Detaches one edge from every parent cycle.
///
/// Walks each chain of ancestors once, colouring nodes as it goes. Re-entering a
/// node that is still on the current walk means the walk has closed a loop, and
/// that node's parent edge is the one cut. Because records are sorted by PID
/// before this runs, the choice of edge is deterministic: the same snapshot
/// always produces the same tree.
fn break_cycles(parents: &mut [Option<usize>]) {
    /// Not yet examined.
    const NEW: u8 = 0;
    /// On the walk currently in progress.
    const WALKING: u8 = 1;
    /// Settled by an earlier walk.
    const SETTLED: u8 = 2;

    let mut state = vec![NEW; parents.len()];
    let mut walked = Vec::new();

    for start in 0..parents.len() {
        if state[start] != NEW {
            continue;
        }

        walked.clear();
        let mut current = start;
        loop {
            match state[current] {
                WALKING => {
                    // Closed a loop back onto the current walk: cut here.
                    parents[current] = None;
                    break;
                }
                SETTLED => break,
                _ => {}
            }

            state[current] = WALKING;
            walked.push(current);

            match parents[current] {
                Some(parent) => current = parent,
                None => break,
            }
        }

        for &index in &walked {
            state[index] = SETTLED;
        }
    }
}

/// Turns a parent table into a child table, preserving PID order.
fn invert(parents: &[Option<usize>], len: usize) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); len];
    for (index, parent) in parents.iter().enumerate() {
        if let Some(parent) = *parent {
            children[parent].push(index);
        }
    }
    children
}

/// Orders indices so that every child appears before its parent.
///
/// Iterative rather than recursive: a pathological or malicious process chain
/// could be thousands deep, and a stack overflow in a system monitor is not an
/// acceptable failure mode.
fn post_order(parents: &[Option<usize>], children: &[Vec<usize>]) -> Vec<usize> {
    let mut order = Vec::with_capacity(parents.len());
    let mut seen = HashSet::with_capacity(parents.len());

    for root in (0..parents.len()).filter(|&index| parents[index].is_none()) {
        let mut stack = vec![(root, false)];
        while let Some((index, expanded)) = stack.pop() {
            if expanded {
                order.push(index);
                continue;
            }
            if !seen.insert(index) {
                continue;
            }
            stack.push((index, true));
            for &child in children[index].iter().rev() {
                stack.push((child, false));
            }
        }
    }

    order
}

/// What changed between two ticks.
///
/// `changed` is the field that decides whether this is a diff or a full snapshot
/// wearing a diff's name — see [`differs_on_screen`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProcessDiff {
    /// Processes that were not in the previous tick.
    pub added: Vec<ProcessRecord>,
    /// Identities that were in the previous tick and are gone.
    ///
    /// Carries the full [`ProcessKey`] rather than a bare PID: when a PID is
    /// recycled within one tick, the same number appears here as removed and in
    /// `added` as a new process, and the start time is what tells them apart.
    pub removed: Vec<ProcessKey>,
    /// Processes whose displayed values moved.
    pub changed: Vec<ProcessRecord>,
}

impl ProcessDiff {
    /// Whether nothing at all changed between the two ticks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Computes what to send the front-end for this tick.
///
/// Records are matched by [`ProcessKey`], so a recycled PID reads as one process
/// ending and a different one starting rather than as a single process that
/// mysteriously restarted.
#[must_use]
pub fn diff_processes(previous: &[ProcessRecord], next: &[ProcessRecord]) -> ProcessDiff {
    let before: HashMap<ProcessKey, &ProcessRecord> = previous
        .iter()
        .map(|record| (record.key(), record))
        .collect();
    let mut surviving = HashSet::with_capacity(next.len());

    let mut diff = ProcessDiff::default();

    for record in next {
        let key = record.key();
        surviving.insert(key);

        match before.get(&key) {
            None => diff.added.push(record.clone()),
            Some(earlier) if differs_on_screen(earlier, record) => {
                diff.changed.push(record.clone());
            }
            Some(_) => {}
        }
    }

    for key in before.keys() {
        if !surviving.contains(key) {
            diff.removed.push(*key);
        }
    }

    diff.removed.sort_unstable();
    diff
}

/// Whether two readings of the same process would look different on screen.
///
/// This is the load-bearing decision in the whole diff. Compared at raw
/// precision, a CPU float moves for nearly every process on nearly every tick,
/// every record lands in `changed`, and the diff degenerates into a full
/// snapshot — with all the cost ADR-007 was trying to avoid and none of the
/// benefit. Comparing at the precision the UI actually renders means a process
/// is reported as changed when, and only when, a user could see it change.
fn differs_on_screen(before: &ProcessRecord, after: &ProcessRecord) -> bool {
    deci_percent(before.cpu) != deci_percent(after.cpu)
        || before.memory / MEBIBYTE != after.memory / MEBIBYTE
        || before.disk_read_rate / KIBIBYTE != after.disk_read_rate / KIBIBYTE
        || before.disk_write_rate / KIBIBYTE != after.disk_write_rate / KIBIBYTE
        || before.status != after.status
        || before.name != after.name
        || before.user != after.user
}

/// CPU percentage at the tenth of a percent the UI renders.
///
/// Saturating rather than wrapping on absurd input, and NaN-tolerant: a garbage
/// reading from a platform must not be able to panic the sampler.
#[allow(
    clippy::cast_possible_truncation,
    reason = "clamped to a range that fits i32 before the cast"
)]
fn deci_percent(value: f32) -> i32 {
    if value.is_nan() {
        return 0;
    }
    (value.clamp(0.0, 100_000.0) * 10.0).round() as i32
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn record(pid: u32, parent_pid: Option<u32>) -> ProcessRecord {
        ProcessRecord {
            pid,
            parent_pid,
            name: format!("proc-{pid}"),
            exe: None,
            user: Some("tester".into()),
            cpu: 1.0,
            memory: MEBIBYTE,
            disk_read_rate: 0,
            disk_write_rate: 0,
            started_at: 1_000,
            status: ProcessStatus::Running,
        }
    }

    fn costed(pid: u32, parent_pid: Option<u32>, cpu: f32, memory: u64) -> ProcessRecord {
        ProcessRecord {
            cpu,
            memory,
            ..record(pid, parent_pid)
        }
    }

    /// Collects every node in a tree, regardless of depth.
    fn walk(tree: &ProcessTree) -> Vec<u32> {
        fn visit(node: &ProcessNode, out: &mut Vec<u32>) {
            out.push(node.record.pid);
            for child in &node.children {
                visit(child, out);
            }
        }

        let mut out = Vec::new();
        for root in &tree.roots {
            visit(root, &mut out);
        }
        out.sort_unstable();
        out
    }

    #[test]
    fn an_empty_snapshot_builds_an_empty_tree() {
        let tree = ProcessTree::build(Vec::new());

        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
        assert!(tree.roots.is_empty());
    }

    #[test]
    fn children_hang_off_their_parent() {
        let tree = ProcessTree::build(vec![
            record(1, None),
            record(2, Some(1)),
            record(3, Some(1)),
            record(4, Some(2)),
        ]);

        assert_eq!(tree.roots.len(), 1);
        let root = &tree.roots[0];
        assert_eq!(root.record.pid, 1);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].record.pid, 2);
        assert_eq!(root.children[0].children[0].record.pid, 4);
    }

    #[test]
    fn children_are_ordered_by_pid_so_the_tree_is_stable_between_ticks() {
        let tree = ProcessTree::build(vec![
            record(1, None),
            record(9, Some(1)),
            record(3, Some(1)),
            record(7, Some(1)),
        ]);

        let order: Vec<u32> = tree.roots[0]
            .children
            .iter()
            .map(|child| child.record.pid)
            .collect();
        assert_eq!(order, vec![3, 7, 9]);
    }

    #[test]
    fn an_orphan_becomes_a_root_rather_than_disappearing() {
        // PID 2's parent exited between the two reads that made this snapshot.
        let tree = ProcessTree::build(vec![record(1, None), record(2, Some(404))]);

        assert_eq!(tree.len(), 2, "the orphan is still counted");
        assert_eq!(walk(&tree), vec![1, 2]);
        assert_eq!(tree.roots.len(), 2);
    }

    #[test]
    fn a_self_parenting_process_becomes_a_root() {
        // PID 0 names itself as its own parent on more than one platform.
        let tree = ProcessTree::build(vec![record(0, Some(0)), record(1, Some(0))]);

        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.roots[0].record.pid, 0);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn a_two_process_cycle_is_broken_and_neither_process_is_lost() {
        let tree = ProcessTree::build(vec![record(1, Some(2)), record(2, Some(1))]);

        assert_eq!(tree.len(), 2);
        assert_eq!(walk(&tree), vec![1, 2]);
    }

    #[test]
    fn a_long_cycle_is_broken_and_every_process_survives() {
        let tree = ProcessTree::build(vec![
            record(1, Some(4)),
            record(2, Some(1)),
            record(3, Some(2)),
            record(4, Some(3)),
        ]);

        assert_eq!(tree.len(), 4);
        assert_eq!(walk(&tree), vec![1, 2, 3, 4]);
    }

    #[test]
    fn cycle_breaking_is_deterministic() {
        let build = || {
            ProcessTree::build(vec![
                record(5, Some(7)),
                record(7, Some(9)),
                record(9, Some(5)),
            ])
        };

        assert_eq!(build(), build(), "the same snapshot yields the same tree");
    }

    #[test]
    fn a_cycle_with_an_innocent_subtree_keeps_the_subtree_attached() {
        let tree = ProcessTree::build(vec![
            record(1, Some(2)),
            record(2, Some(1)),
            record(3, Some(1)),
        ]);

        assert_eq!(tree.len(), 3);
        assert_eq!(walk(&tree), vec![1, 2, 3]);
    }

    #[test]
    fn a_duplicate_pid_is_counted_once() {
        let tree = ProcessTree::build(vec![record(1, None), record(1, None)]);

        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn a_deep_chain_does_not_overflow_the_stack() {
        let mut records = vec![record(1, None)];
        for pid in 2..5_000 {
            records.push(record(pid, Some(pid - 1)));
        }

        let tree = ProcessTree::build(records);

        assert_eq!(tree.len(), 4_999);
    }

    #[test]
    fn a_collapsed_parent_reports_the_cost_of_its_whole_subtree() {
        let tree = ProcessTree::build(vec![
            costed(1, None, 1.0, 100),
            costed(2, Some(1), 2.0, 200),
            costed(3, Some(2), 4.0, 400),
        ]);

        let root = &tree.roots[0];
        assert!((root.rollup.cpu - 7.0).abs() < 1e-9, "1 + 2 + 4");
        assert_eq!(root.rollup.memory, 700);
        assert_eq!(root.rollup.processes, 3);
    }

    #[test]
    fn a_leaf_rolls_up_only_itself() {
        let tree = ProcessTree::build(vec![costed(1, None, 3.0, 300)]);

        assert!((tree.roots[0].rollup.cpu - 3.0).abs() < 1e-9);
        assert_eq!(tree.roots[0].rollup.processes, 1);
    }

    #[test]
    fn the_total_equals_the_sum_of_every_record_however_the_tree_is_shaped() {
        // The invariant that makes collapsing safe: nothing is double-counted
        // and nothing is dropped, whatever the parentage looks like.
        let records = vec![
            costed(1, None, 0.5, 10),
            costed(2, Some(1), 1.5, 20),
            costed(3, Some(2), 2.5, 30),
            costed(4, Some(99), 3.5, 40), // orphan
            costed(5, Some(6), 4.5, 50),  // cycle with 6
            costed(6, Some(5), 5.5, 60),
            costed(7, Some(7), 6.5, 70), // self-parenting
        ];
        let expected_cpu: f64 = records.iter().map(|r| f64::from(r.cpu)).sum();
        let expected_memory: u64 = records.iter().map(|r| r.memory).sum();
        let expected_count = u32::try_from(records.len()).unwrap();

        let tree = ProcessTree::build(records);

        assert!((tree.total.cpu - expected_cpu).abs() < 1e-9);
        assert_eq!(tree.total.memory, expected_memory);
        assert_eq!(tree.total.processes, expected_count);
    }

    #[test]
    fn summing_many_processes_does_not_drift() {
        // f32 accumulation would visibly under-report here; the roll-up sums in f64.
        let mut records = vec![costed(1, None, 0.0, 0)];
        for pid in 2..1_002 {
            records.push(costed(pid, Some(1), 0.1, 0));
        }

        let tree = ProcessTree::build(records);

        assert!(
            (tree.total.cpu - 100.0).abs() < 0.01,
            "1000 x 0.1 should be 100, got {}",
            tree.total.cpu
        );
    }

    #[test]
    fn a_new_process_is_reported_as_added() {
        let before = vec![record(1, None)];
        let after = vec![record(1, None), record(2, None)];

        let diff = diff_processes(&before, &after);

        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].pid, 2);
        assert!(diff.removed.is_empty());
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn a_departed_process_is_reported_as_removed_with_its_full_identity() {
        let before = vec![record(1, None), record(2, None)];
        let after = vec![record(1, None)];

        let diff = diff_processes(&before, &after);

        assert_eq!(
            diff.removed,
            vec![ProcessKey {
                pid: 2,
                started_at: 1_000
            }]
        );
    }

    #[test]
    fn an_unchanged_tick_produces_an_empty_diff() {
        let before = vec![record(1, None), record(2, Some(1))];
        let after = before.clone();

        assert!(diff_processes(&before, &after).is_empty());
    }

    #[test]
    fn a_recycled_pid_reads_as_one_process_ending_and_another_starting() {
        let before = vec![ProcessRecord {
            started_at: 1_000,
            ..record(42, None)
        }];
        let after = vec![ProcessRecord {
            started_at: 2_000,
            name: "something-else".into(),
            ..record(42, None)
        }];

        let diff = diff_processes(&before, &after);

        assert_eq!(diff.added.len(), 1, "the new process is an addition");
        assert_eq!(diff.added[0].started_at, 2_000);
        assert_eq!(
            diff.removed,
            vec![ProcessKey {
                pid: 42,
                started_at: 1_000
            }],
            "the old one is removed, distinguished by its start time"
        );
        assert!(
            diff.changed.is_empty(),
            "a recycled PID must never look like the same process updating"
        );
    }

    #[test]
    fn cpu_noise_below_display_precision_is_not_reported_as_a_change() {
        // The whole point of the diff: without this, every process changes every tick.
        let before = vec![costed(1, None, 12.34, MEBIBYTE)];
        let after = vec![costed(1, None, 12.3401, MEBIBYTE)];

        assert!(
            diff_processes(&before, &after).is_empty(),
            "a change a user cannot see is not a change"
        );
    }

    #[test]
    fn cpu_movement_at_display_precision_is_reported() {
        let before = vec![costed(1, None, 12.3, MEBIBYTE)];
        let after = vec![costed(1, None, 12.5, MEBIBYTE)];

        let diff = diff_processes(&before, &after);

        assert_eq!(diff.changed.len(), 1);
    }

    #[test]
    fn memory_noise_below_a_mebibyte_is_not_reported_as_a_change() {
        let before = vec![costed(1, None, 1.0, 8 * MEBIBYTE)];
        let after = vec![costed(1, None, 1.0, 8 * MEBIBYTE + 1_024)];

        assert!(diff_processes(&before, &after).is_empty());
    }

    #[test]
    fn memory_movement_of_a_whole_mebibyte_is_reported() {
        let before = vec![costed(1, None, 1.0, 8 * MEBIBYTE)];
        let after = vec![costed(1, None, 1.0, 9 * MEBIBYTE)];

        assert_eq!(diff_processes(&before, &after).changed.len(), 1);
    }

    #[test]
    fn a_status_change_is_reported_even_with_identical_numbers() {
        let before = vec![record(1, None)];
        let after = vec![ProcessRecord {
            status: ProcessStatus::Zombie,
            ..record(1, None)
        }];

        assert_eq!(diff_processes(&before, &after).changed.len(), 1);
    }

    #[test]
    fn a_garbage_cpu_reading_does_not_panic_the_diff() {
        let before = vec![costed(1, None, f32::NAN, 0)];
        let after = vec![costed(1, None, f32::INFINITY, 0)];

        let diff = diff_processes(&before, &after);

        assert_eq!(
            diff.changed.len(),
            1,
            "NaN and infinity are distinguishable"
        );
    }

    #[test]
    fn removals_are_ordered_so_the_payload_is_reproducible() {
        let before = vec![record(9, None), record(3, None), record(5, None)];
        let after = Vec::new();

        let diff = diff_processes(&before, &after);

        let pids: Vec<u32> = diff.removed.iter().map(|key| key.pid).collect();
        assert_eq!(pids, vec![3, 5, 9]);
    }

    #[test]
    fn records_serialise_with_camel_case_keys() {
        let json = serde_json::to_value(record(1, Some(0))).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("parentPid"));
        assert!(object.contains_key("diskReadRate"));
        assert!(object.contains_key("startedAt"));
    }

    #[test]
    fn a_tree_round_trips_through_json() {
        let original = ProcessTree::build(vec![record(1, None), record(2, Some(1))]);
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: ProcessTree = serde_json::from_str(&encoded).unwrap();

        assert_eq!(original, decoded);
    }
}
