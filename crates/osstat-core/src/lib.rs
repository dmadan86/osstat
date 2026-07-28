//! Platform-agnostic core of osstat.
//!
//! This crate holds the domain types, capability traits and engines that are
//! shared by every front-end (the Tauri desktop shell today, a headless CLI
//! later). It performs no I/O against OS-specific APIs itself — that is the job
//! of `osstat-platform`, which implements the traits declared here (ADR-003).
//!
//! Because the crate is free of platform code it compiles and tests on any CI
//! runner, which is what makes the cleaning-rule engine and the LLM
//! runnability maths cheap to test exhaustively.
//!
//! # Module layout
//!
//! - [`error`] — the shared error type every capability returns.
//! - [`meta`] — build and release metadata surfaced in the UI and bug reports.

#![forbid(unsafe_code)]

pub mod error;
pub mod meta;

pub use error::{Error, Result};
pub use meta::{BuildInfo, BuildProfile};
