//! macOS implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: signal-based process control, socket
//! enumeration, LaunchAgents and Login Items startup entries, unified-memory
//! reporting for Apple Silicon, and the privileged-helper elevation path
//! (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::MacOs;
pub(crate) const DISPLAY_NAME: &str = "macOS";

/// Names a disk the way a macOS user would.
///
/// Finder shows mounted volumes by their name, so `/Volumes/Backup` reads as
/// `Backup`. The boot volume is the exception: it mounts at `/` and calling it
/// by its device node would help nobody.
pub(crate) fn disk_display_name(device: &str, mount_point: &str) -> String {
    let mount_point = mount_point.trim();

    if let Some(volume) = mount_point.strip_prefix("/Volumes/")
        && !volume.is_empty()
    {
        return volume.to_owned();
    }

    match mount_point {
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
        // `kill(2)` failing is not one thing. `kill_with` collapses both ESRCH
        // (the process ended in the gap between our identity check and this
        // call — gone is what the caller wanted) and EPERM (a real refusal,
        // overwhelmingly because the process belongs to someone else) into the
        // same `false`. ADR-006 needs those distinct, so the errno is read back
        // here rather than trusted to `kill_with`'s boolean.
        Some(false) => {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                Ok(osstat_core::Termination::AlreadyGone)
            } else {
                Err(osstat_core::Error::PermissionDenied(format!(
                    "cannot signal pid {}",
                    process.pid().as_u32()
                )))
            }
        }
        Some(true) => Ok(osstat_core::Termination::Signalled),
        None => Err(osstat_core::Error::Unsupported(format!(
            "{signal:?} is not supported on this platform"
        ))),
    }
}

/// macOS reports no adapter memory, and that is the correct answer rather than
/// a gap.
///
/// Apple Silicon has one memory pool, not two: the GPU probe's existing
/// `GpuSource::UnifiedMemory` path already reports the whole of system memory
/// as the GPU's. A shared figure here would draw a second meter describing the
/// same bytes as the first.
///
/// The one figure genuinely missing is live GPU usage, reachable only through
/// the undocumented `PerformanceStatistics` dictionary on the `IOAccelerator`
/// `IOKit` service. ADR-008 requires every figure to carry a source the user
/// can weigh, and an API Apple can change in a point release is a poor
/// foundation for a number labelled "measured".
pub(crate) fn adapter_memory() -> Vec<crate::AdapterMemory> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_mounted_volume_is_named_as_finder_names_it() {
        assert_eq!(
            disk_display_name("/dev/disk4s1", "/Volumes/Backup"),
            "Backup"
        );
    }

    #[test]
    fn the_boot_volume_keeps_its_mount_point() {
        assert_eq!(disk_display_name("/dev/disk1s1", "/"), "/");
    }

    #[test]
    fn an_empty_volumes_path_does_not_produce_an_empty_name() {
        assert_eq!(disk_display_name("/dev/disk9", "/Volumes/"), "/Volumes/");
    }

    #[test]
    fn a_mount_without_a_path_falls_back_to_the_device() {
        assert_eq!(disk_display_name("/dev/disk9", ""), "/dev/disk9");
    }
}
