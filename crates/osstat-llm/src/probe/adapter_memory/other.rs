//! Platforms with no adapter-memory source of their own.
//!
//! macOS is the one that matters, and its emptiness is the correct answer
//! rather than a gap. Apple Silicon has one memory pool, not two: the existing
//! `GpuSource::UnifiedMemory` path already reports the whole of system memory
//! as the GPU's, and already says "Unified memory (shared with the system)".
//! A `shared_total` there would draw a second meter describing the same bytes
//! as the first.
//!
//! The one figure genuinely missing on macOS is live GPU memory usage, and the
//! only route to it is the undocumented `PerformanceStatistics` dictionary on
//! the `IOAccelerator` `IOKit` service. ADR-008's rule is that every figure
//! carries a source the user can weigh, and an API Apple can change in a point
//! release is a poor foundation for a number labelled "measured".
//!
//! This file exists rather than being `cfg`'d away for the reason the NVML
//! stub gives: the probe's control flow stays identical on every platform
//! instead of scattering `cfg` through it.

use super::AdapterMemory;

/// No adapter memory is available on this platform.
pub(crate) fn adapter_memory() -> Vec<AdapterMemory> {
    Vec::new()
}
