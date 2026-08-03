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
    on_progress: &mut (dyn FnMut(Progress) + Send),
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

/// How many bytes of `part` are already on disk.
///
/// Anything that is not a readable file counts as nothing, so a directory or a
/// vanished path starts the download from zero rather than failing.
fn bytes_already_there(part: &Path) -> u64 {
    std::fs::metadata(part)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map_or(0, |metadata| metadata.len())
}

/// The whole-file size a response claims, if it says.
///
/// A `206` reports the total after the slash in `Content-Range`; its
/// `Content-Length` is only the slice being sent. A `200` reports the whole
/// file in `Content-Length`. Reading the wrong one of those would reject every
/// resumed download as the wrong size.
fn advertised_total(response: &reqwest::Response) -> Option<u64> {
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)?
            .to_str()
            .ok()?
            .rsplit('/')
            .next()?
            .trim()
            .parse()
            .ok();
    }

    response.content_length()
}

/// Fetches into `part`, resuming from the `already` bytes it holds.
///
/// Split out so the verification below runs against whatever ended up on disk,
/// whichever way the bytes got there.
async fn fetch_into_part(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    already: u64,
    expected_size: u64,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), AcquireError> {
    let mut request = client.get(url);
    if already > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={already}-"));
    }

    let response = request
        .send()
        .await
        .map_err(|source| AcquireError::Network {
            url: url.to_owned(),
            source,
        })?;
    let status = response.status();

    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // The part no longer lines up with what is being served. It has to go:
        // keeping it would ask for the same impossible range forever, and every
        // retry would fail identically.
        let _ = std::fs::remove_file(part);
        return Err(AcquireError::HttpStatus {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }

    if !status.is_success() {
        // The part is left alone. A 503 is exactly the case resuming exists
        // for, and discarding what had already arrived would be the worst
        // possible response to a transient failure.
        return Err(AcquireError::HttpStatus {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }

    // Checked here, before anything is written, so a changed upload costs
    // nothing and is reported as what it is.
    if let Some(total) = advertised_total(&response)
        && total != expected_size
    {
        return Err(AcquireError::SizeMismatch {
            file: file_name_of(part),
            expected_bytes: expected_size,
            actual_bytes: total,
        });
    }

    // A 206 is the server agreeing to carry on from `already`. Anything else is
    // the file from zero: appending that to what is already on disk produces a
    // file that is simply wrong, one that fails its hash for a reason nobody
    // could diagnose. So a 200 answering a ranged request truncates and starts
    // over, which costs bandwidth and never costs correctness.
    let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = if resuming {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .await
            .map_err(|source| AcquireError::Io {
                path: part.to_path_buf(),
                source,
            })?
    } else {
        tokio::fs::File::create(part)
            .await
            .map_err(|source| AcquireError::Io {
                path: part.to_path_buf(),
                source,
            })?
    };

    // Counted from what is already on disk, or a resumed download reports from
    // zero and appears to have restarted.
    let mut downloaded_bytes = if resuming { already } else { 0 };
    let mut stream = response.bytes_stream();

    let outcome = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|source| AcquireError::Network {
                url: url.to_owned(),
                source,
            })?;

            file.write_all(&chunk)
                .await
                .map_err(|source| AcquireError::Io {
                    path: part.to_path_buf(),
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
            path: part.to_path_buf(),
            source,
        })
    }
    .await;

    // Closed before anything renames or removes the part, which matters on
    // Windows where an open handle blocks both.
    drop(file);

    outcome
}

/// Downloads `url` into `destination`, resuming an interrupted attempt.
///
/// Bytes accumulate in `part`, which survives a dropped connection so the next
/// call can ask for only what is missing. `destination` is created by renaming
/// `part` and only after the whole file hashes to `expected_sha256`, so nothing
/// partial can ever be mistaken for a usable model.
///
/// Unlike [`download_verified`] the hash is computed at the end, in one pass
/// over the finished file, rather than while streaming: a resumed download
/// cannot carry the hash state of the attempt that was interrupted.
///
/// # Errors
///
/// [`AcquireError::SizeMismatch`] if the server offers a different size from
/// the pin, before anything is downloaded. [`AcquireError::ChecksumMismatch`]
/// if the finished file does not match, in which case neither `part` nor
/// `destination` is left behind. Also [`AcquireError::Network`],
/// [`AcquireError::HttpStatus`] and [`AcquireError::Io`] — after which `part`
/// is deliberately kept, because that is what makes the next attempt a resume.
pub async fn download_resumable(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    destination: &Path,
    expected_sha256: &str,
    expected_size: u64,
    // `+ Send` because the caller spawns this onto Tauri's async runtime, and a
    // future holding a non-Send callback cannot cross a thread boundary.
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), AcquireError> {
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AcquireError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    // A part longer than the pin cannot be a prefix of it, so there is nothing
    // to resume from and the whole file is fetched again. Clamping it to the
    // pinned length instead would make an oversized part look complete, and it
    // would then be verified rather than replaced.
    let on_disk = bytes_already_there(part);
    let already = if on_disk > expected_size { 0 } else { on_disk };

    // A part that is already the full length is verified rather than refetched.
    // Asking for `bytes=<size>-` would earn a 416 and throw away a download
    // that may well be complete and correct.
    if already < expected_size {
        fetch_into_part(client, url, part, already, expected_size, on_progress).await?;
    }

    let actual_size = bytes_already_there(part);
    let failed = if actual_size == expected_size {
        match sha256_file(part)? {
            actual if actual == expected_sha256 => None,
            actual => Some(AcquireError::ChecksumMismatch {
                file: file_name_of(destination),
                expected: expected_sha256.to_owned(),
                actual,
            }),
        }
    } else {
        Some(AcquireError::SizeMismatch {
            file: file_name_of(destination),
            expected_bytes: expected_size,
            actual_bytes: actual_size,
        })
    };

    if let Some(error) = failed {
        // Both, not just the part: a leftover part would be resumed next time
        // and fail identically, forever, and anything at the destination would
        // be a file this call was about to replace anyway.
        let _ = std::fs::remove_file(part);
        let _ = std::fs::remove_file(destination);
        return Err(error);
    }

    // Verified, so the rename cannot promote anything unusable.
    std::fs::rename(part, destination).map_err(|source| AcquireError::Io {
        path: destination.to_path_buf(),
        source,
    })
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

    // --- Resumable downloads ---------------------------------------------
    //
    // The fixture below is `serve_once` grown a range parser. Still a real
    // socket for the same reason: what is under test is the interaction
    // between a `Range` header, the status that comes back and the bytes
    // already on disk, and a mock would replace the very thing that could be
    // wrong. A `206` here is a genuine `206`, with a genuine `Content-Range`.

    /// What the fixture does with one connection.
    #[derive(Debug, Clone, Copy)]
    enum Answer {
        /// Honour `Range` and serve through to the end of the body.
        Whole,
        /// Honour `Range` and claim the full remaining length, then send only
        /// `sent` bytes and close: a connection dropped mid-transfer.
        Cut {
            /// How many bytes actually reach the client.
            sent: usize,
        },
        /// Ignore `Range` and answer `200` with the whole body, which is what
        /// a server or CDN with no range support does.
        IgnoreRange,
        /// Answer with a status line and no body.
        Status(&'static str),
    }

    /// A fixture server, and what was asked of it.
    struct Fixture {
        /// Where to point the client.
        url: String,
        /// The raw text of every request received, in order.
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Fixture {
        /// The `N` from each request's `Range: bytes=N-`, or `None` if it had
        /// no `Range` header at all.
        fn ranges(&self) -> Vec<Option<u64>> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| range_start(request))
                .collect()
        }
    }

    /// Reads the `N` out of a `Range: bytes=N-` request header.
    fn range_start(request: &str) -> Option<u64> {
        request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("range:"))
            .and_then(|line| line.split("bytes=").nth(1))
            .and_then(|rest| rest.trim().trim_end_matches('-').parse().ok())
    }

    /// Serves `body`, answering successive connections as `answers` says.
    fn serve_ranges(body: Vec<u8>, answers: Vec<Answer>) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::clone(&requests);

        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };

                // Drained before responding, for the reason serve_once gives.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = stream.read(&mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let start = range_start(&text);
                recorded.lock().unwrap().push(text);

                let total = body.len();
                let (header, payload) = match answer {
                    Answer::Status(status) => (
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        ),
                        Vec::new(),
                    ),
                    Answer::IgnoreRange => (
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                        ),
                        body.clone(),
                    ),
                    Answer::Whole | Answer::Cut { .. } => {
                        let from = start.map_or(0, |n| usize::try_from(n).unwrap()).min(total);
                        let rest = &body[from..];
                        let header = if start.is_some() {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                                 Content-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                                rest.len(),
                                from,
                                total - 1,
                                total
                            )
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                rest.len()
                            )
                        };
                        // Cut writes fewer bytes than the header promised, so
                        // the client sees the body end early.
                        let payload = match answer {
                            Answer::Cut { sent } => rest[..sent.min(rest.len())].to_vec(),
                            _ => rest.to_vec(),
                        };
                        (header, payload)
                    }
                };

                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });

        Fixture {
            url: format!("http://127.0.0.1:{port}/model.gguf"),
            requests,
        }
    }

    /// A body big enough to be halved, with no repeating block boundary.
    fn filler(len: usize) -> Vec<u8> {
        (0..len)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect()
    }

    /// The SHA256 of `bytes`, as the pin would record it.
    fn sha256_of(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex(&hasher.finalize())
    }

    #[tokio::test]
    async fn an_interrupted_download_resumes_from_where_it_stopped() {
        // The case the whole function exists for.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let body = filler(4096);
        let digest = sha256_of(&body);
        let fixture = serve_ranges(
            body.clone(),
            vec![Answer::Cut { sent: 2048 }, Answer::Whole],
        );
        let client = reqwest::Client::new();

        let interrupted = download_resumable(
            &client,
            &fixture.url,
            &part,
            &dest,
            &digest,
            4096,
            &mut |_| {},
        )
        .await;

        assert!(
            interrupted.is_err(),
            "the fixture closed the connection half way through"
        );
        assert_eq!(
            std::fs::metadata(&part).unwrap().len(),
            2048,
            "the part must survive a dropped connection or there is nothing to resume"
        );
        assert!(!dest.exists());

        download_resumable(
            &client,
            &fixture.url,
            &part,
            &dest,
            &digest,
            4096,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            fixture.ranges(),
            vec![None, Some(2048)],
            "the retry must ask for exactly the bytes it is missing"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            body,
            "the reassembled file is not the one that was served"
        );
        assert!(!part.exists(), "the part was renamed into place");
    }

    #[tokio::test]
    async fn progress_counts_the_bytes_already_on_disk() {
        // A resumed download reporting from zero looks like it restarted,
        // which is the one thing resuming exists to avoid.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let body = filler(4096);
        let digest = sha256_of(&body);
        let fixture = serve_ranges(
            body.clone(),
            vec![Answer::Cut { sent: 2048 }, Answer::Whole],
        );
        let client = reqwest::Client::new();

        let _ = download_resumable(
            &client,
            &fixture.url,
            &part,
            &dest,
            &digest,
            4096,
            &mut |_| {},
        )
        .await;

        let mut seen = Vec::new();
        download_resumable(
            &client,
            &fixture.url,
            &part,
            &dest,
            &digest,
            4096,
            &mut |progress| seen.push(progress.downloaded_bytes),
        )
        .await
        .unwrap();

        assert!(
            seen.first().is_some_and(|first| *first > 2048),
            "progress restarted from zero on resume: {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some(4096));
    }

    #[tokio::test]
    async fn a_server_ignoring_range_restarts_rather_than_corrupting() {
        // A 200 with the whole body answering a ranged request must truncate
        // the .part and start over. Appending would produce a file that is
        // simply wrong, failing its hash for a reason nobody could diagnose.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        std::fs::write(&part, b"ab").unwrap();
        let fixture = serve_ranges(b"abc".to_vec(), vec![Answer::IgnoreRange]);

        download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            ABC_SHA256,
            3,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            fixture.ranges(),
            vec![Some(2)],
            "a range was still asked for"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"abc",
            "the ignored range was appended to instead of replacing the part"
        );
    }

    #[tokio::test]
    async fn a_hash_mismatch_leaves_nothing_usable() {
        // Neither the .part nor the destination may survive: a leftover .part
        // would be resumed next time and fail identically, forever.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let body = b"not abc at all".to_vec();
        let fixture = serve_ranges(body.clone(), vec![Answer::Whole]);

        let error = download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            ABC_SHA256,
            u64::try_from(body.len()).unwrap(),
            &mut |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AcquireError::ChecksumMismatch { .. }));
        assert!(!error.is_retryable(), "a mismatch must not invite a retry");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert!(
            leftovers.is_empty(),
            "a file survived a failed verification: {:?}",
            leftovers
                .iter()
                .map(std::fs::DirEntry::path)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn the_destination_appears_only_after_the_hash_matches() {
        // Nothing partial may ever be mistaken for a usable model.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let body = filler(4096);
        let digest = sha256_of(&body);
        let fixture = serve_ranges(body, vec![Answer::Whole]);

        let mut ticks = 0_u32;
        let mut appeared_early = false;
        download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            &digest,
            4096,
            &mut |_| {
                ticks += 1;
                appeared_early |= dest.exists();
            },
        )
        .await
        .unwrap();

        assert!(ticks > 0, "no progress was reported at all");
        assert!(
            !appeared_early,
            "the destination existed while bytes were still arriving"
        );
        assert!(dest.is_file());
    }

    #[tokio::test]
    async fn a_size_that_disagrees_with_the_pin_is_refused_before_hashing() {
        // Cheap early rejection: a wrong size means the upload changed, and
        // saying so beats a hash mismatch the user will read as corruption.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let fixture = serve_ranges(b"abcdef".to_vec(), vec![Answer::Whole]);

        let error = download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            ABC_SHA256,
            3,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        let mut checked = false;
        if let AcquireError::SizeMismatch {
            expected_bytes,
            actual_bytes,
            ..
        } = &error
        {
            assert_eq!(*expected_bytes, 3);
            assert_eq!(*actual_bytes, 6);
            checked = true;
        }
        assert!(checked, "expected a size mismatch, got {error:?}");
        assert!(
            !error.is_retryable(),
            "the upload changed; fetching it again returns the same wrong file"
        );
        assert!(
            !part.exists(),
            "bytes were written before the size was checked"
        );
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_first_download_asks_for_no_range_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        let fixture = serve_ranges(b"abc".to_vec(), vec![Answer::Whole]);

        download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            ABC_SHA256,
            3,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            fixture.ranges(),
            vec![None],
            "a first attempt has nothing to resume and must not claim to"
        );
    }

    #[tokio::test]
    async fn a_failed_status_keeps_the_part_for_the_next_attempt() {
        // A 503 is exactly the case resuming exists for. Discarding the part
        // would throw away the gigabytes that had already arrived.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        std::fs::write(&part, filler(2048)).unwrap();
        let fixture = serve_ranges(
            filler(4096),
            vec![Answer::Status("503 Service Unavailable")],
        );

        let error = download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            &sha256_of(&filler(4096)),
            4096,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            AcquireError::HttpStatus { status: 503, .. }
        ));
        assert!(error.is_retryable());
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 2048);
    }

    #[tokio::test]
    async fn a_range_the_server_will_not_satisfy_discards_the_part() {
        // 416 means the part no longer lines up with what is being served.
        // Keeping it would ask for the same impossible range forever.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        std::fs::write(&part, filler(2048)).unwrap();
        let fixture = serve_ranges(
            filler(4096),
            vec![Answer::Status("416 Range Not Satisfiable")],
        );

        let error = download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            &sha256_of(&filler(4096)),
            4096,
            &mut |_| {},
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            AcquireError::HttpStatus { status: 416, .. }
        ));
        assert!(
            !part.exists(),
            "a part the server rejects must go, or every retry repeats it"
        );
    }

    #[tokio::test]
    async fn a_part_longer_than_the_pin_is_discarded_rather_than_trusted() {
        // Left by a previous pin, or by an append that should have truncated.
        // It cannot be a prefix of the pinned file, so there is nothing to
        // resume from and the whole thing is fetched again. Treating it as
        // complete would hand a hash failure to a user who did nothing wrong.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        let dest = dir.path().join("model.gguf");
        std::fs::write(&part, filler(9000)).unwrap();
        let fixture = serve_ranges(b"abc".to_vec(), vec![Answer::Whole]);

        download_resumable(
            &reqwest::Client::new(),
            &fixture.url,
            &part,
            &dest,
            ABC_SHA256,
            3,
            &mut |_| {},
        )
        .await
        .unwrap();

        assert_eq!(
            fixture.ranges(),
            vec![None],
            "an oversized part must not be resumed from"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
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
