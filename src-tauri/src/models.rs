//! IPC adapters for downloading pinned models and choosing where they live.
//!
//! Adapters only, per AGENTS.md: `osstat-inference` fetches, verifies, records
//! and relocates; these translate its types into the shapes the webview sees
//! and its progress into events. The shape follows [`crate::runtime`] and
//! [`crate::chat`] exactly — event-name constants, one payload struct per
//! event, `app.emit(...)` from a task spawned onto `tauri::async_runtime`, and
//! commands that return as soon as the work is under way.
//!
//! **The webview never makes an HTTP request.** Every byte moves in Rust and
//! reaches the front end over IPC, which is what keeps the CSP in
//! `tauri.conf.json` an unweakened control (SECURITY.md threat 3). It is also
//! what keeps "only pinned files are downloadable" true: there is no command
//! here that takes a URL.

// Tauri resolves `AppHandle` and `State<'_, T>` by injecting them by value; a
// reference is not part of the command signature it accepts. Both are cheap
// handles, so the lint's concern does not apply. Same reason as `commands.rs`.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use osstat_inference::{
    AcquireError, ModelKey, ModelRecord, ModelStore, MovePlan, Progress, download_resumable,
    download_url, move_library, plan_move, require_space,
};
use osstat_llm::registry::{ModelDownload, seeded_registry};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Emitted repeatedly while a model downloads or the library moves.
pub const MODEL_PROGRESS_EVENT: &str = "model:progress";
/// Emitted once when a download or a move finishes.
pub const MODEL_DONE_EVENT: &str = "model:done";
/// Emitted once when a download or a move could not be finished.
pub const MODEL_FAILED_EVENT: &str = "model:failed";

/// The folder downloads land in when the user has chosen none.
///
/// A subdirectory rather than app data itself, so pointing the picker at the
/// default and then emptying it does not take the conversation store and the
/// index with it.
const DEFAULT_FOLDER: &str = "models";

/// The index of downloaded models, inside the app-data directory.
///
/// Deliberately not inside the model folder: that folder is the one the user
/// may move, empty, or point at a network share, and the record of where the
/// files went must outlive all three.
const INDEX_FILE: &str = "models.json";

/// The suffix a download accumulates into before it has verified.
const PART_SUFFIX: &str = "part";

/// One fit-matrix cell that has a pinned file, a downloaded one, or both.
///
/// The catalogue is a join, not a copy of the registry: a cell with no entry
/// here is one nobody has pinned, which is what lets the advisor say "not
/// pinned" rather than offer a control that fails. The four states the UI
/// renders are therefore *no entry*, plus the three [`Self::state`] can hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogueEntry {
    /// Which cell this describes.
    pub key: ModelKey,
    /// What the cell can do: `downloadable`, `downloading`, or `downloaded`.
    pub state: String,
    /// Who published the re-quantization, so provenance is visible on the
    /// control rather than only in the manifest.
    ///
    /// `None` only for a cell that is downloaded but no longer pinned, which
    /// happens when a pin is withdrawn after someone fetched it.
    pub publisher: Option<String>,
    /// The Hugging Face repository the file comes from.
    pub repo: Option<String>,
    /// The file name within that repository.
    pub file: Option<String>,
    /// What the file weighs, from the pin or from the record.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub size_bytes: u64,
    /// Where the file is, once it has been downloaded.
    ///
    /// This is what the Run control hands to `chat_open_model`, so there is one
    /// path into a session rather than two.
    pub path: Option<String>,
}

/// How a download or a move is progressing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelProgress {
    /// The cell being downloaded, or `None` while the whole library moves.
    pub key: Option<ModelKey>,
    /// What is happening: `downloading` or `moving`.
    pub phase: String,
    /// Bytes transferred so far, counting anything a resumed download already
    /// had, or a resume appears to start over.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub downloaded_bytes: u64,
    /// Bytes expected in total, from the pin rather than from any response.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub total_bytes: u64,
}

/// A download or a move that finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelDone {
    /// The cell that finished downloading, or `None` when a move finished.
    pub key: Option<ModelKey>,
    /// The file that landed, or the folder the library moved to.
    pub path: String,
}

/// A download or a move that could not be finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ModelFailure {
    /// The cell that failed, or `None` when a move failed.
    pub key: Option<ModelKey>,
    /// What went wrong, in a sentence the user can act on.
    pub message: String,
    /// Whether offering a retry makes sense.
    ///
    /// False for a checksum mismatch above all: retrying a hash that did not
    /// match either wastes a multi-gigabyte download or invites the user to
    /// keep trying until a tampered file slips through.
    pub retryable: bool,
    /// Whether this was a verification failure rather than a transport one.
    ///
    /// The UI says something different for these: a mismatch is a security
    /// event, not a bad day on the network.
    pub verification_failure: bool,
    /// Whether the user stopped it rather than anything going wrong.
    ///
    /// Carried rather than left to the message because a cancellation is not a
    /// failure, and the partial file is deliberately kept so the next attempt
    /// resumes from it.
    pub cancelled: bool,
}

/// What moving the library to a chosen folder would cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct LibraryMovePlan {
    /// How many downloaded files would move.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub files: u32,
    /// How many bytes would move.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number"))]
    pub bytes: u64,
    /// Whether every file can be renamed rather than copied — instant against
    /// an hour, which is why Settings states it before asking.
    pub same_volume: bool,
}

impl From<MovePlan> for LibraryMovePlan {
    fn from(plan: MovePlan) -> Self {
        Self {
            files: u32::try_from(plan.files).unwrap_or(u32::MAX),
            bytes: plan.bytes,
            same_volume: plan.same_volume,
        }
    }
}

impl ModelFailure {
    /// Describes a failure of the cell `key`, or of a library move.
    fn of(key: Option<ModelKey>, error: &AcquireError) -> Self {
        Self {
            key,
            message: error.to_string(),
            retryable: error.is_retryable(),
            verification_failure: matches!(
                error,
                AcquireError::ChecksumMismatch { .. } | AcquireError::SizeMismatch { .. }
            ),
            cancelled: false,
        }
    }

    /// Describes the user stopping a download of `key`.
    fn cancelled(key: ModelKey) -> Self {
        Self {
            key: Some(key),
            message: "the download was stopped; starting it again resumes from where it \
                      stopped"
                .to_owned(),
            // Resuming is exactly the right response, which is what this flag
            // is asked in order to decide.
            retryable: true,
            verification_failure: false,
            cancelled: true,
        }
    }
}

/// What the single work slot is doing, for the refusal that names it.
#[derive(Debug, Clone)]
struct Busy {
    /// The cell being downloaded, if this is a download.
    key: Option<ModelKey>,
    /// What to call it when refusing a second request.
    what: String,
}

/// The chosen folder, and the one download that may be running.
pub struct ModelState {
    /// Where downloads land; `None` means [`DEFAULT_FOLDER`] under app data.
    ///
    /// Held here rather than persisted, exactly as [`crate::window_state`]
    /// holds the close behaviour: the front end stores the preference and
    /// re-applies it at startup. Records carry absolute paths, so a folder
    /// setting that has not been re-applied yet cannot lose anything.
    folder: Mutex<Option<PathBuf>>,
    /// The download or move currently under way, if any.
    ///
    /// One at a time. Two multi-gigabyte downloads competing for bandwidth and
    /// disk both finish later than one after the other, and a progress bar that
    /// cannot say which file it describes is worse than no progress bar.
    busy: Mutex<Option<Busy>>,
    /// How to stop the download currently running.
    cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// The app-data directory, which is where the index lives.
    root: PathBuf,
}

impl ModelState {
    /// Creates state rooted at `root`, the Tauri app-data directory.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self {
            folder: Mutex::new(None),
            busy: Mutex::new(None),
            cancel: Mutex::new(None),
            root,
        }
    }

    /// The index of what has been downloaded.
    fn store(&self) -> ModelStore {
        ModelStore::new(self.root.join(INDEX_FILE))
    }

    /// Where downloads land, chosen or defaulted.
    fn folder(&self) -> PathBuf {
        self.folder
            .lock()
            .ok()
            .and_then(|held| held.clone())
            .unwrap_or_else(|| self.root.join(DEFAULT_FOLDER))
    }

    /// Chooses where downloads land from now on.
    fn set_folder(&self, folder: PathBuf) {
        if let Ok(mut held) = self.folder.lock() {
            *held = Some(folder);
        }
    }

    /// Takes the work slot, or refuses naming what already holds it.
    ///
    /// Checked and set under one guard, so two invokes arriving together cannot
    /// both find it free.
    fn claim(&self, busy: Busy) -> Result<(), String> {
        let Ok(mut held) = self.busy.lock() else {
            return Err("the download state could not be read".to_owned());
        };

        if let Some(running) = held.as_ref() {
            return Err(format!(
                "{} is already in progress; wait for it to finish or stop it first",
                running.what
            ));
        }

        *held = Some(busy);
        Ok(())
    }

    /// Frees the work slot and forgets how to stop it.
    fn release(&self) {
        if let Ok(mut held) = self.busy.lock() {
            *held = None;
        }
        if let Ok(mut held) = self.cancel.lock() {
            *held = None;
        }
    }

    /// The cell currently downloading, if any.
    fn downloading(&self) -> Option<ModelKey> {
        self.busy
            .lock()
            .ok()
            .and_then(|held| held.as_ref().and_then(|busy| busy.key.clone()))
    }
}

/// The pinned file for one cell, if there is one.
fn pinned(key: &ModelKey) -> Option<ModelDownload> {
    seeded_registry()
        .models
        .into_iter()
        .find(|model| model.id == key.model_id)?
        .downloads
        .into_iter()
        .find(|download| download.quant_id == key.quant_id)
}

/// Creates `folder` if it is not there yet.
fn ensure_folder(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder)
        .map_err(|error| format!("{} could not be created: {error}", folder.display()))
}

/// A path as the webview sees it.
fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// Every cell the UI can act on: pinned, downloaded, or both.
///
/// Cells with neither are absent rather than listed as empty, so the advisor
/// says "not pinned" from the absence instead of from a fourth string that
/// could be misspelled.
#[tauri::command]
#[must_use]
pub fn models_catalogue(state: State<'_, ModelState>) -> Vec<ModelCatalogueEntry> {
    let present = state.store().present();
    let running = state.downloading();
    let mut entries: Vec<ModelCatalogueEntry> = Vec::new();

    for model in seeded_registry().models {
        for download in model.downloads {
            let key = ModelKey {
                model_id: model.id.clone(),
                quant_id: download.quant_id.clone(),
            };
            let record = present.iter().find(|record| record.key == key);

            entries.push(ModelCatalogueEntry {
                state: state_of(&key, record.is_some(), running.as_ref()).to_owned(),
                publisher: Some(download.publisher),
                repo: Some(download.repo),
                file: Some(download.file),
                size_bytes: record.map_or(download.size_bytes, |record| record.size_bytes),
                path: record.map(|record| shown(&record.path)),
                key,
            });
        }
    }

    // A record whose pin has since been withdrawn still describes a file on
    // disk the user can run. Dropping it here would make a model vanish from
    // the advisor while its gigabytes stayed put.
    for record in present {
        if entries.iter().any(|entry| entry.key == record.key) {
            continue;
        }
        entries.push(ModelCatalogueEntry {
            key: record.key,
            state: "downloaded".to_owned(),
            publisher: Some(record.publisher),
            repo: Some(record.repo),
            file: record
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            size_bytes: record.size_bytes,
            path: Some(shown(&record.path)),
        });
    }

    entries
}

/// Which of the three actionable states a cell is in.
fn state_of(key: &ModelKey, downloaded: bool, running: Option<&ModelKey>) -> &'static str {
    if downloaded {
        "downloaded"
    } else if running == Some(key) {
        "downloading"
    } else {
        "downloadable"
    }
}

/// Starts downloading a pinned model, reporting through `model:*` events.
///
/// Returns as soon as the work is spawned rather than when it finishes: a model
/// is gigabytes, and holding the IPC call open for its duration would freeze
/// the command channel the rest of the app uses. This is the same arrangement
/// [`crate::runtime::acquire_runtime`] and [`crate::chat::chat_send`] use.
///
/// # Errors
///
/// If the cell has no pinned file, another download is already running, the
/// folder cannot be created, or there is not enough room — the last of these
/// **before any HTTP request is made**, naming the shortfall. Failures *during*
/// the download arrive on `model:failed`, not here.
#[tauri::command]
pub fn models_download(
    app: AppHandle,
    state: State<'_, ModelState>,
    model_id: String,
    quant_id: String,
) -> Result<(), String> {
    let key = ModelKey { model_id, quant_id };
    let download = pinned(&key)
        .ok_or_else(|| format!("no file is pinned for {} at {}", key.model_id, key.quant_id))?;

    let folder = state.folder();
    ensure_folder(&folder)?;

    let destination = folder.join(&download.file);
    let part = destination.with_extension(PART_SUFFIX);

    // Only the bytes that are still missing have to fit: a resumed download's
    // part file is already occupying its share of the disk, and demanding the
    // whole size again would refuse a download that would in fact complete.
    let already = std::fs::metadata(&part).map_or(0, |data| data.len());
    let outstanding = download.size_bytes.saturating_sub(already);

    // Before any request, as `acquire.rs` does. A refusal that arrives after
    // 20 GB have been fetched is not a refusal, it is a waste.
    require_space(&folder, outstanding).map_err(|error| error.to_string())?;

    state.claim(Busy {
        key: Some(key.clone()),
        what: download.file.clone(),
    })?;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut held) = state.cancel.lock() {
        *held = Some(sender);
    }

    let store = state.store();
    let url = download_url(&download.repo, &download.file);

    tauri::async_runtime::spawn(async move {
        let outcome = fetch(&app, &url, &part, &destination, &download, &key, receiver).await;

        match outcome {
            Ok(()) => {
                let recorded = store.record(ModelRecord {
                    key: key.clone(),
                    path: destination.clone(),
                    size_bytes: download.size_bytes,
                    sha256: download.sha256,
                    publisher: download.publisher,
                    repo: download.repo,
                });

                // A verified file the index does not know about is a file the
                // user cannot run and cannot find, so the write failing is
                // reported rather than swallowed.
                let _ = match recorded {
                    Ok(()) => app.emit(
                        MODEL_DONE_EVENT,
                        ModelDone {
                            key: Some(key),
                            path: shown(&destination),
                        },
                    ),
                    Err(error) => app.emit(MODEL_FAILED_EVENT, ModelFailure::of(Some(key), &error)),
                };
            }
            Err(failure) => {
                let _ = app.emit(MODEL_FAILED_EVENT, failure);
            }
        }

        if let Some(state) = app.try_state::<ModelState>() {
            state.release();
        }
    });

    Ok(())
}

/// Fetches one pinned file, stopping early if the user asks.
///
/// Cancelling drops the transfer mid-write, which leaves the part file exactly
/// as far along as it got — that is what makes stopping cheap rather than
/// destructive.
async fn fetch(
    app: &AppHandle,
    url: &str,
    part: &Path,
    destination: &Path,
    download: &ModelDownload,
    key: &ModelKey,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), ModelFailure> {
    let client = reqwest::Client::new();
    let emitter = app.clone();
    let reported = key.clone();
    let mut on_progress = move |progress: Progress| {
        let _ = emitter.emit(
            MODEL_PROGRESS_EVENT,
            ModelProgress {
                key: Some(reported.clone()),
                phase: "downloading".to_owned(),
                downloaded_bytes: progress.downloaded_bytes,
                total_bytes: progress.total_bytes,
            },
        );
    };

    let downloading = download_resumable(
        &client,
        url,
        part,
        destination,
        &download.sha256,
        download.size_bytes,
        &mut on_progress,
    );

    tokio::select! {
        finished = downloading => finished.map_err(|error| ModelFailure::of(Some(key.clone()), &error)),
        // The sender being dropped means nothing can ever stop this download,
        // which is not the same as being stopped. Answering `Ok` there would
        // abandon the transfer and then record a file that never landed, so
        // that branch never resolves and lets the other one decide -- the same
        // arrangement `chat.rs` uses for a reply nothing can cancel.
        stopped = cancel => match stopped {
            Ok(()) => Err(ModelFailure::cancelled(key.clone())),
            Err(_) => downloading_forever().await,
        },
    }
}

/// A future that never resolves, for a cancel channel that can never fire.
///
/// `select!` needs both branches to produce the same type, and a dropped sender
/// must not be mistaken for a finished download.
async fn downloading_forever() -> Result<(), ModelFailure> {
    std::future::pending().await
}

/// Stops the download currently running, if there is one.
///
/// The partial file is kept, so starting the same download again resumes from
/// where it stopped rather than from zero.
#[tauri::command]
pub fn models_cancel(state: State<'_, ModelState>) {
    if let Ok(mut held) = state.cancel.lock()
        && let Some(sender) = held.take()
    {
        // Fails only if the download task has already finished, which means
        // there is nothing left to stop.
        let _ = sender.send(());
    }
}

/// Deletes a downloaded model: the file, and the record naming it.
///
/// Both, because osstat is a disk cleaner. A record left behind would offer a
/// Run control for bytes that are gone, and a file left behind would be
/// gigabytes nothing in the app admits to.
///
/// # Errors
///
/// If the file cannot be removed, or the index cannot be written.
#[tauri::command]
pub fn models_delete(
    state: State<'_, ModelState>,
    model_id: String,
    quant_id: String,
) -> Result<(), String> {
    let key = ModelKey { model_id, quant_id };
    let store = state.store();

    if let Some(path) = store.path_of(&key)
        && let Err(error) = std::fs::remove_file(&path)
    {
        return Err(format!("{} could not be deleted: {error}", path.display()));
    }

    store.forget(&key).map_err(|error| error.to_string())
}

/// Where downloaded models are kept.
///
/// Reported without creating anything: the folder comes into existence on the
/// first download or move, and merely opening Settings should not leave an
/// empty directory behind.
#[tauri::command]
#[must_use]
pub fn models_folder(state: State<'_, ModelState>) -> String {
    shown(&state.folder())
}

/// Chooses where downloaded models are kept from now on.
///
/// Changes nothing already on disk. Moving what is there is
/// [`models_move`], which is a separate, explicit action because it can take an
/// hour.
#[tauri::command]
pub fn models_set_folder(state: State<'_, ModelState>, path: String) {
    state.set_folder(PathBuf::from(path));
}

/// States what moving the library to `path` would cost, without moving it.
#[tauri::command]
#[must_use]
pub fn models_plan_move(state: State<'_, ModelState>, path: String) -> LibraryMovePlan {
    plan_move(&state.store(), Path::new(&path)).into()
}

/// Moves every downloaded model into `path`, reporting through `model:*`.
///
/// Returns as soon as the work is spawned, for the same reason
/// [`models_download`] does: a cross-volume move of a full library is an
/// hours-long operation.
///
/// # Errors
///
/// If a download or another move is already running, or the folder cannot be
/// created. Failures *during* the move arrive on `model:failed`, not here.
#[tauri::command]
pub fn models_move(
    app: AppHandle,
    state: State<'_, ModelState>,
    path: String,
) -> Result<(), String> {
    let destination = PathBuf::from(path);
    ensure_folder(&destination)?;

    state.claim(Busy {
        key: None,
        what: "moving the model library".to_owned(),
    })?;

    // Set before the move rather than after it. A library that moved while new
    // downloads still land in the old folder is the worst of both, and it is
    // what a failure halfway through would otherwise leave behind.
    state.set_folder(destination.clone());

    let mut store = state.store();

    tauri::async_runtime::spawn(async move {
        let emitter = app.clone();
        let mut on_progress = move |progress: Progress| {
            let _ = emitter.emit(
                MODEL_PROGRESS_EVENT,
                ModelProgress {
                    key: None,
                    phase: "moving".to_owned(),
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: progress.total_bytes,
                },
            );
        };

        let _ = match move_library(&mut store, &destination, &mut on_progress).await {
            Ok(()) => app.emit(
                MODEL_DONE_EVENT,
                ModelDone {
                    key: None,
                    path: shown(&destination),
                },
            ),
            Err(error) => app.emit(MODEL_FAILED_EVENT, ModelFailure::of(None, &error)),
        };

        if let Some(state) = app.try_state::<ModelState>() {
            state.release();
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A key for a cell the seed registry actually pins.
    fn sample_key() -> ModelKey {
        ModelKey {
            model_id: "qwen2.5-0.5b".to_owned(),
            quant_id: "Q4_K_M".to_owned(),
        }
    }

    #[test]
    fn the_seed_registry_pins_a_file_for_the_smallest_model() {
        // If this stops being true the advisor shows no download control at
        // all, which reads as the feature being broken rather than as a pin
        // being withdrawn.
        let download = pinned(&sample_key()).expect("qwen2.5-0.5b Q4_K_M is pinned");

        assert_eq!(
            Path::new(&download.file)
                .extension()
                .and_then(|e| e.to_str()),
            Some("gguf")
        );
        assert!(download.size_bytes > 0);
        assert_eq!(download.sha256.len(), 64);
    }

    #[test]
    fn a_cell_nobody_pinned_has_no_file_to_offer() {
        assert!(
            pinned(&ModelKey {
                model_id: "qwen2.5-0.5b".to_owned(),
                quant_id: "not-a-quantization".to_owned(),
            })
            .is_none()
        );
    }

    #[test]
    fn a_downloaded_cell_reports_downloaded_whatever_else_is_running() {
        let key = sample_key();

        assert_eq!(state_of(&key, true, None), "downloaded");
        assert_eq!(state_of(&key, true, Some(&key)), "downloaded");
    }

    #[test]
    fn only_the_cell_actually_running_reports_downloading() {
        // A spinner on every cell at once would say the app is fetching seven
        // models when it is fetching one.
        let running = sample_key();
        let other = ModelKey {
            model_id: "llama-3.2-1b".to_owned(),
            quant_id: "Q4_K_M".to_owned(),
        };

        assert_eq!(state_of(&running, false, Some(&running)), "downloading");
        assert_eq!(state_of(&other, false, Some(&running)), "downloadable");
        assert_eq!(state_of(&other, false, None), "downloadable");
    }

    #[test]
    fn the_work_slot_refuses_a_second_download_naming_the_first() {
        // The message is the requirement: "a download is already running" does
        // not tell the user which one, and the answer decides whether they
        // wait or stop it.
        let directory = tempfile::tempdir().unwrap();
        let state = ModelState::new(directory.path().to_path_buf());

        state
            .claim(Busy {
                key: Some(sample_key()),
                what: "Qwen2.5-0.5B-Instruct-Q4_K_M.gguf".to_owned(),
            })
            .unwrap();
        let refusal = state
            .claim(Busy {
                key: None,
                what: "moving the model library".to_owned(),
            })
            .unwrap_err();

        assert!(
            refusal.contains("Qwen2.5-0.5B-Instruct-Q4_K_M.gguf"),
            "the refusal did not name the file in progress: {refusal}"
        );
    }

    #[test]
    fn the_work_slot_is_free_again_once_released() {
        let directory = tempfile::tempdir().unwrap();
        let state = ModelState::new(directory.path().to_path_buf());

        state
            .claim(Busy {
                key: Some(sample_key()),
                what: "a.gguf".to_owned(),
            })
            .unwrap();
        state.release();

        assert!(
            state
                .claim(Busy {
                    key: None,
                    what: "b.gguf".to_owned()
                })
                .is_ok(),
            "a finished download left the slot held forever"
        );
        assert_eq!(state.downloading(), None, "the released cell still reports");
    }

    #[test]
    fn an_unset_folder_means_a_models_directory_under_app_data() {
        let directory = tempfile::tempdir().unwrap();
        let state = ModelState::new(directory.path().to_path_buf());

        assert_eq!(state.folder(), directory.path().join(DEFAULT_FOLDER));
    }

    #[test]
    fn a_chosen_folder_replaces_the_default_and_leaves_the_index_where_it_was() {
        // The index must not follow the files: the folder is the thing the user
        // may point at a removable drive, and the record of where everything
        // went has to outlive that.
        let directory = tempfile::tempdir().unwrap();
        let elsewhere = directory.path().join("on-the-big-disk");
        let state = ModelState::new(directory.path().to_path_buf());

        state.set_folder(elsewhere.clone());

        assert_eq!(state.folder(), elsewhere);
        assert_eq!(
            state.store().index_path(),
            directory.path().join(INDEX_FILE)
        );
    }

    #[test]
    fn asking_where_models_live_creates_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let state = ModelState::new(directory.path().to_path_buf());

        let folder = state.folder();

        assert!(!folder.exists(), "a query created a directory");
    }

    #[test]
    fn a_checksum_mismatch_reaches_the_ui_as_unretryable_and_a_verification_failure() {
        // The user-facing half of the security control. Both flags matter: one
        // hides the retry button, the other changes what the message says.
        let failure = ModelFailure::of(
            Some(sample_key()),
            &AcquireError::ChecksumMismatch {
                file: "model.gguf".to_owned(),
                expected: "a".repeat(64),
                actual: "b".repeat(64),
            },
        );

        assert!(!failure.retryable);
        assert!(failure.verification_failure);
        assert!(!failure.cancelled);
        assert!(failure.message.contains("model.gguf"));
    }

    #[test]
    fn a_size_that_disagrees_with_the_pin_is_a_verification_failure_too() {
        // The upload changed. Fetching it again returns the same wrong file, so
        // this must not be offered as a retry.
        let failure = ModelFailure::of(
            Some(sample_key()),
            &AcquireError::SizeMismatch {
                file: "model.gguf".to_owned(),
                expected_bytes: 400_000_000,
                actual_bytes: 12,
            },
        );

        assert!(!failure.retryable);
        assert!(failure.verification_failure);
    }

    #[test]
    fn a_full_disk_names_the_shortfall_and_is_not_a_verification_failure() {
        let failure = ModelFailure::of(
            None,
            &AcquireError::NotEnoughSpace {
                path: PathBuf::from("/models"),
                needed_bytes: 400_000_000,
                available_bytes: 4_096,
            },
        );

        assert!(failure.message.contains("400000000"), "{}", failure.message);
        assert!(failure.message.contains("4096"), "{}", failure.message);
        assert!(!failure.verification_failure);
    }

    #[test]
    fn a_stopped_download_is_offered_as_resumable_rather_than_broken() {
        let failure = ModelFailure::cancelled(sample_key());

        assert!(failure.cancelled);
        assert!(failure.retryable, "a stopped download resumes");
        assert!(!failure.verification_failure);
    }

    #[test]
    fn a_plan_that_would_move_nothing_still_answers() {
        let plan = LibraryMovePlan::from(MovePlan {
            files: 0,
            bytes: 0,
            same_volume: true,
        });

        assert_eq!(plan.files, 0);
        assert_eq!(plan.bytes, 0);
        assert!(plan.same_volume);
    }

    #[test]
    fn every_payload_is_camel_case() {
        let progress = serde_json::to_value(ModelProgress {
            key: Some(sample_key()),
            phase: "downloading".to_owned(),
            downloaded_bytes: 10,
            total_bytes: 100,
        })
        .unwrap();
        assert!(
            progress
                .as_object()
                .unwrap()
                .contains_key("downloadedBytes")
        );
        assert!(progress.as_object().unwrap().contains_key("totalBytes"));
        assert_eq!(
            progress["key"]["modelId"],
            serde_json::json!("qwen2.5-0.5b")
        );
        assert_eq!(progress["key"]["quantId"], serde_json::json!("Q4_K_M"));

        let failure = serde_json::to_value(ModelFailure::cancelled(sample_key())).unwrap();
        let object = failure.as_object().unwrap();
        assert!(object.contains_key("verificationFailure"));
        assert!(object.contains_key("cancelled"));

        let plan = serde_json::to_value(LibraryMovePlan {
            files: 2,
            bytes: 3072,
            same_volume: false,
        })
        .unwrap();
        assert!(plan.as_object().unwrap().contains_key("sameVolume"));

        let entry = serde_json::to_value(ModelCatalogueEntry {
            key: sample_key(),
            state: "downloadable".to_owned(),
            publisher: Some("bartowski".to_owned()),
            repo: Some("bartowski/Qwen2.5-0.5B-Instruct-GGUF".to_owned()),
            file: Some("a.gguf".to_owned()),
            size_bytes: 400,
            path: None,
        })
        .unwrap();
        assert!(entry.as_object().unwrap().contains_key("sizeBytes"));
    }

    #[test]
    fn a_move_reports_no_cell_because_it_is_not_about_one() {
        // The UI keys progress by cell. A move borrowing some arbitrary cell's
        // identity would drive a bar on a model that is not being touched.
        let progress = ModelProgress {
            key: None,
            phase: "moving".to_owned(),
            downloaded_bytes: 0,
            total_bytes: 0,
        };

        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["key"], serde_json::json!(null));
        assert_eq!(progress.phase, "moving");
    }
}
