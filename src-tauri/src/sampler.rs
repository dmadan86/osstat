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
//! # Why the window state is tied to minimising, not to focus
//!
//! Sampling drops to the background — slower, silent, but still running —
//! when the window is minimised or hidden, which is when nobody can see it.
//! It stops altogether only when the user explicitly pauses it (see
//! [`Activity`] for the three states). Neither transition happens when the
//! window merely loses focus: alt-tabbing away to make something happen and
//! then coming back to look at the graph is the single most common way this
//! app will be used, and a sampler that slowed on blur would erase exactly the
//! history the user went to fetch.

use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use osstat_core::{
    GpuDevice, GpuProvider, MetricsHistory, MetricsSample, ProcessProvider, ProcessRecord,
    SystemDescription, SystemInfoProvider, diff_processes,
};
use osstat_llm::HardwareProbe;
use osstat_platform::SysinfoSource;
use tauri::{AppHandle, Emitter};

use crate::tray;

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

/// The floor for how often the sampler ticks while the window cannot be seen.
///
/// This is a floor, not a fixed rate: the only thing background sampling feeds
/// is a tray tooltip, so it never needs to tick faster than this, but it does
/// not speed a slower foreground rate up either — see [`effective_interval`].
/// It is deliberately not configurable: a setting for it would be a knob whose
/// only effect is how much battery osstat burns while nobody is looking.
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);

/// What the sampler is currently doing, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// The window is visible: full rate, everything measured, events emitted.
    Foreground,
    /// The window cannot be seen: slow rate, metrics only, no events.
    Background,
    /// The user asked for sampling to stop.
    Paused,
}

impl Activity {
    /// Whether this state reads the process table.
    ///
    /// Only the foreground does. The process read is the expensive half of a
    /// tick and nothing a hidden app displays needs it.
    #[must_use]
    pub const fn reads_processes(self) -> bool {
        matches!(self, Self::Foreground)
    }

    /// Whether this state emits events to the webview.
    ///
    /// Only the foreground does. Delivering IPC payloads to a hidden webview is
    /// work nobody can observe.
    #[must_use]
    pub const fn emits(self) -> bool {
        matches!(self, Self::Foreground)
    }

    /// Whether this state measures anything at all.
    #[must_use]
    pub const fn samples(self) -> bool {
        !matches!(self, Self::Paused)
    }
}

/// Decides what the sampler should be doing.
///
/// An explicit pause always wins: someone who chose "Paused" in Settings should
/// not find sampling quietly resumed by restoring the window.
#[must_use]
pub const fn activity_for(visible: bool, minimized: bool, user_paused: bool) -> Activity {
    if user_paused {
        Activity::Paused
    } else if !visible || minimized {
        Activity::Background
    } else {
        Activity::Foreground
    }
}

/// The tick length a given activity actually uses.
///
/// `BACKGROUND_INTERVAL` is a floor rather than a replacement: someone who
/// deliberately slowed sampling down must not find it sped up the moment the
/// window is hidden. `Duration`'s comparison operators are not `const`, so the
/// floor is applied by comparing nanoseconds, which is.
const fn effective_interval(activity: Activity, foreground: Duration) -> Duration {
    match activity {
        Activity::Background if foreground.as_nanos() > BACKGROUND_INTERVAL.as_nanos() => {
            foreground
        }
        Activity::Background => BACKGROUND_INTERVAL,
        _ => foreground,
    }
}

/// What the front-end can read between ticks.
#[derive(Default)]
struct Snapshot {
    history: MetricsHistory,
    processes: Vec<ProcessRecord>,
    devices: Option<Vec<GpuDevice>>,
}

/// Knobs the UI can turn, and the signal that they have turned.
///
/// Four independent bools, not a combinatorial state machine: each one is set
/// from a different source (a preference, a settings toggle, two window
/// events) and they are only ever collapsed together by [`Control::activity`].
#[allow(clippy::struct_excessive_bools)]
struct Control {
    /// The tick the user asked for, used in the foreground.
    interval: Duration,
    /// Whether the user has explicitly paused sampling.
    user_paused: bool,
    /// Whether the window is currently on screen.
    visible: bool,
    /// Whether the window is minimised.
    minimized: bool,
    stopping: bool,
}

impl Control {
    /// What the sampler should be doing given the current knobs.
    const fn activity(&self) -> Activity {
        activity_for(self.visible, self.minimized, self.user_paused)
    }
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
                user_paused: false,
                visible: true,
                minimized: false,
                stopping: false,
            }),
            changed: Condvar::new(),
        });

        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("osstat-sampler".into())
            .spawn(move || {
                run(
                    &app,
                    &worker,
                    source,
                    HardwareProbe::new(system_memory),
                    system_memory,
                );
            })
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

    /// Every process seen on the most recent tick, flat and unordered.
    ///
    /// Flat because the front-end arranges them: it maintains its own structure
    /// in order to apply the per-tick diffs, so a hierarchy assembled here would
    /// be rebuilt on arrival anyway.
    #[must_use]
    pub fn processes(&self) -> Vec<ProcessRecord> {
        read(&self.shared.snapshot).processes.clone()
    }

    /// The GPUs found, or `None` while the probe is still running.
    #[must_use]
    pub fn devices(&self) -> Option<Vec<GpuDevice>> {
        read(&self.shared.snapshot).devices.clone()
    }

    /// Changes the foreground tick interval, taking effect immediately.
    ///
    /// Out-of-range values are clamped rather than rejected: this arrives from
    /// a preference that a stale or hand-edited client could get wrong, and a
    /// clamped tick is better than a failed command or a busy loop.
    pub fn set_interval(&self, interval: Duration) {
        let mut control = lock(&self.shared.control);
        control.interval = clamp_interval(interval);
        control.user_paused = false;
        self.shared.changed.notify_all();
    }

    /// Suspends or resumes sampling at the user's explicit request.
    pub fn set_paused(&self, paused: bool) {
        lock(&self.shared.control).user_paused = paused;
        self.shared.changed.notify_all();
    }

    /// Whether sampling is suspended at the user's explicit request.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        lock(&self.shared.control).user_paused
    }

    /// Records where the window is, which decides foreground versus background.
    pub fn set_window_state(&self, visible: bool, minimized: bool) {
        let mut control = lock(&self.shared.control);
        control.visible = visible;
        control.minimized = minimized;
        self.shared.changed.notify_all();
    }

    /// What the sampler is currently doing.
    #[must_use]
    pub fn activity(&self) -> Activity {
        lock(&self.shared.control).activity()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        lock(&self.shared.control).stopping = true;
        self.shared.changed.notify_all();
    }
}

/// The sampler thread's body.
fn run(
    app: &AppHandle,
    shared: &Arc<Shared>,
    mut source: SysinfoSource,
    mut gpus: HardwareProbe,
    total_memory: u64,
) {
    // Probe for GPUs first: it is the slowest thing here and it only happens
    // once, so it runs off the startup path rather than delaying the window.
    let devices = gpus.devices().unwrap_or_default();
    write(&shared.snapshot).devices = Some(devices);
    let _ = app.emit(GPUS_READY_EVENT, ());

    let mut previous: Vec<ProcessRecord> = Vec::new();

    loop {
        let Some(activity) = wait_for_tick(shared) else {
            return;
        };

        let Ok(mut sample) = source.sample() else {
            continue;
        };
        sample.gpus = gpus.measure().unwrap_or_default();

        // The process table is read only in the foreground. It is the expensive
        // half of a tick, and nothing a hidden window shows depends on it.
        if activity.reads_processes() {
            let processes = source.processes().unwrap_or_default();
            let diff = diff_processes(&previous, &processes);

            write(&shared.snapshot).processes.clone_from(&processes);
            previous = processes;

            if activity.emits() && !diff.is_empty() {
                let _ = app.emit(PROCESSES_EVENT, &diff);
            }
        }

        write(&shared.snapshot).history.push(sample.clone());

        // Refreshed in every state that samples at all, including the
        // background: the tooltip is the only thing a hidden window still
        // shows, so it is precisely what the background tick exists to feed.
        tray::set_tooltip(app, &tray::tooltip(Some(&sample), total_memory));

        if activity.emits() {
            let _ = app.emit(METRICS_EVENT, &sample);
        }
    }
}

/// Sleeps until the next tick is due.
///
/// Returns the activity the tick should run as, or `None` when the sampler
/// should stop. While paused it waits without a timeout, so a paused sampler
/// costs nothing at all rather than costing a wakeup per interval.
fn wait_for_tick(shared: &Arc<Shared>) -> Option<Activity> {
    let mut control = lock(&shared.control);

    let activity = loop {
        if control.stopping {
            return None;
        }
        let activity = control.activity();
        if activity.samples() {
            break activity;
        }
        control = shared
            .changed
            .wait(control)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    };

    let interval = effective_interval(activity, control.interval);
    let (control, _) = shared
        .changed
        .wait_timeout(control, interval)
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if control.stopping {
        None
    } else {
        // Re-read: the window may have moved between states during the sleep.
        Some(control.activity())
    }
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

    #[test]
    fn a_visible_window_samples_at_the_users_rate() {
        assert_eq!(activity_for(true, false, false), Activity::Foreground);
    }

    #[test]
    fn a_hidden_or_minimised_window_drops_to_the_background() {
        assert_eq!(activity_for(false, false, false), Activity::Background);
        assert_eq!(activity_for(true, true, false), Activity::Background);
        assert_eq!(activity_for(false, true, false), Activity::Background);
    }

    #[test]
    fn an_explicit_pause_beats_every_window_state() {
        // The user chose "Paused" in Settings. Restoring the window must not
        // quietly start sampling again.
        for (visible, minimized) in [(true, false), (false, false), (true, true)] {
            assert_eq!(activity_for(visible, minimized, true), Activity::Paused);
        }
    }

    #[test]
    fn hiding_the_window_never_samples_faster_than_the_foreground_would() {
        for foreground in [
            MIN_INTERVAL,
            Duration::from_secs(2),
            BACKGROUND_INTERVAL,
            Duration::from_secs(30),
            MAX_INTERVAL,
        ] {
            let background = effective_interval(Activity::Background, foreground);
            assert!(
                background >= foreground,
                "hiding must never speed sampling up: {foreground:?} became {background:?}"
            );
            assert!(
                background >= BACKGROUND_INTERVAL,
                "background must never tick faster than its floor"
            );
        }
    }

    #[test]
    // Paired with `hiding_the_window_never_samples_faster_than_the_foreground_would`
    // above: that test covers the floor from the slow side, this one covers it
    // from the fast side.
    fn a_fast_foreground_rate_drops_to_the_background_floor() {
        assert_eq!(
            effective_interval(Activity::Background, Duration::from_secs(1)),
            BACKGROUND_INTERVAL
        );
        assert_eq!(
            effective_interval(Activity::Foreground, Duration::from_secs(1)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn only_the_foreground_reads_the_process_table() {
        // The expensive half: 23.8 ms of a ~24 ms tick, and nothing in a tray
        // tooltip needs it.
        assert!(Activity::Foreground.reads_processes());
        assert!(!Activity::Background.reads_processes());
        assert!(!Activity::Paused.reads_processes());
    }

    #[test]
    fn only_the_foreground_emits_to_a_webview_that_can_see_it() {
        assert!(Activity::Foreground.emits());
        assert!(!Activity::Background.emits());
        assert!(!Activity::Paused.emits());
    }
}
