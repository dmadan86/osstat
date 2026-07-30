//! Hardware probing and the LLM runnability advisor.
//!
//! This crate is all of ADR-008: [`probe`] finds the GPUs in a machine and
//! measures them where the vendor allows, [`registry`] holds the models as
//! data rather than code, and [`calculator`] weighs the two against each
//! other as pure functions with no OS access of their own.
//!
//! The probe came first because the Overview page shows GPUs, and the honest
//! way to show a GPU is to actually look for one. That honesty is the whole
//! design constraint on the rest: the calculator reasons over three VRAM
//! states rather than two, so "a GPU whose memory nothing can read" stays
//! distinguishable from "no GPU" all the way to the verdict.
//!
//! # What each source can and cannot tell you
//!
//! | Source | Name | VRAM total | VRAM used | Utilisation | Temperature |
//! |---|---|---|---|---|---|
//! | NVML | yes | measured | yes | yes | yes |
//! | `wgpu` | yes | **no** | no | no | no |
//! | Unified memory | yes | shared system RAM | no | no | no |
//!
//! The `wgpu` row is the one that surprises people, and it is worth being blunt
//! about: **`wgpu` has no API that reports video memory.** ADR-008 anticipated
//! "name plus memory heuristics" from it, but adapter enumeration exposes
//! device limits, not physical memory. So a non-NVIDIA discrete card is
//! reported by name with its VRAM absent, rather than with a number invented
//! from a limit that does not mean what it looks like it means.
//!
//! That is the whole reason [`osstat_core::GpuSource`] travels with every
//! device: ADR-008 names presenting a heuristic as a measurement the most
//! damaging thing this feature could do, and a missing number the user can see
//! is missing beats a confident wrong one.

#![forbid(unsafe_code)]

pub mod calculator;
pub mod probe;
pub mod registry;

pub use probe::HardwareProbe;
