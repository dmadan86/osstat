//! Build script for the osstat desktop shell.
//!
//! `tauri_build::build()` reads `tauri.conf.json`, generates the capability
//! schemas under `gen/`, and embeds the Windows resources and app manifest.

fn main() {
    tauri_build::build();
}
