//! The whole acquisition, in one order that never varies.
//!
//! Select, look up, check space, download, verify, extract, mark executable,
//! confirm the server is there. Each step's failure is a distinct variant, so a
//! full disk and a tampered archive never look alike to the person reading the
//! message.
//!
//! Nothing here starts on its own. Acquisition happens because a user asked for
//! it, which is what keeps SECURITY.md's "no network requests except ones you
//! explicitly trigger" true.

use std::path::Path;

use crate::download::{Progress, download_verified};
use crate::error::AcquireError;
use crate::manifest::{RuntimeArtifact, RuntimeManifest, pinned_manifest};
use crate::select::{Backend, CudaChoice, GpuCapability, select};
use crate::store::{InstalledRuntime, RuntimeStore, directory_size};
use crate::target::Target;
use crate::unpack::{find_server, unpack};

/// Which stage acquisition has reached, for the UI to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Fetching the main archive.
    Downloading(Progress),
    /// Fetching the separate CUDA runtime archive.
    ///
    /// Its own stage because it is the larger of the two downloads on Windows
    /// CUDA — 391 MB against 147 — and a progress bar that restarts without
    /// explanation looks like a failure.
    DownloadingCudaRuntime(Progress),
    /// Hashing and unpacking.
    Verifying,
    /// Finished.
    Done,
}

/// Free bytes on the filesystem holding `path`.
///
/// Walks up to the nearest existing ancestor, because the install directory may
/// not have been created yet when the check runs.
fn available_space(path: &Path) -> u64 {
    let mut candidate = path;

    loop {
        if candidate.exists() {
            return fs4::available_space(candidate).unwrap_or(0);
        }
        match candidate.parent() {
            Some(parent) => candidate = parent,
            None => return 0,
        }
    }
}

/// Refuses if `needed` bytes are not free at `path`.
///
/// # Errors
///
/// [`AcquireError::NotEnoughSpace`], naming both figures so the user can see
/// how far short they are rather than being told only that it failed.
fn require_space(path: &Path, needed: u64) -> Result<(), AcquireError> {
    let available = available_space(path);

    if available < needed {
        return Err(AcquireError::NotEnoughSpace {
            path: path.to_path_buf(),
            needed_bytes: needed,
            available_bytes: available,
        });
    }

    Ok(())
}

/// Resolves the artifact a machine should install.
///
/// # Errors
///
/// [`AcquireError::UnsupportedTarget`] for a target with no published build,
/// [`AcquireError::UnknownArtifact`] if selection names something the manifest
/// does not contain — which a test proves cannot happen for any reachable
/// combination, so it exists to fail legibly if that test is ever weakened.
pub fn resolve<'m>(
    manifest: &'m RuntimeManifest,
    target: Target,
    gpu: &GpuCapability,
    cuda: CudaChoice,
) -> Result<(Backend, &'m RuntimeArtifact), AcquireError> {
    let unsupported = || AcquireError::UnsupportedTarget {
        os: format!("{:?}", target.os),
        arch: format!("{:?}", target.arch),
    };

    let backend = select(target, gpu, cuda).ok_or_else(unsupported)?;
    let id = backend.artifact_id(target).ok_or_else(unsupported)?;
    let artifact = manifest
        .artifact(id)
        .ok_or_else(|| AcquireError::UnknownArtifact { id: id.to_owned() })?;

    Ok((backend, artifact))
}

/// Chooses, fetches and installs the runtime for `target`.
///
/// # Errors
///
/// Any [`AcquireError`]. A checksum mismatch aborts without retrying and
/// without leaving anything executable behind.
pub async fn acquire(
    store: &RuntimeStore,
    client: &reqwest::Client,
    target: Target,
    gpu: &GpuCapability,
    cuda: CudaChoice,
    // `+ Send` because the caller spawns this onto Tauri's async runtime, and a
    // future holding a non-Send callback cannot cross a thread boundary.
    on_stage: &mut (dyn FnMut(Stage) + Send),
) -> Result<InstalledRuntime, AcquireError> {
    let manifest = pinned_manifest();
    let (_, artifact) = resolve(manifest, target, gpu, cuda)?;

    let destination = store.path_for(&manifest.upstream_tag, &artifact.id);

    // The archive and its extracted contents coexist until the archive is
    // removed, so budget for both rather than for the download alone.
    require_space(&destination, artifact.total_bytes().saturating_mul(2))?;

    std::fs::create_dir_all(&destination).map_err(|source| AcquireError::Io {
        path: destination.clone(),
        source,
    })?;

    // The CUDA runtime first, when there is one: it is the larger download, and
    // failing on it after the main archive has landed would leave a directory
    // that looks installed but cannot start.
    if let (Some(companion), Some(url)) = (&artifact.companion, artifact.companion_url(manifest)) {
        let companion_archive = destination.join(&companion.file);
        on_stage(Stage::DownloadingCudaRuntime(Progress {
            downloaded_bytes: 0,
            total_bytes: companion.size_bytes,
        }));

        download_verified(
            client,
            &url,
            &companion.sha256,
            companion.size_bytes,
            &companion_archive,
            &mut |progress| on_stage(Stage::DownloadingCudaRuntime(progress)),
        )
        .await?;

        on_stage(Stage::Verifying);
        unpack(&companion_archive, &destination)?;
        let _ = std::fs::remove_file(&companion_archive);
    }

    let archive = destination.join(&artifact.file);
    on_stage(Stage::Downloading(Progress {
        downloaded_bytes: 0,
        total_bytes: artifact.size_bytes,
    }));

    download_verified(
        client,
        &artifact.url(manifest),
        &artifact.sha256,
        artifact.size_bytes,
        &archive,
        &mut |progress| on_stage(Stage::Downloading(progress)),
    )
    .await?;

    on_stage(Stage::Verifying);
    unpack(&archive, &destination)?;
    let _ = std::fs::remove_file(&archive);

    let server_path = find_server(&destination).ok_or_else(|| AcquireError::ServerMissing {
        file: artifact.file.clone(),
    })?;

    osstat_platform::mark_executable(&server_path).map_err(|source| {
        AcquireError::NotExecutable {
            path: server_path.clone(),
            source,
        }
    })?;

    on_stage(Stage::Done);

    Ok(InstalledRuntime {
        tag: manifest.upstream_tag.clone(),
        artifact_id: artifact.id.clone(),
        server_path,
        size_bytes: directory_size(&destination),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::target::{TargetArch, TargetOs};

    const NO_GPU: GpuCapability = GpuCapability {
        present: false,
        nvidia_cuda_major: None,
    };
    const NVIDIA_13: GpuCapability = GpuCapability {
        present: true,
        nvidia_cuda_major: Some(13),
    };

    #[test]
    fn an_impossible_space_requirement_is_refused() {
        let dir = tempfile::tempdir().unwrap();

        let error = require_space(dir.path(), u64::MAX).unwrap_err();

        let mut checked = false;
        if let AcquireError::NotEnoughSpace { needed_bytes, .. } = &error {
            assert_eq!(*needed_bytes, u64::MAX);
            checked = true;
        }
        assert!(checked, "expected NotEnoughSpace, got {error:?}");
        assert!(!error.is_retryable(), "a full disk will not empty itself");
    }

    #[test]
    fn a_plausible_space_requirement_passes() {
        let dir = tempfile::tempdir().unwrap();

        require_space(dir.path(), 1024).expect("a temp dir has a kilobyte free");
    }

    #[test]
    fn space_is_checked_against_a_directory_that_does_not_exist_yet() {
        // The real call site: the install directory has not been created when
        // the check runs, so this must walk up rather than report zero free.
        let dir = tempfile::tempdir().unwrap();
        let unborn = dir.path().join("runtimes/b10194/ubuntu-vulkan-x64");

        assert!(
            available_space(&unborn) > 0,
            "walked past a real filesystem"
        );
        require_space(&unborn, 1024).expect("the parent filesystem has room");
    }

    #[test]
    fn available_space_of_an_impossible_path_is_zero_rather_than_a_panic() {
        assert_eq!(available_space(Path::new("")), 0);
    }

    #[test]
    fn resolving_picks_the_artifact_selection_names() {
        let manifest = pinned_manifest();
        let target = Target {
            os: TargetOs::Linux,
            arch: TargetArch::X64,
        };

        let (backend, artifact) = resolve(manifest, target, &NO_GPU, CudaChoice::Declined).unwrap();

        assert_eq!(backend, Backend::Cpu);
        assert_eq!(artifact.id, "ubuntu-x64");
    }

    #[test]
    fn resolving_windows_cuda_yields_the_artifact_with_a_companion() {
        let manifest = pinned_manifest();
        let target = Target {
            os: TargetOs::Windows,
            arch: TargetArch::X64,
        };

        let (backend, artifact) =
            resolve(manifest, target, &NVIDIA_13, CudaChoice::Accepted).unwrap();

        assert_eq!(backend, Backend::Cuda133);
        assert!(
            artifact.companion.is_some(),
            "Windows CUDA needs its cudart archive or the server will not start"
        );
    }

    #[test]
    fn declining_cuda_resolves_to_a_download_an_order_of_magnitude_smaller() {
        let manifest = pinned_manifest();
        let target = Target {
            os: TargetOs::Windows,
            arch: TargetArch::X64,
        };

        let (_, with_cuda) = resolve(manifest, target, &NVIDIA_13, CudaChoice::Accepted).unwrap();
        let (_, without) = resolve(manifest, target, &NVIDIA_13, CudaChoice::Declined).unwrap();

        assert!(
            with_cuda.total_bytes() > without.total_bytes() * 10,
            "the size gap is the entire reason CUDA is opt-in"
        );
    }

    #[test]
    fn resolving_succeeds_for_every_target_and_capability() {
        let manifest = pinned_manifest();

        for os in [TargetOs::Windows, TargetOs::Linux, TargetOs::MacOs] {
            for arch in [TargetArch::X64, TargetArch::Arm64] {
                for gpu in [&NO_GPU, &NVIDIA_13] {
                    for cuda in [CudaChoice::Declined, CudaChoice::Accepted] {
                        let target = Target { os, arch };
                        assert!(
                            resolve(manifest, target, gpu, cuda).is_ok(),
                            "{os:?}/{arch:?} could not resolve an artifact"
                        );
                    }
                }
            }
        }
    }
}
