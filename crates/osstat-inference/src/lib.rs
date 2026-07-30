//! Acquiring the llama.cpp server runtime.
//!
//! osstat's advisor (ADR-008) tells you whether a model fits. This crate is the
//! first half of letting you act on that: it selects a llama.cpp build for the
//! machine, fetches it, and refuses to execute anything whose SHA256 does not
//! match a hash pinned in this repository.
//!
//! # Why the hash is pinned here rather than fetched
//!
//! Upstream publishes a per-asset digest through the GitHub API, and using it
//! would keep osstat permanently current. It would also be close to worthless:
//! the digest and the binary arrive from the same origin, so it proves the
//! transfer was not corrupted, not that the file is the one anybody reviewed.
//! A hash compiled into osstat means compromising a release cannot change what
//! osstat will execute, and changing it is a pull request whose diff a person
//! can read.
//!
//! The cost is that the pin ages — upstream ships builds close to daily. That
//! is the intended trade for a utility whose stated values are predictability
//! and auditability, and it is what makes "works offline once acquired"
//! something that can be tested rather than hoped for.
//!
//! # Shape
//!
//! [`select`] is pure, so the whole three-platform table is provable from any
//! one platform — including CI runners, which have no GPU and therefore
//! exercise the fallback continuously, the same property ADR-008 relies on for
//! the probe itself.

#![forbid(unsafe_code)]

pub mod download;
pub mod error;
pub mod manifest;
pub mod select;
pub mod target;

pub use download::{Progress, download_verified, sha256_file};
pub use error::AcquireError;
pub use manifest::{Companion, RuntimeArtifact, RuntimeManifest, pinned_manifest};
pub use select::{Backend, CudaChoice, GpuCapability, cuda_is_offered, select};
pub use target::{Target, TargetArch, TargetOs};
