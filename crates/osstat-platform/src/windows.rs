//! Windows implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: process control via the Win32 API,
//! socket enumeration, Task Scheduler / `Run` key startup entries, and the
//! `ShellExecuteExW` `runas` elevation helper (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::Windows;
pub(crate) const DISPLAY_NAME: &str = "Windows";
