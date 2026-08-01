//! Confirming a PID still belongs to the process the caller meant.
//!
//! Five seconds separate a graceful termination attempt from a forceful one,
//! which is ample for a PID to be freed and reused. Signalling a recycled PID
//! would end a process the user never selected, with no error and no way to
//! tell afterwards. This is the check that prevents it.
//!
//! The start time is re-read through `sysinfo`, the same source that produced
//! the value stored in the [`ProcessKey`]. Parsing `/proc` or calling
//! `GetProcessTimes` directly would risk a unit or epoch mismatch against a
//! number `sysinfo` computed — and a comparison that is subtly wrong here fails
//! open, which is the one direction it must never fail.

use osstat_core::{Error, ProcessKey, Result};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// What a PID currently holds.
#[derive(Debug)]
pub(crate) enum Identity {
    /// The process is present and is the one the key describes.
    Matches,
    /// Nothing holds this PID any more.
    Gone,
}

/// Re-reads `key`'s PID and checks it is still the same process.
///
/// # Errors
///
/// [`Error::IdentityMismatch`] if a *different* process now holds the PID.
/// Being gone entirely is not an error — the caller wanted it gone.
pub(crate) fn verify(system: &mut System, key: ProcessKey) -> Result<Identity> {
    let pid = Pid::from_u32(key.pid);

    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );

    let Some(process) = system.process(pid) else {
        return Ok(Identity::Gone);
    };

    if process.start_time() == key.started_at {
        Ok(Identity::Matches)
    } else {
        Err(Error::IdentityMismatch { pid: key.pid })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// This test process's own key, read the way the app reads it.
    fn own_key() -> ProcessKey {
        let pid = Pid::from_u32(std::process::id());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let process = system.process(pid).expect("this process exists");

        ProcessKey {
            pid: std::process::id(),
            started_at: process.start_time(),
        }
    }

    #[test]
    fn our_own_key_verifies() {
        let mut system = System::new();

        assert!(matches!(
            verify(&mut system, own_key()).unwrap(),
            Identity::Matches
        ));
    }

    #[test]
    fn a_stale_start_time_is_refused() {
        // The whole point of the module. Same PID, different process.
        let mut system = System::new();
        let mut key = own_key();
        key.started_at = key.started_at.wrapping_sub(1);

        let error = verify(&mut system, key).unwrap_err();

        let mut checked = false;
        if let Error::IdentityMismatch { pid } = &error {
            assert_eq!(*pid, std::process::id());
            checked = true;
        }
        assert!(checked, "expected IdentityMismatch, got {error:?}");
    }

    #[test]
    fn a_pid_nothing_holds_is_gone_rather_than_an_error() {
        // The caller wanted it gone and it is gone. u32::MAX is never a real
        // PID on any of the three platforms — PID 0 does not qualify on
        // Windows, where sysinfo reports it as the System Idle Process.
        let mut system = System::new();
        let key = ProcessKey {
            pid: u32::MAX,
            started_at: 1,
        };

        assert!(matches!(verify(&mut system, key).unwrap(), Identity::Gone));
    }
}
