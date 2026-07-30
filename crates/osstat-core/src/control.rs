//! Ending a process.
//!
//! The trait is here and its implementations are per-OS, following ADR-003. The
//! types are shared because the escalation they describe is the same everywhere
//! even though the mechanism is not: on Unix "graceful" is `SIGTERM`, on Windows
//! it is `WM_CLOSE` posted to the process's windows.

use crate::error::Result;
use crate::process::ProcessKey;

/// How hard to try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminationMode {
    /// Ask the process to exit and let it save first.
    ///
    /// `SIGTERM` on Unix; `WM_CLOSE` posted to each top-level window on Windows.
    Graceful,
    /// End it now, with no chance to save.
    ///
    /// `SIGKILL` on Unix; `TerminateProcess` on Windows.
    Forceful,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Termination {
    /// The request was delivered. Whether the process obeys is up to it.
    Signalled,
    /// It had already exited. Nothing was signalled, and that is a success:
    /// the caller wanted it gone and it is gone.
    AlreadyGone,
    /// Windows only: the process has no top-level window, so there is nothing
    /// to post `WM_CLOSE` to.
    ///
    /// Not a failure. It tells the front end to offer the forceful step
    /// immediately rather than making the user watch a five-second wait whose
    /// outcome is already known.
    NoWindowToClose,
}

/// Ends processes.
pub trait ProcessController {
    /// Ends the process identified by `key`.
    ///
    /// Implementations **must** re-read the PID's start time immediately before
    /// signalling and return [`crate::Error::IdentityMismatch`] if it differs.
    /// A PID freed between two calls can be reused, and this check is the only
    /// thing standing between the user and a process they never selected.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PermissionDenied`] if the process belongs to another
    /// user, [`crate::Error::IdentityMismatch`] if the PID was reused, and
    /// [`crate::Error::Io`] for anything the OS refused for another reason.
    fn terminate(&mut self, key: ProcessKey, mode: TerminationMode) -> Result<Termination>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn modes_serialise_as_camel_case_for_the_ipc_boundary() {
        assert_eq!(
            serde_json::to_string(&TerminationMode::Graceful).unwrap(),
            "\"graceful\""
        );
        assert_eq!(
            serde_json::to_string(&TerminationMode::Forceful).unwrap(),
            "\"forceful\""
        );
    }

    #[test]
    fn outcomes_serialise_as_camel_case_for_the_ipc_boundary() {
        assert_eq!(
            serde_json::to_string(&Termination::NoWindowToClose).unwrap(),
            "\"noWindowToClose\""
        );
        assert_eq!(
            serde_json::to_string(&Termination::AlreadyGone).unwrap(),
            "\"alreadyGone\""
        );
    }

    #[test]
    fn a_mode_round_trips() {
        let encoded = serde_json::to_string(&TerminationMode::Forceful).unwrap();
        let decoded: TerminationMode = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, TerminationMode::Forceful);
    }
}
