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
