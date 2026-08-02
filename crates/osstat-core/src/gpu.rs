//! GPU description and per-tick GPU measurement.
//!
//! The type that matters most here is [`GpuSource`]. ADR-008 probes in order —
//! NVML, then `wgpu` adapter enumeration, then macOS unified memory — and those
//! sources are not equally trustworthy. NVML reports VRAM it has *measured*;
//! `wgpu` reports what a backend chose to advertise, which is a heuristic; and
//! on Apple Silicon there is no discrete VRAM to report at all.
//!
//! Every device therefore carries the source it came from, and the front-end is
//! obliged to label estimates as estimates. ADR-008 names presenting a heuristic
//! as a measurement as the most damaging thing this feature could do, and this
//! enum is the mechanism that prevents it.

use serde::{Deserialize, Serialize};

/// Where a GPU figure came from, and therefore how much it can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum GpuSource {
    /// NVIDIA Management Library: measured VRAM, utilisation and temperature.
    Nvml,
    /// DXGI's adapter description plus the Windows `GPU Adapter Memory`
    /// performance counters: measured, system-wide, and the same source Task
    /// Manager reads.
    ///
    /// Deliberately *not* `IDXGIAdapter3::QueryVideoMemoryInfo`, whose
    /// `CurrentUsage` is documented as the calling application's own usage —
    /// which would put osstat's footprint on screen under a label claiming to
    /// describe the machine.
    Dxgi,
    /// The kernel's DRM sysfs interface, `/sys/class/drm/card*/device/mem_info_*`.
    ///
    /// Measured, and amdgpu only: `i915`, `xe` and NVIDIA's proprietary driver
    /// expose no adapter-wide equivalent.
    DrmSysfs,
    /// `wgpu` adapter enumeration: name and backend are reliable, memory is an
    /// advertised figure and must be presented as an estimate.
    Wgpu,
    /// Apple unified memory: there is no separate VRAM pool, and the figure is
    /// the system memory the GPU may share.
    UnifiedMemory,
}

impl GpuSource {
    /// Whether figures from this source are measurements rather than estimates.
    ///
    /// The front-end uses this to decide whether a number needs an "estimated"
    /// qualifier beside it.
    #[must_use]
    pub const fn is_measured(self) -> bool {
        matches!(self, Self::Nvml | Self::Dxgi | Self::DrmSysfs)
    }
}

/// What kind of device an adapter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum GpuKind {
    /// A discrete card with its own memory.
    Discrete,
    /// An integrated GPU sharing system memory.
    Integrated,
    /// A virtual or paravirtualised adapter.
    Virtual,
    /// A software rasteriser.
    Cpu,
    /// Reported by the platform but not classified.
    Other,
}

/// A GPU present in the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    /// Position in the probe's device list; per-tick samples index by this.
    pub index: u32,
    /// Adapter name as the driver reports it.
    pub name: String,
    /// Vendor name, where it could be resolved.
    pub vendor: Option<String>,
    /// Graphics backend the adapter was found through, e.g. `Vulkan`.
    pub backend: Option<String>,
    /// Discrete, integrated, virtual or software.
    pub kind: GpuKind,
    /// Total video memory in bytes, where the source could supply it.
    ///
    /// Read together with [`GpuDevice::source`]: from `wgpu` this is advertised
    /// rather than measured, and on Apple Silicon it is shared system memory.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub vram_total: Option<u64>,
    /// System memory this GPU may borrow when its own is exhausted, in bytes.
    ///
    /// `None` where the platform has no such pool — Apple unified memory is one
    /// pool, not two — or where it has one that cannot be measured. Never
    /// synthesised from system RAM: a figure that resembles the true answer is
    /// indistinguishable from a measurement until it is wrong.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub shared_total: Option<u64>,
    /// Which probe produced this device, and how far to trust its numbers.
    pub source: GpuSource,
    /// Where the shared figures came from, when that is not [`GpuDevice::source`].
    ///
    /// An NVIDIA card on Windows carries two sources at once: NVML measures the
    /// dedicated pool, and only the performance counters measure the shared
    /// one. One field cannot say that, and collapsing it would label one
    /// figure with another figure's provenance — which is what ADR-008 exists
    /// to prevent.
    ///
    /// `Some` if and only if [`GpuDevice::shared_total`] is `Some`.
    pub shared_source: Option<GpuSource>,
}

/// One GPU's utilisation at a single instant.
///
/// Every field is optional because only NVML supplies them; a `wgpu`-sourced
/// device yields a sample with nothing but its index, and the UI shows the
/// device without a utilisation chart rather than inventing a zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct GpuSample {
    /// Index into the device list returned by the probe.
    pub index: u32,
    /// Core utilisation as a percentage, `0.0..=100.0`.
    pub utilisation: Option<f32>,
    /// Video memory in use, in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub vram_used: Option<u64>,
    /// Shared system memory in use by this GPU, in bytes.
    ///
    /// `Some` only where the device's `shared_total` is `Some`: a usage figure
    /// with no denominator cannot be drawn as a meter.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub shared_used: Option<u64>,
    /// Die temperature in degrees Celsius.
    pub temperature_c: Option<f32>,
}

impl GpuSample {
    /// An empty sample for a device whose source cannot measure utilisation.
    #[must_use]
    pub const fn unmeasured(index: u32) -> Self {
        Self {
            index,
            utilisation: None,
            vram_used: None,
            shared_used: None,
            temperature_c: None,
        }
    }

    /// Whether this sample carries anything worth charting.
    #[must_use]
    pub const fn has_measurements(&self) -> bool {
        self.utilisation.is_some()
            || self.vram_used.is_some()
            || self.temperature_c.is_some()
            || self.shared_used.is_some()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn only_nvml_counts_as_measured() {
        assert!(GpuSource::Nvml.is_measured());
        assert!(
            !GpuSource::Wgpu.is_measured(),
            "wgpu memory is advertised, not measured"
        );
        assert!(!GpuSource::UnifiedMemory.is_measured());
    }

    #[test]
    fn an_unmeasured_sample_carries_nothing_to_chart() {
        let sample = GpuSample::unmeasured(2);

        assert_eq!(sample.index, 2);
        assert!(!sample.has_measurements());
    }

    #[test]
    fn any_single_measurement_makes_a_sample_chartable() {
        let sample = GpuSample {
            utilisation: Some(31.0),
            ..GpuSample::unmeasured(0)
        };

        assert!(sample.has_measurements());
    }

    #[test]
    fn sources_serialise_as_camel_case_tags() {
        let json = serde_json::to_string(&GpuSource::UnifiedMemory).unwrap();
        assert_eq!(json, "\"unifiedMemory\"");
    }

    #[test]
    fn devices_round_trip_through_json() {
        let device = GpuDevice {
            index: 0,
            name: "NVIDIA GeForce RTX 4090".into(),
            vendor: Some("NVIDIA".into()),
            backend: Some("Vulkan".into()),
            kind: GpuKind::Discrete,
            vram_total: Some(25_757_220_864),
            shared_total: None,
            source: GpuSource::Nvml,
            shared_source: None,
        };

        let encoded = serde_json::to_string(&device).unwrap();
        let decoded: GpuDevice = serde_json::from_str(&encoded).unwrap();

        assert_eq!(device, decoded);
    }

    #[test]
    fn devices_serialise_with_camel_case_keys() {
        let device = GpuDevice {
            index: 0,
            name: "Apple M3 Max".into(),
            vendor: Some("Apple".into()),
            backend: Some("Metal".into()),
            kind: GpuKind::Integrated,
            vram_total: Some(38_654_705_664),
            shared_total: None,
            source: GpuSource::UnifiedMemory,
            shared_source: None,
        };

        let json = serde_json::to_value(device).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("vramTotal"));
        assert!(object.contains_key("source"));
    }

    #[test]
    fn the_new_measured_sources_are_measurements() {
        // Dxgi reads DXGI_ADAPTER_DESC1 and the GPU Adapter Memory performance
        // counters; DrmSysfs reads amdgpu's mem_info_* files. Both are figures
        // the platform measured, not figures it advertised.
        assert!(GpuSource::Dxgi.is_measured());
        assert!(GpuSource::DrmSysfs.is_measured());
    }

    #[test]
    fn advertised_sources_are_still_not_measurements() {
        // The property the old `only_nvml_counts_as_measured` guarded, kept
        // intact as the source list grows: these two must never be trusted as
        // measurements, whatever else is added beside them.
        assert!(!GpuSource::Wgpu.is_measured());
        assert!(!GpuSource::UnifiedMemory.is_measured());
    }

    #[test]
    fn a_sample_with_only_a_shared_figure_is_worth_showing() {
        // An integrated GPU on Windows has no dedicated pool, no NVML
        // utilisation and no temperature. Its shared usage is the only thing
        // there is to draw, and dropping it would blank the panel.
        let sample = GpuSample {
            shared_used: Some(134_569_984),
            ..GpuSample::unmeasured(0)
        };

        assert!(sample.has_measurements());
    }

    #[test]
    fn an_unmeasured_sample_has_no_shared_figure_either() {
        assert!(GpuSample::unmeasured(3).shared_used.is_none());
    }

    #[test]
    fn the_shared_fields_serialise_as_camel_case_keys() {
        let device = GpuDevice {
            index: 0,
            name: "Intel(R) UHD Graphics 770".into(),
            vendor: Some("Intel".into()),
            backend: Some("Dxgi".into()),
            kind: GpuKind::Integrated,
            vram_total: None,
            shared_total: Some(17_179_869_184),
            source: GpuSource::Dxgi,
            shared_source: Some(GpuSource::Dxgi),
        };

        let json = serde_json::to_value(device).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("sharedTotal"));
        assert!(object.contains_key("sharedSource"));
    }
}
