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
pub struct Terminator {
    system: System,
}

impl Terminator {
    /// Creates a terminator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            system: System::new(),
        }
    }
}

impl Default for Terminator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessController for Terminator {
    fn terminate(&mut self, key: ProcessKey, mode: TerminationMode) -> Result<Termination> {
        // Order matters: verify, then signal, with nothing in between that
        // could yield and let the PID be reused after the check.
        if matches!(identity::verify(&mut self.system, key)?, Identity::Gone) {
            return Ok(Termination::AlreadyGone);
        }

        let pid = Pid::from_u32(key.pid);
        let Some(process) = self.system.process(pid) else {
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
}
