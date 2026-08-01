//! Adapter memory: the pools a GPU can draw on, and how to name them.
//!
//! Two pools exist on most machines. **Dedicated** memory belongs to the card.
//! **Shared** memory is system RAM the GPU borrows when its own runs out —
//! what Windows Task Manager calls "shared GPU memory" and what Linux exposes
//! as the GTT aperture. Shared memory is roughly a tenth the bandwidth, which
//! is why it is reported beside the dedicated pool rather than summed into it.
//!
//! Everything here is portable and compiled on every platform, so the Windows
//! instance-name parser and the Linux sysfs parser are both covered by tests
//! that run on any developer's machine. Only the calls that fetch raw figures
//! live in the per-OS modules.

use osstat_core::GpuSource;

/// One adapter's memory, as a platform reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterMemory {
    /// PCI vendor ID, for matching against `wgpu`'s `AdapterInfo::vendor`.
    pub pci_vendor: u32,
    /// PCI device ID, for matching against `wgpu`'s `AdapterInfo::device`.
    pub pci_device: u32,
    /// The adapter's name, where the platform supplies one.
    pub name: Option<String>,
    /// Dedicated video memory in bytes.
    pub vram_total: Option<u64>,
    /// Dedicated video memory in use, in bytes.
    pub vram_used: Option<u64>,
    /// System memory this adapter may borrow, in bytes.
    pub shared_total: Option<u64>,
    /// Borrowed system memory in use, in bytes.
    pub shared_used: Option<u64>,
    /// Which platform interface produced these figures.
    pub source: GpuSource,
}

/// A Windows `GPU Adapter Memory` counter instance, identifying one adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LuidKey {
    /// The adapter LUID's `HighPart`.
    pub high: u32,
    /// The adapter LUID's `LowPart`.
    pub low: u32,
    /// Which physical adapter behind that LUID.
    pub phys: u32,
}

/// Treats a reported zero as "not known" rather than "measured as none".
///
/// DXGI returns `DedicatedVideoMemory: 0` for an adapter with no VRAM of its
/// own. Both render as no meter today, but they are different claims, and the
/// difference surfaces the moment anything divides by the total.
#[must_use]
pub const fn non_zero(value: u64) -> Option<u64> {
    if value == 0 { None } else { Some(value) }
}

/// Parses a `GPU Adapter Memory` instance name into the adapter it identifies.
///
/// Instances are `luid_0xHHHHHHHH_0xLLLLLLLL_phys_N`. Anything else — an
/// instance from a different counter set, a truncated name, a non-hex field —
/// yields `None`. A partial parse would key a real figure to the wrong card,
/// which is worse than not reading it at all.
#[must_use]
pub fn parse_luid_instance(instance: &str) -> Option<LuidKey> {
    let rest = instance.strip_prefix("luid_")?;
    let (high, rest) = rest.split_once('_')?;
    let (low, phys) = rest.split_once("_phys_")?;

    Some(LuidKey {
        high: u32::from_str_radix(high.strip_prefix("0x")?, 16).ok()?,
        low: u32::from_str_radix(low.strip_prefix("0x")?, 16).ok()?,
        phys: phys.parse().ok()?,
    })
}

/// Parses one of amdgpu's `mem_info_*` files: a decimal byte count.
///
/// `None` for anything else, including the `0` these files report for a pool
/// that exists but is empty — see [`non_zero`].
#[must_use]
pub fn parse_sysfs_u64(contents: &str) -> Option<u64> {
    non_zero(contents.trim().parse().ok()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_well_formed_luid_instance_parses() {
        let key = parse_luid_instance("luid_0x00000000_0x0001068c_phys_0").unwrap();

        assert_eq!(key.high, 0x0000_0000);
        assert_eq!(key.low, 0x0001_068c);
        assert_eq!(key.phys, 0);
    }

    #[test]
    fn a_second_physical_adapter_behind_one_luid_is_distinguished() {
        let first = parse_luid_instance("luid_0x00000000_0x0001068c_phys_0").unwrap();
        let second = parse_luid_instance("luid_0x00000000_0x0001068c_phys_1").unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn malformed_luid_instances_yield_nothing_rather_than_a_partial_parse() {
        // A half-read instance name would key a reading to the wrong adapter,
        // which is worse than not reading it: the figure would be real and
        // attributed to the wrong card.
        for bad in [
            "",
            "luid_0x00000000",
            "engtype_3D",
            "luid_0x00000000_0x0001068c",
            "luid_0xZZZZZZZZ_0x0001068c_phys_0",
            "luid_0x00000000_0x0001068c_phys_x",
            "pid_1234_luid_0x0_0x1_phys_0",
        ] {
            assert!(
                parse_luid_instance(bad).is_none(),
                "{bad:?} should not have parsed"
            );
        }
    }

    #[test]
    fn a_sysfs_value_parses_past_its_trailing_newline() {
        assert_eq!(parse_sysfs_u64("17163091968\n"), Some(17_163_091_968));
        assert_eq!(parse_sysfs_u64("  8589934592  "), Some(8_589_934_592));
    }

    #[test]
    fn unreadable_sysfs_contents_yield_nothing() {
        for bad in ["", "\n", "N/A", "-1", "0x1000", "12 34"] {
            assert!(parse_sysfs_u64(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_reported_zero_is_unknown_not_a_measurement_of_none() {
        // DXGI reports DedicatedVideoMemory: 0 for an adapter with no VRAM of
        // its own. Some(0) and None render the same today, but they mean
        // different things and differ the moment anything divides by the total.
        assert_eq!(non_zero(0), None);
        assert_eq!(non_zero(1), Some(1));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_reports_usable_readings_or_none_at_all() {
        // Every Windows machine with a display has a DXGI adapter; a CI runner
        // may have only the software one, which is filtered out. Either is a
        // pass. What must not happen is a reading that claims a figure without
        // identifying the adapter it belongs to, or a zero masquerading as a
        // measurement.
        for reading in crate::adapter_memory() {
            assert_ne!(
                (reading.pci_vendor, reading.pci_device),
                (0, 0),
                "a reading that cannot be matched to a device is unusable"
            );
            assert_eq!(reading.source, osstat_core::GpuSource::Dxgi);
            assert_ne!(reading.vram_total, Some(0), "zero must normalise to None");
            assert_ne!(reading.shared_total, Some(0));
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_finds_no_software_adapter() {
        // DXGI enumerates the Microsoft Basic Render Driver independently of
        // wgpu, so the exclusion the GPU probe already makes has to be made
        // again here.
        for reading in crate::adapter_memory() {
            let name = reading.name.unwrap_or_default().to_lowercase();
            assert!(
                !name.contains("basic render"),
                "the software rasteriser reached the reading list"
            );
        }
    }
}
