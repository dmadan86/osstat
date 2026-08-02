//! Running a local llama.cpp server, and talking to it.
//!
//! `osstat-inference` acquires the runtime; this crate runs it. The split is
//! deliberate: acquisition is nearly pure code with hermetic tests, while a
//! session owns a child process and a long-lived HTTP stream.
//!
//! Nothing here reaches the webview directly. `src-tauri/src/chat.rs` adapts
//! this crate's types to Tauri commands and events, so that all egress stays in
//! Rust and ADR-012's "the webview never issues an HTTP request" holds.

#![forbid(unsafe_code)]

pub mod error;
pub mod gguf;

pub use error::ChatError;
pub use gguf::{ModelFile, parse};
