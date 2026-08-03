//! What can go wrong acquiring a runtime, as one variant per real failure.
//!
//! Each variant carries what a user needs in order to act: a path, a size, a
//! URL, the two hashes that disagreed. A single opaque "download failed" would
//! make the difference between a full disk and a tampered archive invisible,
//! and those two need very different responses from the person reading it.
//!
//! [`AcquireError::ChecksumMismatch`] is deliberately its own variant rather
//! than a kind of network error. Retrying is the right response to a dropped
//! connection and the wrong response to a hash that did not match, so the two
//! must not share a code path.

use std::path::PathBuf;

/// A failure while acquiring a runtime.
#[derive(Debug, thiserror::Error)]
pub enum AcquireError {
    /// osstat builds for a target no llama.cpp artifact is published for.
    #[error("no llama.cpp runtime is published for {os} on {arch}")]
    UnsupportedTarget {
        /// The operating system.
        os: String,
        /// The CPU architecture.
        arch: String,
    },

    /// Selection produced an identifier the pinned manifest does not contain.
    ///
    /// A test proves this cannot happen for any reachable combination, so it
    /// exists to be a legible failure if that test is ever weakened.
    #[error("runtimes.json has no artifact named {id}")]
    UnknownArtifact {
        /// The identifier that was looked up.
        id: String,
    },

    /// The download could not be started or completed.
    #[error("could not fetch {url}: {source}")]
    Network {
        /// What was being fetched.
        url: String,
        /// The underlying transport failure.
        source: reqwest::Error,
    },

    /// The server answered, but not with the artifact.
    #[error("fetching {url} returned HTTP {status}")]
    HttpStatus {
        /// What was being fetched.
        url: String,
        /// The status returned.
        status: u16,
    },

    /// There is not enough room for the archive and its extracted contents.
    #[error(
        "need about {needed_bytes} bytes free in {path} but only {available_bytes} are available"
    )]
    NotEnoughSpace {
        /// Where the runtime would be installed.
        path: PathBuf,
        /// Roughly twice the artifact size: archive and contents coexist.
        needed_bytes: u64,
        /// What the filesystem reports free.
        available_bytes: u64,
    },

    /// What arrived did not hash to the pinned value.
    ///
    /// Never retried and never overridden. See the module docs.
    #[error("{file} did not match its pinned checksum (expected {expected}, got {actual})")]
    ChecksumMismatch {
        /// The artifact filename.
        file: String,
        /// The hash `runtimes.json` pins.
        expected: String,
        /// The hash of what actually arrived.
        actual: String,
    },

    /// The file being served is not the size the pin says it should be.
    ///
    /// Its own variant rather than a kind of checksum failure: a wrong size is
    /// detectable from the response headers, before a byte is downloaded, and
    /// it means the upload changed rather than that the transfer was corrupted.
    /// Those read very differently to the person seeing the message.
    #[error("{file} is {actual_bytes} bytes but was pinned at {expected_bytes}")]
    SizeMismatch {
        /// The file name.
        file: String,
        /// The size `models.json` pins.
        expected_bytes: u64,
        /// The size actually on offer.
        actual_bytes: u64,
    },

    /// The archive could not be unpacked, or tried to escape its directory.
    #[error("could not unpack {file}: {message}")]
    Extraction {
        /// The artifact filename.
        file: String,
        /// What went wrong.
        message: String,
    },

    /// The archive verified but did not contain the server.
    ///
    /// Treated as a verification failure: the bytes were what was pinned, so
    /// the pin itself describes the wrong file.
    #[error("{file} verified but contained no llama-server executable")]
    ServerMissing {
        /// The artifact filename.
        file: String,
    },

    /// The extracted server could not be made executable.
    #[error("could not make {path} executable: {source}")]
    NotExecutable {
        /// The file that could not be marked.
        path: PathBuf,
        /// The underlying OS error.
        source: std::io::Error,
    },

    /// Any other filesystem failure, with the path that caused it.
    #[error("filesystem error at {path}: {source}")]
    Io {
        /// Where it happened.
        path: PathBuf,
        /// The underlying OS error.
        source: std::io::Error,
    },
}

impl AcquireError {
    /// A stable, user-data-free name for this variant, safe to write to a log.
    ///
    /// [`Display`](std::fmt::Display) is not: every variant here names a file,
    /// a path or a URL, which is the whole point of the messages and exactly
    /// what must not reach a log line. Logging code takes this instead.
    ///
    /// The match has no wildcard arm on purpose. A new variant must fail to
    /// compile rather than quietly logging as something it is not.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnsupportedTarget { .. } => "unsupported_target",
            Self::UnknownArtifact { .. } => "unknown_artifact",
            Self::Network { .. } => "network",
            Self::HttpStatus { .. } => "http_status",
            Self::NotEnoughSpace { .. } => "not_enough_space",
            Self::ChecksumMismatch { .. } => "checksum_mismatch",
            Self::SizeMismatch { .. } => "size_mismatch",
            Self::Extraction { .. } => "extraction",
            Self::ServerMissing { .. } => "server_missing",
            Self::NotExecutable { .. } => "not_executable",
            Self::Io { .. } => "io",
        }
    }

    /// Whether trying this again could plausibly produce a different answer.
    ///
    /// The single notion of retryability. Both callers ask it: the automatic
    /// backoff loop in `download_resumable_retrying` decides whether to wait
    /// and try again, and the UI decides whether to offer a retry control at
    /// all. Those were once two functions that disagreed — one called every
    /// HTTP status permanent while the other called every HTTP status worth a
    /// button — so a 503 could be retried by hand but never automatically, and
    /// a 404 offered a button that could not possibly work. One question gets
    /// one answer.
    ///
    /// False for [`Self::ChecksumMismatch`] above all: retrying a hash that did
    /// not match either wastes a 600 MB download or, worse, invites the user to
    /// keep trying until a tampered file slips through.
    ///
    /// The match has no wildcard arm, for the same reason [`Self::kind`] has
    /// none: a variant added later must fail to compile and be classified
    /// deliberately, rather than defaulting to retryable and quietly acquiring
    /// a four-attempt loop nobody chose for it.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            // A dropped connection, and a locked file or momentarily
            // unavailable network share. Nothing about either says the next
            // attempt will hit it too -- unlike a full disk, which is
            // `NotEnoughSpace` below and stays full.
            Self::Network { .. } | Self::Io { .. } => true,

            // The status, not the variant. This is the distinction the whole
            // function turns on: an overloaded or rate-limited server is having
            // a bad minute, while a deleted file and a refused credential
            // answer identically however many times they are asked.
            Self::HttpStatus { status, .. } => is_transient_status(*status),

            Self::UnsupportedTarget { .. }
            | Self::UnknownArtifact { .. }
            | Self::NotEnoughSpace { .. }
            | Self::ChecksumMismatch { .. }
            | Self::SizeMismatch { .. }
            | Self::Extraction { .. }
            | Self::ServerMissing { .. }
            | Self::NotExecutable { .. } => false,
        }
    }
}

/// Whether an HTTP status describes a server having a bad minute.
///
/// Enumerated rather than treating `5xx` as a block: 501 and 505 say the server
/// will never do this, and mean exactly the same thing on the fourth attempt as
/// on the first. 408 is absent because this client sets no request timeout, so
/// a server claiming one timed out is describing something other than the wait
/// a retry would change.
const fn is_transient_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_checksum_mismatch_is_never_retryable() {
        // The user-facing half of the security control.
        let error = AcquireError::ChecksumMismatch {
            file: "llama.zip".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };

        assert!(!error.is_retryable());
    }

    #[test]
    fn a_size_mismatch_is_never_retryable() {
        // The upload changed. Fetching it again returns the same wrong file.
        let error = AcquireError::SizeMismatch {
            file: "model.gguf".to_owned(),
            expected_bytes: 4_683_074_240,
            actual_bytes: 12,
        };

        assert!(!error.is_retryable());
        let message = error.to_string();
        assert!(message.contains("4683074240"), "{message}");
        assert!(message.contains("12"), "{message}");
    }

    /// An `HttpStatus` carrying `status`, which is the only field that decides
    /// retryability.
    fn status(status: u16) -> AcquireError {
        AcquireError::HttpStatus {
            url: "https://example.invalid/x".to_owned(),
            status,
        }
    }

    #[test]
    fn a_transport_failure_is_retryable() {
        assert!(status(503).is_retryable());
    }

    #[test]
    fn a_server_having_a_bad_minute_is_retryable() {
        // The four that say "ask me again": overloaded, rate-limited, and the
        // two a proxy returns when the thing behind it did not answer in time.
        for code in [429, 502, 503, 504] {
            let error = status(code);
            assert!(error.is_retryable(), "{code} should be retryable");
        }
    }

    #[test]
    fn a_status_that_will_not_change_is_not_retryable() {
        // A deleted file and a refused credential answer identically however
        // many times they are asked. 501 and 505 are here to prove the rule is
        // the status list and not "5xx": the server is saying it will never do
        // this, which the fourth attempt will not alter.
        for code in [400, 401, 403, 404, 410, 416, 500, 501, 505] {
            let error = status(code);
            assert!(!error.is_retryable(), "{code} should not be retryable");
        }
    }

    #[test]
    fn a_locked_file_is_retryable_but_a_full_disk_is_not() {
        // Both are local failures, and only one of them can resolve itself.
        let locked = AcquireError::Io {
            path: PathBuf::from("/models/model.gguf.part"),
            source: std::io::Error::other("device not ready"),
        };
        let full = AcquireError::NotEnoughSpace {
            path: PathBuf::from("/models"),
            needed_bytes: 4_683_074_240,
            available_bytes: 4_096,
        };

        assert!(locked.is_retryable());
        assert!(!full.is_retryable());
    }

    #[test]
    fn a_full_disk_is_not_retryable_because_nothing_changed() {
        let error = AcquireError::NotEnoughSpace {
            path: PathBuf::from("/tmp"),
            needed_bytes: 1_000,
            available_bytes: 10,
        };

        assert!(!error.is_retryable());
    }

    #[test]
    fn a_mismatch_names_both_hashes_so_it_can_be_checked() {
        let error = AcquireError::ChecksumMismatch {
            file: "llama.zip".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };

        let message = error.to_string();
        assert!(message.contains(&"a".repeat(64)));
        assert!(message.contains(&"b".repeat(64)));
        assert!(message.contains("llama.zip"));
    }

    #[test]
    fn a_space_failure_names_what_was_needed_and_what_there_was() {
        let error = AcquireError::NotEnoughSpace {
            path: PathBuf::from("/tmp/runtimes"),
            needed_bytes: 1_075_000_000,
            available_bytes: 4_096,
        };

        let message = error.to_string();
        assert!(message.contains("1075000000"));
        assert!(message.contains("4096"));
    }

    /// A `reqwest::Error` without a socket: an unusable URL fails in the
    /// builder, before any request is made.
    fn a_transport_error() -> reqwest::Error {
        reqwest::Client::new()
            .get("nowhere-in-particular")
            .build()
            .unwrap_err()
    }

    #[test]
    fn every_variant_has_a_distinct_kind() {
        // A copied-and-not-edited string makes two different failures look
        // identical in a log, which is worse than no log line.
        let kinds = [
            AcquireError::UnsupportedTarget {
                os: String::new(),
                arch: String::new(),
            }
            .kind(),
            AcquireError::UnknownArtifact { id: String::new() }.kind(),
            AcquireError::Network {
                url: String::new(),
                source: a_transport_error(),
            }
            .kind(),
            AcquireError::HttpStatus {
                url: String::new(),
                status: 500,
            }
            .kind(),
            AcquireError::NotEnoughSpace {
                path: PathBuf::new(),
                needed_bytes: 0,
                available_bytes: 0,
            }
            .kind(),
            AcquireError::ChecksumMismatch {
                file: String::new(),
                expected: String::new(),
                actual: String::new(),
            }
            .kind(),
            AcquireError::SizeMismatch {
                file: String::new(),
                expected_bytes: 0,
                actual_bytes: 0,
            }
            .kind(),
            AcquireError::Extraction {
                file: String::new(),
                message: String::new(),
            }
            .kind(),
            AcquireError::ServerMissing {
                file: String::new(),
            }
            .kind(),
            AcquireError::NotExecutable {
                path: PathBuf::new(),
                source: std::io::Error::other("x"),
            }
            .kind(),
            AcquireError::Io {
                path: PathBuf::new(),
                source: std::io::Error::other("x"),
            }
            .kind(),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for kind in kinds {
            assert!(!seen.contains(&kind), "duplicate kind {kind}");
            assert!(!kind.is_empty());
            seen.push(kind);
        }
    }

    #[test]
    fn a_kind_carries_no_user_data() {
        // The whole reason kind() exists: the Display of this variant contains
        // a path, and the kind must not.
        let error = AcquireError::Io {
            path: PathBuf::from("/home/someone/Library/runtimes"),
            source: std::io::Error::other("x"),
        };

        assert_eq!(error.kind(), "io");
        assert!(!error.kind().contains('/'));
    }

    #[test]
    fn a_checksum_mismatch_kind_drops_the_file_name_and_both_hashes() {
        let error = AcquireError::ChecksumMismatch {
            file: "llama.zip".to_owned(),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };

        assert_eq!(error.kind(), "checksum_mismatch");
        assert!(!error.kind().contains("llama"));
        assert!(!error.kind().contains("aaaa"));
    }
}
