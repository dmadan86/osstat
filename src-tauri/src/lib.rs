//! The osstat desktop shell.
//!
//! This crate owns the Tauri runtime and the IPC surface. It holds no system
//! logic of its own: every capability is implemented in `osstat-core` (portable
//! parts) and `osstat-platform` (per-OS parts), and this layer only adapts them
//! to commands and events the webview can call (ADR-002).
//!
//! Keeping it this thin is what allows the same core to back a headless CLI
//! later without moving any logic.

pub mod chat;
pub mod commands;
pub mod log;
pub mod models;
pub mod ports;
pub mod runtime;
pub mod sampler;
pub mod tray;
pub mod window_state;

use std::time::Duration;

use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::ports::PortInspector;
use crate::sampler::Sampler;
use crate::window_state::CloseSetting;

/// How often the sampler ticks until the front-end says otherwise.
///
/// Two seconds is the rate ADR-007 settled on: fast enough that a chart feels
/// live, slow enough that a thousand-process diff is not the machine's main
/// workload.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);

/// The flag the sign-in entry adds, so a login launch opens no window.
const HIDDEN_FLAG: &str = "--hidden";

/// The flag that puts osstat into cold-start measurement mode for the
/// `just cold-start` / `scripts/cold-start.sh` hyperfine check (ROADMAP M5):
/// print the elapsed time from process entry to the main window becoming
/// visible, then exit immediately instead of running normally.
///
/// This is a floor on the product goal, not the whole of it. It times the
/// native window appearing, not the front-end's first paint inside it, which
/// happens asynchronously once the webview finishes navigating to the
/// bundled `index.html`. Closing that gap would mean the front-end reporting
/// back over IPC once mounted, which adds a permanent round trip to every
/// real launch just to serve a benchmark — not a trade worth making here.
/// `scripts/cold-start.sh` states this limitation alongside the numbers it
/// prints, rather than passing a partial measurement off as the full one.
const MEASURE_STARTUP_FLAG: &str = "--measure-startup";

/// Event fired the instant `CloseRequested` hides the window instead of
/// letting it close.
///
/// The tray-explanation banner has to wait on this rather than on the
/// close-behaviour preference: the preference only says a hide *would*
/// happen on the next close, not that one just did, and a banner about an
/// event that has not occurred yet is the bug this exists to avoid. This is
/// also the only way the front end can learn a hide happened at all — the
/// decision is made here, synchronously, with no round trip to the webview.
pub const TRAY_HIDDEN_EVENT: &str = "tray:hidden";

/// Whether this process was started by the sign-in entry.
///
/// `args` is the process arguments, including the program name.
#[must_use]
pub fn starts_hidden(args: &[String]) -> bool {
    args.iter().any(|arg| arg == HIDDEN_FLAG)
}

/// Whether this process was asked to measure and report its own cold-start
/// time instead of running normally. See [`MEASURE_STARTUP_FLAG`].
#[must_use]
pub fn wants_startup_measurement(args: &[String]) -> bool {
    args.iter().any(|arg| arg == MEASURE_STARTUP_FLAG)
}

/// Everything that has to happen once the Tauri runtime exists.
///
/// A named function rather than the closure it used to be: this is where new
/// startup work lands, and it had already grown to the length at which one
/// function stops reading as one thing.
///
/// # Errors
///
/// Returns an error only if the sampler cannot be started. Everything else
/// here degrades: no app data directory means chat and downloads are
/// unavailable, and no tray icon means no tray icon. An app that refuses to
/// start is worse than an app missing a feature.
fn setup(
    app: &mut tauri::App,
    process_start: std::time::Instant,
    measuring_startup: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Before anything else here: a failure further down is exactly what a log
    // is for, and there is no point starting one after the event worth
    // recording has already happened. A log directory that cannot be made
    // leaves the app running without a log rather than not running.
    if let Ok(root) = app.path().app_data_dir() {
        let directory = root.join("logs");
        let level = log::LogLevel::default();

        if let Some(guard) = log::init(&directory, level) {
            app.manage(log::LogGuard::new(guard));
        }

        // After `init`, not before. The appender opens today's file when it is
        // built, so pruning afterwards is what lets retention see that file and
        // keep it; pruning first would leave yesterday's as the newest and
        // count one file too few.
        log::prune(&directory, log::RETENTION);
        app.manage(log::LogState::new(directory, level));
    }

    let sampler = Sampler::start(app.handle().clone(), DEFAULT_INTERVAL)?;
    app.manage(sampler);
    app.manage(CloseSetting::default());
    app.manage(PortInspector::default());

    // Conversations, the session record and the index of downloaded models all
    // live under app data. A directory that cannot be resolved leaves chat and
    // downloads unavailable rather than stopping the app from starting: osstat
    // is a system monitor first, and the tray and the sampler need neither.
    match app.path().app_data_dir() {
        Ok(root) => {
            app.manage(models::ModelState::new(root.clone()));
            app.manage(chat::ChatState::new(root));
            // Before anything else can start a server: an osstat that crashed
            // mid-session left one holding VRAM, and this is the only run that
            // will ever be in a position to end it.
            chat::reap_orphan(app.handle());
        }
        // The error is dropped rather than logged: it comes from Tauri's path
        // resolver and its message names the directory it could not resolve.
        Err(_) => log::app_data_unavailable(),
    }

    // A tray that could not be created is logged and moved past. An app
    // without a tray icon still works; an app that refuses to start does not.
    if tray::create(app.handle()).is_err() {
        log::tray_unavailable();
    }

    // Last, so it means what it says. Everything above can degrade, and a line
    // claiming the app had started before the sampler was running would be the
    // one line in the file that could not be trusted.
    log::app_started();

    // The window is configured invisible so a sign-in launch paints nothing.
    // An ordinary launch has to ask for it, which also costs the old flash of
    // unstyled window before React mounts.
    if !starts_hidden(&std::env::args().collect::<Vec<_>>())
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.show();

        // The measurement run's whole job is this one line, then a clean exit:
        // hyperfine times the process's wall-clock life, so cold-start mode
        // makes that lifetime *be* the number.
        if measuring_startup {
            println!(
                "osstat_cold_start_ms={}",
                process_start.elapsed().as_millis()
            );
            std::process::exit(0);
        }
    }

    Ok(())
}

/// Starts the application and blocks until the last window closes.
///
/// # Errors
///
/// Returns an error if the webview runtime, the bundled context or the event
/// loop fails to initialise — most often a missing system webview runtime.
pub fn run() -> tauri::Result<()> {
    // Taken as early as this crate's own code can observe: it excludes the
    // OS loader and dynamic-linking time before `main` runs, which nothing
    // inside the process can measure. `scripts/cold-start.sh` states this.
    let process_start = std::time::Instant::now();
    let measuring_startup = wants_startup_measurement(&std::env::args().collect::<Vec<_>>());

    tauri::Builder::default()
        // Registered first, as this plugin requires. Once osstat starts at
        // sign-in, clicking the desktop shortcut would otherwise launch a rival
        // copy: two tray icons, two samplers, two windows with one title.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_FLAG]),
        ))
        .setup(move |app| setup(app, process_start, measuring_startup))
        .on_window_event(|window, event| {
            // Both signals matter now: minimising and hiding to the tray are
            // different events, and both mean nobody can see the window. See
            // the module docs on `sampler` for why focus is still the wrong
            // signal to use.
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_))
                && let Some(sampler) = window.try_state::<Sampler>()
            {
                sampler.set_window_state(
                    window.is_visible().unwrap_or(true),
                    window.is_minimized().unwrap_or(false),
                );
            }

            if let WindowEvent::CloseRequested { api, .. } = event
                && window
                    .try_state::<CloseSetting>()
                    .is_some_and(|setting| setting.get().hides())
            {
                api.prevent_close();
                let _ = window.hide();
                if let Some(sampler) = window.try_state::<Sampler>() {
                    sampler.set_window_state(false, false);
                }
                let _ = window.emit(TRAY_HIDDEN_EVENT, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::system_description,
            commands::metrics_history,
            commands::process_list,
            commands::port_list,
            commands::gpu_devices,
            commands::model_registry,
            commands::llm_advice,
            runtime::runtime_status,
            runtime::acquire_runtime,
            runtime::delete_runtime,
            chat::chat_open_model,
            chat::chat_send,
            chat::chat_stop,
            chat::chat_close,
            chat::chat_list,
            chat::chat_load,
            chat::chat_delete,
            models::models_catalogue,
            models::models_download,
            models::models_search,
            models::models_download_searched,
            models::models_pause,
            models::models_cancel,
            models::models_delete,
            models::models_folder,
            models::models_set_folder,
            models::models_plan_move,
            models::models_move,
            log::log_level,
            log::log_set_level,
            log::log_save,
            log::log_directory,
            log::ui_log,
            commands::set_sample_interval,
            commands::set_sampling_paused,
            commands::set_close_behaviour,
            commands::terminate_process,
            commands::critical_processes,
        ])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::{starts_hidden, wants_startup_measurement};

    #[test]
    fn recognises_the_hidden_flag_anywhere_in_argv() {
        let args = vec!["osstat".to_owned(), "--hidden".to_owned()];
        assert!(starts_hidden(&args));
    }

    #[test]
    fn plain_launch_does_not_start_hidden() {
        let args = vec!["osstat".to_owned()];
        assert!(!starts_hidden(&args));
    }

    #[test]
    fn recognises_the_measure_startup_flag() {
        let args = vec!["osstat".to_owned(), "--measure-startup".to_owned()];
        assert!(wants_startup_measurement(&args));
    }

    #[test]
    fn plain_launch_does_not_measure_startup() {
        let args = vec!["osstat".to_owned()];
        assert!(!wants_startup_measurement(&args));
    }
}
