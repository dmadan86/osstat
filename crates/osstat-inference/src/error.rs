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

    /// Whether this failure is worth offering a retry for.
    ///
    /// False for [`Self::ChecksumMismatch`] above all: retrying a hash that did
    /// not match either wastes a 600 MB download or, worse, invites the user to
    /// keep trying until a tampered file slips through. The UI uses this to
    /// decide whether to show the control at all.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Network { .. } | Self::HttpStatus { .. })
    }
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

    #[test]
    fn a_transport_failure_is_retryable() {
        let error = AcquireError::HttpStatus {
            url: "https://example.invalid/x".to_owned(),
            status: 503,
        };

        assert!(error.is_retryable());
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
