//! IPC adapters for acquiring the llama.cpp runtime.
//!
//! Adapters only, per AGENTS.md: `osstat-inference` does the work, these
//! translate its types into the shapes the webview sees and its progress into
//! events.
//!
//! Kept out of `commands.rs` because acquisition is the only command family
//! that owns background work and an event stream of its own, and folding a
//! spawned task and three events into the file of one-line adapters would blur
//! what that file is for.
//!
//! **The webview never makes an HTTP request.** Every byte moves in Rust and
//! reaches the front end over IPC, which is what keeps the CSP in
//! `tauri.conf.json` an unweakened control (SECURITY.md threat 3).

// Tauri resolves `AppHandle` and `State<'_, T>` by injecting them by value; a
// reference is not part of the command signature it accepts. Both are cheap
// handles, so the lint's concern does not apply. Same reason as `commands.rs`.
#![allow(clippy::needless_pass_by_value)]

use std::path::PathBuf;

use osstat_inference::{
    AcquireError, CudaChoice, GpuCapability, RuntimeStore, Stage, Target, acquire, cuda_is_offered,
    pinned_manifest, resolve,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::sampler::Sampler;

/// Emitted repeatedly while an acquisition runs.
pub const RUNTIME_PROGRESS_EVENT: &str = "runtime:progress";
/// Emitted once when a runtime is installed and usable.
pub const RUNTIME_READY_EVENT: &str = "runtime:ready";
/// Emitted once when an acquisition fails.
pub const RUNTIME_FAILED_EVENT: &str = "runtime:failed";

/// One artifact the user could install, and what it would cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOption {
    /// The `runtimes.json` identifier.
    pub artifact_id: String,
    /// Human-readable backend name, such as `Vulkan`.
    pub backend: String,
    /// Every byte that must be downloaded, including any companion archive.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub download_bytes: u64,
}

/// A runtime present on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntimeInfo {
    /// Upstream release tag.
    pub tag: String,
    /// Artifact identifier.
    pub artifact_id: String,
    /// Bytes this install occupies.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub size_bytes: u64,
}

/// What the runtime section of Settings needs, in one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// Runtimes already on disk.
    pub installed: Vec<InstalledRuntimeInfo>,
    /// The artifact this machine would install by default.
    ///
    /// `None` only on a target upstream publishes nothing for.
    pub recommended: Option<RuntimeOption>,
    /// The CUDA alternative, when one exists for this machine.
    ///
    /// A separate field rather than a different value of `recommended`, because
    /// the UI's job here is to present a choice with its cost rather than to
    /// have made it already: CUDA is 538–642 MB against Vulkan's 34 MB.
    pub cuda_alternative: Option<RuntimeOption>,
    /// The upstream release every option comes from.
    pub upstream_tag: String,
}

/// How an acquisition is progressing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProgress {
    /// What is happening: `downloading`, `cudaRuntime`, or `verifying`.
    pub phase: String,
    /// Bytes fetched so far in the current phase.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub downloaded_bytes: u64,
    /// Bytes expected in the current phase, from the pinned manifest.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total_bytes: u64,
}

/// Why an acquisition failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFailure {
    /// What went wrong, in a sentence the user can act on.
    pub message: String,
    /// Whether offering a retry makes sense.
    ///
    /// False for a checksum mismatch above all. Retrying a hash that did not
    /// match either wastes a 600 MB download or invites the user to keep
    /// trying until a tampered file slips through, so the control is not shown
    /// rather than shown and discouraged.
    pub retryable: bool,
    /// Whether this was a verification failure rather than a transport one.
    ///
    /// The UI says something different for these: a mismatch is a security
    /// event, not a bad day on the network.
    pub verification_failure: bool,
}

impl From<&AcquireError> for RuntimeFailure {
    fn from(error: &AcquireError) -> Self {
        Self {
            message: error.to_string(),
            retryable: error.is_retryable(),
            verification_failure: matches!(
                error,
                AcquireError::ChecksumMismatch { .. } | AcquireError::ServerMissing { .. }
            ),
        }
    }
}

/// Turns a stage into the payload the front end renders.
fn progress_of(stage: &Stage) -> Option<RuntimeProgress> {
    let (phase, progress) = match stage {
        Stage::Downloading(progress) => ("downloading", Some(progress)),
        Stage::DownloadingCudaRuntime(progress) => ("cudaRuntime", Some(progress)),
        Stage::Verifying => ("verifying", None),
        // Completion travels on `runtime:ready` with the install, not here.
        Stage::Done => return None,
    };

    Some(RuntimeProgress {
        phase: phase.to_owned(),
        downloaded_bytes: progress.map_or(0, |p| p.downloaded_bytes),
        total_bytes: progress.map_or(0, |p| p.total_bytes),
    })
}

/// What the probe found, in the shape selection needs.
fn capability_of(sampler: &Sampler) -> Option<GpuCapability> {
    let devices = sampler.devices()?;

    Some(GpuCapability {
        present: !devices.is_empty(),
        nvidia_cuda_major: sampler.cuda_driver_major(),
    })
}

/// Describes one installable option.
fn option_for(target: Target, gpu: &GpuCapability, cuda: CudaChoice) -> Option<RuntimeOption> {
    let manifest = pinned_manifest();
    let (backend, artifact) = resolve(manifest, target, gpu, cuda).ok()?;

    Some(RuntimeOption {
        artifact_id: artifact.id.clone(),
        backend: backend.label().to_owned(),
        download_bytes: artifact.total_bytes(),
    })
}

/// Where runtimes are installed for this app.
fn store_for(app: &AppHandle) -> Result<RuntimeStore, AcquireError> {
    let root: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|error| AcquireError::Io {
            path: PathBuf::from("app data directory"),
            source: std::io::Error::other(error.to_string()),
        })?;

    Ok(RuntimeStore::new(root))
}

/// What is installed, and what could be.
///
/// Returns `None` while the GPU probe is still running, the same convention
/// [`crate::commands::llm_advice`] uses and for the same reason: recommending a
/// CPU build on a machine with a GPU is a confident wrong answer, which is the
/// failure mode ADR-008 cares most about. The `gpus:ready` event is the cue to
/// ask again.
#[tauri::command]
#[must_use]
pub fn runtime_status(app: AppHandle, sampler: State<'_, Sampler>) -> Option<RuntimeStatus> {
    let gpu = capability_of(&sampler)?;
    let target = Target::host();

    let installed = store_for(&app)
        .map(|store| {
            store
                .installed()
                .into_iter()
                .map(|runtime| InstalledRuntimeInfo {
                    tag: runtime.tag,
                    artifact_id: runtime.artifact_id,
                    size_bytes: runtime.size_bytes,
                })
                .collect()
        })
        .unwrap_or_default();

    let (recommended, cuda_alternative) = target.map_or((None, None), |target| {
        let recommended = option_for(target, &gpu, CudaChoice::Declined);
        let alternative = if cuda_is_offered(target, &gpu) {
            option_for(target, &gpu, CudaChoice::Accepted)
        } else {
            None
        };
        (recommended, alternative)
    });

    Some(RuntimeStatus {
        installed,
        recommended,
        cuda_alternative,
        upstream_tag: pinned_manifest().upstream_tag.clone(),
    })
}

/// Starts acquiring a runtime, reporting through `runtime:*` events.
///
/// Returns as soon as the work is spawned rather than when it finishes: a
/// Windows CUDA acquisition moves 642 MB, and holding the IPC call open for its
/// duration would freeze the command channel the rest of the app uses.
///
/// # Errors
///
/// If the probe has not finished, or the app data directory cannot be resolved.
/// Failures *during* acquisition arrive on `runtime:failed`, not here.
#[tauri::command]
pub fn acquire_runtime(
    app: AppHandle,
    sampler: State<'_, Sampler>,
    accept_cuda: bool,
) -> Result<(), String> {
    let gpu =
        capability_of(&sampler).ok_or_else(|| "the GPU probe has not finished yet".to_owned())?;
    let target = Target::host().ok_or_else(|| {
        "no llama.cpp runtime is published for this operating system and architecture".to_owned()
    })?;
    let store = store_for(&app).map_err(|error| error.to_string())?;

    let cuda = if accept_cuda {
        CudaChoice::Accepted
    } else {
        CudaChoice::Declined
    };

    // The one interesting fact about which build this is, and it is a boolean:
    // CUDA is 642 MB against Vulkan's 34 MB, so an acquisition that seems to
    // have stalled is either explained by this line or is worth looking into.
    crate::log::runtime_acquiring(accept_cuda);
    let started = std::time::Instant::now();

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let mut on_stage = |stage: Stage| {
            if let Some(progress) = progress_of(&stage) {
                let _ = app.emit(RUNTIME_PROGRESS_EVENT, &progress);
            }
        };

        match acquire(&store, &client, target, &gpu, cuda, &mut on_stage).await {
            Ok(installed) => {
                crate::log::runtime_acquired(installed.size_bytes, started.elapsed().as_secs());
                let _ = app.emit(
                    RUNTIME_READY_EVENT,
                    InstalledRuntimeInfo {
                        tag: installed.tag,
                        artifact_id: installed.artifact_id,
                        size_bytes: installed.size_bytes,
                    },
                );
            }
            Err(error) => {
                // By kind: every `AcquireError` message names a file or a URL.
                crate::log::runtime_acquire_failed(error.kind());
                let _ = app.emit(RUNTIME_FAILED_EVENT, RuntimeFailure::from(&error));
            }
        }
    });

    Ok(())
}

/// Deletes an installed runtime.
///
/// osstat is a disk cleaner; anything it downloaded has to be removable from
/// inside it.
///
/// # Errors
///
/// If the app data directory cannot be resolved, or the files cannot be removed.
#[tauri::command]
pub fn delete_runtime(app: AppHandle, tag: String, artifact_id: String) -> Result<(), String> {
    store_for(&app)
        .and_then(|store| store.remove(&tag, &artifact_id))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use osstat_inference::Progress;

    #[test]
    fn a_checksum_mismatch_reaches_the_ui_as_unretryable_and_a_verification_failure() {
        // The user-facing half of the security control. Both flags matter: one
        // hides the retry button, the other changes what the message says.
        let error = AcquireError::ChecksumMismatch {
            file: "llama.zip".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };

        let failure = RuntimeFailure::from(&error);

        assert!(!failure.retryable);
        assert!(failure.verification_failure);
        assert!(failure.message.contains("llama.zip"));
    }

    #[test]
    fn a_network_failure_is_retryable_and_not_a_verification_problem() {
        let error = AcquireError::HttpStatus {
            url: "https://example.invalid/x".to_owned(),
            status: 503,
        };

        let failure = RuntimeFailure::from(&error);

        assert!(failure.retryable);
        assert!(!failure.verification_failure);
    }

    #[test]
    fn an_archive_missing_its_server_is_a_verification_failure() {
        // The bytes were what was pinned, so the pin describes the wrong file.
        let error = AcquireError::ServerMissing {
            file: "llama.tar.gz".to_owned(),
        };

        let failure = RuntimeFailure::from(&error);

        assert!(failure.verification_failure);
        assert!(!failure.retryable);
    }

    #[test]
    fn each_download_stage_reports_its_own_phase() {
        let progress = Progress {
            downloaded_bytes: 10,
            total_bytes: 100,
        };

        assert_eq!(
            progress_of(&Stage::Downloading(progress)).unwrap().phase,
            "downloading"
        );
        assert_eq!(
            progress_of(&Stage::DownloadingCudaRuntime(progress))
                .unwrap()
                .phase,
            "cudaRuntime",
            "the cudart archive is the larger download and needs its own label, \
             or the bar appears to restart for no reason"
        );
        assert_eq!(progress_of(&Stage::Verifying).unwrap().phase, "verifying");
    }

    #[test]
    fn completion_does_not_travel_as_progress() {
        // It arrives on runtime:ready with the install description instead.
        assert!(progress_of(&Stage::Done).is_none());
    }

    #[test]
    fn verifying_reports_no_byte_counts_rather_than_a_stale_pair() {
        let progress = progress_of(&Stage::Verifying).unwrap();

        assert_eq!(progress.downloaded_bytes, 0);
        assert_eq!(progress.total_bytes, 0);
    }

    #[test]
    fn a_machine_with_a_gpu_is_offered_a_recommendation_smaller_than_its_cuda_option() {
        let Some(target) = Target::host() else {
            return;
        };
        let gpu = GpuCapability {
            present: true,
            nvidia_cuda_major: Some(13),
        };

        let recommended = option_for(target, &gpu, CudaChoice::Declined)
            .expect("every supported target has a default option");

        if cuda_is_offered(target, &gpu) {
            let alternative = option_for(target, &gpu, CudaChoice::Accepted)
                .expect("cuda_is_offered promised an artifact");
            assert!(
                alternative.download_bytes > recommended.download_bytes,
                "if CUDA were not the larger download it would not need to be opt-in"
            );
        }
    }

    #[test]
    fn the_status_payload_is_camel_case() {
        let status = RuntimeStatus {
            installed: Vec::new(),
            recommended: None,
            cuda_alternative: None,
            upstream_tag: "b10194".to_owned(),
        };

        let json = serde_json::to_value(&status).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("cudaAlternative"));
        assert!(object.contains_key("upstreamTag"));
        assert!(object.contains_key("recommended"));
        assert!(object.contains_key("installed"));
    }

    #[test]
    fn the_failure_payload_is_camel_case() {
        let json = serde_json::to_value(RuntimeFailure {
            message: "x".to_owned(),
            retryable: false,
            verification_failure: true,
        })
        .unwrap();

        assert!(
            json.as_object()
                .unwrap()
                .contains_key("verificationFailure")
        );
    }
}
