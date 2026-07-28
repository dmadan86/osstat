//! Live system metrics: what is sampled on every tick, and the buffer that
//! remembers it.
//!
//! Two kinds of data live here and the distinction matters:
//!
//! - A [`SystemDescription`] is the machine's identity — CPU model, disk list,
//!   interface list. It changes rarely, is full of strings, and is fetched once
//!   with a command.
//! - A [`MetricsSample`] is one tick of measurement. It crosses the IPC boundary
//!   every second or two, so it carries scalars only: disks and interfaces are
//!   referenced by their index into the description rather than by name. Sending
//!   a mount path 1800 times an hour to say "still 66% full" is exactly the cost
//!   ADR-007 pushes back on.
//!
//! [`MetricsHistory`] is the ring buffer the sampler writes into. Keeping the
//! history in Rust rather than React is what lets a chart paint filled on first
//! render and survive a navigation change.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::gpu::GpuSample;

/// Number of samples the history buffer holds.
///
/// Sized by the longest history window the UI offers at the *fastest* tick rate
/// — ten minutes at one second — so no combination of the refresh-interval and
/// history-window settings can ask for more history than exists. At the two
/// second default this is twenty minutes of history; at five, fifty.
pub const HISTORY_CAPACITY: usize = 600;

/// How a disk presents itself to the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "lowercase")]
pub enum DiskKind {
    /// A spinning disk.
    Hdd,
    /// A solid-state disk.
    Ssd,
    /// The platform did not say.
    Unknown,
}

/// The CPU installed in the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CpuDescription {
    /// Marketing name, e.g. `AMD Ryzen 9 7950X`.
    pub brand: String,
    /// Vendor string, e.g. `AuthenticAMD`.
    pub vendor: String,
    /// Physical core count, where the platform reports it.
    pub physical_cores: Option<u32>,
    /// Logical core count — the length of [`MetricsSample::cpu_per_core`].
    pub logical_cores: u32,
    /// Nominal frequency in MHz, or zero if the platform did not say.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub frequency_mhz: u64,
}

/// A mounted filesystem.
///
/// The `index` is this disk's position in [`SystemDescription::disks`], and is
/// how per-tick samples refer back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DiskDescription {
    /// Position in [`SystemDescription::disks`].
    pub index: u32,
    /// Device or volume name.
    pub name: String,
    /// Where it is mounted — `C:\` or `/home`.
    pub mount_point: String,
    /// Filesystem, e.g. `NTFS` or `ext4`.
    pub file_system: String,
    /// Spinning, solid state, or unreported.
    pub kind: DiskKind,
    /// Whether the volume is removable.
    pub removable: bool,
}

/// A network interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct InterfaceDescription {
    /// Position in [`SystemDescription::interfaces`].
    pub index: u32,
    /// Interface name, e.g. `Wi-Fi` or `eth0`.
    pub name: String,
    /// Hardware address, formatted for display.
    pub mac: Option<String>,
    /// Assigned addresses, formatted for display.
    pub addresses: Vec<String>,
}

/// The identity of the machine osstat is running on.
///
/// Fetched once per session rather than per tick. Everything here is either
/// constant or changes on a human timescale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SystemDescription {
    /// Machine hostname.
    pub host_name: Option<String>,
    /// Long OS name, e.g. `Windows 11 Home`.
    pub os_name: Option<String>,
    /// OS version or build, e.g. `26200`.
    pub os_version: Option<String>,
    /// Kernel version.
    pub kernel_version: Option<String>,
    /// Seconds since boot.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub uptime_seconds: u64,
    /// The installed CPU.
    pub cpu: CpuDescription,
    /// Total physical memory in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total_memory: u64,
    /// Total swap in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total_swap: u64,
    /// Mounted filesystems, in the order per-tick samples index them.
    pub disks: Vec<DiskDescription>,
    /// Network interfaces, in the order per-tick samples index them.
    pub interfaces: Vec<InterfaceDescription>,
}

/// One disk's occupancy at a single instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DiskSample {
    /// Index into [`SystemDescription::disks`].
    pub index: u32,
    /// Bytes in use.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub used: u64,
    /// Capacity in bytes. Repeated per tick because removable media change.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total: u64,
}

impl DiskSample {
    /// Fraction of the volume in use, in `0.0..=1.0`.
    ///
    /// Returns zero for a zero-capacity volume rather than a NaN, so a chart
    /// never has to defend against one.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fraction_used(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.used as f64 / self.total as f64
    }
}

/// One interface's throughput over the tick that just elapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct InterfaceSample {
    /// Index into [`SystemDescription::interfaces`].
    pub index: u32,
    /// Bytes received per second during the last tick.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub rx_per_second: u64,
    /// Bytes transmitted per second during the last tick.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub tx_per_second: u64,
    /// Bytes received since osstat started watching.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub rx_total: u64,
    /// Bytes transmitted since osstat started watching.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub tx_total: u64,
}

/// Everything measured in a single tick.
///
/// Deliberately free of strings and paths: this is the payload that crosses IPC
/// on every tick, and the names it would otherwise repeat are already in the
/// [`SystemDescription`] the front-end fetched once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MetricsSample {
    /// Milliseconds since the Unix epoch, for the time axis.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub at_ms: u64,
    /// Total CPU utilisation as a percentage, `0.0..=100.0`.
    pub cpu_total: f32,
    /// Per-logical-core utilisation, same order and length as the CPU
    /// description's `logical_cores`.
    pub cpu_per_core: Vec<f32>,
    /// Physical memory in use, in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub memory_used: u64,
    /// Physical memory available to new allocations, in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub memory_available: u64,
    /// Swap in use, in bytes.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub swap_used: u64,
    /// Per-disk occupancy.
    pub disks: Vec<DiskSample>,
    /// Per-interface throughput.
    pub interfaces: Vec<InterfaceSample>,
    /// Per-GPU utilisation, where the probe could measure it.
    pub gpus: Vec<GpuSample>,
}

/// A fixed-capacity ring of recent [`MetricsSample`]s.
///
/// Pure data with no I/O and no clock of its own, so wraparound and windowing
/// are testable without waiting for real time to pass.
#[derive(Debug, Clone)]
pub struct MetricsHistory {
    capacity: usize,
    samples: VecDeque<MetricsSample>,
}

impl MetricsHistory {
    /// Creates a history holding at most `capacity` samples.
    ///
    /// A zero capacity is raised to one: a history that silently discards
    /// everything would turn an off-by-one into an empty chart with no error.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    /// Appends a sample, evicting the oldest once the ring is full.
    pub fn push(&mut self, sample: MetricsSample) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Samples held right now, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &MetricsSample> {
        self.samples.iter()
    }

    /// The most recent `count` samples, oldest first.
    ///
    /// Asking for more than are held yields everything rather than failing —
    /// the caller is a chart choosing a window, not a caller making a claim.
    #[must_use]
    pub fn recent(&self, count: usize) -> Vec<MetricsSample> {
        let skip = self.samples.len().saturating_sub(count);
        self.samples.iter().skip(skip).cloned().collect()
    }

    /// The newest sample, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&MetricsSample> {
        self.samples.back()
    }

    /// How many samples are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The maximum number of samples this history holds.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for MetricsHistory {
    fn default() -> Self {
        Self::new(HISTORY_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn sample(at_ms: u64) -> MetricsSample {
        MetricsSample {
            at_ms,
            cpu_total: 10.0,
            cpu_per_core: vec![10.0, 10.0],
            memory_used: 1024,
            memory_available: 2048,
            swap_used: 0,
            disks: Vec::new(),
            interfaces: Vec::new(),
            gpus: Vec::new(),
        }
    }

    #[test]
    fn history_keeps_samples_in_order() {
        let mut history = MetricsHistory::new(4);
        for tick in 0..3 {
            history.push(sample(tick));
        }

        let seen: Vec<u64> = history.iter().map(|s| s.at_ms).collect();
        assert_eq!(seen, vec![0, 1, 2]);
    }

    #[test]
    fn history_evicts_the_oldest_once_full() {
        let mut history = MetricsHistory::new(3);
        for tick in 0..5 {
            history.push(sample(tick));
        }

        assert_eq!(history.len(), 3);
        let seen: Vec<u64> = history.iter().map(|s| s.at_ms).collect();
        assert_eq!(seen, vec![2, 3, 4], "the three newest survive, in order");
    }

    #[test]
    fn history_never_exceeds_its_capacity() {
        let mut history = MetricsHistory::new(2);
        for tick in 0..1000 {
            history.push(sample(tick));
        }

        assert_eq!(history.len(), 2);
        assert_eq!(history.capacity(), 2);
    }

    #[test]
    fn zero_capacity_is_raised_to_one_rather_than_discarding_everything() {
        let mut history = MetricsHistory::new(0);
        history.push(sample(7));

        assert_eq!(history.capacity(), 1);
        assert_eq!(history.latest().unwrap().at_ms, 7);
    }

    #[test]
    fn recent_returns_the_newest_window_oldest_first() {
        let mut history = MetricsHistory::new(10);
        for tick in 0..6 {
            history.push(sample(tick));
        }

        let window: Vec<u64> = history.recent(3).iter().map(|s| s.at_ms).collect();
        assert_eq!(window, vec![3, 4, 5]);
    }

    #[test]
    fn recent_yields_everything_when_asked_for_more_than_is_held() {
        let mut history = MetricsHistory::new(10);
        history.push(sample(1));

        assert_eq!(history.recent(500).len(), 1);
    }

    #[test]
    fn a_fresh_history_is_empty_and_has_no_latest() {
        let history = MetricsHistory::new(4);

        assert!(history.is_empty());
        assert!(history.latest().is_none());
        assert_eq!(history.recent(3), Vec::new());
    }

    #[test]
    fn default_capacity_covers_ten_minutes_at_the_fastest_tick() {
        assert_eq!(MetricsHistory::default().capacity(), HISTORY_CAPACITY);
        const { assert!(HISTORY_CAPACITY >= 10 * 60) }
    }

    #[test]
    fn disk_fraction_is_the_ratio_of_used_to_total() {
        let disk = DiskSample {
            index: 0,
            used: 50,
            total: 200,
        };
        assert!((disk.fraction_used() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn an_empty_volume_reports_zero_rather_than_a_nan() {
        let disk = DiskSample {
            index: 0,
            used: 0,
            total: 0,
        };
        assert!(disk.fraction_used().abs() < f64::EPSILON);
        assert!(
            disk.fraction_used().is_finite(),
            "a zero-capacity volume must not produce a NaN for a chart to trip over"
        );
    }

    #[test]
    fn samples_serialise_with_camel_case_keys() {
        let json = serde_json::to_value(sample(3)).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("atMs"));
        assert!(object.contains_key("cpuPerCore"));
        assert!(object.contains_key("memoryAvailable"));
    }

    #[test]
    fn samples_round_trip_through_json() {
        let original = sample(11);
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: MetricsSample = serde_json::from_str(&encoded).unwrap();

        assert_eq!(original, decoded);
    }
}
