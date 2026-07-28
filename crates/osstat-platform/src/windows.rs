//! Windows implementations of the osstat capability traits.
//!
//! Milestone-specific work lands here: process control via the Win32 API,
//! socket enumeration, Task Scheduler / `Run` key startup entries, and the
//! `ShellExecuteExW` `runas` elevation helper (ADR-006).

use crate::PlatformId;

pub(crate) const PLATFORM_ID: PlatformId = PlatformId::Windows;
pub(crate) const DISPLAY_NAME: &str = "Windows";

/// Names a disk the way a Windows user would.
///
/// Windows users navigate by drive letter, so the letter leads and the volume
/// label — which is frequently empty — is a parenthetical. `sysinfo` reports the
/// mount point with a trailing separator (`C:\`), which nobody says aloud.
pub(crate) fn disk_display_name(label: &str, mount_point: &str) -> String {
    let letter = mount_point.trim_end_matches(['\\', '/']);
    let letter = if letter.is_empty() {
        mount_point
    } else {
        letter
    };

    match label.trim() {
        "" => letter.to_owned(),
        label => format!("{letter} ({label})"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_drive_letter_loses_its_trailing_separator() {
        assert_eq!(disk_display_name("", "C:\\"), "C:");
    }

    #[test]
    fn a_volume_label_becomes_a_parenthetical() {
        assert_eq!(disk_display_name("Windows", "C:\\"), "C: (Windows)");
    }

    #[test]
    fn a_whitespace_label_is_treated_as_absent() {
        assert_eq!(disk_display_name("   ", "D:\\"), "D:");
    }

    #[test]
    fn a_mount_point_that_is_only_a_separator_survives() {
        assert!(!disk_display_name("", "\\").is_empty());
    }
}
