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
