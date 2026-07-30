//! Linux implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: `/proc` and signal-based process
//! control, socket enumeration, systemd user units and `~/.config/autostart`
//! startup entries, and the Polkit (`pkexec`) elevation helper (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::Linux;
pub(crate) const DISPLAY_NAME: &str = "Linux";

/// Names a disk the way a Linux user would.
///
/// Linux users navigate by mount point, not by device node: `/home` is the
/// useful name and `/dev/nvme0n1p2` is trivia. The device is kept only as a
/// fallback for the rare mount that reports no path.
pub(crate) fn disk_display_name(device: &str, mount_point: &str) -> String {
    match mount_point.trim() {
        "" => device.trim().to_owned(),
        mount_point => mount_point.to_owned(),
    }
}

/// Makes a file runnable by its owner.
///
/// Adds the owner-execute bit rather than replacing the mode, so whatever the
/// archive already set — group and other bits included — survives.
pub(crate) fn mark_executable(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o700);
    std::fs::set_permissions(path, permissions)
}

/// Ends a process that has already been identity-checked.
///
/// Uses `sysinfo`'s signal API rather than `libc::kill` so this crate keeps its
/// `#![deny(unsafe_code)]`. `kill_with` returns `None` when the platform cannot
/// send the signal at all, which on Unix means the signal is unsupported rather
/// than that the process refused it.
pub(crate) fn terminate(
    process: &sysinfo::Process,
    mode: osstat_core::TerminationMode,
) -> osstat_core::Result<osstat_core::Termination> {
    let signal = match mode {
        osstat_core::TerminationMode::Graceful => sysinfo::Signal::Term,
        osstat_core::TerminationMode::Forceful => sysinfo::Signal::Kill,
    };

    match process.kill_with(signal) {
        // `false` is the OS refusing. The overwhelmingly common reason is that
        // the process belongs to someone else, and ADR-006 needs that distinct.
        Some(false) => Err(osstat_core::Error::PermissionDenied(format!(
            "cannot signal pid {}",
            process.pid().as_u32()
        ))),
        Some(true) => Ok(osstat_core::Termination::Signalled),
        None => Err(osstat_core::Error::Unsupported(format!(
            "{signal:?} is not supported on this platform"
        ))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_mount_point_is_the_name() {
        assert_eq!(disk_display_name("/dev/nvme0n1p2", "/home"), "/home");
    }

    #[test]
    fn the_root_filesystem_keeps_its_slash() {
        assert_eq!(disk_display_name("/dev/sda1", "/"), "/");
    }

    #[test]
    fn a_mount_without_a_path_falls_back_to_the_device() {
        assert_eq!(disk_display_name("/dev/sdb1", ""), "/dev/sdb1");
    }
}
