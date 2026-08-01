//! Memory figures from sources the NVML and `wgpu` probes cannot supply.
//!
//! ADR-008's three sources leave a gap: NVML measures only NVIDIA hardware, and
//! `wgpu` has no memory API at all, so every AMD and Intel adapter reports no
//! video memory whatsoever. Neither source knows about the shared pool — the
//! system RAM a GPU borrows when its own runs out — which Task Manager has
//! shown users for years.
//!
//! All parsing and merging is here rather than in the per-OS files, for the
//! reason `provider.rs` gives about `ProcessTree::build`: arranging data is
//! portable logic and belongs where it can be tested without an operating
//! system. It also means the Linux parser is covered by tests that run on a
//! Windows development machine, which is the only place they would otherwise
//! never run.

use osstat_core::{GpuDevice, GpuSample, GpuSource};

// Every platform, for now: `other.rs` is the only implementation that exists.
// Tasks 3 and 5 add an arm each as they add a file, so the cfg always names
// exactly the platforms that have a real implementation and the crate compiles
// everywhere at every point in the sequence. A placeholder returning nothing
// would be indistinguishable from `other.rs` and would ship a silent no-op if
// a later task were dropped.
#[path = "adapter_memory/other.rs"]
mod imp;

/// One adapter's memory, as a platform reported it.
///
/// Crate-internal: this never crosses the IPC boundary. It is folded into
/// [`GpuDevice`] and [`GpuSample`], which do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdapterMemory {
    /// PCI vendor ID, for matching against `wgpu`'s `AdapterInfo::vendor`.
    pub(crate) pci_vendor: u32,
    /// PCI device ID, for matching against `wgpu`'s `AdapterInfo::device`.
    pub(crate) pci_device: u32,
    /// The adapter's name, where the platform supplies one.
    pub(crate) name: Option<String>,
    /// Dedicated video memory in bytes.
    pub(crate) vram_total: Option<u64>,
    /// Dedicated video memory in use, in bytes.
    pub(crate) vram_used: Option<u64>,
    /// System memory this adapter may borrow, in bytes.
    pub(crate) shared_total: Option<u64>,
    /// Borrowed system memory in use, in bytes.
    pub(crate) shared_used: Option<u64>,
    /// Which platform interface produced these figures.
    pub(crate) source: GpuSource,
}

/// A Windows `GPU Adapter Memory` counter instance, identifying one adapter.
#[allow(
    dead_code,
    reason = "constructed by parse_luid_instance, whose only caller lands in Task 3"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LuidKey {
    /// The adapter LUID's `HighPart`.
    pub(crate) high: u32,
    /// The adapter LUID's `LowPart`.
    pub(crate) low: u32,
    /// Which physical adapter behind that LUID.
    pub(crate) phys: u32,
}

/// Reads this platform's adapter memory, or nothing where it has no source.
pub(crate) fn adapter_memory() -> Vec<AdapterMemory> {
    imp::adapter_memory()
}

/// Treats a reported zero as "not known" rather than "measured as none".
///
/// DXGI returns `DedicatedVideoMemory: 0` for an adapter with no VRAM of its
/// own. Both render as no meter today, but they are different claims, and the
/// difference surfaces the moment anything divides by the total.
#[allow(
    dead_code,
    reason = "called by the Windows module in Task 3 and via parse_sysfs_u64 in Task 5"
)]
pub(crate) const fn non_zero(value: u64) -> Option<u64> {
    if value == 0 { None } else { Some(value) }
}

/// Parses a `GPU Adapter Memory` instance name into the adapter it identifies.
///
/// Instances are `luid_0xHHHHHHHH_0xLLLLLLLL_phys_N`. Anything else — an
/// instance from a different counter set, a truncated name, a non-hex field —
/// yields `None`. A partial parse would key a real figure to the wrong card,
/// which is worse than not reading it at all.
#[allow(dead_code, reason = "lands ahead of its only caller, added in Task 3")]
pub(crate) fn parse_luid_instance(instance: &str) -> Option<LuidKey> {
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
#[allow(dead_code, reason = "lands ahead of its only caller, added in Task 5")]
pub(crate) fn parse_sysfs_u64(contents: &str) -> Option<u64> {
    non_zero(contents.trim().parse().ok()?)
}

/// Finds the reading that describes a device, by PCI ID and then by name.
///
/// PCI IDs are exact and are tried first. **Both halves must match**: a
/// vendor-only comparison would attribute one card's memory to another from the
/// same maker, which is a real configuration and not a rare one.
///
/// A known PCI pair that matches no reading returns `None` outright rather than
/// falling back to a name guess: the name fallback exists only for the case
/// where the device's own PCI pair is unknown (`(0, 0)`), not as a second
/// attempt after a real PCI pair failed to find its reading — a card with a
/// known PCI pair and a name shared by another card must not pick up the
/// other's figures.
///
/// The name fallback covers Windows, where a device found by NVML carries no
/// PCI pair here; `normalise` is the same comparison that already dedupes NVML
/// against `wgpu` in the parent module. Linux readings arrive nameless — DRM
/// has no adapter name — so there the PCI match is the only one that can fire.
fn reading_for<'a>(
    device: &GpuDevice,
    pci: (u32, u32),
    readings: &'a [AdapterMemory],
) -> Option<&'a AdapterMemory> {
    if pci != (0, 0) {
        return readings
            .iter()
            .find(|reading| (reading.pci_vendor, reading.pci_device) == pci);
    }

    readings.iter().find(|reading| {
        reading
            .name
            .as_ref()
            .is_some_and(|name| super::normalise(name) == super::normalise(&device.name))
    })
}

/// Folds platform readings into the device list.
///
/// Fills only what the device does not already know: NVML's dedicated figure is
/// never displaced (ADR-008 trusts it most), while a device that knew nothing
/// about its memory is promoted to the reading's source, so the front end stops
/// labelling a measured figure as an estimate.
pub(crate) fn apply_to_devices(
    devices: &mut [GpuDevice],
    pci: &[(u32, u32)],
    readings: &[AdapterMemory],
) {
    for (position, device) in devices.iter_mut().enumerate() {
        let key = pci.get(position).copied().unwrap_or((0, 0));
        let Some(reading) = reading_for(device, key, readings) else {
            continue;
        };

        if device.vram_total.is_none()
            && let Some(total) = reading.vram_total
        {
            device.vram_total = Some(total);
            device.source = reading.source;
        }

        if let Some(total) = reading.shared_total {
            device.shared_total = Some(total);
            device.shared_source = Some(reading.source);
        }
    }
}

/// Builds one sample per device, merging NVML's readings with the platform's.
///
/// Without this an AMD-only machine would produce no samples at all: NVML has
/// no handle there, and the parent module's `measure` returns early when that
/// is the case.
pub(crate) fn samples_from(
    devices: &[GpuDevice],
    pci: &[(u32, u32)],
    readings: &[AdapterMemory],
    nvml: &[GpuSample],
) -> Vec<GpuSample> {
    devices
        .iter()
        .enumerate()
        .map(|(position, device)| {
            let mut sample = nvml
                .iter()
                .find(|sample| sample.index == device.index)
                .copied()
                .unwrap_or_else(|| GpuSample::unmeasured(device.index));

            let key = pci.get(position).copied().unwrap_or((0, 0));
            if let Some(reading) = reading_for(device, key, readings) {
                sample.shared_used = reading.shared_used;
                // NVML measured this pool itself where it could; only fill a
                // gap it left.
                if sample.vram_used.is_none() {
                    sample.vram_used = reading.vram_used;
                }
            }

            sample
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use osstat_core::{GpuDevice, GpuKind, GpuSource};

    /// A device as `wgpu` would leave it: named, and knowing nothing else.
    ///
    /// `_vendor` is not stored on `GpuDevice` — a PCI ID has no field there,
    /// per the parallel-slice design — but callers use the same value to build
    /// the matching `pci` slice they pass to `apply_to_devices`/`samples_from`.
    fn wgpu_device(index: u32, _vendor: u32) -> GpuDevice {
        GpuDevice {
            index,
            name: "Radeon RX 7800 XT".into(),
            vendor: Some("AMD".into()),
            backend: Some("Vulkan".into()),
            kind: GpuKind::Discrete,
            vram_total: None,
            shared_total: None,
            source: GpuSource::Wgpu,
            shared_source: None,
        }
    }

    /// A reading as a platform module would return it.
    fn reading(vendor: u32) -> AdapterMemory {
        AdapterMemory {
            pci_vendor: vendor,
            pci_device: 0x747e,
            name: Some("Radeon RX 7800 XT".into()),
            vram_total: Some(17_163_091_968),
            vram_used: Some(6_657_474_560),
            shared_total: Some(17_179_869_184),
            shared_used: Some(322_961_408),
            source: GpuSource::Dxgi,
        }
    }

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
    fn a_reading_fills_the_gap_wgpu_left() {
        let mut devices = vec![wgpu_device(0, 0x1002)];

        apply_to_devices(&mut devices, &[(0x1002, 0x747e)], &[reading(0x1002)]);

        assert_eq!(devices[0].vram_total, Some(17_163_091_968));
        assert_eq!(devices[0].shared_total, Some(17_179_869_184));
        assert_eq!(devices[0].shared_source, Some(GpuSource::Dxgi));
    }

    #[test]
    fn a_wgpu_device_given_measured_memory_stops_claiming_to_be_an_estimate() {
        // The front end appends "-- estimated" from source.is_measured(). A
        // real DXGI figure under source: Wgpu would be labelled a guess.
        let mut devices = vec![wgpu_device(0, 0x1002)];

        apply_to_devices(&mut devices, &[(0x1002, 0x747e)], &[reading(0x1002)]);

        assert_eq!(devices[0].source, GpuSource::Dxgi);
        assert!(devices[0].source.is_measured());
    }

    #[test]
    fn nvml_keeps_its_own_dedicated_figure() {
        // NVML is the vendor's own instrumentation and ADR-008 trusts it most.
        // A second-hand figure must not displace it.
        let mut devices = vec![GpuDevice {
            vram_total: Some(8_589_934_592),
            source: GpuSource::Nvml,
            ..wgpu_device(0, 0x10de)
        }];

        apply_to_devices(
            &mut devices,
            &[(0, 0)],
            &[AdapterMemory {
                pci_vendor: 0x10de,
                vram_total: Some(1),
                ..reading(0x10de)
            }],
        );

        assert_eq!(devices[0].vram_total, Some(8_589_934_592));
        assert_eq!(devices[0].source, GpuSource::Nvml);
        assert_eq!(
            devices[0].shared_source,
            Some(GpuSource::Dxgi),
            "the shared pool still comes from the reading"
        );
    }

    #[test]
    fn a_reading_for_a_different_card_is_not_applied() {
        let mut devices = vec![wgpu_device(0, 0x1002)];

        // The device's own PCI pair is known (0x1002, ...); the reading's is
        // not (0x8086), even though the two share a name. A known PCI pair
        // that fails to match must not fall back to the name.
        apply_to_devices(&mut devices, &[(0x1002, 0x747e)], &[reading(0x8086)]);

        assert_eq!(devices[0].vram_total, None);
        assert_eq!(devices[0].shared_total, None);
    }

    #[test]
    fn a_device_with_no_pci_pair_matches_by_name() {
        // An NVML-supplied device reaches the merge with no PCI pair -- see
        // `probe.rs`, which seeds those entries with (0, 0). The name branch is
        // what lets such a device pick up a shared figure from a source that
        // never measured its dedicated one, which is why `shared_source` exists
        // as a separate field.
        let mut devices = vec![GpuDevice {
            source: GpuSource::Nvml,
            vram_total: Some(8_589_934_592),
            ..wgpu_device(0, 0x10de)
        }];

        apply_to_devices(&mut devices, &[(0, 0)], &[reading(0x10de)]);

        assert_eq!(
            devices[0].shared_total,
            Some(17_179_869_184),
            "the name branch did not fire for a device with no PCI pair"
        );
        assert_eq!(devices[0].shared_source, Some(GpuSource::Dxgi));
        assert_eq!(
            devices[0].vram_total,
            Some(8_589_934_592),
            "NVML's own figure must survive"
        );
    }

    #[test]
    fn shared_source_is_set_exactly_when_shared_total_is() {
        let mut devices = vec![wgpu_device(0, 0x1002)];

        apply_to_devices(
            &mut devices,
            &[(0x1002, 0x747e)],
            &[AdapterMemory {
                shared_total: None,
                shared_used: None,
                ..reading(0x1002)
            }],
        );

        assert!(devices[0].shared_total.is_none());
        assert!(
            devices[0].shared_source.is_none(),
            "a source with no figure to attribute is a dangling label"
        );
    }

    #[test]
    fn a_reading_produces_a_sample_where_nvml_produced_none() {
        // The gap this feature would otherwise never reach: an AMD-only Windows
        // machine has no NVML handle at all.
        let mut devices = vec![wgpu_device(0, 0x1002)];
        apply_to_devices(&mut devices, &[(0x1002, 0x747e)], &[reading(0x1002)]);

        let samples = samples_from(&devices, &[(0x1002, 0x747e)], &[reading(0x1002)], &[]);

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].shared_used, Some(322_961_408));
        assert_eq!(samples[0].vram_used, Some(6_657_474_560));
    }

    #[test]
    fn an_nvml_sample_keeps_its_vram_used_and_gains_a_shared_figure() {
        let mut devices = vec![GpuDevice {
            vram_total: Some(8_589_934_592),
            source: GpuSource::Nvml,
            ..wgpu_device(0, 0x10de)
        }];
        apply_to_devices(&mut devices, &[(0, 0)], &[reading(0x10de)]);

        let nvml = vec![GpuSample {
            index: 0,
            utilisation: Some(14.0),
            vram_used: Some(3_328_737_280),
            temperature_c: Some(47.0),
            shared_used: None,
        }];

        let samples = samples_from(&devices, &[(0, 0)], &[reading(0x10de)], &nvml);

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].vram_used, Some(3_328_737_280), "NVML's figure");
        assert_eq!(samples[0].utilisation, Some(14.0));
        assert_eq!(samples[0].shared_used, Some(322_961_408));
    }

    #[test]
    fn a_device_with_neither_source_yields_an_unmeasured_sample() {
        let devices = vec![wgpu_device(0, 0x1002)];

        let samples = samples_from(&devices, &[(0, 0)], &[], &[]);

        assert_eq!(samples.len(), 1);
        assert!(!samples[0].has_measurements());
    }

    #[test]
    fn samples_are_indexed_to_the_devices_they_describe() {
        let devices = vec![wgpu_device(0, 0x1002), wgpu_device(1, 0x8086)];

        let samples = samples_from(&devices, &[(0, 0), (0, 0)], &[], &[]);

        assert_eq!(samples[0].index, 0);
        assert_eq!(samples[1].index, 1);
    }
}
