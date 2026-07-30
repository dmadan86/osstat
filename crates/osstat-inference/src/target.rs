//! What machine a runtime build is for.
//!
//! Separated from selection so the target can be faked in tests: the selection
//! table must be provable for all three platforms from any one of them, and CI
//! never runs all three in the same process.

/// The operating system a runtime build targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    /// Microsoft Windows.
    Windows,
    /// Linux, against the Ubuntu-built artifacts.
    Linux,
    /// Apple macOS.
    MacOs,
}

/// The CPU architecture a runtime build targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArch {
    /// 64-bit x86.
    X64,
    /// 64-bit ARM.
    Arm64,
}

/// A machine a runtime can be selected for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Its operating system.
    pub os: TargetOs,
    /// Its CPU architecture.
    pub arch: TargetArch,
}

impl Target {
    /// The machine this build of osstat is running on.
    ///
    /// `None` on a target osstat compiles for but upstream publishes no runtime
    /// for. Saying so is better than picking a near-enough architecture and
    /// downloading something that cannot run.
    #[must_use]
    pub const fn host() -> Option<Self> {
        let os = if cfg!(target_os = "windows") {
            TargetOs::Windows
        } else if cfg!(target_os = "linux") {
            TargetOs::Linux
        } else if cfg!(target_os = "macos") {
            TargetOs::MacOs
        } else {
            return None;
        };

        let arch = if cfg!(target_arch = "x86_64") {
            TargetArch::X64
        } else if cfg!(target_arch = "aarch64") {
            TargetArch::Arm64
        } else {
            return None;
        };

        Some(Self { os, arch })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_host_resolves_on_every_platform_ci_runs() {
        // CI covers ubuntu-latest, windows-latest and macos-latest, so this
        // asserting anything at all means all three are representable.
        assert!(
            Target::host().is_some(),
            "osstat is being built for a target with no published runtime"
        );
    }

    #[test]
    fn the_host_matches_what_the_compiler_was_told() {
        let host = Target::host().expect("a supported target");

        #[cfg(target_os = "windows")]
        assert_eq!(host.os, TargetOs::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(host.os, TargetOs::Linux);
        #[cfg(target_os = "macos")]
        assert_eq!(host.os, TargetOs::MacOs);

        #[cfg(target_arch = "x86_64")]
        assert_eq!(host.arch, TargetArch::X64);
        #[cfg(target_arch = "aarch64")]
        assert_eq!(host.arch, TargetArch::Arm64);
    }
}
