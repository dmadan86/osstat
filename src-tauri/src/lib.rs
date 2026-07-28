//! The osstat desktop shell.
//!
//! This crate owns the Tauri runtime and the IPC surface. It holds no system
//! logic of its own: every capability is implemented in `osstat-core` (portable
//! parts) and `osstat-platform` (per-OS parts), and this layer only adapts them
//! to commands and events the webview can call (ADR-002).
//!
//! Keeping it this thin is what allows the same core to back a headless CLI
//! later without moving any logic.

pub mod commands;
pub mod ports;
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

/// Starts the application and blocks until the last window closes.
///
/// # Errors
///
/// Returns an error if the webview runtime, the bundled context or the event
/// loop fails to initialise — most often a missing system webview runtime.
pub fn run() -> tauri::Result<()> {
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
        .setup(|app| {
            let sampler = Sampler::start(app.handle().clone(), DEFAULT_INTERVAL)?;
            app.manage(sampler);
            app.manage(CloseSetting::default());
            app.manage(PortInspector::default());

            // A tray that could not be created is logged and moved past. An app
            // without a tray icon still works; an app that refuses to start
            // does not.
            if let Err(error) = tray::create(app.handle()) {
                eprintln!("osstat: could not create the tray icon: {error}");
            }

            // The window is configured invisible so a sign-in launch paints
            // nothing. An ordinary launch has to ask for it, which also costs
            // the old flash of unstyled window before React mounts.
            if !starts_hidden(&std::env::args().collect::<Vec<_>>())
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.show();
            }

            Ok(())
        })
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
            commands::set_sample_interval,
            commands::set_sampling_paused,
            commands::set_close_behaviour,
        ])
        .run(tauri::generate_context!())
}
