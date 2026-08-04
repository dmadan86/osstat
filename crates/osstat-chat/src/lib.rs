//! Running a local llama.cpp server, and talking to it.
//!
//! `osstat-inference` acquires the runtime; this crate runs it. The split is
//! deliberate: acquisition is nearly pure code with hermetic tests, while a
//! session owns a child process and a long-lived HTTP stream.
//!
//! Nothing here reaches the webview directly. `src-tauri/src/chat.rs` adapts
//! this crate's types to Tauri commands and events, so that all egress stays in
//! Rust and ADR-012's "the webview never issues an HTTP request" holds.

#![forbid(unsafe_code)]

pub mod client;
pub mod error;
pub mod gguf;
pub mod plan;
pub mod remote;
pub mod session;
pub mod store;

pub use client::{
    ChatClient, Content, ImageUrl, Message, Part, ServerProps, StreamEvent, Timings, Usage,
};
pub use error::ChatError;
pub use gguf::{GgufNeed, ModelFile, parse, parse_prefix};
pub use plan::{DEFAULT_CONTEXT_CEILING, LaunchPlan, plan_launch};
pub use remote::{MAX_HEADER_FETCH, RemoteHeaderError, fetch_header};
pub use session::{Launch, Session, free_port, reap, start};
pub use store::{Conversation, ConversationStore, Role};
