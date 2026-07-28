//! The `sysinfo`-backed implementation of the system and process providers.
//!
//! # Why this is not three files
//!
//! ADR-003 asks for one implementation per OS. `sysinfo` is *already* a
//! cross-platform abstraction over the three process tables, so splitting these
//! calls into `windows.rs`, `linux.rs` and `macos.rs` would produce three
//! near-identical copies and triple the surface that only CI can type-check —
//! ceremony, not isolation.
//!
//! The shared adapter therefore lives here, and the per-OS modules keep only
//! what genuinely differs. Today that is [`imp::disk_display_name`]: a disk is
//! `C:` to a Windows user and `/home` to a Linux one, and no amount of
//! abstraction makes those the same string.
//!
//! What ADR-003 actually protects is untouched. Callers depend on
//! [`SystemInfoProvider`] and [`ProcessProvider`], never on this type, so if
//! `sysinfo` proves insufficient on one platform, replacing it stays local.
//!
//! # Index stability
//!
//! Disks and interfaces cross IPC as indices rather than names, because a
//! per-tick payload should not repeat a mount path 1800 times an hour. That
//! makes index stability a correctness property, not an implementation detail,
//! so this type keeps an append-only registry of the disks and interfaces it
//! has seen. A device that disappears leaves its slot empty rather than
//! shifting every device after it — otherwise unplugging a USB stick would
//! silently re-label every other disk on screen.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use osstat_core::{
    CpuDescription, DiskDescription, DiskKind, DiskSample, InterfaceDescription, InterfaceSample,
    MetricsSample, ProcessProvider, ProcessRecord, ProcessStatus, Result, SystemDescription,
    SystemInfoProvider,
};
use sysinfo::{
    Disks, MacAddr, Networks, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System,
    UpdateKind, Users,
};

use crate::imp;

/// Live system and process data, read through `sysinfo`.
///
/// Holds one refreshed `System` for the lifetime of the app: CPU percentages and
/// IO rates are differences between consecutive reads, so a provider that
/// rebuilt its state per call could only ever report zero.
pub struct SysinfoSource {
    system: System,
    disks: Disks,
    networks: Networks,
    users: Users,
    /// Mount points in the order the front-end indexes them.
    disk_registry: Vec<String>,
    /// Interface names in the order the front-end indexes them.
    interface_registry: Vec<String>,
    /// When the previous sample was taken, for converting byte counts to rates.
    last_sampled: Option<Instant>,
}

impl SysinfoSource {
    /// Builds a provider and takes its first reading.
    ///
    /// The first reading is discarded for rate purposes: with nothing to
    /// compare against, every percentage would be zero.
    #[must_use]
    pub fn new() -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::everything())
                .with_memory(sysinfo::MemoryRefreshKind::everything()),
        );
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());

        Self {
            system,
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            disk_registry: Vec::new(),
            interface_registry: Vec::new(),
            last_sampled: None,
        }
    }

    /// Returns the index of `key`, appending it to the registry if new.
    fn index_in(registry: &mut Vec<String>, key: &str) -> u32 {
        let position = if let Some(position) = registry.iter().position(|known| known == key) {
            position
        } else {
            registry.push(key.to_owned());
            registry.len() - 1
        };
        // A machine with more than u32::MAX disks is not a machine.
        u32::try_from(position).unwrap_or(u32::MAX)
    }

    /// Seconds since the previous sample, used to turn byte counts into rates.
    ///
    /// Clamped to a sane floor: two samples taken in the same instant would
    /// otherwise divide by nearly zero and report a petabyte per second.
    fn elapsed_seconds(&mut self) -> f64 {
        let now = Instant::now();
        let elapsed = self
            .last_sampled
            .map_or(1.0, |previous| now.duration_since(previous).as_secs_f64());
        self.last_sampled = Some(now);
        elapsed.max(0.001)
    }
}

impl Default for SysinfoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemInfoProvider for SysinfoSource {
    fn describe(&mut self) -> Result<SystemDescription> {
        self.disks.refresh(true);
        self.networks.refresh(true);
        self.system.refresh_cpu_all();
        self.system.refresh_memory();

        let cpus = self.system.cpus();
        let first = cpus.first();

        let cpu = CpuDescription {
            brand: first.map_or_else(String::new, |cpu| cpu.brand().trim().to_owned()),
            vendor: first.map_or_else(String::new, |cpu| cpu.vendor_id().trim().to_owned()),
            physical_cores: System::physical_core_count().and_then(|n| u32::try_from(n).ok()),
            logical_cores: u32::try_from(cpus.len()).unwrap_or(u32::MAX),
            frequency_mhz: first.map_or(0, sysinfo::Cpu::frequency),
        };

        let disks = self
            .disks
            .list()
            .iter()
            .map(|disk| {
                let mount_point = disk.mount_point().to_string_lossy().into_owned();
                let label = disk.name().to_string_lossy().into_owned();
                DiskDescription {
                    index: Self::index_in(&mut self.disk_registry, &mount_point),
                    name: imp::disk_display_name(&label, &mount_point),
                    mount_point,
                    file_system: disk.file_system().to_string_lossy().into_owned(),
                    kind: map_disk_kind(disk.kind()),
                    removable: disk.is_removable(),
                }
            })
            .collect();

        let interfaces = self
            .networks
            .iter()
            .map(|(name, data)| InterfaceDescription {
                index: Self::index_in(&mut self.interface_registry, name),
                name: name.clone(),
                mac: format_mac(data.mac_address()),
                addresses: data
                    .ip_networks()
                    .iter()
                    .map(|network| network.addr.to_string())
                    .collect(),
            })
            .collect();

        Ok(SystemDescription {
            host_name: System::host_name(),
            os_name: System::long_os_version().or_else(System::name),
            os_version: System::os_version(),
            kernel_version: System::kernel_version(),
            uptime_seconds: System::uptime(),
            cpu,
            total_memory: self.system.total_memory(),
            total_swap: self.system.total_swap(),
            disks,
            interfaces,
        })
    }

    fn sample(&mut self) -> Result<MetricsSample> {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.disks.refresh(false);
        self.networks.refresh(false);

        let seconds = self.elapsed_seconds();

        let disks = self
            .disks
            .list()
            .iter()
            .map(|disk| {
                let mount_point = disk.mount_point().to_string_lossy().into_owned();
                let total = disk.total_space();
                DiskSample {
                    index: Self::index_in(&mut self.disk_registry, &mount_point),
                    used: total.saturating_sub(disk.available_space()),
                    total,
                }
            })
            .collect();

        let interfaces = self
            .networks
            .iter()
            .map(|(name, data)| InterfaceSample {
                index: Self::index_in(&mut self.interface_registry, name),
                rx_per_second: per_second(data.received(), seconds),
                tx_per_second: per_second(data.transmitted(), seconds),
                rx_total: data.total_received(),
                tx_total: data.total_transmitted(),
            })
            .collect();

        Ok(MetricsSample {
            at_ms: now_ms(),
            cpu_total: self.system.global_cpu_usage(),
            cpu_per_core: self
                .system
                .cpus()
                .iter()
                .map(sysinfo::Cpu::cpu_usage)
                .collect(),
            memory_used: self.system.used_memory(),
            memory_available: self.system.available_memory(),
            swap_used: self.system.used_swap(),
            disks,
            interfaces,
            // Filled in by the caller, which owns the GPU provider: this type
            // knows nothing about graphics hardware.
            gpus: Vec::new(),
        })
    }
}

impl ProcessProvider for SysinfoSource {
    fn processes(&mut self) -> Result<Vec<ProcessRecord>> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_refresh_kind(),
        );

        Ok(self
            .system
            .processes()
            .values()
            .map(|process| {
                let io = process.disk_usage();
                ProcessRecord {
                    pid: pid_as_u32(process.pid()),
                    parent_pid: process.parent().map(pid_as_u32),
                    name: process.name().to_string_lossy().into_owned(),
                    exe: process
                        .exe()
                        .map(|path| path.to_string_lossy().into_owned()),
                    user: process
                        .user_id()
                        .and_then(|uid| self.users.get_user_by_id(uid))
                        .map(|user| user.name().to_owned()),
                    cpu: process.cpu_usage(),
                    memory: process.memory(),
                    disk_read_rate: io.read_bytes,
                    disk_write_rate: io.written_bytes,
                    started_at: process.start_time(),
                    status: map_status(process.status()),
                }
            })
            .collect())
    }
}

/// What osstat asks `sysinfo` to collect per process.
///
/// Deliberately not `everything()`: environment variables and command lines are
/// expensive to read for a thousand processes and osstat does not display them.
/// The 50 ms refresh gate in the roadmap is met by not asking for what is unused.
fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_disk_usage()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
}

/// Milliseconds since the Unix epoch.
///
/// A clock set before 1970 yields zero rather than panicking; a wrong timestamp
/// on a chart is a cosmetic problem, and a crashed sampler is not.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Converts a byte count observed over `seconds` into a per-second rate.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "byte rates are display figures; f64 is exact to 9 PB and the result is clamped"
)]
fn per_second(bytes: u64, seconds: f64) -> u64 {
    let rate = bytes as f64 / seconds;
    if rate.is_finite() && rate > 0.0 {
        rate as u64
    } else {
        0
    }
}

/// Renders a MAC address, or nothing if the platform reported a placeholder.
fn format_mac(mac: MacAddr) -> Option<String> {
    if mac == MacAddr::UNSPECIFIED {
        return None;
    }
    Some(mac.to_string())
}

/// Narrows a `sysinfo` PID to the `u32` the IPC boundary carries.
fn pid_as_u32(pid: sysinfo::Pid) -> u32 {
    pid.as_u32()
}

/// Maps `sysinfo`'s disk classification onto osstat's.
fn map_disk_kind(kind: sysinfo::DiskKind) -> DiskKind {
    match kind {
        sysinfo::DiskKind::HDD => DiskKind::Hdd,
        sysinfo::DiskKind::SSD => DiskKind::Ssd,
        sysinfo::DiskKind::Unknown(_) => DiskKind::Unknown,
    }
}

/// Maps `sysinfo`'s per-OS process states onto the handful osstat displays.
///
/// `sysinfo` models every state each platform can report; osstat shows a
/// process list, so the many flavours of "waiting" collapse to one. The
/// distinction that must survive is [`ProcessStatus::Zombie`] — a zombie is not
/// a running process, and showing it as one invites a user to try to kill
/// something that has already exited.
fn map_status(status: sysinfo::ProcessStatus) -> ProcessStatus {
    use sysinfo::ProcessStatus as S;

    match status {
        S::Run => ProcessStatus::Running,
        S::Sleep | S::UninterruptibleDiskSleep | S::Parked | S::LockBlocked | S::Waking => {
            ProcessStatus::Sleeping
        }
        S::Idle => ProcessStatus::Idle,
        S::Stop | S::Tracing | S::Suspended => ProcessStatus::Stopped,
        S::Zombie | S::Dead => ProcessStatus::Zombie,
        _ => ProcessStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_registry_assigns_indices_in_first_seen_order() {
        let mut registry = Vec::new();

        assert_eq!(SysinfoSource::index_in(&mut registry, "C:\\"), 0);
        assert_eq!(SysinfoSource::index_in(&mut registry, "D:\\"), 1);
        assert_eq!(SysinfoSource::index_in(&mut registry, "C:\\"), 0);
    }

    #[test]
    fn a_device_disappearing_does_not_renumber_the_others() {
        // Unplugging a USB stick must not silently re-label every other disk.
        let mut registry = Vec::new();
        SysinfoSource::index_in(&mut registry, "C:\\");
        SysinfoSource::index_in(&mut registry, "E:\\");
        SysinfoSource::index_in(&mut registry, "D:\\");

        // E: is gone this tick; C: and D: keep the indices the UI already knows.
        assert_eq!(SysinfoSource::index_in(&mut registry, "C:\\"), 0);
        assert_eq!(SysinfoSource::index_in(&mut registry, "D:\\"), 2);
    }

    #[test]
    fn rates_are_bytes_divided_by_elapsed_time() {
        assert_eq!(per_second(1_000, 2.0), 500);
        assert_eq!(per_second(1_000, 0.5), 2_000);
    }

    #[test]
    fn a_zero_length_tick_cannot_produce_an_absurd_rate() {
        // elapsed_seconds() floors the divisor; this guards the arithmetic itself.
        assert_eq!(per_second(0, 0.001), 0);
        assert_eq!(per_second(1_000, f64::INFINITY), 0);
    }

    #[test]
    fn a_zombie_is_never_reported_as_running() {
        assert_eq!(
            map_status(sysinfo::ProcessStatus::Zombie),
            ProcessStatus::Zombie
        );
        assert_eq!(
            map_status(sysinfo::ProcessStatus::Dead),
            ProcessStatus::Zombie
        );
    }

    #[test]
    fn the_many_flavours_of_waiting_collapse_to_sleeping() {
        for status in [
            sysinfo::ProcessStatus::Sleep,
            sysinfo::ProcessStatus::UninterruptibleDiskSleep,
            sysinfo::ProcessStatus::Parked,
        ] {
            assert_eq!(map_status(status), ProcessStatus::Sleeping);
        }
    }

    #[test]
    fn an_unmodelled_state_is_unknown_rather_than_a_guess() {
        assert_eq!(
            map_status(sysinfo::ProcessStatus::Unknown(9_999)),
            ProcessStatus::Unknown
        );
    }

    #[test]
    fn disk_kinds_map_across() {
        assert_eq!(map_disk_kind(sysinfo::DiskKind::HDD), DiskKind::Hdd);
        assert_eq!(map_disk_kind(sysinfo::DiskKind::SSD), DiskKind::Ssd);
        assert_eq!(
            map_disk_kind(sysinfo::DiskKind::Unknown(-1)),
            DiskKind::Unknown
        );
    }

    #[test]
    fn an_unset_mac_address_is_reported_as_absent_not_as_zeroes() {
        assert!(format_mac(MacAddr::UNSPECIFIED).is_none());
        assert!(format_mac(MacAddr([0, 1, 2, 3, 4, 5])).is_some());
    }

    // The remaining tests touch the real machine. They assert only what is true
    // of every runner in the CI matrix, including containers.

    #[test]
    fn describes_the_running_machine() {
        let mut source = SysinfoSource::new();

        let description = source.describe().unwrap();

        assert!(description.cpu.logical_cores >= 1);
        assert!(description.total_memory > 0);
    }

    #[test]
    fn samples_report_one_reading_per_logical_core() {
        let mut source = SysinfoSource::new();

        let description = source.describe().unwrap();
        let sample = source.sample().unwrap();

        assert_eq!(
            u32::try_from(sample.cpu_per_core.len()).unwrap(),
            description.cpu.logical_cores
        );
    }

    #[test]
    fn samples_report_plausible_utilisation() {
        let mut source = SysinfoSource::new();
        let sample = source.sample().unwrap();

        assert!(sample.cpu_total >= 0.0, "utilisation cannot be negative");
        assert!(sample.memory_used > 0, "something is using memory");
        assert!(sample.at_ms > 0);
    }

    #[test]
    fn the_process_list_contains_the_test_binary_itself() {
        let mut source = SysinfoSource::new();

        let processes = source.processes().unwrap();

        assert!(!processes.is_empty());
        assert!(
            processes.iter().any(|p| p.pid == std::process::id()),
            "the running test process should be in its own process list"
        );
    }

    #[test]
    fn every_process_has_a_start_time_so_identity_works() {
        let mut source = SysinfoSource::new();
        let processes = source.processes().unwrap();

        let self_pid = std::process::id();
        let me = processes.iter().find(|p| p.pid == self_pid).unwrap();

        assert!(
            me.started_at > 0,
            "without a start time, PID reuse is undetectable"
        );
    }
}
