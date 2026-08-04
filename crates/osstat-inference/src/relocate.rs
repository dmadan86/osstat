//! Moving the downloaded library to a folder the user chose.
//!
//! One rule governs everything here: **a source file is never deleted before
//! its destination has verified.** A model is gigabytes, and a move that
//! deletes first turns a full disk or an unplugged drive into permanent loss of
//! something that took an hour to fetch. It is the same discipline
//! [`crate::download`] applies to a partial download, applied to the other
//! direction.
//!
//! Work is per file, not per library. An interruption therefore leaves a
//! mixture — some records naming the new folder, some the old — where every
//! record still names a file that exists. A library half in each place is
//! usable; a library whose index disagrees with the disk is not.

use std::path::{Path, PathBuf};

use crate::download::{Progress, file_name_of, sha256_file};
use crate::error::AcquireError;
use crate::model_store::{ModelRecord, ModelStore};

/// How much is read and written at a time while copying across volumes.
///
/// A megabyte is large enough that a 20 GB file is not twenty million syscalls
/// and small enough that progress arrives often enough to look continuous.
const COPY_CHUNK_BYTES: usize = 1024 * 1024;

/// What moving the library to a chosen folder would involve.
///
/// Produced before anything is touched so Settings can state the cost and ask,
/// rather than starting an hour of copying on a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovePlan {
    /// How many downloaded files would move.
    ///
    /// Counts only records whose files are actually on disk: a total including
    /// bytes nobody can copy is a progress bar that stops short of its end for
    /// a reason the user cannot see.
    pub files: usize,
    /// How many bytes would move, from the sizes verified at download time.
    pub bytes: u64,
    /// Whether every file can be renamed rather than copied.
    ///
    /// The difference between instant and an hour, which is the whole reason
    /// the question is asked before the confirmation rather than after it.
    pub same_volume: bool,
}

/// The nearest ancestor of `path` that exists, including `path` itself.
///
/// The destination folder is named in Settings before it is created, so the
/// question "which volume is this on" has to be answerable about a path that is
/// not there yet.
fn nearest_existing(path: &Path) -> Option<&Path> {
    let mut candidate = path;

    loop {
        if candidate.exists() {
            return Some(candidate);
        }
        candidate = candidate.parent()?;
    }
}

/// Whether a file in `from` could be renamed into `to`.
///
/// Answered by actually renaming an empty probe file rather than by comparing
/// drive letters on Windows and `st_dev` on Unix. Those comparisons are
/// per-platform code that is wrong in exactly the cases that matter — a
/// directory junction, a mount point, a bind mount and a network share can all
/// share a prefix with a different volume, or differ in prefix on the same one
/// — whereas the probe asks the filesystem the same question the move will ask
/// it, on every platform, for the cost of one empty file.
///
/// Anything that goes wrong answers `false`. Copy-and-verify is always safe;
/// attempting a rename that was going to fail is not, on 20 GB.
fn same_volume(from: &Path, to: &Path) -> bool {
    let (Some(source), Some(landing)) = (nearest_existing(from), nearest_existing(to)) else {
        return false;
    };

    // Unique per process and per call, so two osstat windows probing the same
    // folder at once cannot delete each other's probe.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let name = format!(".osstat-move-probe-{}-{nanos}", std::process::id());

    let probe = source.join(&name);
    if std::fs::write(&probe, b"").is_err() {
        return false;
    }

    let moved = landing.join(&name);
    let renamed = std::fs::rename(&probe, &moved).is_ok();

    // Whichever of the two exists is removed; the other call is a no-op.
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&moved);

    renamed
}

/// Describes what moving the library to `to` would cost, without moving it.
#[must_use]
pub fn plan_move(store: &ModelStore, to: &Path) -> MovePlan {
    let present = store.present();
    let bytes = present.iter().fold(0_u64, |total, record| {
        total.saturating_add(record.size_bytes)
    });

    // Records hold absolute paths and can name more than one folder, so every
    // distinct source directory has to clear the bar for the move as a whole to
    // be the instant one. Deduplicated because the probe writes a file, and in
    // the ordinary case of one folder that makes this exactly one probe.
    let mut directories: Vec<&Path> = present
        .iter()
        .filter_map(|record| record.path.parent())
        .collect();
    directories.sort_unstable();
    directories.dedup();

    MovePlan {
        files: present.len(),
        bytes,
        same_volume: directories
            .into_iter()
            .all(|directory| same_volume(directory, to)),
    }
}

/// Moves every downloaded model into `to`, updating the index as it goes.
///
/// Progress is reported against the whole library rather than one file at a
/// time: a bar that restarts at each file cannot answer the only question being
/// asked of it.
///
/// # Errors
///
/// [`AcquireError::ChecksumMismatch`] if a copy does not verify at its
/// destination, in which case the source is still there and its record still
/// names it. [`AcquireError::Io`] for a folder that cannot be created or a file
/// that cannot be read or written. Whatever moved before the failure stays
/// moved, with its records updated to match.
pub async fn move_library(
    store: &mut ModelStore,
    to: &Path,
    // `+ Send` because the caller spawns this onto Tauri's async runtime, and a
    // future holding a non-Send callback cannot cross a thread boundary.
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), AcquireError> {
    let plan = plan_move(store, to);

    move_all(store, to, plan, on_progress).await
}

/// The body of [`move_library`], with the plan supplied rather than derived.
///
/// Split out so the cross-volume path — the one with an order that has to be
/// right — is reachable from a test on a machine with a single volume.
async fn move_all(
    store: &mut ModelStore,
    to: &Path,
    plan: MovePlan,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), AcquireError> {
    std::fs::create_dir_all(to).map_err(|source| AcquireError::Io {
        path: to.to_path_buf(),
        source,
    })?;

    let mut moved_bytes = 0_u64;

    for record in store.records() {
        // A record whose file is already gone is left exactly as it is. One
        // absent file must not abort the move of the rest, and rewriting its
        // path to a folder it was never in would turn a legible "missing" into
        // a lie about where to look.
        if !record.path.is_file() {
            continue;
        }
        let Some(name) = record.path.file_name() else {
            continue;
        };

        let destination = to.join(name);
        let size_bytes = record.size_bytes;

        if destination != record.path {
            let landed = move_one(
                &record,
                &destination,
                plan.same_volume,
                moved_bytes,
                plan.bytes,
                on_progress,
            )
            .await?;
            store.record(ModelRecord {
                path: landed,
                ..record
            })?;
        }

        moved_bytes = moved_bytes.saturating_add(size_bytes);
    }

    Ok(())
}

/// Moves one file, and answers with where it ended up.
async fn move_one(
    record: &ModelRecord,
    destination: &Path,
    same_volume: bool,
    moved_bytes: u64,
    total_bytes: u64,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<PathBuf, AcquireError> {
    // A rename is atomic: there is no moment at which the bytes exist in
    // neither place, so the rule this module is built around holds trivially.
    //
    // A rename that fails despite the probe falls through to the copy rather
    // than failing the move. The probe answers about a volume; this answers
    // about this file, and copy-and-verify is correct either way.
    if same_volume && std::fs::rename(&record.path, destination).is_ok() {
        on_progress(Progress {
            downloaded_bytes: moved_bytes.saturating_add(record.size_bytes),
            total_bytes,
        });
        return Ok(destination.to_path_buf());
    }

    copy_verify_delete(record, destination, moved_bytes, total_bytes, on_progress).await
}

/// Copies one file across volumes, in the only order that cannot lose it.
///
/// Copy, then hash what landed, then — and only then — delete the source. Every
/// other order has a window in which a full disk, a yanked drive or a power cut
/// destroys the only copy of something that took an hour to fetch.
///
/// The hash is taken from the destination rather than carried from the read, so
/// what is verified is the bytes that are actually on the far disk.
async fn copy_verify_delete(
    record: &ModelRecord,
    destination: &Path,
    moved_bytes: u64,
    total_bytes: u64,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<PathBuf, AcquireError> {
    copy_with_progress(
        &record.path,
        destination,
        moved_bytes,
        total_bytes,
        on_progress,
    )
    .await?;

    let verified = match sha256_file(destination) {
        Ok(actual) if actual == record.sha256 => Ok(()),
        Ok(actual) => Err(AcquireError::ChecksumMismatch {
            file: file_name_of(destination),
            expected: record.sha256.clone(),
            actual,
        }),
        Err(error) => Err(error),
    };

    if let Err(error) = verified {
        // The copy is removed and the source is not touched at all. Leaving an
        // unverified file where a model file is expected would let a later run
        // find it and trust it.
        let _ = std::fs::remove_file(destination);
        return Err(error);
    }

    // Only now, and the failure is deliberately swallowed: the destination has
    // verified, so a source that will not delete is a wasted copy rather than a
    // lost one. Returning an error here would strand a verified file with no
    // record naming it, which is the worse of the two outcomes by far.
    let _ = std::fs::remove_file(&record.path);

    Ok(destination.to_path_buf())
}

/// Copies `from` to `to`, reporting library-wide progress as it goes.
///
/// `moved_bytes` is what the rest of the library has already contributed, so a
/// bar spanning several files never goes backwards.
async fn copy_with_progress(
    from: &Path,
    to: &Path,
    moved_bytes: u64,
    total_bytes: u64,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), AcquireError> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut source = tokio::fs::File::open(from)
        .await
        .map_err(|source| AcquireError::Io {
            path: from.to_path_buf(),
            source,
        })?;
    let mut sink = tokio::fs::File::create(to)
        .await
        .map_err(|source| AcquireError::Io {
            path: to.to_path_buf(),
            source,
        })?;

    let mut buffer = vec![0_u8; COPY_CHUNK_BYTES];
    let mut done = moved_bytes;

    let outcome = async {
        loop {
            let read = source
                .read(&mut buffer)
                .await
                .map_err(|source| AcquireError::Io {
                    path: from.to_path_buf(),
                    source,
                })?;
            if read == 0 {
                break;
            }

            sink.write_all(&buffer[..read])
                .await
                .map_err(|source| AcquireError::Io {
                    path: to.to_path_buf(),
                    source,
                })?;

            done = done.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            on_progress(Progress {
                downloaded_bytes: done,
                total_bytes,
            });
        }

        sink.flush().await.map_err(|source| AcquireError::Io {
            path: to.to_path_buf(),
            source,
        })
    }
    .await;

    // Closed before anything removes the copy, which matters on Windows where
    // an open handle blocks it. Same reason as `download.rs`.
    drop(sink);

    if outcome.is_err() {
        // A truncated file under a model's name is worse than no file: a later
        // run would find it and have no way to know it is half of one.
        let _ = std::fs::remove_file(to);
    }

    outcome
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::download::sha256_file;
    use crate::model_store::{ModelRecord, ModelStore};
    use crate::models::ModelKey;
    use std::path::{Path, PathBuf};

    /// Writes `bytes` at `path` and returns a record naming it, hashed truly.
    fn downloaded(path: PathBuf, model_id: &str, bytes: &[u8]) -> ModelRecord {
        std::fs::write(&path, bytes).unwrap();
        ModelRecord {
            key: ModelKey {
                model_id: model_id.to_owned(),
                quant_id: "Q4_K_M".to_owned(),
            },
            sha256: sha256_file(&path).unwrap(),
            path,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            publisher: "bartowski".to_owned(),
            repo: format!("bartowski/{model_id}-GGUF"),
            provenance: crate::model_store::Provenance::Pinned,
            projector_path: None,
        }
    }

    /// A callback that counts nothing, for tests that do not read progress.
    fn ignored() -> impl FnMut(Progress) + Send {
        |_| {}
    }

    #[tokio::test]
    async fn a_same_volume_move_relocates_every_record() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("old");
        let to = root.path().join("new");
        std::fs::create_dir_all(&from).unwrap();
        let mut store = ModelStore::new(root.path().join("models.json"));
        let first = downloaded(from.join("qwen.gguf"), "qwen2.5-0.5b", b"one");
        let second = downloaded(from.join("llama.gguf"), "llama-3.2-1b", b"two");
        store.record(first.clone()).unwrap();
        store.record(second.clone()).unwrap();

        move_library(&mut store, &to, &mut ignored()).await.unwrap();

        let mut paths: Vec<PathBuf> = store.records().into_iter().map(|r| r.path).collect();
        paths.sort_unstable();
        assert_eq!(paths, vec![to.join("llama.gguf"), to.join("qwen.gguf")]);
        assert!(!first.path.exists(), "the source was left behind");
        assert!(!second.path.exists(), "the source was left behind");
        assert_eq!(
            store.present().len(),
            2,
            "a moved record must still resolve"
        );
    }

    #[tokio::test]
    async fn a_failure_after_the_copy_leaves_the_source_intact() {
        // THE load-bearing test. A move that deletes before verifying can lose
        // a 20 GB file to a full disk or a yanked drive. The injected failure
        // is a record whose pinned hash does not describe its bytes, which
        // fails at exactly the point between the copy and the delete.
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        std::fs::create_dir_all(&to).unwrap();
        let record = ModelRecord {
            sha256: "a".repeat(64),
            ..downloaded(root.path().join("qwen.gguf"), "qwen2.5-0.5b", b"weights")
        };
        let truthful = sha256_file(&record.path).unwrap();

        // `same_volume: false` is the cross-volume path: copy, verify, delete.
        let outcome = move_one(
            &record,
            &to.join("qwen.gguf"),
            false,
            0,
            record.size_bytes,
            &mut ignored(),
        )
        .await;

        assert!(
            outcome.is_err(),
            "a wrong hash was accepted at the destination"
        );
        assert!(
            record.path.is_file(),
            "the source was deleted before the destination verified"
        );
        assert_eq!(
            sha256_file(&record.path).unwrap(),
            truthful,
            "the source survived but not intact"
        );
        assert!(
            !to.join("qwen.gguf").exists(),
            "an unverified copy was left where a model file is expected"
        );
    }

    #[tokio::test]
    async fn a_failed_move_leaves_the_record_pointing_at_the_file_that_still_exists() {
        // The other half of the property above: bytes that survive a failure
        // are worthless if the index has already been told they moved.
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        let mut store = ModelStore::new(root.path().join("models.json"));
        let record = ModelRecord {
            sha256: "a".repeat(64),
            ..downloaded(root.path().join("qwen.gguf"), "qwen2.5-0.5b", b"weights")
        };
        store.record(record.clone()).unwrap();

        let plan = MovePlan {
            files: 1,
            bytes: record.size_bytes,
            same_volume: false,
        };
        let outcome = move_all(&mut store, &to, plan, &mut ignored()).await;

        assert!(
            outcome.is_err(),
            "a wrong hash was accepted at the destination"
        );
        assert_eq!(store.path_of(&record.key), Some(record.path));
    }

    #[tokio::test]
    async fn a_cross_volume_move_removes_the_source_once_the_copy_verifies() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        std::fs::create_dir_all(&to).unwrap();
        let record = downloaded(root.path().join("qwen.gguf"), "qwen2.5-0.5b", b"weights");
        let landing = to.join("qwen.gguf");

        let landed = move_one(
            &record,
            &landing,
            false,
            0,
            record.size_bytes,
            &mut ignored(),
        )
        .await
        .unwrap();

        assert_eq!(landed, landing);
        assert!(!record.path.exists(), "the source outlived a verified copy");
        assert_eq!(sha256_file(&landing).unwrap(), record.sha256);
    }

    #[test]
    fn a_move_reports_what_it_will_do_before_doing_it() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("new");
        let store = ModelStore::new(root.path().join("models.json"));
        store
            .record(downloaded(
                root.path().join("a.gguf"),
                "qwen2.5-0.5b",
                &vec![0_u8; 1024],
            ))
            .unwrap();
        store
            .record(downloaded(
                root.path().join("b.gguf"),
                "llama-3.2-1b",
                &vec![0_u8; 2048],
            ))
            .unwrap();

        let plan = plan_move(&store, &elsewhere);

        assert_eq!(plan.files, 2);
        assert_eq!(plan.bytes, 3072);
    }

    #[test]
    fn a_plan_counts_only_the_files_that_are_actually_there() {
        // A total that includes bytes nobody can copy is a progress bar that
        // stops short of the end for a reason the user cannot see.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        let here = downloaded(
            root.path().join("a.gguf"),
            "qwen2.5-0.5b",
            &vec![0_u8; 1024],
        );
        let gone = ModelRecord {
            path: root.path().join("vanished.gguf"),
            ..downloaded(
                root.path().join("b.gguf"),
                "llama-3.2-1b",
                &vec![0_u8; 2048],
            )
        };
        store.record(here).unwrap();
        store.record(gone).unwrap();

        let plan = plan_move(&store, &root.path().join("new"));

        assert_eq!(plan.files, 1);
        assert_eq!(plan.bytes, 1024);
    }

    #[test]
    fn a_move_within_one_temporary_directory_is_recognised_as_the_same_volume() {
        // The probe's whole job. A same-volume move renames rather than
        // copying, which is the difference between instant and an hour.
        let root = tempfile::tempdir().unwrap();
        let store = ModelStore::new(root.path().join("models.json"));
        store
            .record(downloaded(
                root.path().join("a.gguf"),
                "qwen2.5-0.5b",
                b"one",
            ))
            .unwrap();

        assert!(plan_move(&store, &root.path().join("new")).same_volume);
    }

    #[tokio::test]
    async fn a_record_whose_file_is_already_missing_is_skipped_not_fatal() {
        // One absent file must not abort the move of the rest.
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        let mut store = ModelStore::new(root.path().join("models.json"));
        let gone = ModelRecord {
            path: root.path().join("vanished.gguf"),
            ..downloaded(root.path().join("scratch.gguf"), "llama-3.2-1b", b"two")
        };
        let here = downloaded(root.path().join("a.gguf"), "qwen2.5-0.5b", b"one");
        store.record(gone.clone()).unwrap();
        store.record(here.clone()).unwrap();

        move_library(&mut store, &to, &mut ignored()).await.unwrap();

        assert_eq!(store.path_of(&here.key), Some(to.join("a.gguf")));
        assert_eq!(
            store
                .records()
                .into_iter()
                .find(|r| r.key == gone.key)
                .map(|r| r.path),
            Some(gone.path),
            "a record with no file must be left alone rather than rewritten"
        );
    }

    #[tokio::test]
    async fn moving_a_library_into_the_folder_it_is_already_in_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut store = ModelStore::new(root.path().join("models.json"));
        let record = downloaded(root.path().join("a.gguf"), "qwen2.5-0.5b", b"one");
        store.record(record.clone()).unwrap();

        move_library(&mut store, root.path(), &mut ignored())
            .await
            .unwrap();

        assert_eq!(store.records(), vec![record.clone()]);
        assert!(record.path.is_file(), "a no-op move deleted the file");
    }

    #[tokio::test]
    async fn an_empty_library_moves_without_incident() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        let mut store = ModelStore::new(root.path().join("models.json"));

        move_library(&mut store, &to, &mut ignored()).await.unwrap();

        assert!(to.is_dir(), "the chosen folder should exist afterwards");
    }

    #[tokio::test]
    async fn progress_is_reported_against_the_whole_library_rather_than_one_file() {
        // A bar that restarts at each file cannot say how long the move has
        // left, which is the only question being asked of it.
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        std::fs::create_dir_all(&to).unwrap();
        let record = downloaded(
            root.path().join("b.gguf"),
            "llama-3.2-1b",
            &vec![7_u8; 4096],
        );
        let mut seen: Vec<Progress> = Vec::new();

        move_one(&record, &to.join("b.gguf"), false, 8192, 12_288, &mut |p| {
            seen.push(p);
        })
        .await
        .unwrap();

        let last = seen.last().copied().expect("a copy reported no progress");
        assert_eq!(last.total_bytes, 12_288);
        assert_eq!(
            last.downloaded_bytes, 12_288,
            "the bytes already moved must be carried, or the bar goes backwards"
        );
    }

    #[test]
    fn a_probe_against_a_folder_that_does_not_exist_yet_still_answers() {
        // The real call site: Settings names a folder before it is created.
        let root = tempfile::tempdir().unwrap();

        assert!(same_volume(
            root.path(),
            &root.path().join("deep/not/made/yet")
        ));
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let root = tempfile::tempdir().unwrap();
        let to = root.path().join("new");
        std::fs::create_dir_all(&to).unwrap();

        assert!(same_volume(root.path(), &to));

        for directory in [root.path(), to.as_path()] {
            let strays: Vec<PathBuf> = std::fs::read_dir(directory)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path != &to)
                .collect();
            assert!(strays.is_empty(), "the probe left {strays:?}");
        }
    }

    #[test]
    fn an_impossible_destination_is_treated_as_a_different_volume() {
        // Conservative by design: copy-and-verify is always safe, and a rename
        // that was going to fail anyway should not be attempted on 20 GB.
        let root = tempfile::tempdir().unwrap();

        assert!(!same_volume(root.path(), Path::new("")));
    }
}
