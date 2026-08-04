//! What has been downloaded, and where the user put it.
//!
//! Model files are large and live wherever the user chose — an external drive,
//! a second disk, a folder they will move next week. So the index records an
//! absolute path per file rather than assuming a layout under one root, which
//! is what separates this from [`crate::store`].
//!
//! The index is advisory. The files are the truth: a record whose bytes are
//! gone is reported by [`ModelStore::records`] but not by
//! [`ModelStore::present`], so a removable drive being unplugged shows up as a
//! model that is not ready rather than as a load failure much later.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::AcquireError;
use crate::models::ModelKey;

/// Schema version written into every index.
///
/// An index from a future osstat is treated as unreadable, which by the rule
/// above means an empty library rather than a crash.
const INDEX_VERSION: u32 = 1;

/// Which of osstat's two verification tiers a file was fetched under.
///
/// The distinction outlives the download, which is why it is recorded rather
/// than inferred: once the bytes are on disk nothing about them says whether
/// the hash they were checked against had been reviewed by a person. A model
/// list that showed both the same way would quietly retire a guarantee
/// SECURITY.md still makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum Provenance {
    /// Verified against a hash reviewed in a pull request against this
    /// repository. The bytes are the ones somebody looked at.
    ///
    /// The default, and deliberately so: every record written before this field
    /// existed came from the pinned registry, so an index from an older osstat
    /// reads correctly rather than being downgraded or discarded.
    #[default]
    Pinned,
    /// Verified against the hash Hugging Face reports beside the file.
    ///
    /// Detects a corrupted transfer. Cannot detect a replaced upload, because
    /// the digest and the file come from the same origin.
    Searched,
}

/// One model file osstat has downloaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    /// The fit-matrix cell this file fills.
    pub key: ModelKey,
    /// Absolute path to the `.gguf` file.
    pub path: PathBuf,
    /// Size in bytes, as verified at download time.
    pub size_bytes: u64,
    /// Lower-case hex SHA256 the file was verified against.
    pub sha256: String,
    /// Who published the re-quantization, kept so provenance stays visible
    /// after the download rather than only on the button that started it.
    pub publisher: String,
    /// The Hugging Face repository it came from.
    pub repo: String,
    /// Which verification tier fetched it.
    ///
    /// `#[serde(default)]` rather than a schema bump: users have models on disk
    /// from before this field existed, and treating those indexes as unreadable
    /// would strand multi-gigabyte files the app then refused to admit to.
    /// Everything written before this feature came from the pinned registry, so
    /// [`Provenance::Pinned`] is not merely a safe default — it is the true one.
    #[serde(default)]
    pub provenance: Provenance,
    /// Absolute path to the multimodal projector, on a vision model.
    ///
    /// `None` means the model is text-only, which is what every record written
    /// before this field existed is — so `#[serde(default)]` for the same
    /// reason as `provenance` above: an index the app refuses to read strands
    /// gigabytes the user already downloaded.
    ///
    /// Recorded as a path rather than re-derived from the registry at launch:
    /// a pin can be withdrawn or re-quantized, and a file already on disk must
    /// keep working. It is also what lets `session::start` decide whether to
    /// pass `--mmproj` from the record alone.
    #[serde(default)]
    pub projector_path: Option<PathBuf>,
}

/// The on-disk shape of the index.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    /// Schema version of this file.
    version: u32,
    /// Every model osstat believes it has downloaded.
    records: Vec<ModelRecord>,
}

/// The record of every downloaded model.
///
/// Cheap to construct and holds no cached state: every method reads the index
/// from disk. It holds tens of entries, not thousands, and the alternative — a
/// cache — would go stale exactly when a second window deleted something.
#[derive(Debug, Clone)]
pub struct ModelStore {
    index_path: PathBuf,
}

impl ModelStore {
    /// Creates a store backed by the JSON index at `index_path`.
    #[must_use]
    pub const fn new(index_path: PathBuf) -> Self {
        Self { index_path }
    }

    /// Where the index itself is written.
    #[must_use]
    pub fn index_path(&self) -> &Path {
        &self.index_path
    }

    /// Every record, whether or not its file is still there.
    ///
    /// An index that is absent, unreadable or malformed reads as an empty
    /// library. It is a file on the user's disk and can be truncated by a power
    /// cut mid-write; refusing to start over it would strand a working install.
    #[must_use]
    pub fn records(&self) -> Vec<ModelRecord> {
        let Ok(text) = std::fs::read_to_string(&self.index_path) else {
            return Vec::new();
        };
        let Ok(index) = serde_json::from_str::<Index>(&text) else {
            return Vec::new();
        };
        if index.version > INDEX_VERSION {
            return Vec::new();
        }

        index.records
    }

    /// Only the records whose files are still on disk.
    #[must_use]
    pub fn present(&self) -> Vec<ModelRecord> {
        let mut records = self.records();
        records.retain(|record| record.path.is_file());
        records
    }

    /// The usable path for a cell, if one has been downloaded and is still there.
    ///
    /// Deliberately answers `None` for a record whose file has vanished: this is
    /// what the Run control hands to the loader, and a path with no bytes behind
    /// it turns a legible "not downloaded" into a loader error far from the
    /// cause. [`Self::records`] is the way to ask what was once downloaded.
    #[must_use]
    pub fn path_of(&self, key: &ModelKey) -> Option<PathBuf> {
        self.records()
            .into_iter()
            .find(|record| &record.key == key && record.path.is_file())
            .map(|record| record.path)
    }

    /// The projector belonging to a cell, if it has one still on disk.
    ///
    /// Deliberately **not** conditional on the weights still being there, which
    /// is the one way this differs from [`Self::path_of`]. A library where the
    /// weights were removed and the projector was not is exactly the state that
    /// leaves an 800 MB file nothing refers to, and a lookup that answered
    /// `None` for it could never be used to clean it up.
    ///
    /// Filtered on the file existing for the same reason [`Self::path_of`] is:
    /// every caller wants a path with bytes behind it, and a record naming a
    /// projector the user already deleted by hand is not an error to report.
    #[must_use]
    pub fn projector_of(&self, key: &ModelKey) -> Option<PathBuf> {
        self.records()
            .into_iter()
            .find(|record| &record.key == key)?
            .projector_path
            .filter(|projector| projector.is_file())
    }

    /// Records a downloaded model, replacing any earlier record of the same cell.
    ///
    /// # Errors
    ///
    /// [`AcquireError::Io`] if the index cannot be written.
    pub fn record(&self, record: ModelRecord) -> Result<(), AcquireError> {
        let mut records = self.records();
        records.retain(|existing| existing.key != record.key);
        records.push(record);

        self.write(records)
    }

    /// Drops a record without touching the file it names.
    ///
    /// Deleting bytes is a separate, explicit action: forgetting is bookkeeping,
    /// and a bookkeeping call that silently freed 5 GB would be the worst kind
    /// of surprise.
    ///
    /// # Errors
    ///
    /// [`AcquireError::Io`] if the index cannot be written.
    pub fn forget(&self, key: &ModelKey) -> Result<(), AcquireError> {
        let mut records = self.records();
        records.retain(|record| &record.key != key);

        self.write(records)
    }

    /// Rewrites the whole index.
    ///
    /// Written to a sibling and renamed, so an interruption leaves either the
    /// old index or the new one. A half-written index would still be survivable
    /// — [`Self::records`] treats it as empty — but surviving it means the user
    /// re-downloading everything they had.
    fn write(&self, records: Vec<ModelRecord>) -> Result<(), AcquireError> {
        let io = |source| AcquireError::Io {
            path: self.index_path.clone(),
            source,
        };

        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AcquireError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let index = Index {
            version: INDEX_VERSION,
            records,
        };
        let text = serde_json::to_string_pretty(&index)
            .map_err(|error| io(std::io::Error::other(error)))?;

        let temporary = self.index_path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|source| AcquireError::Io {
            path: temporary.clone(),
            source,
        })?;
        std::fs::rename(&temporary, &self.index_path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A plausible record. `path` is overridden by nearly every test.
    fn sample() -> ModelRecord {
        ModelRecord {
            key: ModelKey {
                model_id: "qwen2.5-0.5b".to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            path: std::env::temp_dir().join("osstat-never-written.gguf"),
            size_bytes: 1024,
            sha256: "a".repeat(64),
            publisher: "bartowski".to_owned(),
            repo: "bartowski/Qwen2.5-0.5B-Instruct-GGUF".to_owned(),
            provenance: Provenance::Pinned,
            projector_path: None,
        }
    }

    /// Writes `bytes` at `path` and returns a record naming it.
    fn sample_file(path: PathBuf, bytes: &[u8]) -> ModelRecord {
        std::fs::write(&path, bytes).unwrap();
        ModelRecord {
            path,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            ..sample()
        }
    }

    #[test]
    fn a_recorded_model_is_found_again() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let gguf = root.path().join("model.gguf");
        let record = sample_file(gguf.clone(), b"weights");

        store.record(record.clone()).unwrap();

        assert_eq!(store.path_of(&record.key), Some(gguf));
    }

    #[test]
    fn a_vision_model_reports_the_projector_beside_its_weights() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let gguf = root.path().join("model.gguf");
        let mmproj = root.path().join("mmproj-model.gguf");
        std::fs::write(&mmproj, b"projector").unwrap();

        let record = ModelRecord {
            projector_path: Some(mmproj.clone()),
            ..sample_file(gguf, b"weights")
        };
        store.record(record.clone()).unwrap();

        assert_eq!(store.projector_of(&record.key), Some(mmproj));
    }

    #[test]
    fn a_text_only_model_reports_no_projector() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let record = sample_file(root.path().join("model.gguf"), b"weights");

        store.record(record.clone()).unwrap();

        assert_eq!(store.projector_of(&record.key), None);
    }

    #[test]
    fn a_projector_is_still_reported_once_its_weights_are_gone() {
        // The whole reason this is not `path_of` with a different field. A
        // library where the weights were removed and the projector was not is
        // exactly the state that strands hundreds of megabytes, and a lookup
        // that answered `None` here could never be used to clean it up.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let gguf = root.path().join("model.gguf");
        let mmproj = root.path().join("mmproj-model.gguf");
        std::fs::write(&mmproj, b"projector").unwrap();

        let record = ModelRecord {
            projector_path: Some(mmproj.clone()),
            ..sample_file(gguf.clone(), b"weights")
        };
        store.record(record.clone()).unwrap();

        std::fs::remove_file(&gguf).unwrap();

        assert_eq!(
            store.path_of(&record.key),
            None,
            "the weights were deleted and still reported"
        );
        assert_eq!(
            store.projector_of(&record.key),
            Some(mmproj),
            "the orphaned projector became unreachable, which is how it leaks"
        );
    }

    #[test]
    fn a_projector_the_user_already_deleted_is_not_reported() {
        // Same rule `path_of` follows: every caller wants a path with bytes
        // behind it, and a record naming a file that is already gone is not an
        // error to hand back.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let record = ModelRecord {
            projector_path: Some(root.path().join("mmproj-never-written.gguf")),
            ..sample_file(root.path().join("model.gguf"), b"weights")
        };
        store.record(record.clone()).unwrap();

        assert_eq!(store.projector_of(&record.key), None);
    }

    #[test]
    fn one_model_s_projector_is_not_reported_for_another_cell() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let mmproj = root.path().join("mmproj-vision.gguf");
        std::fs::write(&mmproj, b"projector").unwrap();

        let vision = ModelRecord {
            key: ModelKey {
                model_id: "qwen2.5-vl-3b".to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            projector_path: Some(mmproj),
            ..sample_file(root.path().join("vision.gguf"), b"weights")
        };
        let text = ModelRecord {
            key: ModelKey {
                model_id: "llama-3.2-1b".to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            ..sample_file(root.path().join("text.gguf"), b"weights")
        };
        store.record(vision).unwrap();
        store.record(text.clone()).unwrap();

        assert_eq!(store.projector_of(&text.key), None);
    }

    #[test]
    fn a_record_survives_a_reopen_of_the_index() {
        // The index is the only memory of where a 5 GB file went. Losing it on
        // restart means downloading again, which is the failure this whole
        // feature exists to avoid.
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("models.json");
        let record = sample_file(root.path().join("model.gguf"), b"weights");

        ModelStore::new(index.clone())
            .record(record.clone())
            .unwrap();

        let reopened = ModelStore::new(index);
        assert_eq!(reopened.records(), vec![record]);
    }

    #[test]
    fn a_record_whose_file_vanished_is_reported_missing_not_present() {
        // Files live wherever the user chose, including a removable drive.
        // Listing a model as ready when its bytes are gone produces a failure
        // at load time, further from the cause and with a worse message.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let gone = root.path().join("absent.gguf");
        store
            .record(ModelRecord {
                path: gone,
                ..sample()
            })
            .unwrap();

        assert!(store.present().is_empty());
        assert_eq!(store.records().len(), 1, "the record itself must survive");
    }

    #[test]
    fn the_path_of_a_vanished_file_is_not_offered() {
        // `path_of` is what the Run control hands to the loader. Answering with
        // a path whose bytes are gone turns a legible "not downloaded" into a
        // loader error much further from the cause.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let record = ModelRecord {
            path: root.path().join("absent.gguf"),
            ..sample()
        };
        store.record(record.clone()).unwrap();

        assert_eq!(store.path_of(&record.key), None);
    }

    #[test]
    fn an_index_that_will_not_parse_is_an_empty_library_not_a_crash() {
        // The index is a file on the user's disk. It can be truncated by a
        // power cut mid-write.
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("models.json"), "{{{ not json").unwrap();

        assert!(
            ModelStore::new(root.path().join("models.json"))
                .records()
                .is_empty()
        );
    }

    #[test]
    fn an_index_that_is_not_there_yet_is_an_empty_library() {
        let root = tempfile::tempdir().unwrap();

        assert!(
            ModelStore::new(root.path().join("models.json"))
                .records()
                .is_empty()
        );
    }

    #[test]
    fn forgetting_a_model_removes_only_that_record() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let kept = sample_file(root.path().join("kept.gguf"), b"one");
        let dropped = ModelRecord {
            key: ModelKey {
                model_id: "llama-3.2-1b".to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            ..sample_file(root.path().join("dropped.gguf"), b"two")
        };

        store.record(kept.clone()).unwrap();
        store.record(dropped.clone()).unwrap();
        store.forget(&dropped.key).unwrap();

        assert_eq!(store.records(), vec![kept]);
    }

    #[test]
    fn forgetting_a_model_leaves_its_bytes_alone() {
        // Deleting a multi-gigabyte file is an explicit action with its own
        // command. Forgetting is bookkeeping, and must not destroy anything.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let record = sample_file(root.path().join("model.gguf"), b"weights");
        store.record(record.clone()).unwrap();

        store.forget(&record.key).unwrap();

        assert!(record.path.is_file(), "forgetting deleted the file");
    }

    #[test]
    fn recording_the_same_cell_twice_replaces_rather_than_duplicates() {
        // Re-downloading after a delete must not leave two rows claiming one
        // cell, one of which names a path that is gone.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let first = sample_file(root.path().join("old.gguf"), b"one");
        let second = sample_file(root.path().join("new.gguf"), b"two");

        store.record(first).unwrap();
        store.record(second.clone()).unwrap();

        assert_eq!(store.records(), vec![second.clone()]);
        assert_eq!(store.path_of(&second.key), Some(second.path));
    }

    #[test]
    fn a_record_written_before_provenance_existed_still_loads() {
        // Users have models on disk from before this feature. Losing them
        // would be worse than the feature is worth. `#[serde(default)]` with
        // Pinned as the default is correct: everything that exists today came
        // from the pinned registry.
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("models.json");
        let gguf = root.path().join("model.gguf");
        std::fs::write(&gguf, b"weights").unwrap();

        // Written by hand in exactly the shape the previous release produced,
        // rather than by serialising a struct with the field removed — the
        // point is the bytes already sitting on someone's disk.
        std::fs::write(
            &index,
            serde_json::json!({
                "version": 1,
                "records": [{
                    "key": { "modelId": "qwen2.5-0.5b", "quantId": "Q4_K_M" },
                    "path": gguf,
                    "sizeBytes": 7,
                    "sha256": "a".repeat(64),
                    "publisher": "bartowski",
                    "repo": "bartowski/Qwen2.5-0.5B-Instruct-GGUF"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let records = ModelStore::new(index).records();

        assert_eq!(records.len(), 1, "an older index was read as empty");
        assert_eq!(
            records[0].provenance,
            Provenance::Pinned,
            "a model from the pinned registry was relabelled as searched"
        );
        assert_eq!(records[0].path, gguf);
        assert_eq!(
            records[0].projector_path, None,
            "a text-only model was given a projector it never downloaded"
        );
    }

    #[test]
    fn a_record_written_before_the_projector_existed_still_loads() {
        // Same argument as the test above, for the field this feature added:
        // every record on disk today names one file, and reading `None` from
        // its absence is the truth rather than a fallback.
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("models.json");
        let gguf = root.path().join("model.gguf");
        std::fs::write(&gguf, b"weights").unwrap();

        std::fs::write(
            &index,
            serde_json::json!({
                "version": 1,
                "records": [{
                    "key": { "modelId": "qwen2.5-0.5b", "quantId": "Q4_K_M" },
                    "path": gguf,
                    "sizeBytes": 7,
                    "sha256": "a".repeat(64),
                    "publisher": "bartowski",
                    "repo": "bartowski/Qwen2.5-0.5B-Instruct-GGUF",
                    "provenance": "pinned"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let records = ModelStore::new(index).records();

        assert_eq!(records.len(), 1, "an older index was read as empty");
        assert_eq!(records[0].projector_path, None);
    }

    #[test]
    fn a_projector_path_survives_a_round_trip_through_the_index() {
        // The path is what `session::start` reads to decide on `--mmproj`, so
        // an index that dropped it would produce a vision model that loads and
        // cannot see -- the failure this whole feature exists to avoid.
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("models.json");
        let gguf = root.path().join("model.gguf");
        let mmproj = root.path().join("mmproj.gguf");
        std::fs::write(&gguf, b"weights").unwrap();
        std::fs::write(&mmproj, b"projector").unwrap();

        let store = ModelStore::new(index);
        let record = ModelRecord {
            path: gguf,
            projector_path: Some(mmproj.clone()),
            ..sample()
        };
        store.record(record).unwrap();

        assert_eq!(store.records()[0].projector_path, Some(mmproj));
    }

    #[test]
    fn a_searched_model_is_recorded_as_searched() {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("models.json");
        let record = ModelRecord {
            key: ModelKey {
                model_id: "bartowski/Some-GGUF".to_owned(),
                quant_id: "Some-Q4_K_M.gguf".to_owned(),
            },
            provenance: Provenance::Searched,
            ..sample_file(root.path().join("searched.gguf"), b"weights")
        };

        ModelStore::new(index.clone())
            .record(record.clone())
            .unwrap();

        // Through a reopen, because the label has to survive the round trip
        // that the UI actually reads it back through.
        assert_eq!(ModelStore::new(index).records(), vec![record]);
    }

    #[test]
    fn the_two_tiers_are_distinguishable_on_the_wire() {
        // The label is the feature. A serialisation that spelled both the same
        // way would make the UI's distinction unimplementable.
        let pinned = serde_json::to_value(Provenance::Pinned).unwrap();
        let searched = serde_json::to_value(Provenance::Searched).unwrap();

        assert_eq!(pinned, serde_json::json!("pinned"));
        assert_eq!(searched, serde_json::json!("searched"));
        assert_ne!(pinned, searched);
    }

    #[test]
    fn present_reports_only_the_records_whose_files_are_there() {
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let here = sample_file(root.path().join("here.gguf"), b"one");
        let gone = ModelRecord {
            key: ModelKey {
                model_id: "llama-3.2-1b".to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            path: root.path().join("gone.gguf"),
            ..sample()
        };

        store.record(here.clone()).unwrap();
        store.record(gone).unwrap();

        assert_eq!(store.present(), vec![here]);
    }
}
