//! Every way running a model can fail, as one enum.
//!
//! Each variant names something the user can act on. A single opaque
//! "inference failed" would be technically accurate and useless: the difference
//! between a missing runtime, a model that is not a GGUF, and a server that
//! died holding an out-of-memory message is the whole of what a person needs.

use std::path::PathBuf;

/// A failure somewhere between opening a model file and finishing a message.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    /// No `llama-server` is installed. Acquire one in Settings first.
    #[error("no inference runtime is installed")]
    NoRuntime,

    /// The model file could not be opened at all.
    #[error("cannot read the model file at {0}")]
    ModelUnreadable(PathBuf),

    /// The file opened but is not a GGUF this crate can read.
    #[error("{file} is not a readable GGUF file: {reason}")]
    NotAGguf {
        /// The file that failed to parse.
        file: PathBuf,
        /// Which invariant it broke, for the message shown to the user.
        reason: &'static str,
    },

    /// The child process would not start. Carries the tail of its stderr.
    #[error("llama-server did not start: {0}")]
    SpawnFailed(String),

    /// Startup was cancelled before the server became healthy.
    #[error("cancelled while the model was loading")]
    NeverReady,

    /// The child exited unexpectedly. Carries the tail of its stderr, which is
    /// where an out-of-memory message appears.
    #[error("llama-server exited: {0}")]
    ServerDied(String),

    /// The response stream closed part-way through a message.
    #[error("the response stream ended before the message finished")]
    StreamBroken,

    /// A server-sent event did not parse.
    #[error("unreadable response chunk: {0}")]
    BadChunk(String),

    /// The user stopped generation.
    #[error("generation stopped")]
    Cancelled,

    /// Anything the filesystem or the socket refused.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ChatError {
    /// A stable, user-data-free name for this variant, safe to write to a log.
    ///
    /// [`Display`](std::fmt::Display) is not: `SpawnFailed` and `ServerDied`
    /// carry the tail of `llama-server`'s stderr, which names the model path it
    /// was given, and `ModelUnreadable` and `NotAGguf` carry the path outright.
    /// Logging code takes this instead.
    ///
    /// The match has no wildcard arm on purpose. A new variant must fail to
    /// compile rather than quietly logging as something it is not.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NoRuntime => "no_runtime",
            Self::ModelUnreadable(_) => "model_unreadable",
            Self::NotAGguf { .. } => "not_a_gguf",
            Self::SpawnFailed(_) => "spawn_failed",
            Self::NeverReady => "never_ready",
            Self::ServerDied(_) => "server_died",
            Self::StreamBroken => "stream_broken",
            Self::BadChunk(_) => "bad_chunk",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn every_variant_has_a_distinct_kind() {
        // A copied-and-not-edited string makes two different failures look
        // identical in a log, which is worse than no log line.
        let kinds = [
            ChatError::NoRuntime.kind(),
            ChatError::ModelUnreadable(PathBuf::new()).kind(),
            ChatError::NotAGguf {
                file: PathBuf::new(),
                reason: "",
            }
            .kind(),
            ChatError::SpawnFailed(String::new()).kind(),
            ChatError::NeverReady.kind(),
            ChatError::ServerDied(String::new()).kind(),
            ChatError::StreamBroken.kind(),
            ChatError::BadChunk(String::new()).kind(),
            ChatError::Cancelled.kind(),
            ChatError::from(std::io::Error::other("x")).kind(),
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
        let error = ChatError::ModelUnreadable("/home/someone/secret.gguf".into());

        assert_eq!(error.kind(), "model_unreadable");
        assert!(!error.kind().contains('/'));
    }

    #[test]
    fn a_spawn_failure_kind_drops_the_captured_stderr() {
        // SpawnFailed carries llama-server's stderr, which names the model
        // path it was given. Only the variant may reach a log line.
        let error = ChatError::SpawnFailed(
            "error loading model /home/someone/models/private.gguf: no such file".to_owned(),
        );

        assert_eq!(error.kind(), "spawn_failed");
        assert!(!error.kind().contains('/'));
    }
}
