//! Processes: the flat records a platform reports, and the diff sent between
//! ticks.
//!
//! Everything here is a pure function over plain data, so identity and change
//! detection can be tested exhaustively on a CI runner with no interesting
//! processes on it at all.
//!
//! Two invariants carry the weight, and both are bugs that would be easy to
//! ship and hard to notice:
//!
//! 1. **A PID is not an identity.** Operating systems reuse them. Identity is
//!    [`ProcessKey`] — the PID *and* the start time. A tick that matches on PID
//!    alone will show a recycled PID as the same process quietly changing into
//!    something else.
//! 2. **A change the user cannot see is not a change.** See
//!    [`differs_on_screen`].
//!
//! # Why the tree is not built here
//!
//! Arranging these records by parentage lives in the front-end
//! (`ui/src/lib/processTree.ts`), not in this crate. Because the sampler sends
//! *diffs* rather than snapshots, the consumer has to maintain its own
//! structure to apply them to — so a tree built here could not be used by the
//! only thing that renders one, and keeping both would mean two
//! implementations of the same delicate orphan-and-cycle handling drifting
//! apart. One implementation, in the language of its consumer.

// The public types re-export from the crate root as `ProcessRecord`,
// `ProcessStatus` and so on; naming them `Record` and `Status` to satisfy the
// lint would make every call site read worse than the lint is worth.
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
}
