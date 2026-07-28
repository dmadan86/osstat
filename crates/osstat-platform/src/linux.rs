//! Linux implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: `/proc` and signal-based process
//! control, socket enumeration, systemd user units and `~/.config/autostart`
//! startup entries, and the Polkit (`pkexec`) elevation helper (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::Linux;
pub(crate) const DISPLAY_NAME: &str = "Linux";
