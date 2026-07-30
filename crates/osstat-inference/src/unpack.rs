//! Unpacking a verified archive into a destination tree.
//!
//! Upstream ships `.zip` on Windows and `.tar.gz` elsewhere, so both are
//! handled here rather than branching at the call site.
//!
//! Every entry's destination is checked to be inside the target directory
//! before anything is written. A verified hash proves the archive is the one
//! that was pinned; it says nothing about whether its contents are well-behaved,
//! and osstat already refuses path traversal in cleaning rules (SECURITY.md
//! threat 2). An archive deserves no more trust than a manifest.

use std::path::{Component, Path, PathBuf};

use crate::error::AcquireError;

/// Extracts `archive` into `into`, creating the directory if needed.
///
/// # Errors
///
/// [`AcquireError::Extraction`] if the archive is unreadable or any entry would
/// be written outside `into`; [`AcquireError::Io`] for filesystem failures.
pub fn unpack(archive: &Path, into: &Path) -> Result<(), AcquireError> {
    std::fs::create_dir_all(into).map_err(|source| AcquireError::Io {
        path: into.to_path_buf(),
        source,
    })?;

    let name = archive.file_name().map_or_else(
        || archive.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );

    if archive
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        unpack_zip(archive, into, &name)
    } else {
        unpack_tar_gz(archive, into, &name)
    }
}

/// Joins `entry` onto `root`, refusing anything that would escape it.
///
/// `..`, absolute paths and Windows drive prefixes are all rejected outright
/// rather than normalised away: an archive containing one is not something to
/// be repaired, it is something to be refused.
fn safe_destination(root: &Path, entry: &Path) -> Option<PathBuf> {
    let mut destination = root.to_path_buf();

    for component in entry.components() {
        match component {
            Component::Normal(part) => destination.push(part),
            // `.` is harmless and meaningless; skip it.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    destination.starts_with(root).then_some(destination)
}

/// Builds the error for an entry that tried to leave its directory.
fn escaped(name: &str, entry: &str) -> AcquireError {
    AcquireError::Extraction {
        file: name.to_owned(),
        message: format!("entry {entry} would be written outside the runtime directory"),
    }
}

/// Extracts a zip archive.
fn unpack_zip(archive: &Path, into: &Path, name: &str) -> Result<(), AcquireError> {
    let file = std::fs::File::open(archive).map_err(|source| AcquireError::Io {
        path: archive.to_path_buf(),
        source,
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| AcquireError::Extraction {
        file: name.to_owned(),
        message: error.to_string(),
    })?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| AcquireError::Extraction {
                file: name.to_owned(),
                message: error.to_string(),
            })?;

        let raw_name = entry.name().to_owned();
        let Some(relative) = entry.enclosed_name() else {
            return Err(escaped(name, &raw_name));
        };
        let Some(destination) = safe_destination(into, &relative) else {
            return Err(escaped(name, &raw_name));
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|source| AcquireError::Io {
                path: destination.clone(),
                source,
            })?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AcquireError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut out = std::fs::File::create(&destination).map_err(|source| AcquireError::Io {
            path: destination.clone(),
            source,
        })?;
        std::io::copy(&mut entry, &mut out).map_err(|source| AcquireError::Io {
            path: destination.clone(),
            source,
        })?;
    }

    Ok(())
}

/// Extracts a gzipped tar archive.
fn unpack_tar_gz(archive: &Path, into: &Path, name: &str) -> Result<(), AcquireError> {
    let file = std::fs::File::open(archive).map_err(|source| AcquireError::Io {
        path: archive.to_path_buf(),
        source,
    })?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    let entries = tar.entries().map_err(|error| AcquireError::Extraction {
        file: name.to_owned(),
        message: error.to_string(),
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|error| AcquireError::Extraction {
            file: name.to_owned(),
            message: error.to_string(),
        })?;

        let relative = entry
            .path()
            .map_err(|error| AcquireError::Extraction {
                file: name.to_owned(),
                message: error.to_string(),
            })?
            .into_owned();

        let Some(destination) = safe_destination(into, &relative) else {
            return Err(escaped(name, &relative.display().to_string()));
        };

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| AcquireError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        entry
            .unpack(&destination)
            .map_err(|error| AcquireError::Extraction {
                file: name.to_owned(),
                message: error.to_string(),
            })?;
    }

    Ok(())
}

/// The filename of the llama.cpp server on this platform.
#[must_use]
pub const fn server_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Finds the llama.cpp server anywhere under `root`.
///
/// Located rather than assumed at a fixed path: upstream's archive layout has
/// moved between releases, and a pin that ages is exactly the situation where
/// a hard-coded `build/bin/` would break quietly.
#[must_use]
pub fn find_server(root: &Path) -> Option<PathBuf> {
    let wanted = server_file_name();
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == wanted) {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Builds a .tar.gz containing one entry at `entry_path` holding `body`.
    fn tar_gz_with(dir: &Path, file_name: &str, entry_path: &str, body: &[u8]) -> PathBuf {
        let path = dir.join(file_name);
        let file = std::fs::File::create(&path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, entry_path, body).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        path
    }

    /// Builds a .tar.gz whose entry name is written into the header directly.
    ///
    /// Both `Builder::append_data` and `Header::set_path` refuse a name
    /// containing `..`, so the tar crate will not let a hostile fixture be
    /// built through its safe API. Writing the 100-byte name field by hand is
    /// the only way to produce the archive an attacker would, and testing
    /// against anything less would be testing the tar crate's guard rather
    /// than ours.
    fn tar_gz_with_raw_name(dir: &Path, file_name: &str, entry_path: &str, body: &[u8]) -> PathBuf {
        let path = dir.join(file_name);
        let file = std::fs::File::create(&path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);

        let name = entry_path.as_bytes();
        assert!(
            name.len() < 100,
            "fixture name must fit the ustar name field"
        );
        header.as_mut_bytes()[..name.len()].copy_from_slice(name);

        header.set_cksum();
        builder.append(&header, body).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        path
    }

    /// Builds a .zip containing one entry at `entry_path` holding `body`.
    fn zip_with(dir: &Path, file_name: &str, entry_path: &str, body: &[u8]) -> PathBuf {
        use std::io::Write as _;

        let path = dir.join(file_name);
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);

        writer
            .start_file(entry_path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(body).unwrap();
        writer.finish().unwrap();

        path
    }

    #[test]
    fn a_tar_gz_extracts_and_the_server_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz_with(
            dir.path(),
            "fixture.tar.gz",
            &format!("build/bin/{}", server_file_name()),
            b"#!/bin/sh\n",
        );
        let into = dir.path().join("out");

        unpack(&archive, &into).unwrap();

        let server = find_server(&into).expect("llama-server should be found after extraction");
        assert_eq!(std::fs::read(&server).unwrap(), b"#!/bin/sh\n");
    }

    #[test]
    fn a_zip_extracts_and_the_server_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let archive = zip_with(
            dir.path(),
            "fixture.zip",
            &format!("build/bin/{}", server_file_name()),
            b"MZ fake",
        );
        let into = dir.path().join("out");

        unpack(&archive, &into).unwrap();

        assert!(find_server(&into).is_some());
    }

    #[test]
    fn an_archive_without_the_server_is_detected() {
        // The archive verified but holds the wrong thing. That is a pin
        // describing the wrong file, and the caller reports it as such.
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz_with(
            dir.path(),
            "fixture.tar.gz",
            "build/bin/llama-cli",
            b"not the server",
        );
        let into = dir.path().join("out");

        unpack(&archive, &into).unwrap();

        assert!(
            find_server(&into).is_none(),
            "an archive that verified but holds no server must be detectable"
        );
    }

    #[test]
    fn a_corrupt_archive_reports_extraction_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("broken.tar.gz");
        std::fs::write(&archive, b"this is not a gzip stream").unwrap();

        let error = unpack(&archive, &dir.path().join("out")).unwrap_err();

        assert!(matches!(error, AcquireError::Extraction { .. }));
    }

    #[test]
    fn a_tar_entry_escaping_the_destination_is_refused() {
        // Zip-slip, through tar. osstat refuses this for cleaning rules and an
        // archive gets no more trust than a manifest does.
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz_with_raw_name(dir.path(), "evil.tar.gz", "../escaped.txt", b"owned");
        let into = dir.path().join("out");

        let error = unpack(&archive, &into).unwrap_err();

        assert!(matches!(error, AcquireError::Extraction { .. }));
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "a tar entry escaped its root"
        );
    }

    #[test]
    fn a_zip_entry_escaping_the_destination_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = zip_with(dir.path(), "evil.zip", "../escaped.txt", b"owned");
        let into = dir.path().join("out");

        let error = unpack(&archive, &into).unwrap_err();

        assert!(matches!(error, AcquireError::Extraction { .. }));
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "a zip entry escaped its root"
        );
    }

    #[test]
    fn a_deeply_nested_escape_is_refused_too() {
        // `a/../../escaped` normalises to an escape only after traversal, which
        // is the case a naive `starts_with` on the raw string would miss.
        let dir = tempfile::tempdir().unwrap();
        let archive =
            tar_gz_with_raw_name(dir.path(), "evil2.tar.gz", "a/../../escaped.txt", b"owned");
        let into = dir.path().join("out");

        let error = unpack(&archive, &into).unwrap_err();

        assert!(matches!(error, AcquireError::Extraction { .. }));
    }

    #[test]
    fn an_absolute_entry_is_refused() {
        assert!(safe_destination(Path::new("/root"), Path::new("/etc/passwd")).is_none());
    }

    #[test]
    fn a_current_directory_component_is_harmless() {
        let resolved = safe_destination(Path::new("/root"), Path::new("./build/bin/x"));

        assert_eq!(resolved, Some(PathBuf::from("/root/build/bin/x")));
    }

    #[test]
    fn an_ordinary_relative_entry_resolves_inside_the_root() {
        let resolved = safe_destination(Path::new("/root"), Path::new("build/bin/llama-server"));

        assert_eq!(
            resolved,
            Some(PathBuf::from("/root/build/bin/llama-server"))
        );
    }

    #[test]
    fn finding_a_server_in_an_absent_directory_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();

        assert!(find_server(&dir.path().join("nope")).is_none());
    }
}
