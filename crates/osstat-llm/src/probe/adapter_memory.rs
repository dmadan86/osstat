//! Merges platform memory readings into the devices the GPU probe found.
//!
//! ADR-008's three sources leave a gap: NVML measures only NVIDIA hardware, and
//! `wgpu` has no memory API at all, so every AMD and Intel adapter reports no
//! video memory whatsoever. Neither source knows about the shared pool — the
//! system RAM a GPU borrows when its own runs out — which Task Manager has
//! shown users for years.
//!
//! The readings themselves come from `osstat_platform::adapter_memory`, which
//! forbids nothing here can use: DXGI and PDH are raw FFI, and this crate
//! forbids unsafe code outright. This module is the portable half — arranging
//! data the per-OS modules fetched, for the reason `provider.rs` gives about
//! `ProcessTree::build`.

use osstat_core::{GpuDevice, GpuSample};
use osstat_platform::AdapterMemory;

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
    /// `GpuDevice` carries no PCI field — a PCI ID is a matching key with no
    /// meaning to the front end, and `GpuDevice` crosses the IPC boundary.
    /// Callers build the matching `pci` slice they pass to
    /// `apply_to_devices`/`samples_from` separately.
    fn wgpu_device(index: u32) -> GpuDevice {
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
    fn a_reading_fills_the_gap_wgpu_left() {
        let mut devices = vec![wgpu_device(0)];

        apply_to_devices(&mut devices, &[(0x1002, 0x747e)], &[reading(0x1002)]);

        assert_eq!(devices[0].vram_total, Some(17_163_091_968));
        assert_eq!(devices[0].shared_total, Some(17_179_869_184));
        assert_eq!(devices[0].shared_source, Some(GpuSource::Dxgi));
    }

    #[test]
    fn a_wgpu_device_given_measured_memory_stops_claiming_to_be_an_estimate() {
        // The front end appends "-- estimated" from source.is_measured(). A
        // real DXGI figure under source: Wgpu would be labelled a guess.
        let mut devices = vec![wgpu_device(0)];

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
            ..wgpu_device(0)
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
        let mut devices = vec![wgpu_device(0)];

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
            ..wgpu_device(0)
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
        let mut devices = vec![wgpu_device(0)];

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
        let mut devices = vec![wgpu_device(0)];
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
            ..wgpu_device(0)
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
        let devices = vec![wgpu_device(0)];

        let samples = samples_from(&devices, &[(0, 0)], &[], &[]);

        assert_eq!(samples.len(), 1);
        assert!(!samples[0].has_measurements());
    }

    #[test]
    fn samples_are_indexed_to_the_devices_they_describe() {
        let devices = vec![wgpu_device(0), wgpu_device(1)];

        let samples = samples_from(&devices, &[(0, 0), (0, 0)], &[], &[]);

        assert_eq!(samples[0].index, 0);
        assert_eq!(samples[1].index, 1);
    }
}
