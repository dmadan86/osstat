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
//! The single exception is [`init`]'s `dir`, which is where the log is
//! *written to*. It is never a field in a line, and it is the only path this
//! module ever holds.
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

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tracing::{Level, debug, error, info, trace, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// Bytes in a mebibyte, for the size fields.
const MIB: u64 = 1_048_576;

/// Tokens in a kibi-token, for the context-length field.
const KIBI_TOKENS: u32 = 1_024;

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
    // so the files sort together and open in whatever reads a `.log`.
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("osstat")
        .filename_suffix("log")
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

/// The app finished starting. `probe_gpus` is how many GPUs were found — the
/// count, never what they are.
pub fn app_started(probe_gpus: usize) {
    info!(gpus = probe_gpus, "app started");
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

/// One sampler tick, as two counts.
///
/// At `trace`, so it is only written under Verbose. osstat ticks every two
/// seconds all day; at `info` this line alone would be most of the log.
pub fn sampler_tick(processes: usize, sockets: usize) {
    trace!(processes, sockets, "sampled");
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
        app_started(2);
        session_started(35, 131_072);
        session_stopped(4_215);
        session_failed("spawn_failed");
        download_started(4_683_074_240);
        download_finished(4_683_074_240, 612);
        download_failed("checksum_mismatch");
        library_moved(9_663_676_416, 128);
        library_move_failed("not_enough_space");
        conversation_save_failed("io");
        app_data_unavailable();
        tray_unavailable();
        ui_event(UiEventKind::Ready);
        ui_event(UiEventKind::PageChanged);
        ui_event(UiEventKind::SettingChanged);
        ui_event(UiEventKind::CommandFailed);
        sampler_tick(1_284, 96);
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
            app_started(2);
            ui_event(UiEventKind::Ready);
            sampler_tick(1_284, 96);
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
            sampler_tick(1_284, 96);
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
        let lines = capture(LogLevel::Verbose, || sampler_tick(1_284, 96));

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
}
