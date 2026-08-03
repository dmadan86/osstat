//! The notification-area icon.
//!
//! The icon itself is static and the tooltip carries the data. Rendering load
//! into a ~16 px image would have to be done for three platforms and has almost
//! no room to say anything legible; a tooltip has room for the two figures that
//! answer "is this machine busy?".

use osstat_core::MetricsSample;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

/// Identifies the tray icon, so the sampler thread can find it again.
pub const TRAY_ID: &str = "osstat-tray";

/// Menu item id for showing the window.
const SHOW_ID: &str = "tray-show";
/// Menu item id for the start-at-sign-in checkbox.
const AUTOSTART_ID: &str = "tray-autostart";
/// Menu item id for quitting.
const QUIT_ID: &str = "tray-quit";

/// Renders the tooltip text.
///
/// `sample` is `None` before the first tick; `total_memory` is installed
/// memory, used as the denominator.
#[must_use]
pub fn tooltip(sample: Option<&MetricsSample>, total_memory: u64) -> String {
    let Some(sample) = sample else {
        return "osstat — starting…".to_owned();
    };

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rounded percentage is 0..=100 by construction"
    )]
    let cpu = sample.cpu_total.round().clamp(0.0, 100.0) as u8;

    format!(
        "osstat — CPU {cpu}%  ·  Memory {} / {}",
        gibibytes(sample.memory_used),
        gibibytes(total_memory)
    )
}

/// Formats bytes as gibibytes to one decimal place.
fn gibibytes(bytes: u64) -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "display precision; the value is rounded to one decimal anyway"
    )]
    let value = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{value:.1} GB")
}

/// Shows, unminimises and focuses the main window.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Creates the tray icon and its menu.
///
/// # Errors
///
/// Returns an error if the menu or the icon cannot be created. Callers are
/// expected to log and carry on: an app with no tray icon still works.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, SHOW_ID, "Show osstat", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        AUTOSTART_ID,
        "Start osstat when I sign in",
        true,
        // Read from the operating system. There is no stored copy to consult,
        // deliberately: a mirror could disagree with reality the moment someone
        // removed the entry through Task Manager's Startup tab.
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit osstat", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        tauri::Error::AssetNotFound("no default window icon to use for the tray".to_owned())
    })?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(tooltip(None, 0))
        .menu(&menu)
        // Left-click shows the window: the Windows convention, and the first
        // thing anyone tries.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                crate::log::tray_action(crate::log::TrayAction::Show);
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => {
                crate::log::tray_action(crate::log::TrayAction::Show);
                show_main_window(app);
            }
            AUTOSTART_ID => {
                crate::log::tray_action(crate::log::TrayAction::Autostart);
                let manager = app.autolaunch();
                // Toggle against what the OS currently says, not against what
                // the checkbox is showing: another window, or Task Manager, may
                // have changed it since this menu was built.
                let _ = if manager.is_enabled().unwrap_or(false) {
                    manager.disable()
                } else {
                    manager.enable()
                };
            }
            QUIT_ID => {
                // Before the exit, not after: this is the last thing the log
                // will contain, and a deliberate quit is what distinguishes it
                // from a crash for whoever reads the file afterwards.
                crate::log::tray_action(crate::log::TrayAction::Quit);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Replaces the tray tooltip, ignoring a failure.
///
/// A tooltip that could not be updated is not worth interrupting a tick for.
pub fn set_tooltip<R: Runtime>(app: &AppHandle<R>, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}
