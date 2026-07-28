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
pub mod sampler;

use std::time::Duration;

use tauri::{Manager, WindowEvent};

use crate::sampler::Sampler;

/// How often the sampler ticks until the front-end says otherwise.
///
/// Two seconds is the rate ADR-007 settled on: fast enough that a chart feels
/// live, slow enough that a thousand-process diff is not the machine's main
/// workload.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);

/// Starts the application and blocks until the last window closes.
///
/// # Errors
///
/// Returns an error if the webview runtime, the bundled context or the event
/// loop fails to initialise — most often a missing system webview runtime.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .setup(|app| {
            let sampler = Sampler::start(app.handle().clone(), DEFAULT_INTERVAL)?;
            app.manage(sampler);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Stop measuring while the window is minimised, and only then. See
            // the module docs on `sampler` for why focus is the wrong signal.
            if matches!(event, WindowEvent::Resized(_))
                && let Some(sampler) = window.try_state::<Sampler>()
            {
                sampler.set_paused(window.is_minimized().unwrap_or(false));
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::system_description,
            commands::metrics_history,
            commands::process_tree,
            commands::gpu_devices,
            commands::set_sample_interval,
            commands::set_sampling_paused,
        ])
        .run(tauri::generate_context!())
}
