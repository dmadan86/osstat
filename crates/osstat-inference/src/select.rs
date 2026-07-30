//! Choosing which published llama.cpp build suits a machine.
//!
//! A pure function of the target and what the ADR-008 probe found, so the whole
//! table is provable from any one platform. Every arm below names a real asset
//! in the pinned upstream release; nothing here may invent one.
//!
//! Three properties of upstream's matrix drive the shape, and each contradicts
//! an assumption that would otherwise be natural: there is **no Linux CUDA
//! build**, Windows CUDA needs a **second archive** for the CUDA runtime, and
//! **Windows on ARM has no Vulkan build**.

use crate::target::{Target, TargetArch, TargetOs};

/// Which GPU acceleration a runtime build was compiled for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Apple Metal, built into both macOS artifacts.
    Metal,
    /// NVIDIA CUDA against a 12.x driver.
    Cuda124,
    /// NVIDIA CUDA against a 13.x driver.
    Cuda133,
    /// Vulkan, which every desktop vendor supports.
    Vulkan,
    /// No GPU acceleration.
    Cpu,
}

impl Backend {
    /// A name for this backend fit to show a user.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Metal => "Metal",
            Self::Cuda124 => "CUDA 12.4",
            Self::Cuda133 => "CUDA 13.3",
            Self::Vulkan => "Vulkan",
            Self::Cpu => "CPU",
        }
    }

    /// The `runtimes.json` artifact identifier for this backend on this target.
    ///
    /// `None` for a combination upstream does not publish.
    #[must_use]
    pub const fn artifact_id(self, target: Target) -> Option<&'static str> {
        match (target.os, target.arch, self) {
            (TargetOs::MacOs, TargetArch::Arm64, Self::Metal) => Some("macos-arm64"),
            (TargetOs::MacOs, TargetArch::X64, Self::Metal) => Some("macos-x64"),
            (TargetOs::Windows, TargetArch::X64, Self::Cuda133) => Some("win-cuda-13.3-x64"),
            (TargetOs::Windows, TargetArch::X64, Self::Cuda124) => Some("win-cuda-12.4-x64"),
            (TargetOs::Windows, TargetArch::X64, Self::Vulkan) => Some("win-vulkan-x64"),
            (TargetOs::Windows, TargetArch::X64, Self::Cpu) => Some("win-cpu-x64"),
            (TargetOs::Windows, TargetArch::Arm64, Self::Cpu) => Some("win-cpu-arm64"),
            (TargetOs::Linux, TargetArch::X64, Self::Vulkan) => Some("ubuntu-vulkan-x64"),
            (TargetOs::Linux, TargetArch::X64, Self::Cpu) => Some("ubuntu-x64"),
            (TargetOs::Linux, TargetArch::Arm64, Self::Vulkan) => Some("ubuntu-vulkan-arm64"),
            (TargetOs::Linux, TargetArch::Arm64, Self::Cpu) => Some("ubuntu-arm64"),
            _ => None,
        }
    }
}

/// What selection needs to know about the machine's graphics hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuCapability {
    /// Whether the probe found any real GPU at all.
    pub present: bool,
    /// The major CUDA version the NVIDIA driver reports, if there is one.
    pub nvidia_cuda_major: Option<u32>,
}

/// Whether the user has accepted CUDA's much larger download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CudaChoice {
    /// Take the CUDA build and its separate runtime archive.
    Accepted,
    /// Take Vulkan instead.
    Declined,
}

/// Whether a CUDA build exists for this machine and is worth offering.
///
/// Windows x64 only. Upstream publishes no Linux CUDA artifact, so an NVIDIA
/// card on Linux is a Vulkan card as far as this runtime is concerned.
#[must_use]
pub const fn cuda_is_offered(target: Target, gpu: &GpuCapability) -> bool {
    matches!(target.os, TargetOs::Windows)
        && matches!(target.arch, TargetArch::X64)
        && gpu.nvidia_cuda_major.is_some()
}

/// Picks the backend for a machine.
///
/// CUDA is never selected unless `cuda` is [`CudaChoice::Accepted`]. The CUDA
/// artifacts are 538–642 MB against Vulkan's 34 MB, so starting that download
/// because a card was detected would be making the user's decision for them.
///
/// `None` only for a target with no published artifact at all.
#[must_use]
pub const fn select(target: Target, gpu: &GpuCapability, cuda: CudaChoice) -> Option<Backend> {
    // macOS ships one artifact per architecture, Metal included. Nothing to pick.
    if matches!(target.os, TargetOs::MacOs) {
        return Some(Backend::Metal);
    }

    // Windows on ARM has no Vulkan build upstream, only CPU and an
    // Adreno-specific OpenCL one this design does not select.
    if matches!(target.os, TargetOs::Windows) && matches!(target.arch, TargetArch::Arm64) {
        return Some(Backend::Cpu);
    }

    if matches!(cuda, CudaChoice::Accepted) && cuda_is_offered(target, gpu) {
        return match gpu.nvidia_cuda_major {
            Some(13) => Some(Backend::Cuda133),
            Some(12) => Some(Backend::Cuda124),
            // A driver older or newer than the two published builds. Vulkan
            // works on it; a mismatched CUDA runtime would not.
            _ => Some(Backend::Vulkan),
        };
    }

    if gpu.present {
        Some(Backend::Vulkan)
    } else {
        Some(Backend::Cpu)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const NO_GPU: GpuCapability = GpuCapability {
        present: false,
        nvidia_cuda_major: None,
    };
    const AMD: GpuCapability = GpuCapability {
        present: true,
        nvidia_cuda_major: None,
    };
    const NVIDIA_13: GpuCapability = GpuCapability {
        present: true,
        nvidia_cuda_major: Some(13),
    };
    const NVIDIA_12: GpuCapability = GpuCapability {
        present: true,
        nvidia_cuda_major: Some(12),
    };

    const fn target(os: TargetOs, arch: TargetArch) -> Target {
        Target { os, arch }
    }

    #[test]
    fn macos_always_gets_metal_whatever_the_gpu() {
        for arch in [TargetArch::Arm64, TargetArch::X64] {
            for gpu in [&NO_GPU, &AMD, &NVIDIA_13] {
                assert_eq!(
                    select(target(TargetOs::MacOs, arch), gpu, CudaChoice::Declined),
                    Some(Backend::Metal),
                    "Metal is built into both macOS builds; there is nothing to choose"
                );
            }
        }
    }

    #[test]
    fn vulkan_is_the_default_on_windows_even_with_an_nvidia_card() {
        // CUDA is 538-642 MB against Vulkan's 34 MB. A download that large is
        // an offer, never an automatic consequence of owning the right card.
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::X64),
                &NVIDIA_13,
                CudaChoice::Declined
            ),
            Some(Backend::Vulkan)
        );
    }

    #[test]
    fn accepting_cuda_picks_the_build_matching_the_driver() {
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::X64),
                &NVIDIA_13,
                CudaChoice::Accepted
            ),
            Some(Backend::Cuda133)
        );
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::X64),
                &NVIDIA_12,
                CudaChoice::Accepted
            ),
            Some(Backend::Cuda124)
        );
    }

    #[test]
    fn accepting_cuda_without_an_nvidia_driver_still_yields_vulkan() {
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::X64),
                &AMD,
                CudaChoice::Accepted
            ),
            Some(Backend::Vulkan),
            "consent to a CUDA download cannot conjure a CUDA-capable driver"
        );
    }

    #[test]
    fn linux_never_selects_cuda_because_upstream_does_not_build_it() {
        // The finding that most contradicts intuition: there is no Linux CUDA
        // asset in the release at all.
        assert_eq!(
            select(
                target(TargetOs::Linux, TargetArch::X64),
                &NVIDIA_13,
                CudaChoice::Accepted
            ),
            Some(Backend::Vulkan)
        );
        assert!(!cuda_is_offered(
            target(TargetOs::Linux, TargetArch::X64),
            &NVIDIA_13
        ));
    }

    #[test]
    fn windows_on_arm_is_cpu_only_because_no_vulkan_build_exists() {
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::Arm64),
                &AMD,
                CudaChoice::Declined
            ),
            Some(Backend::Cpu),
            "upstream ships only win-cpu-arm64 and an Adreno OpenCL build"
        );
    }

    #[test]
    fn a_machine_with_no_gpu_falls_to_cpu_on_windows_and_linux() {
        assert_eq!(
            select(
                target(TargetOs::Windows, TargetArch::X64),
                &NO_GPU,
                CudaChoice::Declined
            ),
            Some(Backend::Cpu)
        );
        assert_eq!(
            select(
                target(TargetOs::Linux, TargetArch::X64),
                &NO_GPU,
                CudaChoice::Declined
            ),
            Some(Backend::Cpu)
        );
    }

    #[test]
    fn linux_arm64_with_a_gpu_gets_vulkan() {
        assert_eq!(
            select(
                target(TargetOs::Linux, TargetArch::Arm64),
                &AMD,
                CudaChoice::Declined
            ),
            Some(Backend::Vulkan)
        );
    }

    #[test]
    fn cuda_is_offered_only_on_windows_x64_with_an_nvidia_driver() {
        assert!(cuda_is_offered(
            target(TargetOs::Windows, TargetArch::X64),
            &NVIDIA_13
        ));
        assert!(!cuda_is_offered(
            target(TargetOs::Windows, TargetArch::X64),
            &AMD
        ));
        assert!(!cuda_is_offered(
            target(TargetOs::Windows, TargetArch::Arm64),
            &NVIDIA_13
        ));
        assert!(!cuda_is_offered(
            target(TargetOs::MacOs, TargetArch::Arm64),
            &NVIDIA_13
        ));
    }

    #[test]
    fn every_combination_resolves_to_some_backend() {
        // The three-platform requirement, made executable. No combination may
        // leave a user with nothing at all.
        for os in [TargetOs::Windows, TargetOs::Linux, TargetOs::MacOs] {
            for arch in [TargetArch::X64, TargetArch::Arm64] {
                for gpu in [&NO_GPU, &AMD, &NVIDIA_12, &NVIDIA_13] {
                    for choice in [CudaChoice::Declined, CudaChoice::Accepted] {
                        assert!(
                            select(target(os, arch), gpu, choice).is_some(),
                            "no backend for {os:?}/{arch:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_selected_backend_names_a_real_artifact() {
        for os in [TargetOs::Windows, TargetOs::Linux, TargetOs::MacOs] {
            for arch in [TargetArch::X64, TargetArch::Arm64] {
                for gpu in [&NO_GPU, &AMD, &NVIDIA_12, &NVIDIA_13] {
                    for choice in [CudaChoice::Declined, CudaChoice::Accepted] {
                        let target = target(os, arch);
                        let backend = select(target, gpu, choice);
                        assert!(backend.is_some(), "no backend for {os:?}/{arch:?}");

                        let backend = backend.unwrap();
                        assert!(
                            backend.artifact_id(target).is_some(),
                            "{os:?}/{arch:?} selected {backend:?}, which names no artifact"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_host_this_test_runs_on_resolves() {
        let host = Target::host().expect("osstat must build for a target it can select for");
        assert!(select(host, &NO_GPU, CudaChoice::Declined).is_some());
    }
}
