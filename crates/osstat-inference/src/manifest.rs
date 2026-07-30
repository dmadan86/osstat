//! The pinned set of llama.cpp artifacts osstat is willing to run.
//!
//! Data, not code, and reviewed as data: bumping the runtime is a pull request
//! whose diff is a set of hashes, which a person can read and revert. The
//! alternative — reading digests from the GitHub API at download time — checks
//! that the transfer was not corrupted and nothing more, because the digest and
//! the binary arrive from the same origin.
//!
//! The manifest and [`crate::select`] are separate files that must agree. Two
//! tests here guard that seam in both directions: every backend selection can
//! produce resolves to a pinned artifact, and every pinned artifact is
//! reachable from some target. Drift in either direction is a user who cannot
//! install anything, or dead weight nobody reviews.

use serde::Deserialize;
use std::sync::OnceLock;

use crate::select::Backend;
use crate::target::Target;

/// The committed manifest, embedded at compile time.
///
/// Embedded rather than read from disk so a corrupted or edited install cannot
/// widen what osstat is willing to execute.
const MANIFEST_JSON: &str = include_str!("../runtimes.json");

/// A second archive an artifact cannot run without.
///
/// Only Windows CUDA has one: upstream ships the CUDA runtime DLLs separately
/// from the llama.cpp build, and the server will not start without them. It is
/// the only artifact in the manifest that is useless on its own.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Companion {
    /// Filename within the upstream release.
    pub file: String,
    /// Lower-case hex SHA256 of the archive.
    pub sha256: String,
    /// Size in bytes, used for the disk-space check before downloading.
    pub size_bytes: u64,
}

/// One published build osstat is willing to fetch and run.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeArtifact {
    /// Stable identifier, matching [`Backend::artifact_id`].
    pub id: String,
    /// Operating system this build targets.
    pub os: String,
    /// CPU architecture this build targets.
    pub arch: String,
    /// GPU backend this build was compiled for.
    pub backend: String,
    /// Filename within the upstream release.
    pub file: String,
    /// Lower-case hex SHA256 of the archive.
    pub sha256: String,
    /// Size in bytes, used for the disk-space check before downloading.
    pub size_bytes: u64,
    /// A second archive this build needs, if any.
    #[serde(default)]
    pub companion: Option<Companion>,
}

impl RuntimeArtifact {
    /// Every byte that must be downloaded for this artifact to be usable.
    ///
    /// Includes the companion, which is why Windows CUDA costs 538–642 MB
    /// rather than the 147–251 MB the main archive suggests.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.size_bytes
            .saturating_add(self.companion.as_ref().map_or(0, |c| c.size_bytes))
    }

    /// Where this artifact is fetched from.
    #[must_use]
    pub fn url(&self, manifest: &RuntimeManifest) -> String {
        release_url(manifest, &self.file)
    }

    /// Where this artifact's companion is fetched from, if it has one.
    #[must_use]
    pub fn companion_url(&self, manifest: &RuntimeManifest) -> Option<String> {
        self.companion
            .as_ref()
            .map(|companion| release_url(manifest, &companion.file))
    }
}

/// Builds a download URL for a file in the pinned release.
fn release_url(manifest: &RuntimeManifest, file: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/{}",
        manifest.upstream_repo, manifest.upstream_tag, file
    )
}

/// Every artifact osstat pins, and the release they come from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    /// Schema version of this file.
    pub version: u32,
    /// The upstream repository, as `owner/name`.
    pub upstream_repo: String,
    /// The upstream release tag every artifact comes from.
    pub upstream_tag: String,
    /// The artifacts themselves.
    pub artifacts: Vec<RuntimeArtifact>,
}

impl RuntimeManifest {
    /// Finds an artifact by identifier.
    #[must_use]
    pub fn artifact(&self, id: &str) -> Option<&RuntimeArtifact> {
        self.artifacts.iter().find(|artifact| artifact.id == id)
    }

    /// The artifact a machine should install for a chosen backend.
    #[must_use]
    pub fn artifact_for(&self, target: Target, backend: Backend) -> Option<&RuntimeArtifact> {
        self.artifact(backend.artifact_id(target)?)
    }
}

/// The pinned manifest, parsed once.
///
/// # Panics
///
/// Never in a shipped build. The manifest is embedded at compile time and
/// [`tests::the_pinned_manifest_parses`] proves it deserialises, so reaching
/// the panic means the binary was built from a tree whose `runtimes.json` was
/// already broken — which CI cannot produce.
#[must_use]
pub fn pinned_manifest() -> &'static RuntimeManifest {
    static MANIFEST: OnceLock<RuntimeManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| match serde_json::from_str(MANIFEST_JSON) {
        Ok(manifest) => manifest,
        Err(error) => unreachable!("embedded runtimes.json is malformed: {error}"),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::select::{CudaChoice, GpuCapability, select};
    use crate::target::{TargetArch, TargetOs};

    const SCHEMA_JSON: &str = include_str!("../runtimes.schema.json");

    /// Every target a published artifact could exist for.
    const TARGETS: [(TargetOs, TargetArch); 6] = [
        (TargetOs::MacOs, TargetArch::Arm64),
        (TargetOs::MacOs, TargetArch::X64),
        (TargetOs::Windows, TargetArch::X64),
        (TargetOs::Windows, TargetArch::Arm64),
        (TargetOs::Linux, TargetArch::X64),
        (TargetOs::Linux, TargetArch::Arm64),
    ];

    /// Every shape of GPU the probe can report.
    const CAPABILITIES: [GpuCapability; 4] = [
        GpuCapability {
            present: false,
            nvidia_cuda_major: None,
        },
        GpuCapability {
            present: true,
            nvidia_cuda_major: None,
        },
        GpuCapability {
            present: true,
            nvidia_cuda_major: Some(12),
        },
        GpuCapability {
            present: true,
            nvidia_cuda_major: Some(13),
        },
    ];

    #[test]
    fn the_pinned_manifest_conforms_to_its_own_json_schema() {
        // The schema and the data are two files that can drift apart silently,
        // the same reason osstat-llm validates its model registry in a test.
        let schema: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
        let data: serde_json::Value = serde_json::from_str(MANIFEST_JSON).unwrap();

        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&data)
            .map(|e| e.to_string())
            .collect();

        assert!(errors.is_empty(), "schema violations: {errors:?}");
    }

    #[test]
    fn the_pinned_manifest_parses() {
        let manifest = pinned_manifest();

        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.upstream_repo, "ggml-org/llama.cpp");
        assert_eq!(manifest.upstream_tag, "b10194");
        assert_eq!(manifest.artifacts.len(), 11);
    }

    #[test]
    fn every_artifact_id_is_unique() {
        let mut ids: Vec<&str> = pinned_manifest()
            .artifacts
            .iter()
            .map(|artifact| artifact.id.as_str())
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();

        assert_eq!(
            ids.len(),
            total,
            "a duplicated artifact id makes lookup ambiguous"
        );
    }

    #[test]
    fn every_selectable_backend_has_an_artifact_in_the_manifest() {
        // The load-bearing test against drift. Selection and the manifest are
        // separate files, and a backend resolving to a missing entry is a user
        // who cannot install anything at all.
        for (os, arch) in TARGETS {
            let target = Target { os, arch };
            for gpu in &CAPABILITIES {
                for choice in [CudaChoice::Declined, CudaChoice::Accepted] {
                    let backend = select(target, gpu, choice);
                    assert!(backend.is_some(), "no backend for {os:?}/{arch:?}");

                    let backend = backend.unwrap();
                    let id = backend.artifact_id(target);
                    assert!(
                        id.is_some(),
                        "{os:?}/{arch:?} selected {backend:?}, which names no artifact"
                    );

                    let id = id.unwrap();
                    assert!(
                        pinned_manifest().artifact(id).is_some(),
                        "selection produced {id}, which runtimes.json does not contain"
                    );
                }
            }
        }
    }

    #[test]
    fn every_manifest_entry_is_reachable_from_selection() {
        // The other direction: a pinned artifact nobody can select is dead
        // weight on the one file whose whole value is being reviewed.
        let reachable: Vec<&str> = TARGETS
            .into_iter()
            .flat_map(|(os, arch)| {
                let target = Target { os, arch };
                [
                    Backend::Metal,
                    Backend::Cuda124,
                    Backend::Cuda133,
                    Backend::Vulkan,
                    Backend::Cpu,
                ]
                .into_iter()
                .filter_map(move |backend| backend.artifact_id(target))
            })
            .collect();

        for artifact in &pinned_manifest().artifacts {
            assert!(
                reachable.contains(&artifact.id.as_str()),
                "{} is pinned but no target can select it",
                artifact.id
            );
        }
    }

    #[test]
    fn every_filename_carries_the_pinned_tag() {
        let manifest = pinned_manifest();

        for artifact in &manifest.artifacts {
            assert!(
                artifact.file.contains(&manifest.upstream_tag),
                "{} does not name the pinned tag {}",
                artifact.file,
                manifest.upstream_tag
            );
        }
    }

    #[test]
    fn companion_archives_are_the_cuda_runtimes() {
        // cudart archives are versioned by CUDA release rather than llama.cpp
        // build, so they are the one filename shape that does not carry the tag.
        for artifact in &pinned_manifest().artifacts {
            if let Some(companion) = &artifact.companion {
                assert!(
                    companion.file.starts_with("cudart-"),
                    "{} has an unexpected companion: {}",
                    artifact.id,
                    companion.file
                );
            }
        }
    }

    #[test]
    fn only_the_windows_cuda_artifacts_have_a_companion() {
        for artifact in &pinned_manifest().artifacts {
            let expected = artifact.id.starts_with("win-cuda-");
            assert_eq!(
                artifact.companion.is_some(),
                expected,
                "{} has the wrong companion arrangement; Windows CUDA needs the \
                 separate cudart archive and nothing else does",
                artifact.id
            );
        }
    }

    #[test]
    fn total_bytes_includes_the_companion() {
        let manifest = pinned_manifest();

        let cuda = manifest.artifact("win-cuda-13.3-x64").unwrap();
        assert_eq!(cuda.total_bytes(), 146_865_373 + 390_970_417);

        let vulkan = manifest.artifact("win-vulkan-x64").unwrap();
        assert_eq!(vulkan.total_bytes(), 33_637_361);
    }

    #[test]
    fn cuda_is_an_order_of_magnitude_larger_than_vulkan() {
        // Encodes why CudaChoice exists at all. If a future pin makes these
        // comparable, the opt-in prompt is worth revisiting.
        let manifest = pinned_manifest();
        let cuda = manifest
            .artifact("win-cuda-13.3-x64")
            .unwrap()
            .total_bytes();
        let vulkan = manifest.artifact("win-vulkan-x64").unwrap().total_bytes();

        assert!(cuda > vulkan * 10, "cuda {cuda} vs vulkan {vulkan}");
    }

    #[test]
    fn the_download_url_points_at_the_pinned_release() {
        let manifest = pinned_manifest();
        let artifact = manifest.artifact("ubuntu-vulkan-x64").unwrap();

        assert_eq!(
            artifact.url(manifest),
            "https://github.com/ggml-org/llama.cpp/releases/download/b10194/llama-b10194-bin-ubuntu-vulkan-x64.tar.gz"
        );
    }

    #[test]
    fn only_a_companion_artifact_has_a_companion_url() {
        let manifest = pinned_manifest();

        assert_eq!(
            manifest
                .artifact("win-cuda-13.3-x64")
                .unwrap()
                .companion_url(manifest),
            Some(
                "https://github.com/ggml-org/llama.cpp/releases/download/b10194/cudart-llama-bin-win-cuda-13.3-x64.zip"
                    .to_owned()
            )
        );
        assert!(
            manifest
                .artifact("ubuntu-vulkan-x64")
                .unwrap()
                .companion_url(manifest)
                .is_none()
        );
    }

    #[test]
    fn artifact_for_resolves_through_a_backend() {
        let manifest = pinned_manifest();
        let target = Target {
            os: TargetOs::Linux,
            arch: TargetArch::X64,
        };

        let artifact = manifest.artifact_for(target, Backend::Vulkan).unwrap();
        assert_eq!(artifact.id, "ubuntu-vulkan-x64");

        assert!(
            manifest.artifact_for(target, Backend::Metal).is_none(),
            "Metal on Linux names no artifact and must not resolve to one"
        );
    }

    #[test]
    fn the_os_and_arch_fields_agree_with_the_id() {
        // The fields are what a reviewer reads; the id is what code looks up.
        // A row where they disagree would review as correct and behave wrongly.
        for artifact in &pinned_manifest().artifacts {
            let expected_os = if artifact.id.starts_with("win-") {
                "windows"
            } else if artifact.id.starts_with("macos-") {
                "macos"
            } else {
                "linux"
            };
            assert_eq!(
                artifact.os, expected_os,
                "{} claims the wrong os",
                artifact.id
            );

            let expected_arch = if artifact.id.ends_with("arm64") {
                "arm64"
            } else {
                "x64"
            };
            assert_eq!(
                artifact.arch, expected_arch,
                "{} claims the wrong arch",
                artifact.id
            );
        }
    }
}
