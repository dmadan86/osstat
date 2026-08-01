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

/// Reads every DRM card that reports its memory.
///
/// GTT — the graphics translation table — is the aperture through which a GPU
/// addresses system memory, and is the Linux equivalent of the shared pool
/// Windows calls "shared GPU memory".
///
/// amdgpu only. `i915`, `xe` and NVIDIA's proprietary driver expose no
/// adapter-wide equivalent, so on those cards this reports nothing. Borrowing
/// amdgpu's numbers for a card that did not report them would be a
/// fabrication, which ADR-008 forbids outright.
pub(crate) fn adapter_memory() -> Vec<crate::AdapterMemory> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                // `card0`, but not `card0-DP-1`, which is a connector.
                name.strip_prefix("card").is_some_and(|rest| {
                    !rest.is_empty() && rest.chars().all(|unit| unit.is_ascii_digit())
                })
            })
        })
        .filter_map(|entry| describe_card(&entry.path().join("device")))
        .collect()
}

/// Reads one card's memory, or skips it if the driver reports none.
fn describe_card(device: &std::path::Path) -> Option<crate::AdapterMemory> {
    let vram_total = read_mem_info(device, "mem_info_vram_total");
    let shared_total = read_mem_info(device, "mem_info_gtt_total");

    // A card whose driver exposes neither file is not an error and not a zero;
    // it is a card this module has nothing to say about.
    if vram_total.is_none() && shared_total.is_none() {
        return None;
    }

    Some(crate::AdapterMemory {
        pci_vendor: read_pci_id(device, "vendor")?,
        pci_device: read_pci_id(device, "device")?,
        // DRM has no adapter name; the PCI pair is the join.
        name: None,
        vram_total,
        vram_used: vram_total.and(read_mem_info(device, "mem_info_vram_used")),
        shared_total,
        shared_used: shared_total.and(read_mem_info(device, "mem_info_gtt_used")),
        source: osstat_core::GpuSource::DrmSysfs,
    })
}

/// Reads one `mem_info_*` file as a byte count.
fn read_mem_info(device: &std::path::Path, file: &str) -> Option<u64> {
    crate::gpu_memory::parse_sysfs_u64(&std::fs::read_to_string(device.join(file)).ok()?)
}

/// Reads a PCI `vendor` or `device` file, which sysfs writes as `0x1002`.
fn read_pci_id(device: &std::path::Path, file: &str) -> Option<u32> {
    let contents = std::fs::read_to_string(device.join(file)).ok()?;
    u32::from_str_radix(contents.trim().strip_prefix("0x")?, 16).ok()
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
