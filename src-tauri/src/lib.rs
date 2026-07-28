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

/// Starts the application and blocks until the last window closes.
///
/// # Errors
///
/// Returns an error if the webview runtime, the bundled context or the event
/// loop fails to initialise — most often a missing system webview runtime.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::app_info])
        .run(tauri::generate_context!())
}
