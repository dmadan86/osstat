//! The capability traits the platform layer implements.
//!
//! Per ADR-003 every capability is declared here and implemented in
//! `osstat-platform`. Call sites depend on these traits and never on a concrete
//! source, which is what keeps replacing `sysinfo` on one platform a local
//! change rather than a rewrite.
//!
//! All three take `&mut self` for sampling. That is not incidental: a rate —
//! CPU percentage, bytes per second — only exists relative to a previous
//! reading, so a sampler is inherently stateful and must be honest about it.

use crate::error::Result;
use crate::gpu::{GpuDevice, GpuSample};
use crate::metrics::{MetricsSample, SystemDescription};
use crate::process::ProcessRecord;

/// Describes the machine and measures it.
pub trait SystemInfoProvider {
    /// The machine's identity: OS, CPU, disks, interfaces.
    ///
    /// Called once per session rather than per tick.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot be interrogated at all.
    fn describe(&mut self) -> Result<SystemDescription>;

    /// One tick of measurement.
    ///
    /// The first sample after construction may report zero CPU utilisation:
    /// a percentage needs a previous reading to be measured against.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform refuses the reading.
    fn sample(&mut self) -> Result<MetricsSample>;
}

/// Enumerates running processes.
pub trait ProcessProvider {
    /// Every process visible to the current user, flat and unordered.
    ///
    /// The hierarchy is built by [`crate::process::ProcessTree::build`] rather
    /// than here: arranging processes is portable logic and belongs where it can
    /// be tested without an operating system.
    ///
    /// # Errors
    ///
    /// Returns an error if the process table cannot be read. A process that
    /// disappears mid-enumeration is not an error — it is omitted.
    fn processes(&mut self) -> Result<Vec<ProcessRecord>>;
}

/// Detects GPUs and, where the vendor allows, measures them.
pub trait GpuProvider {
    /// Every GPU the probe could find.
    ///
    /// Returning an empty list is a success, not a failure: a machine with no
    /// GPU is the ordinary case on a CI runner and a headless server, and
    /// ADR-008 requires it to degrade rather than error.
    ///
    /// # Errors
    ///
    /// Returns an error only if probing itself failed in a way that leaves the
    /// answer unknown — never merely because there is nothing to report.
    fn devices(&mut self) -> Result<Vec<GpuDevice>>;

    /// One tick of GPU measurement, indexed to match [`GpuProvider::devices`].
    ///
    /// Devices whose source cannot measure utilisation yield
    /// [`GpuSample::unmeasured`] rather than a fabricated zero.
    ///
    /// # Errors
    ///
    /// Returns an error if a device that was enumerated can no longer be read.
    fn measure(&mut self) -> Result<Vec<GpuSample>>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::gpu::{GpuKind, GpuSource};

    /// A provider with no hardware behind it, standing in for a CI runner.
    #[derive(Default)]
    struct EmptyGpus;

    impl GpuProvider for EmptyGpus {
        fn devices(&mut self) -> Result<Vec<GpuDevice>> {
            Ok(Vec::new())
        }

        fn measure(&mut self) -> Result<Vec<GpuSample>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_machine_with_no_gpu_is_a_success_not_an_error() {
        let mut provider = EmptyGpus;

        assert!(provider.devices().unwrap().is_empty());
        assert!(provider.measure().unwrap().is_empty());
    }

    /// A provider standing in for an adapter found through `wgpu`, which can
    /// name a device but cannot measure it.
    struct UnmeasurableGpu;

    impl GpuProvider for UnmeasurableGpu {
        fn devices(&mut self) -> Result<Vec<GpuDevice>> {
            Ok(vec![GpuDevice {
                index: 0,
                name: "Intel Arc".into(),
                vendor: Some("Intel".into()),
                backend: Some("Vulkan".into()),
                kind: GpuKind::Discrete,
                vram_total: None,
                source: GpuSource::Wgpu,
            }])
        }

        fn measure(&mut self) -> Result<Vec<GpuSample>> {
            Ok(vec![GpuSample::unmeasured(0)])
        }
    }

    #[test]
    fn an_unmeasurable_device_yields_an_empty_sample_not_a_fabricated_zero() {
        let mut provider = UnmeasurableGpu;

        let samples = provider.measure().unwrap();
        assert_eq!(samples.len(), 1);
        assert!(!samples[0].has_measurements());
        assert!(
            samples[0].utilisation.is_none(),
            "a device that cannot be measured must not report 0%"
        );
    }
}
