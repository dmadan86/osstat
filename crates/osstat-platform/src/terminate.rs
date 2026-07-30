//! The [`ProcessController`] implementation.
//!
//! Identity is verified here, once, for every platform — the check is the same
//! everywhere and duplicating it per OS would mean three chances to get the one
//! safety-critical comparison wrong.

use osstat_core::{ProcessController, ProcessKey, Result, Termination, TerminationMode};
use sysinfo::{Pid, System};

use crate::identity::{self, Identity};
use crate::imp;

/// Ends processes, checking identity before it signals anything.
///
/// Deliberately holds no `System` (or any other state) between calls. `sysinfo`
/// caches per-PID data inside a `System` and, on Windows, `refresh_processes_specifics`
/// never re-reads `start_time` for a PID already in that cache — it only calls
/// `ProcessInner::update`, which touches CPU, memory and exe but not start time.
/// A `Terminator` that reused one `System` across the graceful call and the
/// forceful call five seconds later would, on Windows, compare a *stale*
/// `start_time` on the second call: if the PID had been freed and reused in
/// between, [`identity::verify`] would report `Matches` against the wrong
/// process. Building a fresh `System` per call means the target PID is always
/// new to it, which forces the real first-read path on every platform and
/// keeps the identity check honest. Termination is a deliberate, infrequent
/// user action, so the cost of a full process snapshot per call is not one
/// worth trading this guarantee away for.
#[derive(Debug, Default)]
pub struct Terminator;

impl Terminator {
    /// Creates a terminator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ProcessController for Terminator {
    fn terminate(&mut self, key: ProcessKey, mode: TerminationMode) -> Result<Termination> {
        // A `System` built fresh right here, used only for this one call. See
        // the struct docs: reusing one across calls is what makes the identity
        // check below trust stale data on Windows.
        let mut system = System::new();

        // Order matters: verify, then signal, with nothing in between that
        // could yield and let the PID be reused after the check.
        if matches!(identity::verify(&mut system, key)?, Identity::Gone) {
            return Ok(Termination::AlreadyGone);
        }

        let pid = Pid::from_u32(key.pid);
        let Some(process) = system.process(pid) else {
            // Exited between the check and here. Still what the caller wanted.
            return Ok(Termination::AlreadyGone);
        };

        imp::terminate(process, mode)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use osstat_core::Error;

    #[test]
    fn a_pid_that_holds_nothing_reports_already_gone() {
        // u32::MAX is never a real PID on any of the three platforms — PID 0
        // does not qualify on Windows, where sysinfo reports it as the System
        // Idle Process.
        let mut terminator = Terminator::new();

        let outcome = terminator
            .terminate(
                ProcessKey {
                    pid: u32::MAX,
                    started_at: 1,
                },
                TerminationMode::Graceful,
            )
            .unwrap();

        assert_eq!(outcome, Termination::AlreadyGone);
    }

    #[test]
    fn a_stale_key_is_refused_without_signalling() {
        // Uses this very test process as the target. If the identity check were
        // broken, this test would end the test runner — which is exactly the
        // failure it is guarding against, and a loud way to find out.
        let mut terminator = Terminator::new();
        let mut system = System::new();
        let pid = Pid::from_u32(std::process::id());
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let started_at = system.process(pid).unwrap().start_time();

        let error = terminator
            .terminate(
                ProcessKey {
                    pid: std::process::id(),
                    started_at: started_at.wrapping_add(1),
                },
                TerminationMode::Forceful,
            )
            .unwrap_err();

        assert!(matches!(error, Error::IdentityMismatch { .. }));
    }

    #[test]
    fn two_calls_on_the_same_terminator_verify_independently() {
        // Regression guard for the bug the struct doc explains: a Terminator
        // that cached a `System` across calls would, on Windows, keep trusting
        // the first call's start_time forever, because `update()` never
        // re-reads it for a PID the System has already seen. Two calls on one
        // instance must each get a fresh, correct answer rather than the
        // second inheriting whatever the first one cached.
        let mut terminator = Terminator::new();
        let pid = std::process::id();
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let real_started_at = system.process(Pid::from_u32(pid)).unwrap().start_time();

        let first = terminator.terminate(
            ProcessKey {
                pid,
                started_at: real_started_at.wrapping_add(1),
            },
            TerminationMode::Graceful,
        );
        assert!(matches!(first, Err(Error::IdentityMismatch { .. })));

        let second = terminator
            .terminate(
                ProcessKey {
                    pid: u32::MAX,
                    started_at: 1,
                },
                TerminationMode::Graceful,
            )
            .unwrap();
        assert_eq!(second, Termination::AlreadyGone);
    }
}
