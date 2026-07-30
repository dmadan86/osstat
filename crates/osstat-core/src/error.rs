//! The error type shared by every osstat capability.

use std::path::PathBuf;

/// Result alias used throughout osstat.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Anything that can go wrong while inspecting or modifying the system.
///
/// The variants are deliberately coarse: the UI only needs enough structure to
/// decide how to react. [`Error::PermissionDenied`] is the important one — it is
/// the single signal that drives the "retry elevated" affordance described in
/// ADR-006, so callers must never flatten it into a generic I/O failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The capability has no implementation on the current platform.
    #[error("not supported on this platform: {0}")]
    Unsupported(String),

    /// The operation failed because the process lacks the required privileges.
    ///
    /// Surfacing this variant (rather than a bare I/O error) is what lets the
    /// UI offer a single, narrowly-scoped elevated retry.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// A path the caller expected to exist does not.
    #[error("not found: {}", .path.display())]
    NotFound {
        /// The path that could not be resolved.
        path: PathBuf,
    },

    /// The process at this PID is not the one the caller meant.
    ///
    /// Raised when a [`crate::process::ProcessKey`]'s start time no longer
    /// matches the process now holding its PID. The original exited and the
    /// number was reused, so signalling would end something the user never
    /// selected. Deliberately not a permission problem: elevation would not
    /// help, and offering it would be an invitation to force a mistake.
    #[error("pid {pid} has been reused by a different process; nothing was signalled")]
    IdentityMismatch {
        /// The PID that was reused.
        pid: u32,
    },

    /// An underlying I/O failure that is not a permission problem.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether this failure could plausibly succeed if retried with elevated
    /// privileges.
    ///
    /// Used by the front-end to decide whether to show the elevation prompt, so
    /// it also recognises [`std::io::ErrorKind::PermissionDenied`] arriving
    /// through the [`Error::Io`] variant.
    #[must_use]
    pub fn is_permission_denied(&self) -> bool {
        match self {
            Self::PermissionDenied(_) => true,
            Self::Io(io) => io.kind() == std::io::ErrorKind::PermissionDenied,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn permission_denied_variant_is_recognised() {
        let err = Error::PermissionDenied("kill pid 4".into());
        assert!(err.is_permission_denied());
    }

    #[test]
    fn permission_denied_is_recognised_through_io() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(Error::from(io).is_permission_denied());
    }

    #[test]
    fn other_io_errors_do_not_offer_elevation() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert!(!Error::from(io).is_permission_denied());
    }

    #[test]
    fn unsupported_does_not_offer_elevation() {
        assert!(!Error::Unsupported("sensors".into()).is_permission_denied());
    }

    #[test]
    fn not_found_renders_the_path() {
        let err = Error::NotFound {
            path: PathBuf::from("/tmp/missing"),
        };
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn an_identity_mismatch_is_not_a_permission_problem() {
        // It must never offer "Retry elevated": elevation would not help, and
        // the whole point is that we refused to signal anything at all.
        let error = Error::IdentityMismatch { pid: 4321 };

        assert!(!error.is_permission_denied());
    }

    #[test]
    fn an_identity_mismatch_names_the_pid_that_was_reused() {
        let error = Error::IdentityMismatch { pid: 4321 };

        assert!(error.to_string().contains("4321"));
    }
}
