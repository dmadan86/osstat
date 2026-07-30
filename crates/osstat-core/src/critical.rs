//! Processes that need a second, distinct confirmation before being ended.
//!
//! Data, not code, following the precedent `models.json` and `runtimes.json`
//! set: a JSON file with a sibling schema and a test holding them together.
//!
//! # What this is not
//!
//! **This guards a misclick, not an adversary.** Name matching is spoofable in
//! principle, and an attacker who can already place a binary on the machine has
//! won long before this list matters. It is said here so that nobody later
//! mistakes it for a security control and builds on it.
//!
//! Most entries also fail with [`crate::Error::PermissionDenied`], because they
//! run as SYSTEM or root. The list earns its place on the ones that do not:
//! your own session's shell or window manager, where a confirmed kill signs you
//! out.

use serde::Deserialize;
use std::sync::OnceLock;

/// The committed list, embedded at compile time.
const LIST_JSON: &str = include_str!("../critical-processes.json");

/// One process worth stopping to think about.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CriticalProcess {
    /// Which OS this entry describes: `windows`, `linux` or `macos`.
    pub platform: String,
    /// Executable name, matched exactly and case-insensitively.
    pub name: String,
    /// Paths this executable legitimately lives at, matched as suffixes.
    pub paths: Vec<String>,
    /// What ending it breaks, in words shown to the user.
    pub breaks: String,
}

/// Every listed process.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CriticalList {
    /// Schema version of the file.
    pub version: u32,
    /// The entries.
    pub processes: Vec<CriticalProcess>,
}

impl CriticalList {
    /// Finds the entry matching a process, if any.
    ///
    /// Both the name and the path must match. `exe` being `None` is never a
    /// match: an unreadable path is not evidence of anything, and treating it
    /// as one would put a second confirmation in front of ordinary processes
    /// until people learned to click through it.
    #[must_use]
    pub fn match_for(
        &self,
        platform: &str,
        name: &str,
        exe: Option<&str>,
    ) -> Option<&CriticalProcess> {
        let exe = exe?.replace('\\', "/").to_lowercase();

        self.processes.iter().find(|process| {
            process.platform == platform
                && process.name.eq_ignore_ascii_case(name)
                && process
                    .paths
                    .iter()
                    .any(|path| exe.ends_with(&path.replace('\\', "/").to_lowercase()))
        })
    }
}

/// The committed list, parsed once.
///
/// # Panics
///
/// Never in a shipped build: the file is embedded at compile time and
/// [`tests::the_list_parses_and_covers_all_three_platforms`] proves it
/// deserialises.
#[must_use]
pub fn critical_list() -> &'static CriticalList {
    static LIST: OnceLock<CriticalList> = OnceLock::new();
    LIST.get_or_init(|| match serde_json::from_str(LIST_JSON) {
        Ok(list) => list,
        Err(error) => unreachable!("embedded critical-processes.json is malformed: {error}"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const SCHEMA_JSON: &str = include_str!("../critical-processes.schema.json");

    #[test]
    fn the_list_conforms_to_its_own_json_schema() {
        let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
        let data: serde_json::Value = serde_json::from_str(LIST_JSON).unwrap();

        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&data)
            .map(|e| e.to_string())
            .collect();

        assert!(errors.is_empty(), "schema violations: {errors:?}");
    }

    #[test]
    fn the_list_parses_and_covers_all_three_platforms() {
        let list = critical_list();

        for platform in ["windows", "linux", "macos"] {
            assert!(
                list.processes.iter().any(|p| p.platform == platform),
                "{platform} has no critical processes listed"
            );
        }
    }

    #[test]
    fn the_right_name_at_the_right_path_matches() {
        let found = critical_list()
            .match_for("windows", "explorer.exe", Some("C:\\Windows\\explorer.exe"))
            .expect("the real shell must be recognised");

        assert!(found.breaks.contains("taskbar"));
    }

    #[test]
    fn the_right_name_at_the_wrong_path_does_not_match() {
        // The load-bearing test. A binary the user downloaded and named
        // explorer.exe must not inherit the real shell's protection — which
        // would make an unknown binary *harder* to end than a system process.
        assert!(
            critical_list()
                .match_for(
                    "windows",
                    "explorer.exe",
                    Some("C:\\Users\\me\\Downloads\\explorer.exe")
                )
                .is_none()
        );
    }

    #[test]
    fn a_different_name_at_a_protected_path_does_not_match() {
        assert!(
            critical_list()
                .match_for("windows", "notepad.exe", Some("C:\\Windows\\notepad.exe"))
                .is_none()
        );
    }

    #[test]
    fn matching_is_scoped_to_the_platform_asked_about() {
        assert!(
            critical_list()
                .match_for("linux", "explorer.exe", Some("/Windows/explorer.exe"))
                .is_none()
        );
        assert!(
            critical_list()
                .match_for("linux", "systemd", Some("/usr/lib/systemd/systemd"))
                .is_some()
        );
    }

    #[test]
    fn path_matching_ignores_case_and_leading_directories() {
        // Windows paths vary in case; Linux distributions vary in prefix.
        assert!(
            critical_list()
                .match_for("windows", "explorer.exe", Some("D:\\WINDOWS\\EXPLORER.EXE"))
                .is_some()
        );
    }

    #[test]
    fn a_process_with_no_readable_path_is_not_assumed_critical() {
        // ProcessRecord::exe is Option. Treating an unreadable path as a match
        // would put a second confirmation in front of ordinary processes and
        // teach people to click through it.
        assert!(
            critical_list()
                .match_for("windows", "explorer.exe", None)
                .is_none()
        );
    }

    #[test]
    fn every_entry_says_what_breaks() {
        for process in &critical_list().processes {
            assert!(
                !process.breaks.trim().is_empty(),
                "{} does not say what ending it breaks",
                process.name
            );
        }
    }
}
