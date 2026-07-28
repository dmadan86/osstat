//! macOS implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: signal-based process control, socket
//! enumeration, LaunchAgents and Login Items startup entries, unified-memory
//! reporting for Apple Silicon, and the privileged-helper elevation path
//! (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::MacOs;
pub(crate) const DISPLAY_NAME: &str = "macOS";
