//! Build and release metadata.
//!
//! Small on purpose: this is the payload behind the first end-to-end IPC call
//! and it is what the "copy system info for a bug report" action stamps its
//! output with.

use serde::{Deserialize, Serialize};

/// Which Cargo profile the running binary was compiled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-bindings",
    ts(export, export_to = "../../../ui/src/bindings/")
)]
#[serde(rename_all = "lowercase")]
pub enum BuildProfile {
    /// Compiled with debug assertions on.
    Debug,
    /// Compiled with debug assertions off.
    Release,
}

impl BuildProfile {
    /// The profile the current binary was compiled with.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Debug
        } else {
            Self::Release
        }
    }
}

/// Identifies the running osstat build.
///
/// Field names cross the IPC boundary as `camelCase` so the TypeScript side
/// reads naturally without a mapping layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-bindings",
    ts(export, export_to = "../../../ui/src/bindings/")
)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    /// Package name of the front-end binary, e.g. `osstat`.
    pub name: String,
    /// Semantic version of the release.
    pub version: String,
    /// Profile the binary was compiled with.
    pub profile: BuildProfile,
}

impl BuildInfo {
    /// Builds metadata for the calling binary.
    ///
    /// The name and version are passed in rather than read from this crate's
    /// own `CARGO_PKG_*` variables so the values describe the *front-end*
    /// (`osstat`), not the core library.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            profile: BuildProfile::current(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn profile_matches_compilation_mode() {
        let expected = if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        };
        assert_eq!(BuildProfile::current(), expected);
    }

    #[test]
    fn serialises_with_camel_case_keys() {
        let json = serde_json::to_value(BuildInfo::new("osstat", "0.1.0")).unwrap();
        let object = json.as_object().unwrap();
        assert!(object.contains_key("name"));
        assert!(object.contains_key("version"));
        assert!(object.contains_key("profile"));
    }

    #[test]
    fn profile_serialises_lowercase() {
        let json = serde_json::to_string(&BuildProfile::Release).unwrap();
        assert_eq!(json, "\"release\"");
    }

    #[test]
    fn round_trips_through_json() {
        let original = BuildInfo::new("osstat", "1.2.3");
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: BuildInfo = serde_json::from_str(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
