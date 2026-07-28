//! Desktop entry point.
//!
//! Deliberately thin: everything worth testing lives in `osstat_lib`.

// Release builds must not spawn a console window alongside the GUI on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(error) = osstat_lib::run() {
        eprintln!("osstat failed to start: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
