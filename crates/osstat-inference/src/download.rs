//! Fetching an archive and proving it is the one that was pinned.
//!
//! Downloads land in a `.partial` file beside the destination and are moved
//! into place only after the hash matches. An interrupted or tampered download
//! therefore can never be mistaken for a usable runtime, and nothing partial
//! survives a failure for a later step to find and trust.
//!
//! The hash is computed while streaming, so a 600 MB artifact is never held in
//! memory and never read from disk twice.

use futures_util::StreamExt as _;
use sha2::{Digest as _, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt as _;

use crate::error::AcquireError;

/// How far a download has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Bytes written so far.
    pub downloaded_bytes: u64,
    /// Bytes expected in total, from the pinned manifest.
    ///
    /// Taken from `runtimes.json` rather than the response's `Content-Length`,
    /// so a server cannot make a download look complete by understating it.
    pub total_bytes: u64,
}

/// Renders bytes as lower-case hex.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut out, byte| {
        // Writing to a String cannot fail; the Result is discarded knowingly.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Hashes a file on disk, returning lower-case hex.
///
/// # Errors
///
/// [`AcquireError::Io`] if the file cannot be opened or read.
pub fn sha256_file(path: &Path) -> Result<String, AcquireError> {
    let mut file = std::fs::File::open(path).map_err(|source| AcquireError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();

    std::io::copy(&mut file, &mut hasher).map_err(|source| AcquireError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(hex(&hasher.finalize()))
}

/// Names a path for an error message.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Downloads `url` to `dest`, only if it hashes to `expected_sha256`.
///
/// `expected_size` is used for progress reporting only; the hash is what
/// decides whether the file is accepted.
///
/// # Errors
///
/// [`AcquireError::ChecksumMismatch`] if the body does not match, in which case
/// nothing at all is left on disk. Also [`AcquireError::Network`],
/// [`AcquireError::HttpStatus`] and [`AcquireError::Io`].
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
    expected_size: u64,
    dest: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), AcquireError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|source| AcquireError::Network {
            url: url.to_owned(),
            source,
        })?;

    if !response.status().is_success() {
        return Err(AcquireError::HttpStatus {
            url: url.to_owned(),
            status: response.status().as_u16(),
        });
    }

    let temporary = dest.with_extension("partial");
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|source| AcquireError::Io {
            path: temporary.clone(),
            source,
        })?;

    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0_u64;
    let mut stream = response.bytes_stream();

    // Everything from here can fail, and every failure must leave the partial
    // file removed. The work is done in one block so the cleanup happens once.
    let outcome = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|source| AcquireError::Network {
                url: url.to_owned(),
                source,
            })?;

            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|source| AcquireError::Io {
                    path: temporary.clone(),
                    source,
                })?;

            downloaded_bytes =
                downloaded_bytes.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
            on_progress(Progress {
                downloaded_bytes,
                total_bytes: expected_size,
            });
        }

        file.flush().await.map_err(|source| AcquireError::Io {
            path: temporary.clone(),
            source,
        })?;

        let actual = hex(&hasher.finalize());
        if actual != expected_sha256 {
            return Err(AcquireError::ChecksumMismatch {
                file: file_name_of(dest),
                expected: expected_sha256.to_owned(),
                actual,
            });
        }

        Ok(())
    }
    .await;

    // The handle must be closed before the file is renamed or removed, which
    // matters on Windows where an open handle blocks both.
    drop(file);

    match outcome {
        Ok(()) => tokio::fs::rename(&temporary, dest)
            .await
            .map_err(|source| AcquireError::Io {
                path: dest.to_path_buf(),
                source,
            }),
        Err(error) => {
            // Best effort: the error being reported matters more than a failure
            // to tidy up, but nothing unverified may be left behind.
            let _ = tokio::fs::remove_file(&temporary).await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// SHA256 of the three bytes `abc`, the standard test vector.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// Serves `body` once on a loopback port, then stops. Returns the URL.
    ///
    /// A real socket rather than a mocked client: what is under test is
    /// streaming bytes to disk while hashing them, and a mock would replace the
    /// very thing that could be wrong.
    fn serve_once(status: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // The request must be drained before the response is written.
                // Replying to an unread socket and then closing it sends RST,
                // which discards the buffered response and reaches the client
                // as a transport error rather than the status under test.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }

                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });

        format!("http://127.0.0.1:{port}/artifact.bin")
    }

    #[tokio::test]
    async fn a_matching_body_is_written_to_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let url = serve_once("200 OK", b"abc".to_vec());

        download_verified(
            &reqwest::Client::new(),
            &url,
            ABC_SHA256,
            3,
            &dest,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
    }

    #[tokio::test]
    async fn a_mismatched_body_never_reaches_the_destination() {
        // The single most important test in this sub-project. A tampered or
        // corrupted archive must leave nothing behind that a later step could
        // extract, mark executable, or run.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let url = serve_once("200 OK", b"not abc at all".to_vec());

        let error = download_verified(
            &reqwest::Client::new(),
            &url,
            ABC_SHA256,
            3,
            &dest,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AcquireError::ChecksumMismatch { .. }));
        assert!(!error.is_retryable(), "a mismatch must not invite a retry");
        assert!(
            !dest.exists(),
            "a body that failed verification was left at the destination"
        );

        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert!(
            leftovers.is_empty(),
            "a temporary file survived a failed verification: {:?}",
            leftovers
                .iter()
                .map(std::fs::DirEntry::path)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn the_reported_mismatch_names_the_hash_that_actually_arrived() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let url = serve_once("200 OK", b"abc".to_vec());

        let wrong_expectation = "0".repeat(64);
        let error = download_verified(
            &reqwest::Client::new(),
            &url,
            &wrong_expectation,
            3,
            &dest,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        let mut checked = false;
        if let AcquireError::ChecksumMismatch {
            expected, actual, ..
        } = &error
        {
            assert_eq!(expected, &wrong_expectation);
            assert_eq!(
                actual, ABC_SHA256,
                "the real hash must be reported, not the expectation echoed back"
            );
            checked = true;
        }
        assert!(checked, "expected a checksum mismatch, got {error:?}");
    }

    #[tokio::test]
    async fn a_non_success_status_is_reported_with_its_code() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let url = serve_once("404 Not Found", Vec::new());

        let error = download_verified(
            &reqwest::Client::new(),
            &url,
            ABC_SHA256,
            3,
            &dest,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        let mut checked = false;
        if let AcquireError::HttpStatus { status, .. } = &error {
            assert_eq!(*status, 404);
            checked = true;
        }
        assert!(checked, "expected an HTTP status error, got {error:?}");
        assert!(
            error.is_retryable(),
            "a 404 may be a transient CDN failure; retrying is reasonable"
        );
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn progress_is_reported_and_ends_at_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let url = serve_once("200 OK", b"abc".to_vec());

        let mut seen = Vec::new();
        download_verified(
            &reqwest::Client::new(),
            &url,
            ABC_SHA256,
            3,
            &dest,
            &mut |progress| seen.push(progress.downloaded_bytes),
        )
        .await
        .unwrap();

        assert_eq!(seen.last().copied(), Some(3));
    }

    #[test]
    fn hashing_a_file_matches_the_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();

        assert_eq!(sha256_file(&path).unwrap(), ABC_SHA256);
    }

    #[test]
    fn hashing_a_file_that_is_not_there_is_an_io_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.bin");

        let error = sha256_file(&path).unwrap_err();

        assert!(matches!(error, AcquireError::Io { .. }));
        assert!(error.to_string().contains("absent.bin"));
    }
}
