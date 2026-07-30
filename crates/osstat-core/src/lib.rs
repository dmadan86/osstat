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
//! - [`provider`] — the capability traits `osstat-platform` implements.
//! - [`metrics`] — system description, per-tick samples and the history ring.
//! - [`process`] — process records, their identity, and tick diffing.
//! - [`gpu`] — GPU description and measurement, including how far to trust it.
//! - [`socket`] — socket records and their join to the process table.

#![forbid(unsafe_code)]

pub mod control;
pub mod critical;
pub mod error;
pub mod gpu;
pub mod meta;
pub mod metrics;
pub mod process;
pub mod provider;
pub mod socket;

pub use control::{ProcessController, Termination, TerminationMode};
pub use critical::{CriticalList, CriticalProcess, critical_list};
pub use error::{Error, Result};
pub use gpu::{GpuDevice, GpuKind, GpuSample, GpuSource};
pub use meta::{BuildInfo, BuildProfile};
pub use metrics::{
    CpuDescription, DiskDescription, DiskKind, DiskSample, HISTORY_CAPACITY, InterfaceDescription,
    InterfaceSample, MetricsHistory, MetricsSample, SystemDescription,
};
pub use process::{ProcessDiff, ProcessKey, ProcessRecord, ProcessStatus, diff_processes};
pub use provider::{GpuProvider, ProcessProvider, SocketProvider, SystemInfoProvider};
pub use socket::{
    PortRecord, SocketProtocol, SocketRecord, SocketState, join_sockets_to_processes,
};
