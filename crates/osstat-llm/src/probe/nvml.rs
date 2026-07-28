//! The NVIDIA half of the probe.
//!
//! NVML is the only source in ADR-008 that *measures* rather than describes: it
//! reports VRAM actually in use, core utilisation and die temperature. Compiled
//! on Windows and Linux only — see the stub in the parent module for why.
//!
//! Every call here is fallible and every failure is treated as "this machine
//! has no NVIDIA GPU right now". That is not laziness: NVML fails for entirely
//! ordinary reasons — no driver installed, a laptop's discrete GPU powered
//! down, a container without the device node mapped in — and none of them are
//! errors a user needs to see. A missing GPU section is the correct outcome; a
//! red banner is not.

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use osstat_core::{GpuDevice, GpuKind, GpuSample, GpuSource};

/// A live handle to the NVIDIA management library.
pub(super) struct Nvidia {
    nvml: Nvml,
}

impl Nvidia {
    /// Loads NVML, or returns `None` if this machine has no usable NVIDIA GPU.
    pub(super) fn attach() -> Option<Self> {
        let nvml = Nvml::init().ok()?;
        // A driver can be present with no device behind it — a headless VM with
        // the library installed, for instance. Treat that as no GPU.
        if nvml.device_count().ok()? == 0 {
            return None;
        }
        Some(Self { nvml })
    }

    /// Describes every NVIDIA GPU, numbering from `first_index`.
    pub(super) fn devices(&self, first_index: u32) -> Vec<GpuDevice> {
        let Ok(count) = self.nvml.device_count() else {
            return Vec::new();
        };

        (0..count)
            .filter_map(|nvml_index| {
                let device = self.nvml.device_by_index(nvml_index).ok()?;

                Some(GpuDevice {
                    index: first_index.saturating_add(nvml_index),
                    name: device.name().ok()?,
                    vendor: Some("NVIDIA".to_owned()),
                    backend: Some("NVML".to_owned()),
                    kind: GpuKind::Discrete,
                    vram_total: device.memory_info().ok().map(|memory| memory.total),
                    source: GpuSource::Nvml,
                })
            })
            .collect()
    }

    /// Reads utilisation for the `(device index, NVML index)` pairs given.
    ///
    /// A device that has since become unreadable — an eGPU unplugged mid-session
    /// — yields an unmeasured sample rather than dropping out of the list, so
    /// the UI keeps showing the card it told the user about.
    pub(super) fn measure(&self, indices: &[(u32, u32)]) -> Vec<GpuSample> {
        indices
            .iter()
            .map(|&(device_index, nvml_index)| {
                let Ok(device) = self.nvml.device_by_index(nvml_index) else {
                    return GpuSample::unmeasured(device_index);
                };

                GpuSample {
                    index: device_index,
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "utilisation is a 0-100 integer percentage"
                    )]
                    utilisation: device
                        .utilization_rates()
                        .ok()
                        .map(|rates| rates.gpu as f32),
                    vram_used: device.memory_info().ok().map(|memory| memory.used),
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "temperature is a small integer in degrees Celsius"
                    )]
                    temperature_c: device
                        .temperature(TemperatureSensor::Gpu)
                        .ok()
                        .map(|celsius| celsius as f32),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn attaching_without_an_nvidia_gpu_is_none_rather_than_a_panic() {
        // On a CI runner this is None; on an NVIDIA workstation it is Some.
        // Either way, attaching must not panic or hang.
        let attached = Nvidia::attach();

        if let Some(nvidia) = attached {
            let devices = nvidia.devices(0);
            assert!(
                !devices.is_empty(),
                "attach() reported a device count above zero"
            );
            for device in &devices {
                assert_eq!(device.source, GpuSource::Nvml);
                assert!(device.source.is_measured());
            }
        }
    }

    #[test]
    fn measuring_an_empty_index_list_yields_no_samples() {
        if let Some(nvidia) = Nvidia::attach() {
            assert!(nvidia.measure(&[]).is_empty());
        }
    }

    #[test]
    fn samples_keep_the_device_index_they_were_asked_about() {
        if let Some(nvidia) = Nvidia::attach() {
            let samples = nvidia.measure(&[(7, 0)]);
            assert_eq!(samples.len(), 1);
            assert_eq!(
                samples[0].index, 7,
                "the sample must carry the device index, not the NVML one"
            );
        }
    }
}
