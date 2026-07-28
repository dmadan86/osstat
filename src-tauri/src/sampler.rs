//! The background sampler: one thread that measures the machine and pushes the
//! result to the webview.
//!
//! ADR-007 is explicit that the front-end must not poll. This is the other half
//! of that decision — a single thread owns the `sysinfo` handles, ticks on a
//! configurable interval, and emits two events:
//!
//! - `metrics:tick`, a small scalar payload, sent every tick.
//! - `processes:tick`, a **diff**, sent only when something visible changed.
//!
//! # Why the state is split in two
//!
//! The sampling handles live on the thread and are never shared. What commands
//! read — history, the latest process list, the device list — lives behind an
//! `RwLock` that the thread write-locks briefly once per tick. A single mutex
//! covering both would make every command wait on a tick that can legitimately
//! take tens of milliseconds with a thousand processes in flight.
//!
//! # Why pausing is tied to minimising, not to focus
//!
//! Sampling stops when the window is minimised, which is when nobody can see
//! it. It deliberately does *not* stop when the window merely loses focus:
//! alt-tabbing away to make something happen and then coming back to look at
//! the graph is the single most common way this app will be used, and a
//! sampler that stopped on blur would erase exactly the history the user went
//! to fetch.

use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use osstat_core::{
    GpuDevice, GpuProvider, MetricsHistory, MetricsSample, ProcessProvider, ProcessRecord,
    ProcessTree, SystemDescription, SystemInfoProvider, diff_processes,
};
use osstat_llm::HardwareProbe;
use osstat_platform::SysinfoSource;
use tauri::{AppHandle, Emitter};

/// Event carrying one tick of measurement.
pub const METRICS_EVENT: &str = "metrics:tick";

/// Event carrying what changed in the process table.
pub const PROCESSES_EVENT: &str = "processes:tick";

/// Event fired once, when the GPU probe has finished.
///
/// Probing enumerates graphics adapters, which is slow enough to be worth
/// keeping off the startup path. The UI shows the GPU section as detecting
/// until this arrives.
pub const GPUS_READY_EVENT: &str = "gpus:ready";

/// The slowest tick the UI offers, and the ceiling for a requested interval.
const MAX_INTERVAL: Duration = Duration::from_mins(1);

/// The fastest tick the UI offers.
const MIN_INTERVAL: Duration = Duration::from_millis(500);

/// What the front-end can read between ticks.
#[derive(Default)]
struct Snapshot {
    history: MetricsHistory,
    processes: Vec<ProcessRecord>,
    devices: Option<Vec<GpuDevice>>,
}

/// Knobs the UI can turn, and the signal that they have turned.
struct Control {
    interval: Duration,
    paused: bool,
    stopping: bool,
}

/// State shared between the sampler thread and the command handlers.
struct Shared {
    snapshot: RwLock<Snapshot>,
    control: Mutex<Control>,
    /// Notified whenever [`Control`] changes, so an interval change or an
    /// unpause takes effect immediately rather than after the current sleep.
    changed: Condvar,
}

/// Handle to the running sampler, held as Tauri managed state.
pub struct Sampler {
    shared: Arc<Shared>,
    /// The machine's identity, read once at startup.
    description: SystemDescription,
}

impl Sampler {
    /// Reads the machine's description, then starts sampling in the background.
    ///
    /// # Errors
    ///
    /// Returns an error only if the machine cannot be described at all, which
    /// means there is nothing for the app to show.
    pub fn start(app: AppHandle, interval: Duration) -> osstat_core::Result<Self> {
        let mut source = SysinfoSource::new();
        let description = source.describe()?;
        let system_memory = description.total_memory;

        let shared = Arc::new(Shared {
            snapshot: RwLock::new(Snapshot::default()),
            control: Mutex::new(Control {
                interval: clamp_interval(interval),
                paused: false,
                stopping: false,
            }),
            changed: Condvar::new(),
        });

        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("osstat-sampler".into())
            .spawn(move || run(&app, &worker, source, HardwareProbe::new(system_memory)))
            .map_err(osstat_core::Error::Io)?;

        Ok(Self {
            shared,
            description,
        })
    }

    /// The machine's identity.
    #[must_use]
    pub fn description(&self) -> &SystemDescription {
        &self.description
    }

    /// The most recent `limit` samples, oldest first.
    #[must_use]
    pub fn history(&self, limit: Option<usize>) -> Vec<MetricsSample> {
        let snapshot = read(&self.shared.snapshot);
        match limit {
            Some(limit) => snapshot.history.recent(limit),
            None => snapshot.history.iter().cloned().collect(),
        }
    }

    /// The current process tree.
    ///
    /// Built on demand rather than cached: the flat list is what the sampler
    /// needs for diffing, and arranging it costs far less than the read.
    #[must_use]
    pub fn process_tree(&self) -> ProcessTree {
        ProcessTree::build(read(&self.shared.snapshot).processes.clone())
    }

    /// The GPUs found, or `None` while the probe is still running.
    #[must_use]
    pub fn devices(&self) -> Option<Vec<GpuDevice>> {
        read(&self.shared.snapshot).devices.clone()
    }

    /// Changes the tick interval, taking effect immediately.
    ///
    /// Out-of-range values are clamped rather than rejected: this arrives from
    /// a preference that a stale or hand-edited client could get wrong, and a
    /// clamped tick is better than a failed command or a busy loop.
    pub fn set_interval(&self, interval: Duration) {
        let mut control = lock(&self.shared.control);
        control.interval = clamp_interval(interval);
        control.paused = false;
        self.shared.changed.notify_all();
    }

    /// Suspends or resumes sampling.
    pub fn set_paused(&self, paused: bool) {
        lock(&self.shared.control).paused = paused;
        self.shared.changed.notify_all();
    }

    /// Whether sampling is currently suspended.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        lock(&self.shared.control).paused
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        lock(&self.shared.control).stopping = true;
        self.shared.changed.notify_all();
    }
}

/// The sampler thread's body.
fn run(app: &AppHandle, shared: &Arc<Shared>, mut source: SysinfoSource, mut gpus: HardwareProbe) {
    // Probe for GPUs first: it is the slowest thing here and it only happens
    // once, so it runs off the startup path rather than delaying the window.
    let devices = gpus.devices().unwrap_or_default();
    write(&shared.snapshot).devices = Some(devices);
    let _ = app.emit(GPUS_READY_EVENT, ());

    let mut previous: Vec<ProcessRecord> = Vec::new();

    loop {
        if !wait_for_tick(shared) {
            return;
        }

        let Ok(mut sample) = source.sample() else {
            continue;
        };
        sample.gpus = gpus.measure().unwrap_or_default();

        let processes = source.processes().unwrap_or_default();
        let diff = diff_processes(&previous, &processes);

        {
            let mut snapshot = write(&shared.snapshot);
            snapshot.history.push(sample.clone());
            snapshot.processes.clone_from(&processes);
        }
        previous = processes;

        let _ = app.emit(METRICS_EVENT, &sample);
        if !diff.is_empty() {
            let _ = app.emit(PROCESSES_EVENT, &diff);
        }
    }
}

/// Sleeps until the next tick is due.
///
/// Returns `false` when the sampler should stop. While paused it waits without
/// a timeout, so a minimised window costs nothing at all rather than costing a
/// wakeup per interval.
fn wait_for_tick(shared: &Arc<Shared>) -> bool {
    let mut control = lock(&shared.control);

    loop {
        if control.stopping {
            return false;
        }
        if !control.paused {
            break;
        }
        control = shared
            .changed
            .wait(control)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    let interval = control.interval;
    let (control, _) = shared
        .changed
        .wait_timeout(control, interval)
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    !control.stopping
}

/// Holds a requested interval to something the UI can actually offer.
fn clamp_interval(interval: Duration) -> Duration {
    interval.clamp(MIN_INTERVAL, MAX_INTERVAL)
}

/// Locks a mutex, recovering from a poisoned lock rather than propagating a panic.
///
/// A panic in one tick must not take the whole app's telemetry down with it:
/// the data behind these locks is a rolling window that the next tick replaces
/// anyway, so continuing with it is safe.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Read-locks, recovering from poisoning for the same reason as [`lock`].
fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Write-locks, recovering from poisoning for the same reason as [`lock`].
fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    // `panic` is allowed here because two tests deliberately poison a lock to
    // prove the recovery path works.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn an_absurdly_fast_interval_is_clamped_rather_than_becoming_a_busy_loop() {
        assert_eq!(clamp_interval(Duration::from_millis(1)), MIN_INTERVAL);
        assert_eq!(clamp_interval(Duration::ZERO), MIN_INTERVAL);
    }

    #[test]
    fn an_absurdly_slow_interval_is_clamped_to_something_a_user_would_wait_for() {
        assert_eq!(clamp_interval(Duration::from_hours(24)), MAX_INTERVAL);
    }

    #[test]
    fn the_intervals_the_ui_offers_all_survive_clamping() {
        for millis in [1_000, 2_000, 5_000] {
            let requested = Duration::from_millis(millis);
            assert_eq!(clamp_interval(requested), requested);
        }
    }

    #[test]
    fn a_poisoned_mutex_is_recovered_rather_than_taking_the_app_down() {
        let mutex = Arc::new(Mutex::new(7));
        let poisoner = Arc::clone(&mutex);

        let _ = thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("simulated panic while holding the lock");
        })
        .join();

        assert!(mutex.is_poisoned());
        assert_eq!(
            *lock(&mutex),
            7,
            "the rolling data behind the lock is still usable"
        );
    }

    #[test]
    fn a_poisoned_rwlock_is_recovered_too() {
        let rwlock = Arc::new(RwLock::new(3));
        let poisoner = Arc::clone(&rwlock);

        let _ = thread::spawn(move || {
            let _guard = poisoner.write().unwrap();
            panic!("simulated panic while holding the lock");
        })
        .join();

        assert_eq!(*read(&rwlock), 3);
        assert_eq!(*write(&rwlock), 3);
    }

    #[test]
    fn event_names_are_namespaced_so_they_cannot_collide_with_tauri_internals() {
        for name in [METRICS_EVENT, PROCESSES_EVENT, GPUS_READY_EVENT] {
            assert!(name.contains(':'), "{name} should be namespaced");
            assert!(!name.starts_with("tauri://"));
        }
    }
}
