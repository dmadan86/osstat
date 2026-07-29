//! Hardware probing and, later, the LLM runnability advisor.
//!
//! Today this crate holds only the first half of ADR-008: finding the GPUs in a
//! machine and measuring them where the vendor allows. The model registry and
//! the runnability arithmetic arrive with M4 and will sit alongside it.
//!
//! The probe exists early because the Overview page shows GPUs, and the honest
//! way to show a GPU is to actually look for one.
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

pub mod probe;
pub mod registry;

pub use probe::HardwareProbe;
