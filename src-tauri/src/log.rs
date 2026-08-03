//! The only place osstat writes a log line.
//!
//! **No user data reaches a log line, at any level, and there is no toggle to
//! enable it.** That is the whole point of this module, and the way it is
//! enforced is the shape of the surface below rather than a rule anybody has
//! to remember: every event function takes counts, durations, booleans and
//! `&'static str` kinds. There is no `String`, `&str`, `Path` or `PathBuf`
//! parameter on any of them, so a path cannot be logged because there is
//! nowhere to put one.
//!
//! The exceptions are [`init`], [`prune`] and [`save`], which are about the
//! log *directory* rather than about a log line: where the file is written,
//! how many are kept, and where a copy is put when a user attaches one to a bug
//! report. None of them can put a path in a line, and none of them logs.
//!
//! Errors are logged by [`kind()`](osstat_core::Error::kind), never by
//! `Display` and never by `{:?}`: `ChatError::SpawnFailed` carries
//! `llama-server`'s stderr, which names the model path it was given, and
//! `AcquireError`'s variants name files and URLs.
//!
//! Two further consequences worth stating, because they look like details and
//! are not:
//!
//! - The filter is scoped to the `osstat_lib` target. A bare level directive
//!   would also enable `reqwest` and `hyper`, which log URLs.
//! - Sizes are logged in whole megabytes and context lengths in whole
//!   kibi-tokens. That is partly because nobody reading a log wants
//!   `4683074240`, and partly because it keeps every number this module emits
//!   short — which is what lets the scan test treat a long run of digits as
//!   suspicious rather than routine.
//!
//! See `docs/superpowers/specs/2026-08-03-event-logging-design.md` §2 and §6.

// Tauri resolves managed state by injecting `State<'_, T>` by value; a
// reference is not part of the command signature it accepts. The guard is a
// cheap handle, so the lint's concern does not apply here. Same reason as
// `commands.rs`.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{Level, debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// Bytes in a mebibyte, for the size fields.
const MIB: u64 = 1_048_576;

/// Tokens in a kibi-token, for the context-length field.
const KIBI_TOKENS: u32 = 1_024;

/// How many daily files are kept.
///
/// A week. osstat sits in the tray all day, so unbounded logs on a monitoring
/// utility would be its own bug (design §4); a week is long enough that "it
/// started doing this on Monday" is still in the directory on Friday.
pub const RETENTION: usize = 7;

/// What every file [`init`]'s appender writes is called, before the date.
const FILE_PREFIX: &str = "osstat.";

/// What every file [`init`]'s appender writes is called, after the date.
const FILE_SUFFIX: &str = ".log";

/// The handle that lets [`set_level`] change the filter without a restart.
///
/// Empty until [`init`] succeeds, which is why [`set_level`] is infallible and
/// silent: a build with no log directory should still run.
static RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// How much detail the log carries.
///
/// Three settings are offered over `tracing`'s five. `warn` and `error` are
/// always enabled underneath all three — see [`filter_directive`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    /// Lifecycle only: startup, probe results as counts, session start and
    /// stop, download outcomes.
    #[default]
    Info,
    /// Adds per-operation detail: event emissions and state transitions.
    Debug,
    /// Adds everything, including the sampler's per-tick lines. High volume on
    /// an app that sits in the tray all day — for reproducing a problem rather
    /// than for leaving on.
    Verbose,
}

impl LogLevel {
    /// The `tracing` level this setting maps onto.
    const fn tracing_level(self) -> Level {
        match self {
            Self::Info => Level::INFO,
            Self::Debug => Level::DEBUG,
            Self::Verbose => Level::TRACE,
        }
    }
}

/// What the front end is reporting, as a fixed vocabulary.
///
/// An enum rather than a string for the reason the whole module exists: a
/// string parameter is somewhere a page title or a message could be put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum UiEventKind {
    /// The front end mounted and rendered its first view.
    Ready,
    /// The user moved to a different page.
    PageChanged,
    /// A control in Settings was changed.
    SettingChanged,
    /// A command the front end invoked came back with an error.
    CommandFailed,
}

impl UiEventKind {
    /// The name written to the log for this event.
    ///
    /// No wildcard arm, for the same reason the error kinds have none: a
    /// variant added later must fail to compile rather than log as something
    /// it is not.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::PageChanged => "page_changed",
            Self::SettingChanged => "setting_changed",
            Self::CommandFailed => "command_failed",
        }
    }
}

/// What the user chose from the tray, as a fixed vocabulary.
///
/// An enum for the same reason [`UiEventKind`] is one. No `serde` or `ts-rs`
/// here, unlike that type: the tray menu is built and handled entirely in Rust,
/// so this never crosses the IPC boundary and a binding for it would be a
/// TypeScript type nothing could produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// The window was brought back, from the icon or the menu.
    Show,
    /// Start-at-sign-in was toggled from the menu.
    Autostart,
    /// Quit was chosen.
    Quit,
}

impl TrayAction {
    /// The name written to the log for this action.
    ///
    /// No wildcard arm, for the same reason the error kinds have none.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Autostart => "autostart",
            Self::Quit => "quit",
        }
    }
}

/// The `EnvFilter` directive for a setting.
///
/// Two things are happening here, and both are load-bearing.
///
/// The level is `max`ed with `WARN`. On `tracing`'s ordering the greater level
/// is the more verbose one, so this is a floor: no setting can silence a
/// warning or an error, because an error the user did not opt into is the one
/// they most need. All three settings offered today are already at or above
/// `WARN`, so the `max` currently changes nothing — it is written this way so
/// that a quieter setting added later cannot quietly remove the floor.
///
/// The directive is also scoped to the `osstat_lib` target rather than being a
/// bare level. A bare level would enable every dependency's instrumentation,
/// and `reqwest` and `hyper` log URLs — user data, arriving through the back
/// door, at exactly the setting a user turned on to help someone debug.
fn filter_directive(level: LogLevel) -> &'static str {
    let level = level.tracing_level().max(Level::WARN);

    if level >= Level::TRACE {
        "osstat_lib=trace"
    } else if level >= Level::DEBUG {
        "osstat_lib=debug"
    } else if level >= Level::INFO {
        "osstat_lib=info"
    } else {
        "osstat_lib=warn"
    }
}

/// Starts logging to a rolling daily file in `dir`, at `level`.
///
/// `dir` is a destination, not something to log; see the module docs.
///
/// # Returns
///
/// The [`WorkerGuard`] for the background writer, which the caller **must keep
/// alive for the process lifetime**. Dropping it discards whatever the worker
/// has buffered, which would make the last moments before a crash exactly the
/// part that goes missing.
///
/// `None` if the directory could not be created, the appender could not be
/// built, or a global subscriber is already installed. Logging is a diagnostic
/// aid; failing to set it up must not stop a system monitor from starting.
#[must_use]
pub fn init(dir: &Path, level: LogLevel) -> Option<WorkerGuard> {
    std::fs::create_dir_all(dir).ok()?;

    // `osstat.YYYY-MM-DD.log` rather than the default `osstat.log.YYYY-MM-DD`,
    // so the files sort together and open in whatever reads a `.log`. The dots
    // are the appender's own separators, which is why they are trimmed off here
    // and kept on the constants -- those are what `is_log_file` matches, and a
    // looser match is what would let retention delete a neighbour.
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILE_PREFIX.trim_end_matches('.'))
        .filename_suffix(FILE_SUFFIX.trim_start_matches('.'))
        .build(dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let (filter, handle) = reload::Layer::new(EnvFilter::new(filter_directive(level)));
    let subscriber = Registry::default()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(writer));

    tracing::subscriber::set_global_default(subscriber).ok()?;
    let _ = RELOAD.set(handle);

    Some(guard)
}

/// The process-lifetime home for the writer's [`WorkerGuard`].
///
/// Held as Tauri managed state so it lives exactly as long as the app does.
/// The guard is not decoration: dropping it discards whatever the background
/// writer has buffered, and the buffered lines are the ones written most
/// recently — the last moments before a crash, which is the part of a log
/// anybody actually opens it for.
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

impl LogGuard {
    /// Wraps a guard for [`tauri::Manager::manage`].
    #[must_use]
    pub const fn new(guard: WorkerGuard) -> Self {
        Self(guard)
    }
}

/// Changes how much detail the log carries, without a restart.
///
/// Silently does nothing if [`init`] never succeeded — there is no log to
/// change the level of, and a settings control should not fail because of it.
pub fn set_level(level: LogLevel) {
    if let Some(handle) = RELOAD.get() {
        let _ = handle.reload(EnvFilter::new(filter_directive(level)));
    }
}

/// Whether `name` is one of the files [`init`]'s appender writes.
///
/// Matched at both ends. The log directory sits under app data beside
/// `models.json` and the conversation store, and [`prune`] deletes whatever
/// this admits — so a looser match here is a deletion loop over app data.
fn is_log_file(name: &str) -> bool {
    name.starts_with(FILE_PREFIX) && name.ends_with(FILE_SUFFIX)
}

/// Every log file in `dir`, oldest first.
///
/// Sorted by name, which is chronological because the names carry ISO dates —
/// the reason [`init`] configures that layout rather than the appender's
/// default. Modification times would be the obvious alternative and are worse:
/// copying a directory rewrites them all, and retention would then delete by
/// the order the files were restored in.
///
/// A directory that cannot be read yields nothing rather than an error.
/// Retention and saving are both housekeeping, and neither is worth failing an
/// app start or a settings page over.
fn log_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_log_file)
        })
        .collect();
    files.sort();
    files
}

/// Deletes the oldest log files in `dir` until at most `keep` remain.
///
/// **The newest file is never deleted, whatever `keep` says.** It is the one
/// the appender currently has open, so removing it would truncate the session
/// the user is trying to diagnose — and that is precisely the session anybody
/// runs this app long enough to hit a retention cap in. The floor lives here
/// rather than in the caller so that lowering [`RETENTION`] later cannot
/// quietly remove it.
///
/// Failures are ignored: a file that will not delete costs disk, and refusing
/// to start over it would cost the app.
pub fn prune(dir: &Path, keep: usize) {
    let mut files = log_files(dir);
    let Some(excess) = files.len().checked_sub(keep.max(1)) else {
        return;
    };

    files.truncate(excess);
    for path in files {
        let _ = std::fs::remove_file(path);
    }
}

/// Copies every log file from `from` into `to`, and returns how many.
///
/// Copies rather than moves: the newest file is open and being written, and a
/// "save my logs" control that emptied the log directory would take the
/// evidence with it.
///
/// # Errors
///
/// If `to` cannot be created or a file cannot be copied into it. A `from` that
/// does not exist is not an error — it copies nothing and says so — because a
/// build whose log directory could never be created still has this control, and
/// the truthful answer there is "there is nothing to save".
pub fn save(from: &Path, to: &Path) -> std::io::Result<u32> {
    let files = log_files(from);
    if files.is_empty() {
        return Ok(0);
    }

    std::fs::create_dir_all(to)?;

    let mut copied = 0;
    for path in files {
        let Some(name) = path.file_name() else {
            continue;
        };
        std::fs::copy(&path, to.join(name))?;
        copied += 1;
    }

    Ok(copied)
}

/// Where the log is, and how much detail it is set to carry.
///
/// Held as Tauri managed state rather than persisted here, exactly as
/// [`crate::window_state`] holds the close behaviour: the front end stores the
/// preference and re-applies it at startup. There is one setting and it is not
/// worth a config file of its own.
///
/// Managed only when the app-data directory resolved, following
/// [`crate::models::ModelState`]. If it did not there is no log at all, and a
/// level control that appeared to work would be the wrong answer.
pub struct LogState {
    /// The directory [`init`] was given.
    dir: PathBuf,
    /// What [`set_level`] was last told, so Settings can show it.
    level: Mutex<LogLevel>,
}

impl LogState {
    /// Records that logging started in `dir` at `level`.
    #[must_use]
    pub const fn new(dir: PathBuf, level: LogLevel) -> Self {
        Self {
            dir,
            level: Mutex::new(level),
        }
    }

    /// Where the log files are.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.dir
    }

    /// How much detail the log is currently carrying.
    #[must_use]
    pub fn level(&self) -> LogLevel {
        *self.level.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a new setting. Applying it is [`set_level`].
    pub fn set(&self, level: LogLevel) {
        *self.level.lock().unwrap_or_else(PoisonError::into_inner) = level;
    }
}

/// How much detail the log is currently carrying.
#[tauri::command]
#[must_use]
pub fn log_level(state: State<'_, LogState>) -> LogLevel {
    state.level()
}

/// Changes how much detail the log carries, from now on.
///
/// Takes effect immediately and without a restart, which is what makes Verbose
/// usable at all: the problem someone turns it on for is usually one they are
/// about to reproduce, not one that survives closing the app.
#[tauri::command]
pub fn log_set_level(state: State<'_, LogState>, level: LogLevel) {
    state.set(level);
    set_level(level);
}

/// Where the log files are kept.
///
/// So Settings can show the folder, the same way it shows where models live.
/// Deleting it is safe at any time.
#[tauri::command]
#[must_use]
pub fn log_directory(state: State<'_, LogState>) -> String {
    state.directory().display().to_string()
}

/// Copies every log file into `path`, and returns how many were copied.
///
/// # Errors
///
/// If the folder cannot be created or a file cannot be copied into it.
#[tauri::command]
pub fn log_save(state: State<'_, LogState>, path: String) -> Result<u32, String> {
    save(state.directory(), Path::new(&path))
        // This is the one place in the module an error is formatted, and it is
        // deliberately not a log line: it is the sentence returned to the user
        // who just asked for this, in a folder they just typed. `grep` for
        // `{error}` in this file finds this and nothing else.
        .map_err(|error| format!("the logs could not be copied: {error}"))
}

/// Forwards something the front end did into the one log.
///
/// A command rather than a file the webview writes itself, so a session reads
/// as a single ordered story instead of two files a reader has to interleave by
/// timestamp — which is the work someone opening a log is trying to avoid
/// (design §5).
#[tauri::command]
pub fn ui_log(kind: UiEventKind) {
    ui_event(kind);
}

/// The app finished starting.
///
/// No GPU count here, although the plan sketched one: the probe is the slowest
/// thing the sampler does and deliberately runs *after* startup so it does not
/// delay the window, so a count on this line would be a number invented to fill
/// a field. It arrives on its own line — see [`gpus_probed`].
pub fn app_started() {
    info!("app started");
}

/// The GPU probe finished. `found` is how many devices there are — the count,
/// never what they are.
pub fn gpus_probed(found: usize) {
    info!(gpus = found, "gpus probed");
}

/// App data could not be resolved, so chat and downloads are unavailable.
///
/// No kind: the failure comes from Tauri's path resolver, whose error is a
/// message with a path in it.
pub fn app_data_unavailable() {
    error!("no app data directory; chat and downloads unavailable");
}

/// The tray icon could not be created. The app runs on without one.
pub fn tray_unavailable() {
    warn!("could not create the tray icon");
}

/// A model session started.
///
/// The context length is logged in whole kibi-tokens: `128`, not `131072`.
pub fn session_started(gpu_layers: u32, context_length: u32) {
    info!(
        gpu_layers,
        context_k = context_length / KIBI_TOKENS,
        "session started"
    );
}

/// A model session ended after `seconds`.
pub fn session_stopped(seconds: u64) {
    info!(seconds, "session stopped");
}

/// A model session could not be started or did not survive.
///
/// `kind` is [`ChatError::kind`](osstat_chat::ChatError::kind) — never the
/// error's `Display`, which carries the tail of `llama-server`'s stderr.
pub fn session_failed(kind: &'static str) {
    warn!(kind, "session failed");
}

/// A download began. Size in whole megabytes; no name, no URL.
pub fn download_started(bytes: u64) {
    info!(megabytes = bytes / MIB, "download started");
}

/// A download completed.
pub fn download_finished(bytes: u64, seconds: u64) {
    info!(megabytes = bytes / MIB, seconds, "download finished");
}

/// A download failed.
///
/// `kind` is [`AcquireError::kind`](osstat_inference::AcquireError::kind).
/// Which of two downloads it was is not recoverable from this line, and that
/// is the accepted cost — see the design's §2.
pub fn download_failed(kind: &'static str) {
    warn!(kind, "download failed");
}

/// The user paused a download.
///
/// Its own line rather than a [`download_failed`] with a `cancelled` kind,
/// because it is not a failure: the partial file is kept on purpose and the
/// next attempt resumes from it. A log that reported the two the same way would
/// have someone investigating a network problem that never happened.
pub fn download_paused() {
    info!("download paused");
}

/// The user cancelled a download.
///
/// Separate from [`download_paused`] because the disk ends up in a different
/// place: the partial file is deleted here and kept there, and "why did this
/// download start from zero" is answered by which of these two lines is in the
/// log.
pub fn download_cancelled() {
    info!("download cancelled");
}

/// A model search finished, in results and seconds.
///
/// **Never the search term**, and never a repository or file name. That is the
/// whole of what a log can say about a search without recording what the user
/// went looking for, which is exactly the kind of thing the design's §2 forbids
/// — the count and the duration answer "is search working and is it slow", and
/// nothing else here is worth someone's browsing history.
pub fn search_finished(results: usize, seconds: u64) {
    info!(results, seconds, "model search finished");
}

/// A model search failed.
///
/// `kind` is [`AcquireError::kind`](osstat_inference::AcquireError::kind), not
/// the error's `Display` — which carries the URL, and therefore the term.
pub fn search_failed(kind: &'static str) {
    warn!(kind, "model search failed");
}

/// An inference runtime is being fetched.
///
/// `cuda` is the only interesting fact about which one: it is the difference
/// between a 34 MB download and a 642 MB one, so a slow acquisition is
/// explained by this line or is worth looking into.
pub fn runtime_acquiring(cuda: bool) {
    info!(cuda, "acquiring runtime");
}

/// An inference runtime was installed.
pub fn runtime_acquired(bytes: u64, seconds: u64) {
    info!(megabytes = bytes / MIB, seconds, "runtime acquired");
}

/// An inference runtime could not be installed.
///
/// `kind` is [`AcquireError::kind`](osstat_inference::AcquireError::kind).
pub fn runtime_acquire_failed(kind: &'static str) {
    warn!(kind, "runtime acquisition failed");
}

/// The model library finished moving to another folder.
pub fn library_moved(bytes: u64, seconds: u64) {
    info!(megabytes = bytes / MIB, seconds, "library moved");
}

/// The model library could not be moved. Neither folder is named.
pub fn library_move_failed(kind: &'static str) {
    warn!(kind, "library move failed");
}

/// A conversation could not be written to disk.
///
/// At `error` rather than `warn`: something the user typed is at risk of being
/// lost, which is the most serious thing this module reports.
pub fn conversation_save_failed(kind: &'static str) {
    error!(kind, "conversation save failed");
}

/// Something the front end reported, forwarded into the one log.
pub fn ui_event(kind: UiEventKind) {
    debug!(kind = kind.kind(), "ui event");
}

/// Something the user chose from the tray menu or its icon.
pub fn tray_action(action: TrayAction) {
    debug!(action = action.kind(), "tray action");
}

/// One sampler tick, and how many processes it read.
///
/// Zero on a background tick, which does not read the process table at all —
/// the expensive half of a tick, and nothing a hidden window shows depends on
/// it. A run of zeroes in a log is therefore the window having been hidden,
/// which is worth being able to see.
///
/// At `trace`, so it is only written under Verbose. osstat ticks every two
/// seconds all day; at `info` this line alone would be most of the log.
pub fn sampler_tick(processes: usize) {
    trace!(processes, "sampled");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex, PoisonError};

    use tracing_subscriber::EnvFilter;

    use super::*;

    /// A writer that keeps everything the subscriber hands it.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Every line `emit` produces at `level`.
    ///
    /// The subscriber is thread-local (`with_default`), not global, so these
    /// tests neither call [`init`] nor interfere with each other.
    ///
    /// Timestamps are **omitted** here rather than stripped afterwards. They
    /// are digits and colons and would trip the scan test on every line, for
    /// something the module did not choose to write; stripping instead would
    /// mean guessing where the prefix ends, and a wrong guess would silently
    /// hide the front of every line from the scan. The real appender in
    /// [`init`] keeps its timestamps — a log without them is not much use.
    fn capture(level: LogLevel, emit: impl FnOnce()) -> Vec<String> {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(filter_directive(level)))
            .with_writer(captured.clone())
            .without_time()
            .finish();

        tracing::subscriber::with_default(subscriber, emit);

        let bytes = captured
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Calls every event function in this module.
    ///
    /// The scan test is only worth something if it covers all of them, so a
    /// function added above belongs here too. The values are deliberately the
    /// largest realistic ones — a multi-gigabyte download, a 128k context, a
    /// thousand-process machine — because a scan run on toy numbers would pass
    /// while the real log lines failed.
    fn emit_every_event() {
        app_started();
        gpus_probed(2);
        session_started(35, 131_072);
        session_stopped(4_215);
        session_failed("spawn_failed");
        download_started(4_683_074_240);
        download_finished(4_683_074_240, 612);
        download_failed("checksum_mismatch");
        download_paused();
        download_cancelled();
        search_finished(120, 3);
        search_failed("http_status");
        runtime_acquiring(true);
        runtime_acquired(537_835_790, 94);
        runtime_acquire_failed("http_status");
        library_moved(9_663_676_416, 128);
        library_move_failed("not_enough_space");
        conversation_save_failed("io");
        app_data_unavailable();
        tray_unavailable();
        ui_event(UiEventKind::Ready);
        ui_event(UiEventKind::PageChanged);
        ui_event(UiEventKind::SettingChanged);
        ui_event(UiEventKind::CommandFailed);
        tray_action(TrayAction::Show);
        tray_action(TrayAction::Autostart);
        tray_action(TrayAction::Quit);
        sampler_tick(1_284);
    }

    #[test]
    fn warnings_survive_the_quietest_setting() {
        // An error the user did not opt into is the one they most need.
        let lines = capture(LogLevel::Info, || {
            download_failed("checksum_mismatch");
            conversation_save_failed("io");
        });

        assert!(
            lines.iter().any(|line| line.contains("checksum_mismatch")),
            "a warning was filtered out at the quietest setting: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("WARN")),
            "the warning did not arrive at WARN: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("ERROR")),
            "an error was filtered out at the quietest setting: {lines:?}"
        );
    }

    #[test]
    fn a_higher_setting_does_not_leak_into_a_lower_one() {
        let lines = capture(LogLevel::Info, || {
            app_started();
            ui_event(UiEventKind::Ready);
            sampler_tick(1_284);
        });

        assert!(
            lines.iter().any(|line| line.contains("app started")),
            "Info dropped its own level: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("DEBUG")),
            "Debug leaked into Info: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("TRACE")),
            "Verbose leaked into Info: {lines:?}"
        );
    }

    #[test]
    fn debug_admits_debug_but_still_not_verbose() {
        let lines = capture(LogLevel::Debug, || {
            ui_event(UiEventKind::Ready);
            sampler_tick(1_284);
        });

        assert!(
            lines.iter().any(|line| line.contains("DEBUG")),
            "Debug dropped its own level: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("TRACE")),
            "Verbose leaked into Debug: {lines:?}"
        );
    }

    #[test]
    fn verbose_admits_the_sampler_ticks_nothing_else_does() {
        let lines = capture(LogLevel::Verbose, || sampler_tick(1_284));

        assert!(
            lines.iter().any(|line| line.contains("TRACE")),
            "Verbose dropped its own level: {lines:?}"
        );
    }

    #[test]
    fn nothing_emitted_looks_like_user_data() {
        // A net under the module boundary, not a guarantee -- see spec §9.
        // Catches the realistic breakage: a struct that grew a path field and
        // is logged with {:?}. Verbose so nothing is filtered out of the scan.
        let lines = capture(LogLevel::Verbose, emit_every_event);

        assert!(!lines.is_empty(), "captured nothing, so scanned nothing");
        for line in lines {
            assert!(
                !line.contains('/') && !line.contains('\\'),
                "path-like: {line}"
            );
            assert!(!line.contains('@'), "address-like: {line}");
            assert!(
                !line
                    .chars()
                    .collect::<Vec<_>>()
                    .windows(6)
                    .any(|w| w.iter().all(char::is_ascii_digit)),
                "long digit run: {line}"
            );
        }
    }

    #[test]
    fn the_level_setting_serialises_as_the_strings_the_front_end_stores() {
        assert_eq!(serde_json::to_string(&LogLevel::Info).unwrap(), "\"info\"");
        assert_eq!(
            serde_json::to_string(&LogLevel::Debug).unwrap(),
            "\"debug\""
        );
        assert_eq!(
            serde_json::to_string(&LogLevel::Verbose).unwrap(),
            "\"verbose\""
        );
    }

    #[test]
    fn info_is_the_default_setting() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn no_setting_can_silence_a_warning() {
        // The floor, checked directly rather than only through the emitted
        // output: every offered setting must resolve to WARN or more verbose.
        for level in [LogLevel::Info, LogLevel::Debug, LogLevel::Verbose] {
            let directive = filter_directive(level);
            assert!(
                !directive.ends_with("=error") && !directive.ends_with("=off"),
                "{level:?} silences warnings: {directive}"
            );
        }
    }

    #[test]
    fn the_filter_is_scoped_to_osstat_so_no_dependency_can_log() {
        // reqwest and hyper log URLs. A bare level directive would enable
        // them, which is precisely the leak this feature exists to prevent.
        for level in [LogLevel::Info, LogLevel::Debug, LogLevel::Verbose] {
            let directive = filter_directive(level);
            assert!(
                directive.starts_with("osstat_lib="),
                "{level:?} is not scoped to osstat: {directive}"
            );
        }
    }

    #[test]
    fn retention_deletes_oldest_first_and_never_todays_file() {
        // Deleting the file being written truncates the session you are
        // currently trying to diagnose. The appender holds today's file open,
        // and its ISO name sorts last, so "newest" and "today's" are the same
        // file -- which is what makes retaining the newest sufficient.
        let directory = tempfile::tempdir().unwrap();
        let days = [
            "2026-07-25",
            "2026-07-26",
            "2026-07-27",
            "2026-07-28",
            "2026-07-29",
            "2026-07-30",
            "2026-07-31",
            "2026-08-01",
            "2026-08-02",
            "2026-08-03",
        ];
        for day in days {
            std::fs::write(directory.path().join(format!("osstat.{day}.log")), b"x").unwrap();
        }

        prune(directory.path(), RETENTION);

        let kept: Vec<String> = log_files(directory.path())
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
            .collect();
        assert_eq!(kept.len(), RETENTION, "kept {kept:?}");
        assert!(
            kept.contains(&"osstat.2026-08-03.log".to_owned()),
            "the file being written was deleted: {kept:?}"
        );
        assert!(
            !kept.contains(&"osstat.2026-07-25.log".to_owned()),
            "the oldest file survived the cap: {kept:?}"
        );
        assert!(
            !kept.contains(&"osstat.2026-07-27.log".to_owned()),
            "the cap deleted fewer than it had to: {kept:?}"
        );
        assert!(
            kept.contains(&"osstat.2026-07-28.log".to_owned()),
            "the cap deleted more than it had to: {kept:?}"
        );
    }

    #[test]
    fn the_file_being_written_survives_even_a_cap_of_zero() {
        // The floor is in `prune` rather than in its callers, so a retention
        // figure someone lowers later cannot delete the open file.
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("osstat.2026-08-02.log"), b"old").unwrap();
        std::fs::write(directory.path().join("osstat.2026-08-03.log"), b"today").unwrap();

        prune(directory.path(), 0);

        assert_eq!(log_files(directory.path()).len(), 1);
        assert!(directory.path().join("osstat.2026-08-03.log").exists());
    }

    #[test]
    fn retention_leaves_a_directory_under_the_cap_alone() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("osstat.2026-08-03.log"), b"today").unwrap();

        prune(directory.path(), RETENTION);

        assert_eq!(log_files(directory.path()).len(), 1);
    }

    #[test]
    fn retention_touches_nothing_that_is_not_a_log() {
        // The log directory is under app data, beside files that are not
        // disposable. A prefix match is what keeps this an unremarkable
        // housekeeping step rather than a deletion loop over a directory.
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("models.json"), b"index").unwrap();
        std::fs::write(directory.path().join("osstat.2026-07-01.log"), b"a").unwrap();
        std::fs::write(directory.path().join("osstat.2026-07-02.log"), b"b").unwrap();

        prune(directory.path(), 1);

        assert!(
            directory.path().join("models.json").exists(),
            "retention deleted something that was not a log"
        );
        assert_eq!(log_files(directory.path()).len(), 1);
    }

    #[test]
    fn saving_copies_every_log_and_alters_none() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = directory.path().join("for-the-bug-report");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("osstat.2026-08-02.log"), b"yesterday").unwrap();
        std::fs::write(source.join("osstat.2026-08-03.log"), b"today").unwrap();

        let copied = save(&source, &destination).unwrap();

        assert_eq!(copied, 2);
        assert_eq!(
            std::fs::read_to_string(destination.join("osstat.2026-08-02.log")).unwrap(),
            "yesterday"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("osstat.2026-08-03.log")).unwrap(),
            "today"
        );
        // Copied, not moved: the originals are what the running app is still
        // writing to, and saving must not be a way to lose them.
        assert_eq!(log_files(&source).len(), 2);
        assert_eq!(
            std::fs::read_to_string(source.join("osstat.2026-08-03.log")).unwrap(),
            "today"
        );
    }

    #[test]
    fn saving_takes_the_logs_and_leaves_everything_else() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("logs");
        let destination = directory.path().join("elsewhere");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("osstat.2026-08-03.log"), b"today").unwrap();
        std::fs::write(source.join("notes.txt"), b"not a log").unwrap();

        let copied = save(&source, &destination).unwrap();

        assert_eq!(copied, 1);
        assert!(!destination.join("notes.txt").exists());
    }

    #[test]
    fn saving_from_a_directory_that_never_existed_copies_nothing_rather_than_failing() {
        // A build whose log directory could not be created still has a Settings
        // page, and a control that errors there says something is broken when
        // the truthful answer is that there is nothing to save.
        let directory = tempfile::tempdir().unwrap();

        let copied = save(
            &directory.path().join("never-made"),
            &directory.path().join("out"),
        )
        .unwrap();

        assert_eq!(copied, 0);
    }

    #[test]
    fn the_level_state_holds_what_it_was_last_set_to() {
        let state = LogState::new(PathBuf::from("logs"), LogLevel::Info);
        assert_eq!(state.level(), LogLevel::Info);

        state.set(LogLevel::Verbose);

        assert_eq!(state.level(), LogLevel::Verbose);
        assert_eq!(state.directory(), Path::new("logs"));
    }

    #[test]
    fn every_ui_event_kind_has_a_distinct_name() {
        let kinds = [
            UiEventKind::Ready.kind(),
            UiEventKind::PageChanged.kind(),
            UiEventKind::SettingChanged.kind(),
            UiEventKind::CommandFailed.kind(),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for kind in kinds {
            assert!(!seen.contains(&kind), "duplicate kind {kind}");
            assert!(!kind.is_empty());
            seen.push(kind);
        }
    }

    #[test]
    fn every_tray_action_has_a_distinct_name() {
        // Same reason as the error kinds: a copied-and-not-edited string makes
        // two different actions look identical in a log.
        let kinds = [
            TrayAction::Show.kind(),
            TrayAction::Autostart.kind(),
            TrayAction::Quit.kind(),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for kind in kinds {
            assert!(!seen.contains(&kind), "duplicate kind {kind}");
            assert!(!kind.is_empty());
            seen.push(kind);
        }
    }
}
