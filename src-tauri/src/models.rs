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
//! `tauri.conf.json` an unweakened control (SECURITY.md threat 3). Search does
//! not change that: [`models_search`] takes a term and returns results, and the
//! requests behind it are made here.
//!
//! There is still **no command that takes a URL**.
//! [`models_download_searched`] takes a [`SearchResult`], whose repository and
//! file are re-validated against the same predicates the search filtered on
//! before either is interpolated into one — because by the time the value comes
//! back it has been through the webview, and a repository and a file that were
//! not checked would be a URL by another name.
//!
//! Two verification tiers now exist and they stay visibly different all the way
//! to the model list. A pinned file is checked against a hash reviewed in a pull
//! request against this repository; a searched one against the hash Hugging Face
//! reports beside it, which detects a corrupted transfer and not a replaced
//! upload. [`Provenance`] rides on every catalogue entry so the UI can say
//! which — see SECURITY.md threat 5.

// Tauri resolves `AppHandle` and `State<'_, T>` by injecting them by value; a
// reference is not part of the command signature it accepts. Both are cheap
// handles, so the lint's concern does not apply. Same reason as `commands.rs`.
#![allow(clippy::needless_pass_by_value)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use osstat_inference::{
    AcquireError, ModelKey, ModelRecord, ModelStore, MovePlan, Progress, Provenance,
    SearchProjector, SearchResult, download_resumable_retrying, download_url, move_library,
    plan_move, require_space, search,
};
use osstat_llm::registry::{ModelDownload, ProjectorDownload, seeded_registry};
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

/// How long transfer rate and time remaining are averaged over.
///
/// Short on purpose. Averaged over the whole transfer, a connection that
/// stopped ten seconds ago still reports the speed it was managing a minute
/// before that, and the bar reads as healthy while nothing is arriving. Over a
/// few seconds a stall is visible as the stall it is.
const RATE_WINDOW: Duration = Duration::from_secs(5);

/// How many progress samples the window holds at most.
///
/// Progress arrives once per chunk, which on a fast link is thousands of times
/// a second. The cap keeps the deque small without changing the answer: the
/// rate is computed from the two ends of the window and nothing in between.
const RATE_SAMPLES: usize = 64;

/// How many repositories one search consults.
///
/// Small on purpose. Each repository costs a second request, and one usually
/// holds the same model at a dozen quantizations — so a larger figure buys a
/// longer wait and a list nobody reads to the end of, not better answers.
const SEARCH_LIMIT: usize = 8;

/// How many times a cancelled download's partial file is chased.
const DISCARD_ATTEMPTS: u32 = 5;

/// How long to wait between those attempts.
const DISCARD_PAUSE: Duration = Duration::from_millis(50);

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
    /// Which verification tier this file was fetched under.
    ///
    /// Carried all the way to the model list rather than shown only on the
    /// control that started the download: once the bytes are on disk nothing
    /// about them says whether the hash they were checked against had been
    /// reviewed by a person, and a list that showed both tiers identically
    /// would retire a guarantee SECURITY.md still makes.
    pub provenance: Provenance,
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
    /// How fast bytes are currently arriving, over the last few seconds.
    ///
    /// `None` until there are two samples to measure between, and for a
    /// library move — a same-volume move renames rather than copies, so a byte
    /// rate would describe an operation that never touched the bytes.
    ///
    /// Deliberately not an average over the whole transfer: see
    /// [`RATE_WINDOW`].
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub bytes_per_second: Option<u64>,
    /// How much longer the transfer has, at the current rate.
    ///
    /// `None` when there is no rate to divide by, and — importantly — when the
    /// rate has fallen to zero. A stalled transfer has no honest estimate, and
    /// a number that silently stopped changing would be read as one that is
    /// still being computed.
    #[cfg_attr(feature = "ts-bindings", ts(type = "number | null"))]
    pub seconds_remaining: Option<u64>,
}

/// How fast a transfer is going, and how much longer it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Pace {
    /// Bytes per second over [`RATE_WINDOW`], once there is enough to say.
    bytes_per_second: Option<u64>,
    /// Seconds left at that rate, unless the rate is zero.
    seconds_remaining: Option<u64>,
}

/// The last few progress samples, so the rate describes now rather than ever.
struct Rate {
    /// When each sample arrived, and how many bytes had landed by then.
    samples: VecDeque<(Instant, u64)>,
}

impl Rate {
    /// An empty window, which reports nothing until it has two samples.
    const fn new() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }

    /// Records a sample and answers the rate and estimate it implies.
    ///
    /// Measured between the two ends of the window rather than from the start
    /// of the transfer, which is what makes a stall show up as a rate of zero
    /// instead of being averaged away by the minutes that went well.
    fn observe(&mut self, at: Instant, downloaded_bytes: u64, total_bytes: u64) -> Pace {
        self.samples.push_back((at, downloaded_bytes));

        // Two are always kept, however old. A transfer that has been silent
        // for a minute must still be able to report the zero that says so,
        // and an empty window would report `None` — indistinguishable from a
        // download that has only just started.
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|(when, _)| at.saturating_duration_since(*when) > RATE_WINDOW)
        {
            self.samples.pop_front();
        }
        while self.samples.len() > RATE_SAMPLES {
            self.samples.pop_front();
        }

        let (Some(&(oldest_at, oldest_bytes)), Some(&(newest_at, newest_bytes))) =
            (self.samples.front(), self.samples.back())
        else {
            return Pace::default();
        };

        let nanos = newest_at.saturating_duration_since(oldest_at).as_nanos();
        if self.samples.len() < 2 || nanos == 0 {
            return Pace::default();
        }

        // Integer throughout: the figures are bytes and nanoseconds, both
        // whole, and a float here would only add a rounding step to a number
        // that is displayed to three significant figures anyway.
        let moved = u128::from(newest_bytes.saturating_sub(oldest_bytes));
        let bytes_per_second =
            u64::try_from(moved.saturating_mul(1_000_000_000) / nanos).unwrap_or(u64::MAX);

        Pace {
            bytes_per_second: Some(bytes_per_second),
            seconds_remaining: (bytes_per_second > 0)
                .then(|| total_bytes.saturating_sub(newest_bytes) / bytes_per_second),
        }
    }
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
    /// How the user stopped it, or `None` if something actually went wrong.
    ///
    /// One field rather than a `cancelled` flag and a `paused` flag beside it,
    /// because two booleans would spell a state that cannot happen — paused but
    /// not stopped — and the UI would have to decide what to render for it.
    pub stopped: Option<Stop>,
}

/// Why a download is stopping, and therefore what becomes of the `.part`.
///
/// The two share everything — the same channel, the same signal, the same
/// wind-down — and differ by **one line**, the removal of the partial file in
/// [`fetch`]. They are one enum rather than two mechanisms so that the
/// difference is visible in one place instead of being spread across two
/// near-identical code paths that could drift apart.
///
/// It crosses to the webview because the difference outlives the stop: after a
/// pause the figure the bar reached still describes bytes on disk, and after a
/// cancel it describes bytes that are gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum Stop {
    /// Keep the `.part`, so starting the same download again resumes from it.
    Pause,
    /// Remove the `.part`, so nothing is left occupying the disk.
    Cancel,
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
            stopped: None,
        }
    }

    /// Describes the user stopping a download of `key`, either way.
    fn stopped_by(key: ModelKey, stop: Stop) -> Self {
        Self {
            key: Some(key),
            message: match stop {
                Stop::Pause => {
                    "the download was paused; resuming continues from where it stopped".to_owned()
                }
                Stop::Cancel => {
                    "the download was cancelled; what had already arrived was deleted".to_owned()
                }
            },
            // Starting it again is exactly the right response to both, which is
            // what this flag is asked in order to decide. What differs is where
            // it starts from, and that is [`Self::stopped`].
            retryable: true,
            verification_failure: false,
            stopped: Some(stop),
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
    /// How to stop the download currently running, and which way.
    cancel: Mutex<Option<tokio::sync::oneshot::Sender<Stop>>>,
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

    /// Asks the running download to stop, `stop` saying what to do with the
    /// partial file. Does nothing if there is none.
    fn ask_to_stop(&self, stop: Stop) {
        if let Ok(mut held) = self.cancel.lock()
            && let Some(sender) = held.take()
        {
            // Fails only if the download task has already finished, which means
            // there is nothing left to stop.
            let _ = sender.send(stop);
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

/// One projector to fetch: where it lives upstream and what it must hash to.
///
/// The two tiers name a projector differently — a pinned one carries its own
/// repository, a searched one takes its parent result's — and this is the shape
/// they meet in, so [`fetch_projector`] is one function rather than two that
/// could drift apart on the ordering or the cancel channel.
struct RemoteProjector<'a> {
    /// The Hugging Face repository, as `owner/name`.
    repo: &'a str,
    /// The path of the projector within that repository.
    file: &'a str,
    /// What the transfer is checked against. Which tier the hash came from is
    /// [`Provenance`]'s business, not this function's.
    sha256: &'a str,
    /// Its size, for the progress figures the transfer reports.
    size_bytes: u64,
}

impl<'a> RemoteProjector<'a> {
    /// The projector a registry pin names, which carries its own repository.
    fn pinned(projector: &'a ProjectorDownload) -> Self {
        Self {
            repo: &projector.repo,
            file: &projector.file,
            sha256: &projector.sha256,
            size_bytes: projector.size_bytes,
        }
    }

    /// The projector a search found, which takes its parent result's
    /// repository — a searched projector never names one of its own, and that
    /// is what keeps this tier's single origin single.
    fn searched(repo: &'a str, projector: &'a SearchProjector) -> Self {
        Self {
            repo,
            file: &projector.file,
            sha256: &projector.sha256,
            size_bytes: projector.size_bytes,
        }
    }
}

/// How many of `size_bytes` are still to fetch, given what `part` already holds.
///
/// Only the missing bytes have to fit: a resumed download's part file is
/// already occupying its share of the disk, and demanding the whole size again
/// would refuse a download that would in fact complete.
fn outstanding_bytes(part: &Path, size_bytes: u64) -> u64 {
    let already = std::fs::metadata(part).map_or(0, |data| data.len());
    size_bytes.saturating_sub(already)
}

/// Fetches the projector beside weights that have already landed and verified.
///
/// A function of its own rather than a block inside [`models_download`]: the
/// projector is a second transfer needing a second cancel channel, and inlining
/// that made the ordering the comment at the call site promises — weights
/// first, always — the least visible thing in a very long function.
async fn fetch_projector(
    app: &AppHandle,
    projector: &RemoteProjector<'_>,
    destination: &Path,
    part: &Path,
    key: &ModelKey,
) -> Result<(), ModelFailure> {
    // A second channel: the first was consumed by the download of the weights,
    // and a Pause the user presses during the projector has to reach something.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut held) = app.state::<ModelState>().cancel.lock() {
        *held = Some(sender);
    }

    fetch(
        app,
        &download_url(projector.repo, projector.file),
        part,
        destination,
        Expected {
            sha256: projector.sha256,
            size_bytes: projector.size_bytes,
        },
        key,
        receiver,
    )
    .await
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

/// The multimodal projector downloaded alongside `model`, if there is one.
///
/// Read from the index rather than from the registry: a pin can be withdrawn or
/// re-quantized after the download, and a file already on disk has to keep
/// working. Nothing here consults the model's *name* — "vl" in a filename is a
/// guess, and the record is a fact.
///
/// A record naming a projector that is no longer on disk answers `None`. The
/// user deleted it, or a move left it behind; either way `--mmproj` pointing at
/// nothing stops the server from starting at all, and a model that runs
/// text-only is strictly better than a model that will not run.
pub(crate) fn projector_for(root: &Path, model: &Path) -> Option<PathBuf> {
    ModelStore::new(root.join(INDEX_FILE))
        .records()
        .into_iter()
        .find(|record| record.path == model)?
        .projector_path
        .filter(|projector| projector.is_file())
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
                // A cell of the curated matrix is pinned whether or not it has
                // been downloaded: it is the registry's row, not a record's.
                provenance: Provenance::Pinned,
                key,
            });
        }
    }

    // A record whose pin has since been withdrawn still describes a file on
    // disk the user can run. Dropping it here would make a model vanish from
    // the advisor while its gigabytes stayed put. Every searched model arrives
    // through this branch too — it occupies no cell of the curated matrix, so
    // there is nothing above for it to join against.
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
            // From the record, which is the only thing that still remembers.
            provenance: record.provenance,
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

    // A vision model's projector lands beside its weights and is verified on
    // the same terms. Resolved here, before the space check, because both
    // files have to fit or neither should start.
    let projector = download.projector.clone().map(|projector| {
        let destination = folder.join(&projector.file);
        let part = destination.with_extension(PART_SUFFIX);
        (projector, destination, part)
    });

    // Both files or neither: a vision model whose projector will not fit is
    // not a model that fits.
    let outstanding = outstanding_bytes(&part, download.size_bytes).saturating_add(
        projector.as_ref().map_or(0, |(projector, _, part)| {
            outstanding_bytes(part, projector.size_bytes)
        }),
    );

    // Before any request, as `acquire.rs` does. A refusal that arrives after
    // 20 GB have been fetched is not a refusal, it is a waste.
    require_space(&folder, outstanding).map_err(|error| {
        crate::log::download_failed(error.kind());
        error.to_string()
    })?;

    state.claim(Busy {
        key: Some(key.clone()),
        what: download.file.clone(),
    })?;

    // The size, never the file. Which of two downloads a later line refers to
    // is not recoverable from the log, and that is the accepted cost of the
    // design's §2 rather than an oversight.
    crate::log::download_started(download.size_bytes);
    let started = std::time::Instant::now();

    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut held) = state.cancel.lock() {
        *held = Some(sender);
    }

    let store = state.store();
    let url = download_url(&download.repo, &download.file);

    tauri::async_runtime::spawn(async move {
        let outcome = fetch(
            &app,
            &url,
            &part,
            &destination,
            Expected {
                sha256: &download.sha256,
                size_bytes: download.size_bytes,
            },
            &key,
            receiver,
        )
        .await;

        // The projector is fetched only once the weights have landed and
        // verified. The other order would leave, on a cancel between the two,
        // a projector on disk belonging to no model — and a half-downloaded
        // vision model is better described by the file that is missing than by
        // the one that is not.
        let outcome = match (outcome, projector.as_ref()) {
            (Ok(()), Some((projector, destination, part))) => {
                let remote = RemoteProjector::pinned(projector);
                fetch_projector(&app, &remote, destination, part, &key).await
            }
            (outcome, _) => outcome,
        };

        match outcome {
            Ok(()) => {
                let fetched = projector
                    .as_ref()
                    .map_or(download.size_bytes, |(projector, _, _)| {
                        download.size_bytes.saturating_add(projector.size_bytes)
                    });
                crate::log::download_finished(fetched, started.elapsed().as_secs());

                let recorded = store.record(ModelRecord {
                    key: key.clone(),
                    path: destination.clone(),
                    size_bytes: download.size_bytes,
                    sha256: download.sha256,
                    publisher: download.publisher,
                    repo: download.repo,
                    // Everything the seed registry pins was reviewed in a pull
                    // request against this repository.
                    provenance: Provenance::Pinned,
                    projector_path: projector
                        .as_ref()
                        .map(|(_, destination, _)| destination.clone()),
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
                    Err(error) => {
                        // A verified file the index does not know about is the
                        // one failure here that leaves gigabytes on disk the
                        // user cannot find, so it is logged as well as emitted.
                        crate::log::download_failed(error.kind());
                        app.emit(MODEL_FAILED_EVENT, ModelFailure::of(Some(key), &error))
                    }
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

/// Searches Hugging Face for downloadable GGUF files.
///
/// **The webview makes no request here either.** It hands over a term and gets
/// back results; every byte still moves in Rust, which is what keeps the CSP in
/// `tauri.conf.json` an unweakened control (SECURITY.md threat 3).
///
/// Results from here are a **weaker tier** than the pinned seven: the hash they
/// carry is the one Hugging Face reports beside the file, so verifying it
/// detects a corrupted transfer and not a replaced upload. The UI has to say so
/// — see [`Provenance`].
///
/// # Errors
///
/// If the search term is empty, or Hugging Face could not be reached. A
/// response that will not parse yields no results rather than an error.
#[tauri::command]
pub async fn models_search(query: String) -> Result<Vec<SearchResult>, String> {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Err("type something to search for".to_owned());
    }

    let started = std::time::Instant::now();
    let client = reqwest::Client::new();

    match search(&client, &query, SEARCH_LIMIT).await {
        Ok(results) => {
            // The count and the duration. Never the term, and never a
            // repository or file name: what someone went looking for is the
            // most obviously personal thing this app touches.
            crate::log::search_finished(results.len(), started.elapsed().as_secs());
            Ok(results)
        }
        Err(error) => {
            crate::log::search_failed(error.kind());
            Err(error.to_string())
        }
    }
}

/// What the advisor makes of a searched file, having read its header.
///
/// Every field comes out of [`osstat_chat::plan_launch`] or out of the header it
/// was given. Nothing here is derived from the file size and a quantization tag
/// — that guess is what ADR-008 names as the worst thing this feature could do,
/// and the reason it is not needed is that the header was read rather than
/// guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SearchedFit {
    /// Layers that would be offloaded to the GPU — what `-ngl` would be given.
    pub gpu_layers: u32,
    /// The model's transformer block count, so a partial offload can be shown
    /// as the fraction it is rather than as a bare number.
    pub block_count: u32,
    /// The context window that would be allocated.
    pub context_length: u32,
    /// Whether every layer fitted. `false` is a warning, never a refusal.
    pub fits: bool,
    /// Whether the KV-cache arithmetic rested on a derived head dimension.
    ///
    /// Travels for the same reason it does on [`crate::chat::ModelSession`]: a
    /// derived figure is right for standard attention and wrong for models that
    /// diverge, and a mis-sized cache is otherwise a mystery rather than a
    /// diagnosis.
    pub head_dim_derived: bool,
}

/// Prices one searched file, by reading the header off the front of it.
///
/// **This is the same verdict a downloaded model gets.** A GGUF header sits at
/// the start of the file, so the architecture can be fetched with a `Range`
/// request without the weights behind it, and then handed to the very
/// [`osstat_chat::plan_launch`] that [`crate::chat::chat_open_model`] hands a
/// locally-read header to. There is one calculator and one launch plan in the
/// codebase; a searched model and the same file once downloaded cannot disagree,
/// because nothing here does any arithmetic of its own.
///
/// Called **per result and on demand**, never for a whole page of them: this is
/// a network request against a multi-gigabyte file, and making one for every row
/// a search returned would spend a dozen round trips on rows nobody looked at.
///
/// `result` has been through the webview, so it is checked again rather than
/// trusted — the same reason [`models_download_searched`] checks it. A `Range`
/// request is still a request, and a repository and file interpolated into a URL
/// without validation would be the URL-taking command ADR-012 rests on there not
/// being.
///
/// # Errors
///
/// If the result is not one a search could have produced, the GPU probe has not
/// finished, the file could not be reached, or its header is not readable within
/// the ceiling — the last being what a server that ignores `Range` produces. The
/// caller shows the size alone in every one of those cases.
#[tauri::command]
pub async fn models_price_searched(
    sampler: State<'_, crate::sampler::Sampler>,
    result: SearchResult,
) -> Result<SearchedFit, String> {
    if !result.is_well_formed() {
        return Err("that result cannot be priced: it is not a whole, hashed GGUF file from a Hugging Face repository".to_owned());
    }

    // Read before the request rather than after it, for the same reason
    // `llm_advice` and `chat_open_model` refuse to answer early: pricing a model
    // against "no GPU" on a machine that has one is a confident wrong answer,
    // and it would be the confident wrong answer the user acts on.
    let devices = sampler.devices().ok_or_else(|| {
        crate::log::header_fetch_failed("probe_unfinished");
        "the GPU probe has not finished yet".to_owned()
    })?;
    let budget = osstat_llm::calculator::select_gpu_budget(&devices);

    let url = download_url(&result.repo, &result.file);
    let started = Instant::now();

    let model = osstat_chat::fetch_header(&reqwest::Client::new(), &url)
        .await
        .map_err(|error| {
            // The kind and nothing else. The URL this failed on is built from a
            // repository and a file name, which is what the user searched for.
            crate::log::header_fetch_failed(error.kind());
            error.to_string()
        })?;

    crate::log::header_fetched(started.elapsed().as_secs());

    // The size the search reported, which is the size the loader will read —
    // the same figure `chat_open_model` takes from the file's metadata.
    let plan = osstat_chat::plan_launch(&model, result.size_bytes, budget);

    Ok(SearchedFit {
        gpu_layers: plan.gpu_layers,
        block_count: model.block_count,
        context_length: plan.context_length,
        fits: plan.fits,
        head_dim_derived: model.head_dim_derived,
    })
}

/// The cell a searched file occupies in the index.
///
/// Repository and file name, which cannot collide with a registry cell because
/// a repository always contains a slash and a registry model id never does.
/// That is what lets one [`ModelStore`] hold both tiers, and one
/// [`models_delete`] remove either.
fn searched_key(result: &SearchResult) -> ModelKey {
    ModelKey {
        model_id: result.repo.clone(),
        quant_id: result.file.clone(),
    }
}

/// Starts downloading a searched model, reporting through `model:*` events.
///
/// The same transfer as [`models_download`] in every respect that matters —
/// [`download_resumable_retrying`] unchanged, so pause, resume, the bounded
/// backoff and the free-space check all apply — and the same refusal on a hash
/// that does not match. What differs is [`ModelRecord::provenance`], which
/// records that the hash came from the same origin as the file.
///
/// `result` has been through the webview, so it is checked again rather than
/// trusted. ADR-012 rests on there being no command that takes a URL, and a
/// repository and file interpolated into one are a URL unless both are
/// validated — see [`SearchResult::is_well_formed`].
///
/// # Errors
///
/// If the result is not one a search could have produced, another download is
/// already running, the folder cannot be created, or there is not enough room —
/// the last **before any HTTP request is made**. Failures *during* the download
/// arrive on `model:failed`, not here.
#[tauri::command]
pub fn models_download_searched(
    app: AppHandle,
    state: State<'_, ModelState>,
    result: SearchResult,
) -> Result<(), String> {
    if !result.is_well_formed() {
        return Err("that result cannot be downloaded: it is not a whole, hashed GGUF file from a Hugging Face repository".to_owned());
    }

    let key = searched_key(&result);
    let folder = state.folder();
    ensure_folder(&folder)?;

    // The last segment only: the local library is flat, and nothing here
    // rebuilds a remote directory tree on the user's disk.
    let destination = folder.join(result.file_name());
    let part = destination.with_extension(PART_SUFFIX);

    // A searched vision repository ships its projector in the same tree, so it
    // arrives on the result rather than costing a second search. Resolved here,
    // before the space check, because both files have to fit or neither should
    // start — the same ordering [`models_download`] uses for the pinned tier.
    let projector = result.projector.clone().map(|projector| {
        let destination = folder.join(projector.file_name());
        let part = destination.with_extension(PART_SUFFIX);
        (projector, destination, part)
    });

    let outstanding = outstanding_bytes(&part, result.size_bytes).saturating_add(
        projector.as_ref().map_or(0, |(projector, _, part)| {
            outstanding_bytes(part, projector.size_bytes)
        }),
    );

    require_space(&folder, outstanding).map_err(|error| {
        crate::log::download_failed(error.kind());
        error.to_string()
    })?;

    state.claim(Busy {
        key: Some(key.clone()),
        what: result.file_name().to_owned(),
    })?;

    crate::log::download_started(result.size_bytes);
    let started = std::time::Instant::now();

    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut held) = state.cancel.lock() {
        *held = Some(sender);
    }

    let store = state.store();
    let url = download_url(&result.repo, &result.file);

    tauri::async_runtime::spawn(async move {
        let outcome = fetch(
            &app,
            &url,
            &part,
            &destination,
            Expected {
                sha256: &result.sha256,
                size_bytes: result.size_bytes,
            },
            &key,
            receiver,
        )
        .await;

        // Weights first, always — the same ordering and the same reason as the
        // pinned tier: a cancel between the two must leave no projector on disk
        // belonging to no model.
        let outcome = match (outcome, projector.as_ref()) {
            (Ok(()), Some((projector, destination, part))) => {
                let remote = RemoteProjector::searched(&result.repo, projector);
                fetch_projector(&app, &remote, destination, part, &key).await
            }
            (outcome, _) => outcome,
        };

        match outcome {
            Ok(()) => {
                let fetched = projector
                    .as_ref()
                    .map_or(result.size_bytes, |(projector, _, _)| {
                        result.size_bytes.saturating_add(projector.size_bytes)
                    });
                crate::log::download_finished(fetched, started.elapsed().as_secs());

                let recorded = store.record(ModelRecord {
                    key: key.clone(),
                    path: destination.clone(),
                    size_bytes: result.size_bytes,
                    sha256: result.sha256,
                    publisher: result.publisher,
                    repo: result.repo,
                    // The whole point of the field. A searched file recorded as
                    // pinned would be indistinguishable in the model list from
                    // one whose hash a person reviewed.
                    provenance: Provenance::Searched,
                    // A searched vision model gets its projector recorded on
                    // exactly the same terms as a pinned one. The hash it was
                    // checked against is the weaker tier's — that is what
                    // `provenance` above says, and it says it once for both
                    // files rather than differently for each.
                    projector_path: projector
                        .as_ref()
                        .map(|(_, destination, _)| destination.clone()),
                });

                let _ = match recorded {
                    Ok(()) => app.emit(
                        MODEL_DONE_EVENT,
                        ModelDone {
                            key: Some(key),
                            path: shown(&destination),
                        },
                    ),
                    Err(error) => {
                        crate::log::download_failed(error.kind());
                        app.emit(MODEL_FAILED_EVENT, ModelFailure::of(Some(key), &error))
                    }
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

/// Fetches one file, retrying what is worth retrying and stopping early
/// if the user asks.
///
/// Stopping drops the transfer mid-write, which leaves the part file exactly as
/// far along as it got — that is what makes pausing cheap rather than
/// destructive. Cancelling then removes it; see [`Stop`].
///
/// The retry loop is inside the future the `select!` below races, so a stop
/// interrupts a download **and** the backoff between two attempts. A Pause that
/// took effect only once a sixteen-second wait had elapsed would read as a
/// control that did not work.
/// What a transfer must turn out to be, whichever tier asked for it.
///
/// One struct rather than two loose arguments so that adding a third thing to
/// check reaches both callers, and so [`fetch`] cannot be given a pinned size
/// beside a searched hash.
#[derive(Debug, Clone, Copy)]
struct Expected<'a> {
    /// The hex SHA256 the finished file must have.
    sha256: &'a str,
    /// The size the response must advertise, checked before any byte is written.
    size_bytes: u64,
}

/// Takes the hash and the size rather than a [`ModelDownload`], so a searched
/// model travels this exact code path: pause, resume, the bounded backoff and
/// the refusal on a mismatch are one implementation rather than two that could
/// drift. The tier a file came from changes what is recorded afterwards and
/// nothing about how it is fetched.
async fn fetch(
    app: &AppHandle,
    url: &str,
    part: &Path,
    destination: &Path,
    expected: Expected<'_>,
    key: &ModelKey,
    cancel: tokio::sync::oneshot::Receiver<Stop>,
) -> Result<(), ModelFailure> {
    let client = reqwest::Client::new();
    let emitter = app.clone();
    let reported = key.clone();
    let mut rate = Rate::new();
    let mut on_progress = move |progress: Progress| {
        let pace = rate.observe(
            Instant::now(),
            progress.downloaded_bytes,
            progress.total_bytes,
        );

        let _ = emitter.emit(
            MODEL_PROGRESS_EVENT,
            ModelProgress {
                key: Some(reported.clone()),
                phase: "downloading".to_owned(),
                downloaded_bytes: progress.downloaded_bytes,
                total_bytes: progress.total_bytes,
                bytes_per_second: pace.bytes_per_second,
                seconds_remaining: pace.seconds_remaining,
            },
        );
    };

    let downloading = download_resumable_retrying(
        &client,
        url,
        part,
        destination,
        expected.sha256,
        expected.size_bytes,
        &mut on_progress,
    );

    let mut asked = None;
    let outcome = tokio::select! {
        finished = downloading => finished.map_err(|error| {
            // By kind, never by `Display`: every `AcquireError` variant names a
            // file, a path or a URL, which is the point of the message the user
            // is shown and exactly what must not reach the log.
            crate::log::download_failed(error.kind());
            ModelFailure::of(Some(key.clone()), &error)
        }),
        // The sender being dropped means nothing can ever stop this download,
        // which is not the same as being stopped. Answering `Ok` there would
        // abandon the transfer and then record a file that never landed, so
        // that branch never resolves and lets the other one decide -- the same
        // arrangement `chat.rs` uses for a reply nothing can cancel.
        stopped = cancel => match stopped {
            Ok(stop) => {
                asked = Some(stop);
                // Their own lines rather than failures: neither is one, and a
                // log that reported them the same way as a dropped connection
                // would have someone chasing a network problem that never was.
                match stop {
                    Stop::Pause => crate::log::download_paused(),
                    Stop::Cancel => crate::log::download_cancelled(),
                }
                Err(ModelFailure::stopped_by(key.clone(), stop))
            }
            Err(_) => downloading_forever().await,
        },
    };

    // **This is the entire difference between Pause and Cancel.** Both send the
    // same signal down the same channel and stop the transfer the same way;
    // only whether this runs decides whether the partial file survives to be
    // resumed from. It is one line, so it would otherwise look accidental.
    //
    // After the `select!` rather than inside its arm, because the arm runs
    // while the transfer future — and the file handle it holds — is still
    // alive, and Windows refuses to delete a file anything has open.
    if asked == Some(Stop::Cancel) {
        discard(part).await;
    }

    outcome
}

/// Removes the partial file a cancelled download left behind.
///
/// Retried briefly rather than attempted once. The transfer future has just
/// been dropped, but a `tokio::fs::File` whose last write is still in flight
/// closes its handle on a blocking task rather than immediately — and Windows
/// refuses to delete a file anything still has open. A quarter of a second of
/// patience is the difference between Cancel freeing the disk and leaving
/// gigabytes behind that nothing in the app admits to.
///
/// Still best effort at the end of it: a file that will not delete costs disk,
/// and there is nothing useful to tell the user who has already moved on.
async fn discard(part: &Path) {
    for _ in 0..DISCARD_ATTEMPTS {
        if !part.exists() || std::fs::remove_file(part).is_ok() {
            return;
        }
        tokio::time::sleep(DISCARD_PAUSE).await;
    }
}

/// A future that never resolves, for a cancel channel that can never fire.
///
/// `select!` needs both branches to produce the same type, and a dropped sender
/// must not be mistaken for a finished download.
async fn downloading_forever() -> Result<(), ModelFailure> {
    std::future::pending().await
}

/// Pauses the download currently running, if there is one.
///
/// The partial file is **kept**, so starting the same download again resumes
/// from where it stopped rather than from zero. That is the only thing
/// separating this from [`models_cancel`], which is otherwise the same stop.
#[tauri::command]
pub fn models_pause(state: State<'_, ModelState>) {
    state.ask_to_stop(Stop::Pause);
}

/// Cancels the download currently running, if there is one.
///
/// The partial file is **deleted**, so starting the same download again begins
/// from zero. That is the only thing separating this from [`models_pause`] —
/// deliberately, because a control that abandoned a download and silently left
/// several gigabytes on the disk would be the opposite of what a disk-cleaning
/// utility is for.
#[tauri::command]
pub fn models_cancel(state: State<'_, ModelState>) {
    state.ask_to_stop(Stop::Cancel);
}

/// Deletes a downloaded model: every file it owns, and the record naming them.
///
/// All of them, because osstat is a disk cleaner. A record left behind would
/// offer a Run control for bytes that are gone, and a file left behind would be
/// gigabytes nothing in the app admits to.
///
/// A vision model owns two: the weights, and the multimodal projector that was
/// fetched with them. The projector is the one a user cannot clean up by hand
/// with any confidence — they never chose to download it, its name is not the
/// model's, and nothing in the interface ever showed it to them as a file. So
/// it is this command's job or nobody's.
///
/// # A model that is currently running is refused rather than deleted
///
/// The two platforms fail differently and both fail badly. Windows holds an
/// open file, so `remove_file` returns a sharing violation and the user gets an
/// OS error message about a file they did not know was in use. Unix unlinks it
/// happily: `llama-server` keeps serving from the open inode, the record is
/// forgotten, and the gigabytes stay allocated until the process exits — a disk
/// cleaner that appears to have freed space and has not.
///
/// So the session is asked first, and the refusal names the way out. Stopping
/// the server on the user's behalf would be the other option and is worse: a
/// Delete control that silently killed a running model, and with it whatever
/// was being generated, would do more than it said.
///
/// # Errors
///
/// If the model is currently running, the file cannot be removed, or the index
/// cannot be written.
#[tauri::command]
pub fn models_delete(
    state: State<'_, ModelState>,
    chat: State<'_, crate::chat::ChatState>,
    model_id: String,
    quant_id: String,
) -> Result<(), String> {
    let key = ModelKey { model_id, quant_id };
    let store = state.store();

    if let Some(path) = store.path_of(&key) {
        if chat.has_open(&path) {
            return Err(
                "that model is running. Unload it first, then it can be deleted.".to_owned(),
            );
        }

        if let Err(error) = std::fs::remove_file(&path) {
            return Err(format!("{} could not be deleted: {error}", path.display()));
        }
    }

    // A vision model is two files, and only one of them is the model. Removing
    // the weights alone left the projector behind as several hundred megabytes
    // no record pointed at any more -- the precise thing this command's
    // "gigabytes nothing in the app admits to" exists to prevent, reintroduced
    // by the file the user never chose to download separately.
    //
    // Not guarded by `has_open` a second time. A running server holds its
    // weights open, so the check above refuses before reaching here for as long
    // as the weights are on disk -- which is every state osstat's own controls
    // can produce, since this command is the only thing that unlinks them and
    // it will not run while a session is live.
    if let Some(projector) = store.projector_of(&key)
        && let Err(error) = std::fs::remove_file(&projector)
    {
        return Err(format!(
            "{} could not be deleted: {error}",
            projector.display()
        ));
    }

    // Last, and only if both files are gone. A record forgotten while a file
    // remains is the orphan again, by a different route -- and leaving the
    // record on a failure is what makes a retry able to finish the job, because
    // the paths it needs are still written down.
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

    // Asked before the move so the finished line can say how much moved. The
    // plan is metadata reads over files already indexed, not a walk of the
    // destination, so this is cheap next to the move it describes.
    let planned = plan_move(&store, &destination);
    let started = std::time::Instant::now();

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
                    // No rate and no estimate for a move. A same-volume move
                    // renames rather than copies, so a byte rate would be a
                    // figure invented for an operation that never touched the
                    // bytes — and a cross-volume one is bounded by the disk
                    // rather than by anything this window could observe.
                    bytes_per_second: None,
                    seconds_remaining: None,
                },
            );
        };

        let _ = match move_library(&mut store, &destination, &mut on_progress).await {
            Ok(()) => {
                // Neither folder is named: how much moved and how long it took
                // is the whole of what a log can usefully say about a move.
                crate::log::library_moved(planned.bytes, started.elapsed().as_secs());
                app.emit(
                    MODEL_DONE_EVENT,
                    ModelDone {
                        key: None,
                        path: shown(&destination),
                    },
                )
            }
            Err(error) => {
                crate::log::library_move_failed(error.kind());
                app.emit(MODEL_FAILED_EVENT, ModelFailure::of(None, &error))
            }
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
        assert_eq!(failure.stopped, None);
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
    fn a_paused_download_is_offered_as_resumable_rather_than_broken() {
        let failure = ModelFailure::stopped_by(sample_key(), Stop::Pause);

        assert_eq!(failure.stopped, Some(Stop::Pause));
        assert!(failure.retryable, "a paused download resumes");
        assert!(!failure.verification_failure);
    }

    #[test]
    fn a_cancelled_download_says_the_partial_file_is_gone() {
        // The difference the UI renders from: after a pause the figure the bar
        // reached still describes bytes on disk, and after a cancel it does
        // not. Showing "2.3 GB of 4.6 GB" over a deleted part would promise a
        // resume that would in fact start from zero.
        let failure = ModelFailure::stopped_by(sample_key(), Stop::Cancel);

        assert_eq!(
            failure.stopped,
            Some(Stop::Cancel),
            "a cancel must not be reported as a pause"
        );
        assert!(
            failure.retryable,
            "starting again is still the right control"
        );
        assert!(!failure.verification_failure);
    }

    #[test]
    fn a_real_failure_is_not_reported_as_a_stop() {
        let failure = ModelFailure::of(
            Some(sample_key()),
            &AcquireError::HttpStatus {
                url: "https://example.invalid/x".to_owned(),
                status: 503,
            },
        );

        assert_eq!(failure.stopped, None);
    }

    #[test]
    fn the_two_stops_differ_in_their_wording_as_well_as_their_flag() {
        // A user who paused and a user who cancelled are told different things
        // about what is on their disk, because different things are.
        let paused = ModelFailure::stopped_by(sample_key(), Stop::Pause);
        let cancelled = ModelFailure::stopped_by(sample_key(), Stop::Cancel);

        assert!(paused.message.contains("paused"), "{}", paused.message);
        assert!(
            cancelled.message.contains("deleted"),
            "{}",
            cancelled.message
        );
        assert_ne!(paused.message, cancelled.message);
    }

    #[test]
    fn a_rate_says_nothing_from_a_single_sample() {
        // One sample is a position, not a speed. Reporting a rate from it would
        // mean inventing an elapsed time.
        let mut rate = Rate::new();

        let pace = rate.observe(Instant::now(), 1_000, 10_000);

        assert_eq!(pace.bytes_per_second, None);
        assert_eq!(pace.seconds_remaining, None);
    }

    #[test]
    fn a_rate_is_bytes_moved_over_the_time_they_took() {
        let start = Instant::now();
        let mut rate = Rate::new();

        rate.observe(start, 0, 10_000_000);
        let pace = rate.observe(start + Duration::from_secs(2), 4_000_000, 10_000_000);

        assert_eq!(pace.bytes_per_second, Some(2_000_000));
        // 6 MB left at 2 MB/s.
        assert_eq!(pace.seconds_remaining, Some(3));
    }

    #[test]
    fn a_stall_reports_zero_rather_than_the_speed_it_used_to_manage() {
        // The whole reason the window is short. A transfer that moved fast for
        // a minute and has moved nothing for the last ten seconds is stalled,
        // and an average over the whole transfer would still call it fast.
        let start = Instant::now();
        let mut rate = Rate::new();

        rate.observe(start, 0, 10_000_000);
        rate.observe(start + Duration::from_secs(1), 5_000_000, 10_000_000);
        // Well past the window, and nothing further has arrived.
        let pace = rate.observe(start + Duration::from_secs(30), 5_000_000, 10_000_000);

        assert_eq!(pace.bytes_per_second, Some(0), "a stall was averaged away");
        assert_eq!(
            pace.seconds_remaining, None,
            "a stalled transfer has no honest estimate"
        );
    }

    #[test]
    fn a_rate_describes_the_recent_window_rather_than_the_whole_transfer() {
        // Slow for a long time, then fast. The reported rate must be the fast
        // one: it is what the next few seconds will actually look like.
        let start = Instant::now();
        let mut rate = Rate::new();

        rate.observe(start, 0, 100_000_000);
        rate.observe(start + Duration::from_secs(45), 1_000_000, 100_000_000);
        rate.observe(start + Duration::from_secs(46), 11_000_000, 100_000_000);
        let pace = rate.observe(start + Duration::from_secs(47), 21_000_000, 100_000_000);

        assert_eq!(
            pace.bytes_per_second,
            Some(10_000_000),
            "the first slow minute was still being counted"
        );
    }

    #[test]
    fn a_rate_window_does_not_grow_without_bound() {
        // Progress arrives once per chunk, thousands of times a second on a
        // fast link. An unbounded deque would be a slow memory leak for the
        // duration of every multi-gigabyte download.
        let start = Instant::now();
        let mut rate = Rate::new();

        for index in 0..10_000_u64 {
            rate.observe(
                start + Duration::from_micros(index),
                index * 1_024,
                u64::MAX,
            );
        }

        assert!(
            rate.samples.len() <= RATE_SAMPLES,
            "the window holds {} samples",
            rate.samples.len()
        );
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
            bytes_per_second: Some(5),
            seconds_remaining: Some(18),
        })
        .unwrap();
        assert!(
            progress
                .as_object()
                .unwrap()
                .contains_key("downloadedBytes")
        );
        assert!(progress.as_object().unwrap().contains_key("totalBytes"));
        assert!(progress.as_object().unwrap().contains_key("bytesPerSecond"));
        assert!(
            progress
                .as_object()
                .unwrap()
                .contains_key("secondsRemaining")
        );
        assert_eq!(
            progress["key"]["modelId"],
            serde_json::json!("qwen2.5-0.5b")
        );
        assert_eq!(progress["key"]["quantId"], serde_json::json!("Q4_K_M"));

        let failure =
            serde_json::to_value(ModelFailure::stopped_by(sample_key(), Stop::Pause)).unwrap();
        let object = failure.as_object().unwrap();
        assert!(object.contains_key("verificationFailure"));
        assert_eq!(failure["stopped"], serde_json::json!("pause"));
        assert_eq!(
            serde_json::to_value(ModelFailure::stopped_by(sample_key(), Stop::Cancel)).unwrap()["stopped"],
            serde_json::json!("cancel")
        );
        assert_eq!(
            serde_json::to_value(ModelFailure::of(
                None,
                &AcquireError::ServerMissing {
                    file: "a.zip".to_owned()
                }
            ))
            .unwrap()["stopped"],
            serde_json::json!(null)
        );

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
            provenance: Provenance::Pinned,
        })
        .unwrap();
        assert!(entry.as_object().unwrap().contains_key("sizeBytes"));
    }

    /// A result of the shape [`search`] produces.
    fn sample_result() -> SearchResult {
        SearchResult {
            repo: "bartowski/Some-Model-GGUF".to_owned(),
            publisher: "bartowski".to_owned(),
            file: "Some-Model-Q4_K_M.gguf".to_owned(),
            size_bytes: 4_683_074_240,
            sha256: "a".repeat(64),
            quant_hint: Some("Q4_K_M".to_owned()),
            projector: None,
        }
    }

    #[test]
    fn a_searched_cell_cannot_collide_with_a_pinned_one() {
        // Both tiers share one index and one delete command, so the keys have
        // to be distinguishable. A repository always contains a slash and a
        // registry model id never does, which is what makes that true rather
        // than merely unlikely.
        let key = searched_key(&sample_result());

        assert!(key.model_id.contains('/'));
        assert!(
            !seeded_registry()
                .models
                .iter()
                .any(|model| model.id == key.model_id),
            "a searched key took a registry model's id"
        );
        assert_ne!(key, sample_key());
    }

    #[test]
    fn a_result_the_webview_could_have_altered_is_refused_before_any_request() {
        // The command takes a repository and a file, which become a URL. ADR-012
        // rests on there being no command that takes a URL, so the check has to
        // happen here rather than only in the search that produced the value.
        for bad in [
            SearchResult {
                repo: "https://elsewhere.invalid/x".to_owned(),
                ..sample_result()
            },
            SearchResult {
                file: "../../../../Windows/System32/x.gguf".to_owned(),
                ..sample_result()
            },
            SearchResult {
                sha256: String::new(),
                ..sample_result()
            },
        ] {
            assert!(
                !bad.is_well_formed(),
                "a result the search would never produce was accepted: {bad:?}"
            );
        }

        assert!(sample_result().is_well_formed());
    }

    #[test]
    fn a_searched_file_is_saved_flat_rather_than_under_its_remote_folders() {
        let nested = SearchResult {
            file: "Q4_K_M/Some-Model.gguf".to_owned(),
            ..sample_result()
        };

        assert!(nested.is_well_formed());
        assert_eq!(nested.file_name(), "Some-Model.gguf");
    }

    #[test]
    fn a_pinned_cell_of_the_matrix_reports_pinned_provenance() {
        // Every cell of the curated matrix is pinned whether or not it has been
        // downloaded: it is the registry's row, not a record's.
        let entry = ModelCatalogueEntry {
            key: sample_key(),
            state: "downloadable".to_owned(),
            publisher: Some("bartowski".to_owned()),
            repo: Some("bartowski/Qwen2.5-0.5B-Instruct-GGUF".to_owned()),
            file: Some("a.gguf".to_owned()),
            size_bytes: 400,
            path: None,
            provenance: Provenance::Pinned,
        };

        assert_eq!(entry.provenance, Provenance::Pinned);
    }

    #[test]
    fn the_two_tiers_reach_the_webview_as_different_strings() {
        // The label is the feature, and the UI cannot render a distinction the
        // payload does not carry.
        let entry = |provenance| {
            serde_json::to_value(ModelCatalogueEntry {
                key: sample_key(),
                state: "downloaded".to_owned(),
                publisher: Some("bartowski".to_owned()),
                repo: Some("bartowski/Some-GGUF".to_owned()),
                file: Some("a.gguf".to_owned()),
                size_bytes: 400,
                path: Some("D:\\models\\a.gguf".to_owned()),
                provenance,
            })
            .unwrap()
        };

        assert_eq!(entry(Provenance::Pinned)["provenance"], "pinned");
        assert_eq!(entry(Provenance::Searched)["provenance"], "searched");
    }

    #[test]
    fn a_search_result_reaches_the_webview_in_camel_case() {
        let json = serde_json::to_value(sample_result()).unwrap();
        let object = json.as_object().unwrap();

        assert!(object.contains_key("sizeBytes"));
        assert!(object.contains_key("quantHint"));
        assert!(
            !object.contains_key("verdict") && !object.contains_key("tier"),
            "a searched result grew a field the advisor cannot honestly fill: {object:?}"
        );
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
            bytes_per_second: None,
            seconds_remaining: None,
        };

        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["key"], serde_json::json!(null));
        assert_eq!(progress.phase, "moving");
    }
}
