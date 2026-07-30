//! Per-OS implementations of the capabilities declared in `osstat-core`.
//!
//! Exactly one of the `windows` / `linux` / `macos` modules is compiled, chosen
//! by `cfg(target_os)` and re-exported here as `imp` (ADR-003). Callers depend
//! only on the platform-neutral surface of this crate, so adding a fourth OS
//! later means adding a module and one `cfg` arm — never touching call sites.
//!
//! The three per-OS files are always visible to `cargo fmt`, but only the
//! selected one is type-checked on a given host. That is why CI runs the full
//! Windows / Linux / macOS matrix rather than a single runner.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};

pub mod netstat_source;
pub mod sysinfo_source;

pub use netstat_source::NetstatSource;
pub use sysinfo_source::SysinfoSource;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
compile_error!(
    "osstat supports Windows, Linux and macOS only; add a crates/osstat-platform/src/<os>.rs \
     module and a cfg arm in lib.rs to port it further"
);

/// The operating systems osstat ships for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "lowercase")]
pub enum PlatformId {
    /// Microsoft Windows.
    Windows,
    /// A Linux distribution.
    Linux,
    /// Apple macOS.
    MacOs,
}

impl PlatformId {
    /// The platform this binary was compiled for.
    pub const CURRENT: Self = imp::PLATFORM_ID;

    /// A stable lowercase identifier, suitable for keying cleaning rules.
    ///
    /// These strings are part of the rule-manifest format (the `os = [...]`
    /// field), so they must stay in sync with the manifest schema.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }
}

impl std::fmt::Display for PlatformId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returns the platform the binary was compiled for.
#[must_use]
pub const fn current() -> PlatformId {
    PlatformId::CURRENT
}

/// The human-readable name of the current platform, as its own users say it.
#[must_use]
pub const fn display_name() -> &'static str {
    imp::DISPLAY_NAME
}

/// Makes a file runnable by its owner.
///
/// The only OS-specific step in acquiring the llama.cpp runtime: Unix needs the
/// executable bit set on a file that came out of an archive, and Windows
/// decides by extension and needs nothing. Keeping the no-op here rather than
/// at the call site spares `osstat-inference` a `cfg` branch, which is the
/// whole point of ADR-003's arrangement.
///
/// # Errors
///
/// If the mode cannot be read or written — a `noexec` mount or a policy denying
/// the change. The caller reports the path, so both are diagnosable rather than
/// mysterious.
pub fn mark_executable(path: &std::path::Path) -> std::io::Result<()> {
    imp::mark_executable(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn current_platform_matches_the_compilation_target() {
        let expected = if cfg!(target_os = "windows") {
            PlatformId::Windows
        } else if cfg!(target_os = "linux") {
            PlatformId::Linux
        } else {
            PlatformId::MacOs
        };
        assert_eq!(current(), expected);
    }

    #[test]
    fn identifiers_are_the_manifest_spellings() {
        assert_eq!(PlatformId::Windows.as_str(), "windows");
        assert_eq!(PlatformId::Linux.as_str(), "linux");
        assert_eq!(PlatformId::MacOs.as_str(), "macos");
    }

    #[test]
    fn serialises_to_the_same_spelling_as_as_str() {
        for platform in [PlatformId::Windows, PlatformId::Linux, PlatformId::MacOs] {
            let encoded = serde_json::to_string(&platform).unwrap();
            assert_eq!(encoded, format!("\"{}\"", platform.as_str()));
        }
    }

    #[test]
    fn display_name_is_populated() {
        assert!(!display_name().is_empty());
    }

    #[test]
    fn marking_a_file_executable_succeeds_on_every_platform() {
        // Windows has nothing to set and must still report success, because the
        // caller has no cfg branch to fall back on.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llama-server");
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();

        mark_executable(&path).expect("marking a plain file executable must succeed");
    }

    #[test]
    #[cfg(unix)]
    fn marking_a_file_executable_sets_the_owner_bit() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llama-server");
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        mark_executable(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o100, 0o100, "owner execute bit was not set");
        assert_eq!(
            mode & 0o600,
            0o600,
            "the existing read/write bits were lost"
        );
    }

    #[test]
    fn marking_a_file_that_is_not_there_fails_rather_than_succeeding_quietly() {
        // On Unix this is a real stat failure. On Windows the no-op returns Ok,
        // which is correct: there is no permission to set either way, and the
        // caller checks the server exists separately.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent");

        let result = mark_executable(&missing);

        #[cfg(unix)]
        assert!(result.is_err(), "stat of a missing file must fail");
        #[cfg(windows)]
        assert!(result.is_ok(), "Windows has no bit to set, present or not");
    }
}
