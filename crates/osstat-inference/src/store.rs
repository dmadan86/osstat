//! Where acquired runtimes live, and what is currently installed.
//!
//! Keyed by upstream tag as well as artifact, so a new pin can be fetched
//! without clobbering a working runtime in place. Two versions coexisting for a
//! while is the point, not an accident.
//!
//! osstat is also a disk cleaner. Anything it downloads must be listed with its
//! size and be deletable from inside the app: leaving hundreds of megabytes
//! around that osstat does not admit to is precisely the behaviour this project
//! exists to complain about.

use std::path::{Path, PathBuf};

use crate::error::AcquireError;
use crate::unpack::find_server;

/// A runtime present on disk and usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRuntime {
    /// The upstream release tag it came from.
    pub tag: String,
    /// The artifact identifier from `runtimes.json`.
    pub artifact_id: String,
    /// Absolute path to the server executable.
    pub server_path: PathBuf,
    /// Total bytes this install occupies.
    pub size_bytes: u64,
}

/// The directory tree acquired runtimes live in.
#[derive(Debug, Clone)]
pub struct RuntimeStore {
    root: PathBuf,
}

impl RuntimeStore {
    /// Creates a store rooted at `root`, typically the Tauri app-data directory.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The directory every runtime is installed under.
    #[must_use]
    pub fn runtimes_dir(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    /// Where a given tag and artifact are installed.
    #[must_use]
    pub fn path_for(&self, tag: &str, artifact_id: &str) -> PathBuf {
        self.runtimes_dir().join(tag).join(artifact_id)
    }

    /// Every usable runtime on disk, ordered by tag then artifact.
    ///
    /// A directory with no server executable in it is a half-extracted install
    /// and is not reported. Offering it would fail later, further from the
    /// cause, and with a worse message.
    #[must_use]
    pub fn installed(&self) -> Vec<InstalledRuntime> {
        let mut found = Vec::new();

        let Ok(tags) = std::fs::read_dir(self.runtimes_dir()) else {
            return found;
        };

        for tag_entry in tags.flatten() {
            let Ok(artifacts) = std::fs::read_dir(tag_entry.path()) else {
                continue;
            };
            for artifact_entry in artifacts.flatten() {
                let path = artifact_entry.path();
                let Some(server_path) = find_server(&path) else {
                    continue;
                };
                found.push(InstalledRuntime {
                    tag: tag_entry.file_name().to_string_lossy().into_owned(),
                    artifact_id: artifact_entry.file_name().to_string_lossy().into_owned(),
                    server_path,
                    size_bytes: directory_size(&path),
                });
            }
        }

        found.sort_by(|a, b| (&a.tag, &a.artifact_id).cmp(&(&b.tag, &b.artifact_id)));
        found
    }

    /// Deletes an installed runtime.
    ///
    /// Removing something that is not there succeeds: the caller asked for it
    /// to be gone, and it is.
    ///
    /// # Errors
    ///
    /// [`AcquireError::Io`] if the directory exists but cannot be removed.
    pub fn remove(&self, tag: &str, artifact_id: &str) -> Result<(), AcquireError> {
        let path = self.path_for(tag, artifact_id);
        if !path.exists() {
            return Ok(());
        }

        std::fs::remove_dir_all(&path).map_err(|source| AcquireError::Io { path, source })
    }
}

/// Sums the size of every file under `path`.
///
/// Unreadable subdirectories are skipped rather than failing the whole figure:
/// a size shown to a user is better slightly low than absent.
pub(crate) fn directory_size(path: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![path.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else if let Ok(metadata) = entry.metadata() {
                total = total.saturating_add(metadata.len());
            }
        }
    }

    total
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::unpack::server_file_name;

    /// Creates a store with one runtime installed, whose server holds `body`.
    fn store_with_install(tag: &str, id: &str, body: &[u8]) -> (tempfile::TempDir, RuntimeStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());
        let bin = store.path_for(tag, id).join("build/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(server_file_name()), body).unwrap();

        (dir, store)
    }

    #[test]
    fn an_empty_store_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());

        assert!(store.installed().is_empty());
    }

    #[test]
    fn a_store_whose_directory_does_not_exist_lists_nothing() {
        // The state on a fresh install. It must not be an error.
        let store = RuntimeStore::new(PathBuf::from("/definitely/not/a/real/path"));

        assert!(store.installed().is_empty());
    }

    #[test]
    fn an_installed_runtime_is_listed_with_its_size() {
        let (_dir, store) = store_with_install("b10194", "ubuntu-vulkan-x64", b"0123456789");

        let installed = store.installed();

        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].tag, "b10194");
        assert_eq!(installed[0].artifact_id, "ubuntu-vulkan-x64");
        assert_eq!(installed[0].size_bytes, 10);
        assert!(installed[0].server_path.ends_with(server_file_name()));
    }

    #[test]
    fn two_pinned_versions_can_coexist() {
        // Keyed by tag as well as artifact, so an upgrade never clobbers a
        // working runtime in place.
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());

        for tag in ["b10194", "b10200"] {
            let bin = store.path_for(tag, "ubuntu-vulkan-x64").join("build/bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join(server_file_name()), b"x").unwrap();
        }

        let installed = store.installed();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].tag, "b10194", "listing is ordered by tag");
        assert_eq!(installed[1].tag, "b10200");
    }

    #[test]
    fn a_directory_without_a_server_is_not_reported_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());
        std::fs::create_dir_all(store.path_for("b10194", "ubuntu-vulkan-x64")).unwrap();

        assert!(
            store.installed().is_empty(),
            "a half-extracted directory is not a usable runtime"
        );
    }

    #[test]
    fn removing_a_runtime_deletes_it() {
        let (_dir, store) = store_with_install("b10194", "ubuntu-vulkan-x64", b"x");

        store.remove("b10194", "ubuntu-vulkan-x64").unwrap();

        assert!(store.installed().is_empty());
    }

    #[test]
    fn removing_one_runtime_leaves_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());
        for id in ["ubuntu-vulkan-x64", "ubuntu-x64"] {
            let bin = store.path_for("b10194", id).join("build/bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join(server_file_name()), b"x").unwrap();
        }

        store.remove("b10194", "ubuntu-vulkan-x64").unwrap();

        let installed = store.installed();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].artifact_id, "ubuntu-x64");
    }

    #[test]
    fn removing_something_that_is_not_there_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = RuntimeStore::new(dir.path().to_path_buf());

        assert!(
            store.remove("b10194", "ubuntu-vulkan-x64").is_ok(),
            "the caller asked for it to be gone, and it is"
        );
    }

    #[test]
    fn directory_size_counts_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/one.bin"), b"1234").unwrap();
        std::fs::write(dir.path().join("a/b/two.bin"), b"567").unwrap();

        assert_eq!(directory_size(dir.path()), 7);
    }

    #[test]
    fn directory_size_of_nothing_is_zero() {
        assert_eq!(directory_size(Path::new("/not/a/real/path")), 0);
    }
}
