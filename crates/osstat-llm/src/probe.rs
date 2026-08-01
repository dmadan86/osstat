//! The ADR-008 GPU probe: NVML first, then `wgpu`, then unified memory.
//!
//! Probing runs once at startup and the device list is then fixed for the
//! session. Measurement runs per tick and only touches devices NVML can read.

use osstat_core::{GpuDevice, GpuKind, GpuProvider, GpuSample, GpuSource, Result};

#[cfg(not(target_os = "macos"))]
mod nvml;

#[cfg(target_os = "macos")]
mod nvml {
    //! Apple has not shipped an NVIDIA driver since macOS 10.13, so NVML can
    //! never load here. This stub keeps the probe's control flow identical on
    //! all three platforms rather than scattering `cfg` through it.

    use osstat_core::{GpuDevice, GpuSample};

    /// A handle that is never acquired on macOS.
    pub(super) struct Nvidia;

    impl Nvidia {
        pub(super) fn attach() -> Option<Self> {
            None
        }

        pub(super) fn devices(&self, _first_index: u32) -> Vec<GpuDevice> {
            Vec::new()
        }

        pub(super) fn measure(&self, _indices: &[(u32, u32)]) -> Vec<GpuSample> {
            Vec::new()
        }

        pub(super) fn cuda_driver_major(&self) -> Option<u32> {
            None
        }
    }
}

mod adapter_memory;

/// Finds the GPUs in this machine and measures the ones it can.
///
/// Construction does no probing; call [`GpuProvider::devices`] for that.
pub struct HardwareProbe {
    /// Total system memory, used to report Apple unified memory honestly.
    system_memory: u64,
    /// The NVIDIA management handle, if a driver is present.
    nvidia: Option<nvml::Nvidia>,
    /// `(device index, NVML index)` for every device NVML can measure.
    measurable: Vec<(u32, u32)>,
    /// The devices `devices()` returned, so samples can be indexed to them.
    devices: Vec<GpuDevice>,
    /// The PCI `(vendor, device)` pair of each entry in `devices`, or `(0, 0)`
    /// where NVML supplied the device and no pair was seen.
    ///
    /// Parallel to `devices` rather than a field on `GpuDevice`: a PCI ID is a
    /// matching key with no meaning to the front end, and `GpuDevice` crosses
    /// the IPC boundary.
    pci: Vec<(u32, u32)>,
    /// Whether [`GpuProvider::devices`] has already run.
    probed: bool,
}

impl HardwareProbe {
    /// Creates a probe.
    ///
    /// `system_memory` is passed in rather than read here so this crate stays
    /// free of a system-information dependency, and so the unified-memory path
    /// is testable without an Apple machine.
    #[must_use]
    pub fn new(system_memory: u64) -> Self {
        Self {
            system_memory,
            nvidia: None,
            measurable: Vec::new(),
            devices: Vec::new(),
            pci: Vec::new(),
            probed: false,
        }
    }

    /// The major CUDA version this machine's NVIDIA driver supports.
    ///
    /// `None` when there is no NVIDIA driver, which includes every Mac, every
    /// CI runner, and any machine where [`GpuProvider::devices`] has not run
    /// yet.
    ///
    /// This is a probe concern rather than a rendering one: it exists because
    /// choosing between llama.cpp's two published Windows CUDA builds needs the
    /// driver's CUDA version, and NVML is the only thing that reports it.
    #[must_use]
    pub fn cuda_driver_major(&self) -> Option<u32> {
        self.nvidia
            .as_ref()
            .and_then(nvml::Nvidia::cuda_driver_major)
    }
}

impl GpuProvider for HardwareProbe {
    fn devices(&mut self) -> Result<Vec<GpuDevice>> {
        let mut devices = Vec::new();

        // 1. NVML, which is the only source that measures anything.
        self.nvidia = nvml::Nvidia::attach();
        if let Some(nvidia) = &self.nvidia {
            devices = nvidia.devices(0);
        }

        self.measurable = devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.source == GpuSource::Nvml)
            .filter_map(|(nvml_index, device)| {
                u32::try_from(nvml_index)
                    .ok()
                    .map(|nvml_index| (device.index, nvml_index))
            })
            .collect();

        // 2. Everything else, through adapter enumeration.
        let claimed: Vec<String> = devices
            .iter()
            .map(|device| normalise(&device.name))
            .collect();

        let next_index = u32::try_from(devices.len()).unwrap_or(u32::MAX);
        // NVML devices carry no PCI pair; they are matched by name instead.
        let mut pci = vec![(0_u32, 0_u32); devices.len()];

        for (adapter, ids) in enumerate_adapters(self.system_memory, next_index) {
            if claimed.contains(&normalise(&adapter.name)) {
                // The same card NVML already described, and better.
                continue;
            }
            devices.push(adapter);
            pci.push(ids);
        }

        // Re-index so the list is contiguous whatever the sources contributed.
        for (position, device) in devices.iter_mut().enumerate() {
            device.index = u32::try_from(position).unwrap_or(u32::MAX);
        }

        // Fill what NVML and wgpu could not: shared memory everywhere, and
        // dedicated memory on every adapter NVML does not cover.
        //
        // Enumerating adapters is done once here rather than per tick: which
        // adapters exist and how large their pools are cannot change within a
        // session. Only the usage figures move, and `measure` re-reads those.
        let readings = adapter_memory::adapter_memory();
        adapter_memory::apply_to_devices(&mut devices, &pci, &readings);

        self.devices.clone_from(&devices);
        self.pci = pci;

        self.probed = true;
        Ok(devices)
    }

    fn measure(&mut self) -> Result<Vec<GpuSample>> {
        if !self.probed {
            // Measuring before probing is a programming error, not a user-facing
            // failure; an empty reading is the honest answer.
            return Ok(Vec::new());
        }

        let nvml = self
            .nvidia
            .as_ref()
            .map(|nvidia| nvidia.measure(&self.measurable))
            .unwrap_or_default();

        Ok(adapter_memory::samples_from(
            &self.devices,
            &self.pci,
            &adapter_memory::adapter_memory(),
            &nvml,
        ))
    }
}

/// Enumerates adapters through `wgpu`, starting device indices at `first_index`.
///
/// Returns an empty list rather than failing when there is no usable graphics
/// backend at all — which is the normal state of a CI runner and a headless
/// server, and which ADR-008 requires to degrade rather than error.
fn enumerate_adapters(system_memory: u64, first_index: u32) -> Vec<(GpuDevice, (u32, u32))> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());

    instance
        .enumerate_adapters(wgpu::Backends::all())
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            let kind = map_device_type(info.device_type);
            let (source, vram_total) = classify_memory(kind, system_memory);

            let device = GpuDevice {
                index: 0, // assigned below, once the list is filtered
                name: info.name.clone(),
                vendor: vendor_name(info.vendor),
                backend: Some(info.backend.to_string()),
                kind,
                vram_total,
                shared_total: None,
                source,
                shared_source: None,
            };

            (device, (info.vendor, info.device))
        })
        .filter(|(device, _)| is_real_hardware(device.kind))
        .enumerate()
        .map(|(offset, (device, ids))| {
            (
                GpuDevice {
                    index: first_index.saturating_add(u32::try_from(offset).unwrap_or(0)),
                    ..device
                },
                ids,
            )
        })
        .collect()
}

/// Whether an adapter is a graphics processor rather than a software fallback.
///
/// Windows always enumerates the "Microsoft Basic Render Driver" — a CPU
/// rasteriser that exists so software has something to draw with when no driver
/// is loaded. Listing it under "GPU" tells the user their machine has a
/// graphics card it does not have, and once the LLM advisor lands it would be
/// offered as somewhere to run a model, which it emphatically is not.
const fn is_real_hardware(kind: GpuKind) -> bool {
    !matches!(kind, GpuKind::Cpu)
}

/// Decides what, if anything, can honestly be said about a device's memory.
///
/// On Apple Silicon the GPU shares system memory, so reporting the system total
/// as unified memory is accurate and useful. Everywhere else `wgpu` simply has
/// no memory API, so the answer is "unknown" — and saying so is the point.
const fn classify_memory(kind: GpuKind, system_memory: u64) -> (GpuSource, Option<u64>) {
    if cfg!(target_os = "macos") && matches!(kind, GpuKind::Integrated) {
        (GpuSource::UnifiedMemory, Some(system_memory))
    } else {
        (GpuSource::Wgpu, None)
    }
}

/// Maps `wgpu`'s device classification onto osstat's.
const fn map_device_type(device_type: wgpu::DeviceType) -> GpuKind {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => GpuKind::Discrete,
        wgpu::DeviceType::IntegratedGpu => GpuKind::Integrated,
        wgpu::DeviceType::VirtualGpu => GpuKind::Virtual,
        wgpu::DeviceType::Cpu => GpuKind::Cpu,
        wgpu::DeviceType::Other => GpuKind::Other,
    }
}

/// Resolves a PCI vendor ID to a name.
///
/// Only the vendors that ship GPUs osstat might meet are listed; an unknown ID
/// yields `None` rather than a hex string no user could act on.
fn vendor_name(vendor_id: u32) -> Option<String> {
    let name = match vendor_id {
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x106b => "Apple",
        0x13b5 => "ARM",
        0x5143 => "Qualcomm",
        0x1010 => "Imagination",
        0x10005 => "Mesa",
        _ => return None,
    };
    Some(name.to_owned())
}

/// Normalises an adapter name for comparison between sources.
///
/// NVML and `wgpu` describe the same card with the same words but not always
/// the same spacing or case, and a duplicate GPU on screen is worse than a
/// slightly over-eager match.
pub(super) fn normalise(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const SYSTEM_MEMORY: u64 = 32 * 1024 * 1024 * 1024;

    #[test]
    fn names_differing_only_in_case_and_spacing_are_the_same_card() {
        assert_eq!(
            normalise("NVIDIA  GeForce RTX 4090"),
            normalise("nvidia geforce rtx 4090")
        );
    }

    #[test]
    fn different_cards_do_not_normalise_together() {
        assert_ne!(
            normalise("NVIDIA GeForce RTX 4090"),
            normalise("NVIDIA GeForce RTX 4080")
        );
    }

    #[test]
    fn known_vendors_resolve_to_names() {
        assert_eq!(vendor_name(0x10de).as_deref(), Some("NVIDIA"));
        assert_eq!(vendor_name(0x8086).as_deref(), Some("Intel"));
        assert_eq!(vendor_name(0x106b).as_deref(), Some("Apple"));
    }

    #[test]
    fn an_unknown_vendor_is_absent_rather_than_a_hex_string() {
        assert!(
            vendor_name(0xdead).is_none(),
            "a raw PCI id is not something a user can act on"
        );
    }

    #[test]
    fn device_types_map_across() {
        assert_eq!(
            map_device_type(wgpu::DeviceType::DiscreteGpu),
            GpuKind::Discrete
        );
        assert_eq!(map_device_type(wgpu::DeviceType::Cpu), GpuKind::Cpu);
        assert_eq!(map_device_type(wgpu::DeviceType::Other), GpuKind::Other);
    }

    #[test]
    fn wgpu_never_claims_to_know_discrete_vram() {
        // The load-bearing honesty check: wgpu has no memory API, so a discrete
        // card found this way must report an absent VRAM figure, not a guess.
        let (source, vram) = classify_memory(GpuKind::Discrete, SYSTEM_MEMORY);

        assert_eq!(source, GpuSource::Wgpu);
        assert!(vram.is_none());
        assert!(!source.is_measured());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn apple_integrated_memory_is_reported_as_unified() {
        let (source, vram) = classify_memory(GpuKind::Integrated, SYSTEM_MEMORY);

        assert_eq!(source, GpuSource::UnifiedMemory);
        assert_eq!(vram, Some(SYSTEM_MEMORY));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn integrated_memory_off_apple_is_not_claimed_as_unified() {
        let (source, vram) = classify_memory(GpuKind::Integrated, SYSTEM_MEMORY);

        assert_eq!(source, GpuSource::Wgpu);
        assert!(vram.is_none());
    }

    #[test]
    fn a_software_rasteriser_is_not_a_gpu() {
        // Windows always enumerates the Microsoft Basic Render Driver.
        assert!(!is_real_hardware(GpuKind::Cpu));
        assert!(is_real_hardware(GpuKind::Discrete));
        assert!(is_real_hardware(GpuKind::Integrated));
        assert!(
            is_real_hardware(GpuKind::Virtual),
            "a virtualised GPU is real hardware to the guest"
        );
    }

    #[test]
    fn no_software_rasteriser_reaches_the_device_list() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);
        let devices = probe.devices().unwrap();

        assert!(devices.iter().all(|device| device.kind != GpuKind::Cpu));
    }

    #[test]
    fn a_machine_with_no_nvidia_driver_reports_no_cuda_version() {
        // CI runners and every Mac take this path. So does any machine before
        // `devices()` has run, since nothing has attached yet.
        let probe = HardwareProbe::new(SYSTEM_MEMORY);

        assert!(
            probe.cuda_driver_major().is_none(),
            "no driver has been attached, so there is no version to report"
        );
    }

    #[test]
    fn any_reported_cuda_version_is_plausible() {
        // On a machine with an NVIDIA card this reads the real driver; on CI it
        // reads nothing. Either is a pass — what must not happen is a number
        // that would select a llama.cpp build at random.
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);
        let _ = probe.devices().unwrap();

        if let Some(major) = probe.cuda_driver_major() {
            assert!(
                (10..=99).contains(&major),
                "implausible CUDA major version {major}"
            );
        }
    }

    #[test]
    fn measuring_before_probing_yields_nothing_rather_than_panicking() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);

        assert!(probe.measure().unwrap().is_empty());
    }

    // The path CI actually exercises: runners have no GPU, and ADR-008 requires
    // that to be a successful empty answer rather than an error.

    #[test]
    fn probing_a_machine_with_no_gpu_succeeds_with_an_empty_list() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);

        let devices = probe.devices().expect("probing must never fail outright");

        // On a developer machine this finds real hardware; on CI it finds none.
        // Either is a success — the assertion is that it did not error.
        for device in &devices {
            assert!(!device.name.is_empty());
        }
    }

    #[test]
    fn device_indices_are_contiguous_whatever_the_sources_found() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);
        let devices = probe.devices().unwrap();

        for (position, device) in devices.iter().enumerate() {
            assert_eq!(u32::try_from(position).unwrap(), device.index);
        }
    }

    #[test]
    fn every_measurement_indexes_a_device_that_exists() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);
        let devices = probe.devices().unwrap();
        let samples = probe.measure().unwrap();

        for sample in &samples {
            assert!(
                devices.iter().any(|device| device.index == sample.index),
                "a sample for a device that was never enumerated"
            );
        }
    }

    #[test]
    fn no_device_is_listed_twice() {
        let mut probe = HardwareProbe::new(SYSTEM_MEMORY);
        let devices = probe.devices().unwrap();

        let mut names: Vec<String> = devices.iter().map(|d| normalise(&d.name)).collect();
        names.sort();
        let total = names.len();
        names.dedup();

        assert_eq!(
            names.len(),
            total,
            "the NVML/wgpu dedupe let a card through twice"
        );
    }
}
